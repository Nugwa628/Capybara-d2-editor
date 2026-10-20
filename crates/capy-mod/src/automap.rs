//! The in-game automap: `AutoMap.txt` maps (level type, tile code, style,
//! sequence) to a cel of `MaxiMap.dc6`. Style is a tile's main index and
//! sequence its sub index; the tile code comes from the DT1 orientation.
//!
//! This module holds the model (rules, lookup, per-map pieces), the editing
//! operations over a [`TxtDoc`] and suggestions. Presentation settings live in
//! [`style`], row import in [`import`] and the save-time checks in [`safety`].
//! Every edit changes only the rows it names; all other rows keep their bytes.

pub mod import;
pub mod safety;
pub mod style;

use std::collections::{BTreeMap, HashMap};

use d2_formats::ds1::Ds1;
use d2_formats::txt::TxtDoc;

use crate::doc_util::{data_rows, parse_num, text};

/// Conventional path of the table inside a mod or the game.
pub const AUTOMAP_TXT: &str = "data/global/excel/AutoMap.txt";
/// The sprite sheet the cels index.
pub const AUTOMAP_DC6: &str = "data/global/ui/automap/MaxiMap.dc6";

/// `TileName` for each DT1 orientation (0 floor ... 19 lower walls).
pub const AUTOMAP_CODES: [&str; 20] = [
    "fl", "wl", "wr", "wtlr", "wtll", "wtr", "wbl", "wbr", "wld", "wrd", "wle", "wre", "co", "sh",
    "tr", "rf", "ld", "lr", "lf", "ls",
];

/// Plain-language name of a tile code.
pub fn code_name(code: &str) -> &'static str {
    match code {
        "fl" => "Floor",
        "wl" => "Left wall",
        "wr" => "Right wall",
        "wtlr" => "North corner, right part",
        "wtll" => "North corner, left part",
        "wtr" => "East corner",
        "wbl" => "West corner",
        "wbr" => "South corner",
        "wld" => "Left door",
        "wrd" => "Right door",
        "wle" => "Left wall end",
        "wre" => "Right wall end",
        "co" => "Column / pillar",
        "sh" => "Shadow",
        "tr" => "Tree / object",
        "rf" => "Roof",
        "ld" => "Lower wall, left",
        "lr" => "Lower wall, right",
        "lf" => "Lower wall, north",
        "ls" => "Lower wall, south",
        _ => "Unknown",
    }
}

/// The automap level names the game knows, by level type id. `AutoMap.txt`
/// rows must use one of these (the game stops at start-up otherwise); mods like
/// Project Diablo 2 also read a level type's Id ("47") for types they add.
pub const GAME_AUTOMAP_LEVELS: [&str; 36] = [
    "None",
    "1 Town",
    "1 Wilderness",
    "1 Cave",
    "1 Crypt",
    "1 Monestary",
    "1 Courtyard",
    "1 Barracks",
    "1 Jail",
    "1 Cathedral",
    "1 Catacombs",
    "1 Tristram",
    "2 Town",
    "2 Sewer",
    "2 Harem",
    "2 Basement",
    "2 Desert",
    "2 Tomb",
    "2 Lair",
    "2 Arcane",
    "3 Town",
    "3 Jungle",
    "3 Kurast",
    "3 Spider",
    "3 Dungeon",
    "3 Sewer",
    "4 Town",
    "4 Mesa",
    "4 Lava",
    "5 Town",
    "5 Siege",
    "5 Barricade",
    "5 Temple",
    "5 Ice",
    "5 Baal",
    "5 Lava",
];

/// Marker written in the first `Type` column of rows this crate adds.
pub const ROW_LABEL: &str = "Capybara";

/// Failures of automap edits.
#[derive(Debug, thiserror::Error)]
pub enum AutomapError {
    /// A cel number or orientation the table cannot hold.
    #[error("{0}")]
    Invalid(String),
    /// The tile's piece comes from a wildcard rule that cannot be split safely.
    #[error(
        "this tile gets its automap piece from a wildcard rule for every {what} of {code} tiles in {level}, which can't be split safely; give the cell a tile of its own with no piece, or change that rule directly"
    )]
    Wildcard {
        /// `style` or `sequence`.
        what: &'static str,
        /// Tile code.
        code: String,
        /// Level name.
        level: String,
    },
    /// The table cannot be edited (a column is missing).
    #[error("{0}")]
    Table(String),
    /// A write to the document failed.
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
}

/// One cel of a rule (the `-1` terminator and everything after it dropped).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleCel {
    /// Cel (frame) number in `MaxiMap.dc6`.
    pub cel: i32,
    /// The row's `Type` label for it (may be empty).
    pub label: String,
}

/// One `AutoMap.txt` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Row index in the [`TxtDoc`] (row r is file line r+1).
    pub row: usize,
    /// `LevelName` as written ("1 Town", or a level type number).
    pub level: String,
    /// `TileName`, lower case.
    pub code: String,
    /// `Style` (-1 = every style).
    pub style: i32,
    /// `StartSequence` (-1 = every sequence).
    pub start: i32,
    /// `EndSequence` (-1 = just the start).
    pub end: i32,
    /// `Cel1..Cel4` up to the first -1.
    pub cels: Vec<RuleCel>,
}

impl Rule {
    /// Whether the sequence range covers `sub` (start -1 = every sequence,
    /// end -1 = just the start).
    pub fn covers(&self, sub: i32) -> bool {
        if self.start < 0 {
            return true;
        }
        let end = if self.end < 0 { self.start } else { self.end };
        sub >= self.start && sub <= end
    }

    /// Human description such as `1 Town - fl - style 0 - seq 1-46`.
    pub fn describe(&self) -> String {
        let seq = if self.start < 0 {
            "any".to_string()
        } else if self.end < 0 || self.end == self.start {
            self.start.to_string()
        } else {
            format!("{}-{}", self.start, self.end)
        };
        format!(
            "{} - {} - style {} - seq {}",
            self.level, self.code, self.style, seq
        )
    }
}

type RuleKey = (String, String, i32);

fn key(level: &str, code: &str, style: i32) -> RuleKey {
    (level.trim().to_lowercase(), code.to_string(), style)
}

/// `(type column, cel column)` pairs for `Cel1..Cel4`; the type column is the
/// one just before a cel when it is named `TypeN`.
pub fn cel_columns(doc: &TxtDoc) -> Vec<(Option<usize>, usize)> {
    let headers = doc.headers();
    (1..=4)
        .filter_map(|n| {
            let want = format!("cel{n}");
            let cel = headers
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(&want))?;
            let ty = (cel > 0 && is_type_header(&headers[cel - 1])).then(|| cel - 1);
            Some((ty, cel))
        })
        .collect()
}

fn is_type_header(h: &str) -> bool {
    let h = h.trim().to_ascii_lowercase();
    h.strip_prefix("type")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// A parsed `AutoMap.txt` with the game's first-match lookup.
#[derive(Clone, Debug, Default)]
pub struct AutomapTable {
    /// Rows with a level and a tile code, in file order.
    pub rules: Vec<Rule>,
    /// Level names as written, in first-seen order.
    pub levels: Vec<String>,
    by_key: HashMap<RuleKey, Vec<usize>>,
}

impl AutomapTable {
    /// Reads the rules of `doc`. Columns are located by name, falling back to
    /// the standard positions; blank and label-only rows are skipped.
    pub fn parse(doc: &TxtDoc) -> AutomapTable {
        let col = |name: &str, default: usize| doc.column(name).unwrap_or(default);
        let (cl, cc, cs, c0, c1) = (
            col("LevelName", 0),
            col("TileName", 1),
            col("Style", 2),
            col("StartSequence", 3),
            col("EndSequence", 4),
        );
        let cels = cel_columns(doc);
        let mut t = AutomapTable::default();
        for row in 0..doc.row_count() {
            let level = text(doc, row, Some(cl));
            let code = text(doc, row, Some(cc)).to_lowercase();
            if level.is_empty() || code.is_empty() {
                continue;
            }
            if !t.levels.contains(&level) {
                t.levels.push(level.clone());
            }
            let mut list = Vec::new();
            for &(ty, cel) in &cels {
                let raw = text(doc, row, Some(cel));
                let Ok(v) = raw.parse::<f64>() else { continue };
                if v == -1.0 {
                    break; // the engine stops at the first sentinel
                }
                if v >= 0.0 && v.is_finite() {
                    list.push(RuleCel {
                        cel: v as i32,
                        label: text(doc, row, ty),
                    });
                }
            }
            let rule = Rule {
                row,
                level: level.clone(),
                code: code.clone(),
                style: parse_num(&text(doc, row, Some(cs))) as i32,
                start: parse_num(&text(doc, row, Some(c0))) as i32,
                end: parse_num(&text(doc, row, Some(c1))) as i32,
                cels: list,
            };
            t.by_key
                .entry(key(&level, &code, rule.style))
                .or_default()
                .push(t.rules.len());
            t.rules.push(rule);
        }
        t
    }

    /// Rules for a level/code/style, in file order.
    pub fn rules_for(&self, level: &str, code: &str, style: i32) -> Vec<&Rule> {
        self.by_key
            .get(&key(level, code, style))
            .map(|v| v.iter().map(|&i| &self.rules[i]).collect())
            .unwrap_or_default()
    }

    /// The first rule for a tile (the game's lookup), or `None` when the tile
    /// has none and so does not show on the automap.
    pub fn find_rule(&self, level: &str, orientation: usize, main: i32, sub: i32) -> Option<&Rule> {
        let code = AUTOMAP_CODES.get(orientation)?;
        self.rules_for(level, code, main)
            .into_iter()
            .find(|r| r.covers(sub))
    }

    /// Whether this table names level types by number (a mod such as PD2).
    pub fn reads_level_numbers(&self) -> bool {
        self.levels.iter().any(|l| is_number(l))
    }

    /// Level names the game (and a mod that reads numbers) would not accept.
    pub fn unknown_levels(&self) -> Vec<String> {
        let numbers = self.reads_level_numbers();
        self.levels
            .iter()
            .filter(|l| !GAME_AUTOMAP_LEVELS.contains(&l.trim()) && !(is_number(l) && numbers))
            .cloned()
            .collect()
    }

    /// The `LevelName` this table uses for a level type: the game's own name
    /// for its types, the Id of a type a mod added (when the mod reads
    /// numbers), else the exact name, the vanilla short form ("Act 1 - Town"
    /// is "1 Town") or a prefix match.
    pub fn level_for(
        &self,
        lvl_type_name: Option<&str>,
        act: Option<i32>,
        lvl_type_id: Option<i32>,
    ) -> Option<String> {
        if let Some(id) = lvl_type_id.filter(|&i| i > 0) {
            let id_text = id.to_string();
            if let Some(l) = self.levels.iter().find(|l| l.trim() == id_text) {
                return Some(l.clone());
            }
            let idx = id as usize;
            if idx >= GAME_AUTOMAP_LEVELS.len() {
                return self.reads_level_numbers().then_some(id_text);
            }
            let own = GAME_AUTOMAP_LEVELS[idx];
            if self.levels.iter().any(|l| l == own) {
                return Some(own.to_string());
            }
        }
        let name = lvl_type_name?;
        let lower = name.trim().to_lowercase();
        if let Some(l) = self
            .levels
            .iter()
            .find(|l| l.trim().to_lowercase() == lower)
        {
            return Some(l.clone());
        }
        let want = match act_short_form(name) {
            Some(s) => s,
            None => name.to_lowercase().trim().to_string(),
        };
        let lc = |l: &String| l.to_lowercase();
        if let Some(l) = self.levels.iter().find(|l| lc(l) == want) {
            return Some(l.clone());
        }
        if let Some(l) = self
            .levels
            .iter()
            .find(|l| want.starts_with(&lc(l)) || lc(l).starts_with(&want))
        {
            return Some(l.clone());
        }
        let word: String = want.split(' ').take(2).collect::<Vec<_>>().join(" ");
        if let Some(l) = self.levels.iter().find(|l| lc(l).starts_with(&word)) {
            return Some(l.clone());
        }
        let act = act.filter(|&a| a != 0)?;
        let prefix = format!("{act} ");
        self.levels.iter().find(|l| l.starts_with(&prefix)).cloned()
    }
}

fn is_number(l: &str) -> bool {
    let l = l.trim();
    !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit())
}

/// "Act 1 - Town" -> "1 town".
fn act_short_form(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find("act") {
        let rest = lower[from + p + 3..].trim_start();
        let mut chars = rest.chars();
        if let Some(d) = chars.next().filter(char::is_ascii_digit) {
            let after = chars.as_str().trim_start();
            if let Some(tail) = after.strip_prefix('-') {
                let tail = tail.trim_start();
                if !tail.is_empty() {
                    return Some(format!("{d} {tail}").trim().to_string());
                }
            }
        }
        from += p + 3;
    }
    None
}

/// Which DS1 layer kind a piece comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceLayer {
    /// A floor layer.
    Floor,
    /// A wall layer.
    Wall,
}

/// Identity of a tile for automap purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    /// DT1 orientation.
    pub orientation: usize,
    /// Main index (the style).
    pub main: i32,
    /// Sub index (the sequence).
    pub sub: i32,
}

/// One visible floor or wall tile of a map with its automap rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Cell column.
    pub x: u32,
    /// Cell row.
    pub y: u32,
    /// The tile.
    pub tile: TileId,
    /// Row of the matching rule, if any.
    pub rule_row: Option<usize>,
    /// The cel drawn (random among Cel1..4, stable per cell), if the tile has a rule.
    pub cel: Option<i32>,
    /// Floor or wall.
    pub layer: PieceLayer,
    /// Layer number within its kind.
    pub layer_index: usize,
    /// True when `cel` comes from an unsaved suggestion.
    pub suggested: bool,
}

/// Every visible floor and wall tile of the map with its rule and chosen cel.
/// Hidden and empty cells and special markers (orientation 10/11) are skipped.
pub fn pieces(ds1: &Ds1, table: &AutomapTable, level: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut add = |cell: d2_formats::ds1::Cell, orientation: usize, x: u32, y: u32, layer, li| {
        if cell.is_empty() || cell.hidden() {
            return;
        }
        let tile = TileId {
            orientation,
            main: i32::from(cell.main_index()),
            sub: i32::from(cell.sub_index()),
        };
        let rule = table.find_rule(level, orientation, tile.main, tile.sub);
        let cel = rule.and_then(|r| {
            (!r.cels.is_empty())
                .then(|| r.cels[(x as usize * 7 + y as usize * 13) % r.cels.len()].cel)
        });
        out.push(Piece {
            x,
            y,
            tile,
            rule_row: rule.map(|r| r.row),
            cel,
            layer,
            layer_index: li,
            suggested: false,
        });
    };
    for y in 0..ds1.height {
        for x in 0..ds1.width {
            for (li, f) in ds1.floors.iter().enumerate() {
                if let Some(c) = f.get(x, y) {
                    add(c, 0, x, y, PieceLayer::Floor, li);
                }
            }
            for (li, w) in ds1.walls.iter().enumerate() {
                let Some(c) = w.tiles.get(x, y) else { continue };
                let Some(o) = ds1.orientation(li, x, y) else {
                    continue;
                };
                if o == 10 || o == 11 {
                    continue;
                }
                add(c, usize::from(o), x, y, PieceLayer::Wall, li);
            }
        }
    }
    out
}

/// One edit made to the document, in the order made. Row numbers are those of
/// the document at that moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A new row was inserted at `row`.
    RowInserted {
        /// Row index.
        row: usize,
    },
    /// The row at `row` was removed; `cells` is its old content.
    RowDeleted {
        /// Row index.
        row: usize,
        /// The old cells.
        cells: Vec<String>,
    },
    /// One cell changed.
    CellSet {
        /// Row index.
        row: usize,
        /// Column header.
        column: String,
        /// Old value.
        old: String,
        /// New value.
        new: String,
    },
    /// A column was added to the header.
    ColumnAdded {
        /// The new header.
        name: String,
    },
}

/// What an edit did: the changes and a one-line summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditSummary {
    /// The changes made (empty when the table already had this content).
    pub changes: Vec<Change>,
    /// One-line description for the change list.
    pub summary: String,
}

/// How far an edit reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// This style and sequence only.
    Sequence,
    /// Every sequence of the style.
    Style,
}

fn row_cells(doc: &TxtDoc, row: usize) -> Vec<String> {
    doc.row_cells(row)
        .unwrap_or_default()
        .iter()
        .map(|c| c.iter().map(|&b| b as char).collect())
        .collect()
}

fn complete_columns(doc: &mut TxtDoc, changes: &mut Vec<Change>) -> Result<(), AutomapError> {
    for n in 1..=4 {
        let want = format!("Cel{n}");
        if doc.column(&want).is_some() {
            continue;
        }
        let at = doc.add_column(&want, "")?;
        changes.push(Change::ColumnAdded { name: want });
        for row in 0..doc.row_count() {
            if !text(doc, row, Some(1)).is_empty() {
                doc.set_cell(row, at, "-1")?;
            }
        }
    }
    Ok(())
}

enum Action {
    Delete,
    TrimEnd(i32),
    TrimStart(i32),
    Split(i32, i32),
}

/// Removes the rules that give a tile its piece. Hiding is done by removing
/// matches, never by writing a row with no valid cel (the game's loader halts
/// on `Cel1 = -1`). A range covering the tile is split around it.
fn remove_matches(
    doc: &mut TxtDoc,
    level: &str,
    code: &str,
    style: i32,
    sub: i32,
    scope: Scope,
) -> Result<EditSummary, AutomapError> {
    let mut actions: Vec<(usize, Action)> = Vec::new();
    for row in 0..doc.row_count() {
        if !text(doc, row, Some(0)).eq_ignore_ascii_case(level)
            || !text(doc, row, Some(1)).eq_ignore_ascii_case(code)
        {
            continue;
        }
        let rs = parse_num(&text(doc, row, Some(2))) as i32;
        if rs != style && rs != -1 {
            continue;
        }
        let start = parse_num(&text(doc, row, Some(3))) as i32;
        let e = parse_num(&text(doc, row, Some(4))) as i32;
        let end = if e < 0 { start } else { e };
        if scope == Scope::Sequence && start >= 0 && (sub < start || sub > end) {
            continue;
        }
        if rs == -1 || (scope == Scope::Sequence && start < 0) {
            return Err(AutomapError::Wildcard {
                what: if rs == -1 { "style" } else { "sequence" },
                code: code.to_string(),
                level: level.to_string(),
            });
        }
        actions.push((
            row,
            match (scope, start < sub, end > sub) {
                (Scope::Style, _, _) | (_, false, false) => Action::Delete,
                (_, true, true) => Action::Split(sub - 1, sub + 1),
                (_, true, false) => Action::TrimEnd(sub - 1),
                (_, false, true) => Action::TrimStart(sub + 1),
            },
        ));
    }
    let mut changes = Vec::new();
    let removed = actions.len();
    // Bottom-up so earlier row numbers stay valid.
    for (row, action) in actions.into_iter().rev() {
        match action {
            Action::Delete => {
                let cells = row_cells(doc, row);
                doc.delete_row(row)?;
                changes.push(Change::RowDeleted { row, cells });
            }
            Action::TrimEnd(v) => {
                set_change(doc, row, "EndSequence", 4, &v.to_string(), &mut changes)?
            }
            Action::TrimStart(v) => {
                set_change(doc, row, "StartSequence", 3, &v.to_string(), &mut changes)?
            }
            Action::Split(left_end, right_start) => {
                doc.duplicate_row(row)?;
                changes.push(Change::RowInserted { row: row + 1 });
                set_change(
                    doc,
                    row + 1,
                    "StartSequence",
                    3,
                    &right_start.to_string(),
                    &mut changes,
                )?;
                set_change(
                    doc,
                    row,
                    "EndSequence",
                    4,
                    &left_end.to_string(),
                    &mut changes,
                )?;
            }
        }
    }
    Ok(EditSummary {
        changes,
        summary: format!(
            "Cleared automap piece ({removed} matching rule{} removed or narrowed).",
            if removed == 1 { "" } else { "s" }
        ),
    })
}

fn set_change(
    doc: &mut TxtDoc,
    row: usize,
    name: &str,
    fallback: usize,
    value: &str,
    changes: &mut Vec<Change>,
) -> Result<(), AutomapError> {
    let col = doc.column(name).unwrap_or(fallback);
    let old = doc.cell(row, col).unwrap_or_default();
    if old.trim() == value {
        return Ok(());
    }
    doc.set_cell(row, col, value)?;
    changes.push(Change::CellSet {
        row,
        column: name.to_string(),
        old,
        new: value.to_string(),
    });
    Ok(())
}

/// Sets the automap cel for one tile (`Scope::Sequence`: this style and
/// sequence only) or for every sequence of its style (`Scope::Style`).
///
/// A negative `cel` clears the piece by removing the matching rules (ranges
/// are split around the tile); a wildcard rule cannot be split safely and is
/// an error. Otherwise an existing row covering exactly that is updated, and
/// else a new row goes in front of the other rows of the same level, code and
/// style so the game's first-match lookup finds it. Applying the same edit
/// twice changes nothing the second time.
pub fn set_cel(
    doc: &mut TxtDoc,
    level: &str,
    orientation: usize,
    style: i32,
    sub: i32,
    cel: i32,
    scope: Scope,
) -> Result<EditSummary, AutomapError> {
    let code = *AUTOMAP_CODES.get(orientation).ok_or_else(|| {
        AutomapError::Invalid(format!("orientation {orientation} has no automap code"))
    })?;
    if cel < 0 {
        return remove_matches(doc, level, code, style, sub, scope);
    }
    let mut changes = Vec::new();
    complete_columns(doc, &mut changes)?;
    let table = AutomapTable::parse(doc);
    let rules = table.rules_for(level, code, style);
    let cols = cel_columns(doc);
    let Some(&(first_type, first_cel)) = cols.first() else {
        return Err(AutomapError::Table("AutoMap.txt has no Cel1 column".into()));
    };
    let (start, end) = if scope == Scope::Sequence {
        (sub, sub)
    } else {
        (-1, -1)
    };
    let what = format!(
        "{level} {code} style {style} {} -> cel {cel}",
        if scope == Scope::Sequence {
            format!("seq {sub}")
        } else {
            "all sequences".into()
        }
    );
    let exact = rules.iter().find(|r| match scope {
        Scope::Sequence => r.start == sub && (r.end == sub || r.end < 0),
        Scope::Style => r.start < 0,
    });
    if let Some(r) = exact {
        let row = r.row;
        let name_of = |c: usize| doc.headers().get(c).cloned().unwrap_or_default();
        let (first_name, others): (String, Vec<(usize, String)>) = (
            name_of(first_cel),
            cols[1..].iter().map(|&(_, c)| (c, name_of(c))).collect(),
        );
        if doc.cell(row, first_cel).unwrap_or_default().trim() != cel.to_string() {
            let old = doc.cell(row, first_cel).unwrap_or_default();
            doc.set_cell(row, first_cel, &cel.to_string())?;
            changes.push(Change::CellSet {
                row,
                column: first_name,
                old,
                new: cel.to_string(),
            });
        }
        for (c, name) in others {
            let old = doc.cell(row, c).unwrap_or_default();
            if old.trim() != "-1" {
                doc.set_cell(row, c, "-1")?;
                changes.push(Change::CellSet {
                    row,
                    column: name,
                    old,
                    new: "-1".into(),
                });
            }
        }
        return Ok(EditSummary {
            changes,
            summary: format!("AutoMap.txt: {what} (updated row)"),
        });
    }
    let width = doc.headers().len();
    let mut line = vec![String::new(); width];
    line[0] = level.to_string();
    line[1] = code.to_string();
    line[2] = style.to_string();
    line[3] = start.to_string();
    line[4] = end.to_string();
    if let Some(t) = first_type {
        line[t] = ROW_LABEL.to_string();
    }
    line[first_cel] = cel.to_string();
    for &(_, c) in &cols[1..] {
        line[c] = "-1".to_string();
    }
    let at = if let Some(min) = rules.iter().map(|r| r.row).min() {
        min
    } else {
        let level_rows: Vec<usize> = (0..doc.row_count())
            .filter(|&r| text(doc, r, Some(0)).eq_ignore_ascii_case(level))
            .collect();
        match level_rows.last() {
            Some(&last) => last + 1,
            None => {
                let mut at = doc.row_count();
                while at > 0 && row_is_blank(doc, at - 1) {
                    at -= 1;
                }
                at
            }
        }
    };
    let refs: Vec<&str> = line.iter().map(String::as_str).collect();
    doc.insert_row(at, &refs)?;
    changes.push(Change::RowInserted { row: at });
    Ok(EditSummary {
        changes,
        summary: format!("AutoMap.txt: {what} (new row)"),
    })
}

fn row_is_blank(doc: &TxtDoc, row: usize) -> bool {
    doc.row_cells(row)
        .is_none_or(|cs| cs.iter().all(|c| c.iter().all(u8::is_ascii_whitespace)))
}

/// Gives each listed tile its own rule (`Scope::Sequence`) pointing at `cel`.
/// Use it after tiles were isolated under fresh ids (see [`ClearPlan`]).
pub fn set_cel_for_tiles(
    doc: &mut TxtDoc,
    level: &str,
    tiles: &[TileId],
    cel: i32,
) -> Result<EditSummary, AutomapError> {
    let mut all = EditSummary::default();
    for t in tiles {
        let s = set_cel(
            doc,
            level,
            t.orientation,
            t.main,
            t.sub,
            cel,
            Scope::Sequence,
        )?;
        all.changes.extend(s.changes);
    }
    all.summary = format!("AutoMap.txt: {} tile(s) -> cel {cel}", tiles.len());
    Ok(all)
}

/// A rectangle of cells (inclusive corners).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    /// Left column.
    pub x0: u32,
    /// Top row.
    pub y0: u32,
    /// Right column.
    pub x1: u32,
    /// Bottom row.
    pub y1: u32,
}

impl CellRect {
    /// Whether the cell is inside.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// What clearing the pieces in an area involves. A rule belongs to a tile,
/// not to a cell, so a tile that is also used outside the area (or hidden
/// elsewhere) must be given a fresh id for the cells being cleared first;
/// that needs DT1 and DS1 edits, which the caller makes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClearPlan {
    /// Tiles in the area that currently show a piece.
    pub tiles: Vec<TileId>,
    /// The subset also placed outside the area (these need isolating first).
    pub shared_elsewhere: Vec<TileId>,
}

/// The tiles to clear for the visible pieces inside `area`.
pub fn plan_clear(all: &[Piece], area: CellRect) -> ClearPlan {
    let mut inside: BTreeMap<TileId, ()> = BTreeMap::new();
    for p in all {
        if p.cel.is_some() && area.contains(p.x, p.y) {
            inside.insert(p.tile, ());
        }
    }
    let tiles: Vec<TileId> = inside.keys().copied().collect();
    let shared_elsewhere = tiles
        .iter()
        .copied()
        .filter(|t| all.iter().any(|p| p.tile == *t && !area.contains(p.x, p.y)))
        .collect();
    ClearPlan {
        tiles,
        shared_elsewhere,
    }
}

/// Removes the rules of the given tiles (their pieces leave the automap).
/// Fails with [`AutomapError::Wildcard`] before changing anything if a tile's
/// piece comes from a wildcard rule.
pub fn clear_tiles(
    doc: &mut TxtDoc,
    level: &str,
    tiles: &[TileId],
) -> Result<EditSummary, AutomapError> {
    let backup = doc.clone();
    let mut all = EditSummary::default();
    for t in tiles {
        let code = *AUTOMAP_CODES.get(t.orientation).ok_or_else(|| {
            AutomapError::Invalid(format!("orientation {} has no automap code", t.orientation))
        })?;
        match remove_matches(doc, level, code, t.main, t.sub, Scope::Sequence) {
            Ok(s) => all.changes.extend(s.changes),
            Err(e) => {
                *doc = backup;
                return Err(e);
            }
        }
    }
    all.summary = format!("Cleared {} automap piece(s) in {level}", tiles.len());
    Ok(all)
}

/// A suggested piece for tiles with no rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// Tile code.
    pub code: &'static str,
    /// DT1 orientation.
    pub orientation: usize,
    /// Style (main index).
    pub style: i32,
    /// Sequences (sub indices) of the style without an entry.
    pub seqs: Vec<i32>,
    /// Tiles on the map this covers.
    pub count: usize,
    /// The suggested cel.
    pub cel: i32,
}

type CelCounts = Vec<(i32, usize)>;

fn bump(m: &mut CelCounts, cel: i32) {
    match m.iter_mut().find(|(c, _)| *c == cel) {
        Some(e) => e.1 += 1,
        None => m.push((cel, 1)),
    }
}

fn best(m: &CelCounts) -> Option<i32> {
    let mut best: Option<(i32, usize)> = None;
    for &(c, n) in m {
        if best.is_none_or(|(_, bn)| n > bn) {
            best = Some((c, n));
        }
    }
    best.map(|b| b.0)
}

/// The most used cel per tile code: first among this level's rules, then the
/// same act's levels, then (for levels without an act, or when asked) the
/// whole table.
pub fn usual_cels(t: &AutomapTable, level: &str, across_acts: bool) -> HashMap<String, i32> {
    let count = |filter: &dyn Fn(&Rule) -> bool| {
        let mut by_code: Vec<(String, CelCounts)> = Vec::new();
        for r in t.rules.iter().filter(|r| filter(r)) {
            if r.cels.is_empty() {
                continue;
            }
            let i = by_code
                .iter()
                .position(|(c, _)| *c == r.code)
                .unwrap_or_else(|| {
                    by_code.push((r.code.clone(), Vec::new()));
                    by_code.len() - 1
                });
            for c in &r.cels {
                bump(&mut by_code[i].1, c.cel);
            }
        }
        by_code
            .into_iter()
            .filter_map(|(code, m)| best(&m).map(|b| (code, b)))
            .collect::<HashMap<_, _>>()
    };
    let mut own = count(&|r| r.level == level);
    let act = level
        .chars()
        .next()
        .filter(char::is_ascii_digit)
        .filter(|_| level.as_bytes().get(1) == Some(&b' '));
    if let Some(a) = act {
        let prefix = format!("{a} ");
        for (code, cel) in count(&|r| r.level.starts_with(&prefix)) {
            own.entry(code).or_insert(cel);
        }
    }
    if act.is_none() || across_acts {
        for (code, cel) in count(&|_| true) {
            own.entry(code).or_insert(cel);
        }
    }
    own
}

/// Suggestions for the tiles of the map that have no automap entry (walls
/// always, floors when `floors`). Candidates come from the closest source
/// that has any (this level's pieces for the same code and style, then for
/// the same code, then the same act's levels, then for numbered level types
/// any level); the most used one wins. Colour-based look-alike matching is
/// left to the caller (not part of this model).
pub fn suggest(t: &AutomapTable, level: &str, all: &[Piece], floors: bool) -> Vec<Suggestion> {
    let act = level
        .chars()
        .next()
        .filter(char::is_ascii_digit)
        .filter(|_| level.as_bytes().get(1) == Some(&b' '));
    let use_of = |filter: &dyn Fn(&Rule) -> bool| {
        let mut by_code: HashMap<String, CelCounts> = HashMap::new();
        let mut by_style: HashMap<(String, i32), CelCounts> = HashMap::new();
        for r in t.rules.iter().filter(|r| filter(r)) {
            for c in &r.cels {
                bump(by_code.entry(r.code.clone()).or_default(), c.cel);
                bump(
                    by_style.entry((r.code.clone(), r.style)).or_default(),
                    c.cel,
                );
            }
        }
        (by_code, by_style)
    };
    let own = use_of(&|r| r.level == level);
    let same_act = act.map(|a| {
        let p = format!("{a} ");
        use_of(&move |r| r.level.starts_with(&p))
    });
    let any = act.is_none().then(|| use_of(&|_| true));
    let mut groups: BTreeMap<(usize, i32), (Vec<i32>, usize)> = BTreeMap::new();
    for p in all {
        if p.rule_row.is_some() || (p.layer == PieceLayer::Floor && !floors) {
            continue;
        }
        if p.layer == PieceLayer::Floor && p.tile.orientation != 0 {
            continue;
        }
        let g = groups.entry((p.tile.orientation, p.tile.main)).or_default();
        if !g.0.contains(&p.tile.sub) {
            g.0.push(p.tile.sub);
        }
        g.1 += 1;
    }
    let mut out = Vec::new();
    for ((orientation, style), (mut seqs, count)) in groups {
        let Some(code) = AUTOMAP_CODES.get(orientation).copied() else {
            continue;
        };
        let by_style = (code.to_string(), style);
        let counts = own
            .1
            .get(&by_style)
            .or_else(|| own.0.get(code))
            .or_else(|| same_act.as_ref().and_then(|s| s.0.get(code)))
            .or_else(|| any.as_ref().and_then(|s| s.0.get(code)));
        let Some(cel) = counts.and_then(best) else {
            continue;
        };
        seqs.sort_unstable();
        out.push(Suggestion {
            code,
            orientation,
            style,
            seqs,
            count,
            cel,
        });
    }
    out
}

/// Pieces with the suggestions filled in (for previewing them).
pub fn with_suggestions(pieces: &[Piece], suggestions: &[Suggestion]) -> Vec<Piece> {
    pieces
        .iter()
        .map(|p| {
            if p.rule_row.is_some() {
                return p.clone();
            }
            match suggestions.iter().find(|s| {
                s.orientation == p.tile.orientation
                    && s.style == p.tile.main
                    && s.seqs.contains(&p.tile.sub)
            }) {
                Some(s) => Piece {
                    cel: Some(s.cel),
                    suggested: true,
                    ..p.clone()
                },
                None => p.clone(),
            }
        })
        .collect()
}

/// Writes the suggestions as rules (one per sequence). Idempotent.
pub fn apply_suggestions(
    doc: &mut TxtDoc,
    level: &str,
    suggestions: &[Suggestion],
) -> Result<EditSummary, AutomapError> {
    let mut all = EditSummary::default();
    let mut n = 0;
    for s in suggestions {
        for &sub in &s.seqs {
            let e = set_cel(
                doc,
                level,
                s.orientation,
                s.style,
                sub,
                s.cel,
                Scope::Sequence,
            )?;
            all.changes.extend(e.changes);
            n += 1;
        }
    }
    all.summary = format!("AutoMap.txt: {n} suggested piece(s) for {level}");
    Ok(all)
}

/// Whether the table has any rows for `level`.
pub fn has_level_rows(doc: &TxtDoc, level: &str) -> bool {
    data_rows(doc)
        .into_iter()
        .any(|r| text(doc, r, Some(0)) == level)
}
