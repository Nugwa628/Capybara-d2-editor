//! The compatibility checker: the problems that stop a map or a mod's level
//! tables from loading or playing correctly in the game, as [`Finding`]s.
//!
//! Everything is a pure function over data the caller passes in (the parsed
//! tables, the DS1, what the tile libraries provide); nothing here reads
//! files. A finding may carry one-click [`Fix`]es expressed as [`Edit`]s.
//! Every edit states an absolute end state (set this cell to this value, remove
//! these rows, clear these cells), so applying a fix twice is the same as
//! applying it once.
//!
//! [`check_map`] runs the map checks; [`tables::verify_in_game`] holds the
//! table rules (the ones the game's level builder and table loaders follow).

pub mod rename;
pub mod render_checks;
pub mod tables;

use std::collections::{HashMap, HashSet};

use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::TileKey;
use d2_formats::txt::TxtDoc;
use serde::{Deserialize, Serialize};

use crate::automap::{self, AutomapTable, safety};
use crate::doc_util::{data_rows, get, num};

pub use tables::{
    AUTOMAP_MAX_DIFF, AUTOMAP_MAX_SUM, MAX_LAYER, MAX_LEVEL_STRING, MAX_TILE_PATH, TOWNS,
    automap_fits, free_offset, level_act, norm_path, verify_in_game,
};

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// The game crashes or the map cannot load.
    Error,
    /// Wrong behaviour or a likely mistake.
    Warning,
    /// Worth knowing.
    Info,
    /// A check that passed (reported so the UI can show it).
    Ok,
}

/// Where in the data a finding belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Area {
    /// Tile libraries and placed tiles.
    Tiles,
    /// Excel tables.
    Tables,
    /// The level the map builds.
    Level,
    /// Objects on the map.
    Objects,
    /// Map-wide properties (markers, automap).
    Map,
}

/// Tables the checker reads and its fixes edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableName {
    /// `LvlPrest.txt`.
    LvlPrest,
    /// `Levels.txt`.
    Levels,
    /// `LvlTypes.txt`.
    LvlTypes,
    /// `AutoMap.txt`.
    AutoMap,
}

impl TableName {
    /// File name, e.g. `LvlPrest.txt`.
    pub fn file(self) -> &'static str {
        match self {
            TableName::LvlPrest => "LvlPrest.txt",
            TableName::Levels => "Levels.txt",
            TableName::LvlTypes => "LvlTypes.txt",
            TableName::AutoMap => "AutoMap.txt",
        }
    }
}

/// The level tables, as far as the caller has them.
#[derive(Clone, Debug, Default)]
pub struct Tables {
    /// `LvlPrest.txt`.
    pub prest: Option<TxtDoc>,
    /// `Levels.txt`.
    pub levels: Option<TxtDoc>,
    /// `LvlTypes.txt`.
    pub types: Option<TxtDoc>,
    /// `AutoMap.txt`.
    pub automap: Option<TxtDoc>,
}

impl Tables {
    /// One table.
    pub fn doc(&self, t: TableName) -> Option<&TxtDoc> {
        match t {
            TableName::LvlPrest => self.prest.as_ref(),
            TableName::Levels => self.levels.as_ref(),
            TableName::LvlTypes => self.types.as_ref(),
            TableName::AutoMap => self.automap.as_ref(),
        }
    }

    /// One table, mutable.
    pub fn doc_mut(&mut self, t: TableName) -> Option<&mut TxtDoc> {
        match t {
            TableName::LvlPrest => self.prest.as_mut(),
            TableName::Levels => self.levels.as_mut(),
            TableName::LvlTypes => self.types.as_mut(),
            TableName::AutoMap => self.automap.as_mut(),
        }
    }
}

/// Layer kind of a cell in a DS1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerKind {
    /// Floor layer.
    Floor,
    /// Wall layer.
    Wall,
    /// The shadow layer.
    Shadow,
}

/// A cell of a DS1 layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRef {
    /// Layer kind.
    pub layer: LayerKind,
    /// Layer number within its kind.
    pub index: usize,
    /// Cell column.
    pub x: u32,
    /// Cell row.
    pub y: u32,
}

/// One change a fix makes. Each states an end state, so it is idempotent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    /// Set a table cell (`row` is the document row).
    SetCell {
        /// Table.
        table: TableName,
        /// Document row.
        row: usize,
        /// Column header.
        column: String,
        /// New value.
        value: String,
    },
    /// Put the records of a table back in `column` order (the ids stay).
    ReorderRecords {
        /// Table.
        table: TableName,
        /// The id column (`Id` or `Def`).
        column: String,
    },
    /// Remove the `AutoMap.txt` rows whose `Cel1` is not a valid picture.
    RemoveInvalidAutomapRows,
    /// Take these tile libraries (paths relative to `data/global/tiles`, any
    /// slash style) out of the map's own list.
    RemoveDt1s {
        /// Paths.
        paths: Vec<String>,
    },
    /// Add these tile libraries to the map's own list.
    AddDt1s {
        /// Paths.
        paths: Vec<String>,
    },
    /// Empty these cells.
    ClearCells {
        /// The cells.
        cells: Vec<CellRef>,
    },
    /// Set the main/sub of a special-tile marker in a wall layer.
    SetSpecial {
        /// Wall layer.
        layer: usize,
        /// Cell column.
        x: u32,
        /// Cell row.
        y: u32,
        /// New main index.
        main: u8,
        /// New sub index.
        sub: u8,
    },
    /// Remove objects whose position is outside the map.
    DeleteObjectsOutsideMap,
    /// Set the DS1 header's act (0-based).
    SetAct {
        /// Act.
        act: i32,
    },
    /// Put objects at sub-tile positions: `(object index, x, y)`.
    MoveObjects {
        /// The moves.
        moves: Vec<(usize, i32, i32)>,
    },
}

/// What a fix does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixAction {
    /// Make these edits.
    Edits(Vec<Edit>),
    /// Open a table (at a row key, if given).
    OpenTable {
        /// Table.
        table: TableName,
        /// Row name.
        key: Option<String>,
    },
    /// Open the automap editor.
    OpenAutomapEditor,
    /// Open the "add to game" flow.
    Register,
    /// Rename these over-long tile paths (relative to `data/global/tiles`).
    ShortenPaths {
        /// Paths.
        paths: Vec<String>,
    },
    /// Remember the answer "keep it as it is" under this key.
    Keep {
        /// Key.
        key: String,
    },
}

/// A one-click fix offered with a finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// Button text.
    pub label: String,
    /// What it does.
    pub action: FixAction,
}

impl Fix {
    pub(crate) fn edits(label: impl Into<String>, edits: Vec<Edit>) -> Fix {
        Fix {
            label: label.into(),
            action: FixAction::Edits(edits),
        }
    }

    pub(crate) fn open(table: TableName, key: Option<String>) -> Fix {
        Fix {
            label: format!("Open {}", table.file()),
            action: FixAction::OpenTable { table, key },
        }
    }
}

/// One result of the checker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Severity.
    pub severity: Severity,
    /// Area.
    pub area: Area,
    /// One line.
    pub title: String,
    /// Explanation.
    pub detail: Option<String>,
    /// Cells to show on the map.
    pub cells: Vec<(u32, u32)>,
    /// Table columns it is about.
    pub columns: Vec<(TableName, String)>,
    /// One-click fixes, best first.
    pub fixes: Vec<Fix>,
}

impl Finding {
    pub(crate) fn new(severity: Severity, area: Area, title: impl Into<String>) -> Finding {
        Finding {
            severity,
            area,
            title: title.into(),
            detail: None,
            cells: Vec::new(),
            columns: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub(crate) fn detail(mut self, d: impl Into<String>) -> Finding {
        self.detail = Some(d.into());
        self
    }

    pub(crate) fn cols(mut self, c: &[(TableName, &str)]) -> Finding {
        self.columns = c.iter().map(|(t, c)| (*t, (*c).to_string())).collect();
        self
    }

    pub(crate) fn fix(mut self, f: Fix) -> Finding {
        self.fixes.push(f);
        self
    }

    /// What identifies a result between runs, for "accepted" warnings: its
    /// area and title with the numbers ignored (counts change as a map is
    /// edited).
    pub fn key(&self) -> String {
        let mut t = String::new();
        let mut in_num = false;
        for ch in self.title.chars() {
            if ch.is_ascii_digit() {
                if !in_num {
                    t.push('#');
                }
                in_num = true;
            } else {
                in_num = false;
                t.push(ch);
            }
        }
        format!("{:?}|{}", self.area, t.trim().to_lowercase())
    }
}

/// A tile library the map is drawn with.
#[derive(Clone, Debug, Default)]
pub struct LoadedDt1 {
    /// Path relative to `data/global/tiles`.
    pub path: String,
    /// Whether the file exists.
    pub found: bool,
    /// The tiles it provides (empty when not found).
    pub tiles: HashSet<TileKey>,
}

/// A tile library not loaded by the map that might provide missing tiles.
#[derive(Clone, Debug, Default)]
pub struct Dt1Candidate {
    /// Path relative to `data/global/tiles`.
    pub path: String,
    /// The tiles it provides.
    pub tiles: HashSet<TileKey>,
}

/// The map being checked.
pub struct MapInput<'a> {
    /// The map's path relative to `data/global/tiles` (as LvlPrest writes it).
    pub path: &'a str,
    /// The parsed map.
    pub ds1: &'a Ds1,
    /// The libraries the map is drawn with (the level type's, selected by the
    /// preset's mask).
    pub loaded: &'a [LoadedDt1],
    /// Other libraries that could provide missing tiles, the map's own
    /// folders first.
    pub candidates: &'a [Dt1Candidate],
    /// For each library, how many placed tiles come from it.
    pub used_dt1s: &'a [(String, usize)],
}

/// Facts about the install the caller knows and a pure check cannot.
#[derive(Default)]
pub struct Env<'a> {
    /// Whether a tile library exists (path relative to `data/global/tiles`);
    /// when absent every library is assumed to exist.
    pub dt1_exists: Option<&'a dyn Fn(&str) -> bool>,
    /// Whether a loading-screen image exists (`EntryFile` name); when absent
    /// only empty `EntryFile`s are flagged.
    pub entry_image_exists: Option<&'a dyn Fn(&str) -> bool>,
    /// Whether a question was already answered "keep it".
    pub kept: Option<&'a dyn Fn(&str) -> bool>,
    /// Whether a level is new to the mod (not in its own archive's Levels.txt).
    pub new_level: Option<&'a dyn Fn(u32) -> bool>,
    /// The act the level's tiles were drawn for, judged from their art:
    /// `None` = unknown (use the level type's Act column), `Some(None)` = they
    /// only use colours every act shares.
    pub tiles_act: Option<Option<u8>>,
    /// Whether an object of this (type, id) is a waypoint.
    pub is_waypoint: Option<&'a dyn Fn(i32, i32) -> bool>,
    /// Tables whose `.bin` sits next to the `.txt` (the game reads the `.bin`).
    pub compiled_bins: Vec<String>,
    /// Collision data for the walkability, roof, edge, spawn-room, warp and
    /// arrival checks; when absent those checks are skipped.
    pub render: Option<render_checks::RenderEnv<'a>>,
}

impl Edit {
    /// Applies a table edit. Returns whether anything changed; map edits are
    /// ignored here (see [`Edit::apply_ds1`]).
    pub fn apply_tables(&self, t: &mut Tables) -> Result<bool, crate::automap::AutomapError> {
        match self {
            Edit::SetCell {
                table,
                row,
                column,
                value,
            } => {
                let Some(doc) = t.doc_mut(*table) else {
                    return Ok(false);
                };
                Ok(crate::doc_util::set_if_different(doc, *row, column, value).is_some())
            }
            Edit::ReorderRecords { table, column } => {
                let Some(doc) = t.doc_mut(*table) else {
                    return Ok(false);
                };
                Ok(tables::reorder_records(doc, column).is_ok_and(|moved| moved > 0))
            }
            Edit::RemoveInvalidAutomapRows => {
                let Some(doc) = t.automap.as_mut() else {
                    return Ok(false);
                };
                Ok(safety::remove_invalid_rows(doc)? > 0)
            }
            _ => Ok(false),
        }
    }

    /// Applies a map edit. Returns whether anything changed; table edits are
    /// ignored here (see [`Edit::apply_tables`]).
    pub fn apply_ds1(&self, ds1: &mut Ds1) -> bool {
        match self {
            Edit::RemoveDt1s { paths } => {
                let drop: HashSet<String> = paths.iter().map(|p| dt1_key(p)).collect();
                let before = ds1.files.len();
                ds1.files
                    .retain(|f| ds1_file_dt1(f).is_none_or(|p| !drop.contains(&p)));
                ds1.files.len() != before
            }
            Edit::AddDt1s { paths } => {
                let have: HashSet<String> =
                    ds1.files.iter().filter_map(|f| ds1_file_dt1(f)).collect();
                let mut changed = false;
                for p in paths {
                    if have.contains(&dt1_key(p)) {
                        continue;
                    }
                    let rel = p.replace('/', "\\");
                    let tg1 = match rel.rsplit_once('.') {
                        Some((stem, _)) => format!("{stem}.tg1"),
                        None => format!("{rel}.tg1"),
                    };
                    ds1.files
                        .push(format!("\\d2\\data\\global\\tiles\\{tg1}").into_bytes());
                    changed = true;
                }
                changed
            }
            Edit::ClearCells { cells } => {
                let mut changed = false;
                for c in cells {
                    changed |= match c.layer {
                        LayerKind::Floor => {
                            set_cell(ds1.floors.get_mut(c.index).map(|g| &mut *g), c)
                        }
                        LayerKind::Shadow => set_cell(Some(&mut ds1.shadow), c),
                        LayerKind::Wall => match ds1.walls.get_mut(c.index) {
                            Some(w) => {
                                let a = set_cell(Some(&mut w.tiles), c);
                                let b = w.orientations.get(c.x, c.y).is_some_and(|o| o != 0)
                                    && w.orientations.set(c.x, c.y, 0);
                                a || b
                            }
                            None => false,
                        },
                    };
                }
                changed
            }
            Edit::SetSpecial {
                layer,
                x,
                y,
                main,
                sub,
            } => {
                let Some(w) = ds1.walls.get_mut(*layer) else {
                    return false;
                };
                let Some(c) = w.tiles.get(*x, *y) else {
                    return false;
                };
                let n = c.with_indices(*main, *sub);
                n != c && w.tiles.set(*x, *y, n)
            }
            Edit::DeleteObjectsOutsideMap => {
                let (w, h) = (ds1.width as i32, ds1.height as i32);
                let before = ds1.objects.len();
                ds1.objects.retain(|o| {
                    let (cx, cy) = (o.x.div_euclid(5), o.y.div_euclid(5));
                    cx >= 0 && cy >= 0 && cx < w && cy < h
                });
                ds1.objects.len() != before
            }
            Edit::SetAct { act } => {
                let changed = ds1.act != *act;
                ds1.act = *act;
                changed
            }
            Edit::MoveObjects { moves } => {
                let mut changed = false;
                for &(i, x, y) in moves {
                    if let Some(o) = ds1.objects.get_mut(i)
                        && (o.x, o.y) != (x, y)
                    {
                        (o.x, o.y) = (x, y);
                        changed = true;
                    }
                }
                changed
            }
            _ => false,
        }
    }
}

fn set_cell(g: Option<&mut d2_formats::Grid<Cell>>, c: &CellRef) -> bool {
    let Some(g) = g else { return false };
    match g.get(c.x, c.y) {
        Some(old) if old != Cell::EMPTY => g.set(c.x, c.y, Cell::EMPTY),
        _ => false,
    }
}

/// A DS1 file-list entry as a lower-case slash path from `data/` on, with the
/// extension replaced by `.dt1`; `None` for entries that are not paths.
fn ds1_file_dt1(f: &[u8]) -> Option<String> {
    let s: String = f
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .replace('\\', "/");
    let lower = s.to_lowercase();
    let at = lower.find("data/")?;
    let p = &lower[at..];
    let stem = p.rsplit_once('.').map_or(p, |(a, _)| a);
    Some(
        stem.strip_prefix("data/global/tiles/")
            .unwrap_or(stem)
            .to_string()
            + ".dt1",
    )
}

fn dt1_key(p: &str) -> String {
    let n = norm_path(p);
    let n = n.strip_prefix("data/global/tiles/").unwrap_or(&n);
    let stem = n.rsplit_once('.').map_or(n, |(a, _)| a);
    format!("{stem}.dt1")
}

impl Fix {
    /// Applies an [`FixAction::Edits`] fix to the tables and the map. Other
    /// actions are UI flows and do nothing. Returns whether anything changed.
    pub fn apply(
        &self,
        tables: &mut Tables,
        ds1: Option<&mut Ds1>,
    ) -> Result<bool, crate::automap::AutomapError> {
        let FixAction::Edits(edits) = &self.action else {
            return Ok(false);
        };
        let mut changed = false;
        let mut ds1 = ds1;
        for e in edits {
            changed |= e.apply_tables(tables)?;
            if let Some(d) = ds1.as_deref_mut() {
                changed |= e.apply_ds1(d);
            }
        }
        Ok(changed)
    }
}

/// Every placed tile of a map: (cell, key). Special markers (orientation
/// 10/11) are not tiles of a library and are left out. Orientation 3 and 4
/// tiles are looked up together by the game; the key keeps the stored one.
pub fn placed_tiles(ds1: &Ds1) -> Vec<(CellRef, TileKey)> {
    let mut out = Vec::new();
    let key = |o: u8, c: Cell| TileKey {
        orientation: o,
        main: c.main_index(),
        sub: c.sub_index(),
    };
    for y in 0..ds1.height {
        for x in 0..ds1.width {
            for (i, f) in ds1.floors.iter().enumerate() {
                if let Some(c) = f.get(x, y).filter(|c| !c.is_empty()) {
                    out.push((
                        CellRef {
                            layer: LayerKind::Floor,
                            index: i,
                            x,
                            y,
                        },
                        key(0, c),
                    ));
                }
            }
            for (i, w) in ds1.walls.iter().enumerate() {
                let (Some(c), Some(o)) = (
                    w.tiles.get(x, y).filter(|c| !c.is_empty()),
                    ds1.orientation(i, x, y),
                ) else {
                    continue;
                };
                if o == 10 || o == 11 {
                    continue;
                }
                out.push((
                    CellRef {
                        layer: LayerKind::Wall,
                        index: i,
                        x,
                        y,
                    },
                    key(o, c),
                ));
            }
            if let Some(c) = ds1.shadow.get(x, y).filter(|c| !c.is_empty()) {
                out.push((
                    CellRef {
                        layer: LayerKind::Shadow,
                        index: 0,
                        x,
                        y,
                    },
                    key(13, c),
                ));
            }
        }
    }
    out
}

fn provided(set: &HashSet<TileKey>, k: TileKey) -> bool {
    if set.contains(&k) {
        return true;
    }
    match k.orientation {
        3 => set.contains(&TileKey {
            orientation: 4,
            ..k
        }),
        4 => set.contains(&TileKey {
            orientation: 3,
            ..k
        }),
        _ => false,
    }
}

fn short(p: &str) -> String {
    let n = p.replace('\\', "/");
    n.strip_prefix("data/global/tiles/")
        .unwrap_or(&n)
        .to_string()
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// Checks that a map will load and play in game: every placed tile has a
/// graphic in the libraries the game loads, the map is reachable through
/// LvlPrest, Levels and LvlTypes, it has entry markers, objects are inside it
/// and the automap tables are sound. Results are ordered errors first.
///
/// The walkability of objects, roof-hiding, open map edges, spawn rooms, warp
/// links and arrival-by-portal run when the caller passes collision data in
/// [`Env::render`] (see [`render_checks`]). Not covered: tile-library copies
/// and colour checks.
pub fn check_map(map: &MapInput<'_>, tables: &Tables, env: &Env<'_>) -> Vec<Finding> {
    let mut out = Vec::new();
    let ds1 = map.ds1;

    // --- Tiles ---------------------------------------------------------------
    let not_found: Vec<&LoadedDt1> = map.loaded.iter().filter(|l| !l.found).collect();
    if !not_found.is_empty() {
        let n = not_found.len();
        let paths: Vec<String> = not_found.iter().map(|l| l.path.clone()).collect();
        out.push(
            Finding::new(
                Severity::Error,
                Area::Tiles,
                format!("{n} tile librar{} not found", plural(n, "y", "ies")),
            )
            .detail(format!(
                "{}. The game cannot load {} either.",
                paths
                    .iter()
                    .map(|p| short(p))
                    .collect::<Vec<_>>()
                    .join(", "),
                plural(n, "it", "them")
            ))
            .fix(Fix::edits(
                format!(
                    "Remove {} from the map's libraries",
                    plural(n, "it", "them")
                ),
                vec![Edit::RemoveDt1s { paths }],
            )),
        );
    }
    let have: HashSet<TileKey> = map
        .loaded
        .iter()
        .filter(|l| l.found)
        .flat_map(|l| l.tiles.iter().copied())
        .collect();
    let mut missing: Vec<(CellRef, TileKey)> = placed_tiles(ds1)
        .into_iter()
        .filter(|(_, k)| !provided(&have, *k))
        .collect();
    missing.sort_by_key(|(c, _)| (c.y, c.x, c.index));
    if missing.is_empty() {
        out.push(Finding::new(
            Severity::Ok,
            Area::Tiles,
            "Every placed tile has a graphic",
        ));
    } else {
        let keys: HashSet<TileKey> = missing.iter().map(|(_, k)| *k).collect();
        let (paths, covered) = cover(&keys, map.candidates);
        let n = missing.len();
        let mut f = Finding::new(
            Severity::Error,
            Area::Tiles,
            format!("{n} placed tiles have no graphic"),
        )
        .detail(format!(
            "These cells use tiles (orientation/main/sub) that no loaded DT1 contains. In game they are invisible and may break walkability.{}",
            if paths.is_empty() { " No DT1 in the game or your mod has them." } else { "" }
        ));
        f.cells = missing.iter().map(|(c, _)| (c.x, c.y)).collect();
        if !paths.is_empty() {
            f = f.fix(Fix::edits(
                format!(
                    "Add {} ({} of the missing tile numbers; the graphics may not be the ones meant)",
                    paths.iter().map(|p| short(p)).collect::<Vec<_>>().join(", "),
                    if covered == keys.len() { "has all".to_string() } else { format!("has {covered} of {}", keys.len()) }
                ),
                vec![Edit::AddDt1s { paths }],
            ));
        }
        f = f.fix(Fix::edits(
            format!(
                "Clear those {n} tile{} (they are gone from the map)",
                plural(n, "", "s")
            ),
            vec![Edit::ClearCells {
                cells: missing.iter().map(|(c, _)| *c).collect(),
            }],
        ));
        out.push(f);
    }

    // --- Tables: is the map part of a level? -------------------------------------
    let rel = norm_path(map.path);
    let (prest, levels, types) = (
        tables.prest.as_ref(),
        tables.levels.as_ref(),
        tables.types.as_ref(),
    );
    let mut prest_row: Option<usize> = None;
    match prest {
        None => out.push(
            Finding::new(Severity::Warning, Area::Tables, "LvlPrest.txt not found")
                .detail("Cannot check how the game loads this map."),
        ),
        Some(p) => {
            let listing: Vec<usize> = data_rows(p)
                .into_iter()
                .filter(|&r| (1..=6).any(|i| norm_path(&get(p, r, &format!("File{i}"))) == rel))
                .collect();
            prest_row = listing
                .iter()
                .copied()
                .find(|&r| num(p, r, "LevelId") > 0)
                .or(listing.first().copied());
            match prest_row {
                None => out.push(
                    Finding::new(Severity::Error, Area::Tables, "Not referenced by LvlPrest.txt")
                        .detail(format!("No LvlPrest row lists \"{rel}\" in File1-File6, so the game never loads this map. Add a row (Name, Def, LevelId, File1, Dt1Mask)."))
                        .cols(&[(TableName::LvlPrest, "File1"), (TableName::LvlPrest, "LevelId"), (TableName::LvlPrest, "Dt1Mask")])
                        .fix(Fix { label: "Add to game (creates the LvlPrest / Levels / LvlTypes rows)".into(), action: FixAction::Register })
                        .fix(Fix::open(TableName::LvlPrest, None)),
                ),
                Some(r) => {
                    out.push(Finding::new(
                        Severity::Ok,
                        Area::Tables,
                        format!("LvlPrest: \"{}\" (Def {})", get(p, r, "Name"), get(p, r, "Def")),
                    ));
                    if let (Some(l), Some(t)) = (levels, types) {
                        let ctx = tables::Ctx { prest: p, levels: l, types: t };
                        let size = (ds1.width as usize, ds1.height as usize);
                        for issue in verify_in_game(&ctx, &rel, size, env) {
                            out.push(issue);
                        }
                    }
                    level_checks(&mut out, map, tables, env, r);
                }
            }
        }
    }

    // --- Entry points ------------------------------------------------------------
    let mut specials: Vec<(usize, u32, u32, u8, u8, u8)> = Vec::new();
    for (li, w) in ds1.walls.iter().enumerate() {
        for y in 0..ds1.height {
            for x in 0..ds1.width {
                let (Some(c), Some(o)) = (w.tiles.get(x, y), ds1.orientation(li, x, y)) else {
                    continue;
                };
                if c.prop1() != 0 && (o == 10 || o == 11) {
                    specials.push((li, x, y, o, c.main_index(), c.sub_index()));
                }
            }
        }
    }
    if specials.is_empty() {
        out.push(
            Finding::new(Severity::Warning, Area::Map, "No entry or warp markers").detail(
                "The map has no special tiles (orientation 10/11), which mark where players arrive (town entries, warps, portals). Players entering from another level may be placed at the map edge or not at all.",
            ),
        );
    } else {
        let mut f = Finding::new(
            Severity::Ok,
            Area::Map,
            format!("{} entry/warp marker tiles", specials.len()),
        );
        f.cells = specials.iter().map(|s| (s.1, s.2)).collect();
        out.push(f);
    }
    // Arriving by portal puts players on the Map entry marker (30/11); towns have none.
    let preset_level = prest_row
        .zip(prest)
        .map_or(0, |(r, p)| num(p, r, "LevelId"));
    if preset_level != 0 && !TOWNS.contains(&(preset_level as u32)) {
        let marks: Vec<_> = specials.iter().filter(|s| s.4 >= 30).collect();
        let town = marks
            .iter()
            .find(|m| m.4 == 30 && m.5 == 0)
            .or_else(|| marks.iter().find(|m| m.4 == 31 && m.5 == 0));
        if !marks.iter().any(|m| m.4 == 30 && m.5 == 11) {
            let mut f = Finding::new(
                if town.is_some() { Severity::Warning } else { Severity::Info },
                Area::Map,
                match town {
                    Some(t) => format!("A town entry marker ({}/{}) in a level that is not a town", t.4, t.5),
                    None => "No Map entry marker (10/30/11)".to_string(),
                },
            )
            .detail("Players arriving by portal (a red portal or a map item) appear on the Map entry marker, special tile 10/30/11, as in every portal level of the game. Town entries (30/0, 31/0) only work in towns; without a Map entry the game places players wherever it likes.");
            if let Some(t) = town {
                f.cells = vec![(t.1, t.2)];
                f = f.fix(Fix::edits(
                    format!(
                        "Make the marker at ({}, {}) the Map entry (30/11)",
                        t.1, t.2
                    ),
                    vec![Edit::SetSpecial {
                        layer: t.0,
                        x: t.1,
                        y: t.2,
                        main: 30,
                        sub: 11,
                    }],
                ));
            }
            out.push(f);
        }
    }

    // --- Objects -------------------------------------------------------------------
    let (w, h) = (ds1.width as i32, ds1.height as i32);
    let off: Vec<_> = ds1
        .objects
        .iter()
        .filter(|o| {
            let (cx, cy) = (o.x.div_euclid(5), o.y.div_euclid(5));
            cx < 0 || cy < 0 || cx >= w || cy >= h
        })
        .collect();
    if off.is_empty() {
        out.push(Finding::new(
            Severity::Ok,
            Area::Objects,
            format!("{} objects inside the map", ds1.objects.len()),
        ));
    } else {
        out.push(
            Finding::new(
                Severity::Error,
                Area::Objects,
                format!("{} objects outside the map", off.len()),
            )
            .detail(
                off.iter()
                    .map(|o| format!("type {} id {} at ({}, {})", o.kind, o.id, o.x, o.y))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .fix(Fix::edits(
                format!("Delete {}", plural(off.len(), "it", "them")),
                vec![Edit::DeleteObjectsOutsideMap],
            )),
        );
    }

    if let Some(render) = &env.render {
        render_checks::render_checks(&mut out, ds1, tables, prest_row, render);
    }

    // --- Automap ---------------------------------------------------------------------
    automap_checks(&mut out, map, tables, prest_row);

    // --- Compiled tables ---------------------------------------------------------------
    if !env.compiled_bins.is_empty() {
        out.push(
            Finding::new(
                Severity::Info,
                Area::Tables,
                format!("Compiled .bin files sit next to {}.txt", env.compiled_bins.join(", ")),
            )
            .detail("The game reads the .bin files. After editing the .txt tables, start the game once with -direct -txt (or run your mod's tool) so the .bin files are rebuilt."),
        );
    }
    out.sort_by_key(|f| f.severity);
    out
}

/// Greedy cover: the fewest candidate libraries holding the most `keys`.
fn cover(keys: &HashSet<TileKey>, candidates: &[Dt1Candidate]) -> (Vec<String>, usize) {
    let mut left: HashSet<TileKey> = keys.clone();
    let mut paths = Vec::new();
    let has = |c: &Dt1Candidate, k: &TileKey| provided(&c.tiles, *k);
    while !left.is_empty() {
        let mut best: Option<(&Dt1Candidate, usize)> = None;
        for c in candidates {
            let n = left.iter().filter(|k| has(c, k)).count();
            if n > best.map_or(0, |b| b.1) {
                best = Some((c, n));
            }
        }
        let Some((c, _)) = best else { break };
        paths.push(c.path.clone());
        left.retain(|k| !has(c, k));
    }
    let covered = keys.len() - left.len();
    (paths, covered)
}

fn level_checks(
    out: &mut Vec<Finding>,
    map: &MapInput<'_>,
    tables: &Tables,
    env: &Env<'_>,
    prest_row: usize,
) {
    let ds1 = map.ds1;
    let (Some(prest), Some(levels), Some(types)) = (&tables.prest, &tables.levels, &tables.types)
    else {
        return;
    };
    let level_id = num(prest, prest_row, "LevelId");
    let mask = (num(prest, prest_row, "Dt1Mask") as u64 & 0xffff_ffff) as u32;
    if level_id == 0 {
        out.push(
            Finding::new(Severity::Info, Area::Level, "Shared preset (LevelId 0)")
                .cols(&[(TableName::LvlPrest, "LevelId")])
                .detail("Placed by a level generator (LvlMaze/LvlSub or another level), not a level on its own. Level checks are skipped."),
        );
        return;
    }
    let Some(lrow) = tables::row_of_record(levels, level_id) else {
        out.push(
            Finding::new(
                Severity::Error,
                Area::Level,
                format!("Levels.txt has no level {level_id}"),
            )
            .fix(Fix::open(TableName::Levels, None))
            .fix(Fix {
                label: "Add to game (pick or create the level)".into(),
                action: FixAction::Register,
            }),
        );
        return;
    };
    let type_id = num(levels, lrow, "LevelType");
    let lname = {
        let n = get(levels, lrow, "LevelName");
        if n.is_empty() {
            get(levels, lrow, "Name")
        } else {
            n
        }
    };
    out.push(Finding::new(
        Severity::Ok,
        Area::Level,
        format!(
            "Level {level_id} \"{lname}\" (Act {})",
            num(levels, lrow, "Act") + 1
        ),
    ));
    match tables::row_of_record(types, type_id) {
        None => out.push(
            Finding::new(
                Severity::Error,
                Area::Level,
                format!("LvlTypes.txt has no type {type_id}"),
            )
            .fix(Fix::open(TableName::LvlTypes, None)),
        ),
        Some(trow) => {
            let tname = get(types, trow, "Name");
            let files: Vec<String> = (1..=32)
                .map(|i| get(types, trow, &format!("File {i}")))
                .collect();
            let is_file = |f: &String| !f.is_empty() && f != "0";
            let loaded: HashSet<String> = files
                .iter()
                .enumerate()
                .filter(|(i, f)| is_file(f) && (mask >> i) & 1 == 1)
                .map(|(_, f)| norm_path(&format!("data/global/tiles/{f}")))
                .collect();
            let mut not_loaded: Vec<&(String, usize)> = map
                .used_dt1s
                .iter()
                .filter(|(p, _)| {
                    !loaded.contains(&norm_path(&format!(
                        "data/global/tiles/{}",
                        norm_path(p).trim_start_matches("data/global/tiles/")
                    )))
                })
                .collect();
            not_loaded.sort();
            if not_loaded.is_empty() {
                out.push(Finding::new(
                    Severity::Ok,
                    Area::Level,
                    "Every tile library the map uses is loaded by its level type",
                ));
            } else {
                let n = not_loaded.len();
                let mut f = Finding::new(
                    Severity::Error,
                    Area::Level,
                    format!("{n} tile librar{} used but not loaded in game", plural(n, "y is", "ies are")),
                )
                .detail(format!(
                    "{}. Add {} to LvlTypes \"{tname}\" (File 1-32) and set the matching bits in this preset's Dt1Mask ({mask}).",
                    not_loaded.iter().map(|(p, c)| format!("{} ({c} tiles)", short(p))).collect::<Vec<_>>().join(", "),
                    plural(n, "it", "them")
                ))
                .cols(&[(TableName::LvlTypes, "File 1"), (TableName::LvlPrest, "Dt1Mask")]);
                if let Some(edits) = sync_edits(trow, prest_row, &files, mask, &not_loaded) {
                    f = f.fix(Fix::edits(
                        format!("Update LvlTypes \"{tname}\" and the Dt1Mask automatically"),
                        edits,
                    ));
                }
                f = f.fix(Fix::open(TableName::LvlTypes, Some(tname.clone())));
                out.push(f);
            }
            for (i, f) in files.iter().enumerate() {
                if !is_file(f) || (mask >> i) & 1 == 0 {
                    continue;
                }
                if !env.dt1_exists.is_none_or(|e| e(&norm_path(f))) {
                    out.push(
                        Finding::new(Severity::Error, Area::Level, format!("LvlTypes file missing: {f}"))
                            .detail("The game fails to load this level type. Fix the path or remove it from the row.")
                            .fix(Fix::open(TableName::LvlTypes, Some(tname.clone()))),
                    );
                }
            }
        }
    }
    let act = num(levels, lrow, "Act");
    if act != i64::from(ds1.act) {
        out.push(
            Finding::new(
                Severity::Info,
                Area::Level,
                format!("Level is in Act {}, DS1 header says Act {}", act + 1, i64::from(ds1.act) + 1),
            )
            .detail("The game places the map's objects and NPCs from the level's act (by its number), so the object numbers in the DS1 should be that act's. The colours come from the level's Pal.")
            .cols(&[(TableName::Levels, "Act")])
            .fix(Fix::edits(
                format!("Set the DS1 header to Act {}", act + 1),
                vec![Edit::SetAct { act: act as i32 }],
            )),
        );
    }
    let wp_slot = num(levels, lrow, "Waypoint");
    if wp_slot != 0
        && wp_slot != 255
        && let Some(is_wp) = env.is_waypoint
        && !ds1.objects.iter().any(|o| is_wp(o.kind, o.id))
    {
        out.push(
            Finding::new(Severity::Warning, Area::Level, "Level has a waypoint slot but no waypoint object on this map")
                .detail("Players cannot use the waypoint unless one of the level's presets has a waypoint object.")
                .cols(&[(TableName::Levels, "Waypoint")]),
        );
    }
}

/// Edits that make the level type load `not_loaded` and the preset's mask
/// select them, or `None` when the type has no free slots.
fn sync_edits(
    trow: usize,
    prow: usize,
    files: &[String],
    mask: u32,
    not_loaded: &[&(String, usize)],
) -> Option<Vec<Edit>> {
    let mut slots: Vec<String> = files.to_vec();
    let mut mask = mask;
    let mut edits = Vec::new();
    for (p, _) in not_loaded {
        let want = norm_path(p);
        let want = want
            .strip_prefix("data/global/tiles/")
            .unwrap_or(&want)
            .to_string();
        let slot = slots
            .iter()
            .position(|f| !f.is_empty() && f != "0" && norm_path(f) == want)
            .or_else(|| {
                let free = slots.iter().position(|f| f.is_empty() || f == "0")?;
                slots[free] = p.replace('\\', "/");
                edits.push(Edit::SetCell {
                    table: TableName::LvlTypes,
                    row: trow,
                    column: format!("File {}", free + 1),
                    value: slots[free].clone(),
                });
                Some(free)
            })?;
        mask |= 1 << slot;
    }
    edits.push(Edit::SetCell {
        table: TableName::LvlPrest,
        row: prow,
        column: "Dt1Mask".into(),
        value: mask.to_string(),
    });
    Some(edits)
}

fn automap_checks(
    out: &mut Vec<Finding>,
    map: &MapInput<'_>,
    tables: &Tables,
    prest_row: Option<usize>,
) {
    let Some(doc) = &tables.automap else { return };
    if let Ok(bad) = safety::invalid_rows(doc)
        && !bad.is_empty()
    {
        let n = bad.len();
        let lines: Vec<String> = bad.iter().take(12).map(|r| (r + 2).to_string()).collect();
        out.push(
            Finding::new(
                Severity::Error,
                Area::Tables,
                format!("AutoMap.txt has {n} rule{} with no picture (Cel1 -1): the game crashes while loading", plural(n, "", "s")),
            )
            .detail(format!(
                "Line{} {}{}. The game's automap loader needs a picture in Cel1 of every rule, so a row whose Cel1 is -1 stops it at start-up. Removing those rows is safe: a tile with no rule stays off the automap.",
                plural(n, "", "s"),
                lines.join(", "),
                if n > 12 { "..." } else { "" }
            ))
            .cols(&[(TableName::AutoMap, "Cel1")])
            .fix(Fix::edits(
                format!("Remove the {n} row{}", plural(n, "", "s")),
                vec![Edit::RemoveInvalidAutomapRows],
            ))
            .fix(Fix::open(TableName::AutoMap, None)),
        );
    }
    let table = AutomapTable::parse(doc);
    let unknown = table.unknown_levels();
    if !unknown.is_empty() {
        out.push(
            Finding::new(
                Severity::Warning,
                Area::Tables,
                format!(
                    "AutoMap.txt names level{} the game doesn't know: {}",
                    plural(unknown.len(), "", "s"),
                    unknown.iter().take(4).map(|u| format!("\"{u}\"")).collect::<Vec<_>>().join(", ")
                ),
            )
            .detail(format!("The game only accepts its own {} automap level names (\"1 Town\" ... \"5 Lava\"), and mods like PD2 also a level type's number (\"47\"). An unknown name stops the game with an error at start-up.", automap::GAME_AUTOMAP_LEVELS.len()))
            .cols(&[(TableName::AutoMap, "LevelName")])
            .fix(Fix::open(TableName::AutoMap, None)),
        );
    }
    // The map's level type, through the level its preset builds.
    let (Some(prest), Some(levels), Some(types)) = (&tables.prest, &tables.levels, &tables.types)
    else {
        return;
    };
    let Some(pr) = prest_row else { return };
    let level_id = num(prest, pr, "LevelId");
    let Some(lrow) = (level_id != 0)
        .then(|| tables::row_of_record(levels, level_id))
        .flatten()
    else {
        return;
    };
    let type_id = num(levels, lrow, "LevelType");
    let Some(trow) = tables::row_of_record(types, type_id) else {
        return;
    };
    let type_name = get(types, trow, "Name");
    let act = map.ds1.act + 1;
    let level = table.level_for(Some(&type_name), Some(act), i32::try_from(type_id).ok());
    let mut no_entries = false;
    match &level {
        Some(l) if !automap::has_level_rows(doc, l) => {
            no_entries = true;
            out.push(
                Finding::new(
                    Severity::Warning,
                    Area::Map,
                    format!("This level type ({type_id} \"{type_name}\") has no automap entries, so the automap stays empty here"),
                )
                .detail(format!("AutoMap.txt gives each tile of a level type the piece the automap draws for it, under the level type's number (\"{l}\"). This one has none yet. In the automap editor, pick pieces for the tiles (suggestions come from the most used pieces of the same act)."))
                .cols(&[(TableName::AutoMap, "LevelName")])
                .fix(Fix { label: "Open the automap editor (then Suggest)".into(), action: FixAction::OpenAutomapEditor }),
            );
        }
        None if type_id >= automap::GAME_AUTOMAP_LEVELS.len() as i64 => out.push(
            Finding::new(
                Severity::Info,
                Area::Map,
                format!("The automap can't show this level type ({type_id} \"{type_name}\")"),
            )
            .detail(format!("The game only knows automap entries for its own {} level types. Mods like PD2 read a level type's number too; this install's AutoMap.txt doesn't use numbers, so it probably doesn't.", automap::GAME_AUTOMAP_LEVELS.len())),
        ),
        _ => {}
    }
    if let (Some(l), false) = (&level, no_entries) {
        let pieces = automap::pieces(map.ds1, &table, l);
        let miss: Vec<_> = pieces
            .iter()
            .filter(|p| p.layer == automap::PieceLayer::Wall && p.rule_row.is_none())
            .collect();
        if !miss.is_empty() {
            let mut f = Finding::new(
                Severity::Info,
                Area::Map,
                format!("{} walls have no automap entry", miss.len()),
            )
            .detail("They will not show on the in-game automap. Give them pieces, or mark them hidden if they should not show.")
            .fix(Fix { label: "Open the automap editor".into(), action: FixAction::OpenAutomapEditor });
            f.cells = miss.iter().map(|p| (p.x, p.y)).collect();
            out.push(f);
        }
    }
}

/// Counts findings by severity (for a summary line).
pub fn summarize(findings: &[Finding]) -> HashMap<Severity, usize> {
    let mut m = HashMap::new();
    for f in findings {
        *m.entry(f.severity).or_insert(0) += 1;
    }
    m
}
