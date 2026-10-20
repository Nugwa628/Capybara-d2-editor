//! The table rules the game's loaders and level builder follow, checked for
//! one map: record order, tile path length, the preset-to-level link, level
//! size, act, palette, automap layer, loading screen and world position.

use d2_formats::txt::TxtDoc;

use super::{Area, Edit, Env, Finding, Fix, FixAction, Severity, TableName};
use crate::doc_util::{data_rows, get, num};

/// Longest tile path (relative to `data/global/tiles`) the game holds: it
/// copies tile paths into a 60-byte buffer after `DATA\GLOBAL\TILES\`.
pub const MAX_TILE_PATH: usize = 41;
/// Longest `Levels.txt` string column the game keeps (its longest is 40).
pub const MAX_LEVEL_STRING: usize = 39;
/// Highest `Levels.txt` `Layer` (automap layer) the game takes; 101 or 102
/// crash it as players arrive.
pub const MAX_LAYER: i64 = 100;
/// The game's towns (the only levels whose LvlPrest `AutoMap` may be 1).
pub const TOWNS: [u32; 5] = [1, 40, 75, 103, 109];
/// `|OffsetX - OffsetY|` limit: D2Client keeps automap positions in 16 bits.
pub const AUTOMAP_MAX_DIFF: i64 = 4095;
/// `OffsetX + OffsetY` limit.
pub const AUTOMAP_MAX_SUM: i64 = 8191;
const ENTRY_IMAGE_DIR: &str = "data/local/ui/eng/expansion/";
const DEFAULT_ENTRY_FILE: &str = "A5L1";

/// The three level tables.
pub struct Ctx<'a> {
    /// `LvlPrest.txt`.
    pub prest: &'a TxtDoc,
    /// `Levels.txt`.
    pub levels: &'a TxtDoc,
    /// `LvlTypes.txt`.
    pub types: &'a TxtDoc,
}

/// Lower-case slash path without leading slashes (how paths are compared).
pub fn norm_path(p: &str) -> String {
    p.trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_lowercase()
}

/// The act (0-4) the game assigns a level from its number.
pub fn level_act(id: i64) -> i64 {
    match id {
        109.. => 4,
        103.. => 3,
        75.. => 2,
        40.. => 1,
        _ => 0,
    }
}

/// Document row of the record with this index (the game reads tables by
/// position, not by the Id column).
pub fn row_of_record(doc: &TxtDoc, record: i64) -> Option<usize> {
    usize::try_from(record)
        .ok()
        .and_then(|r| data_rows(doc).get(r).copied())
}

/// Whether a level at (x, y) sized w by h tiles stays inside the automap's
/// range (less `margin` tiles).
pub fn automap_fits(x: i64, y: i64, w: i64, h: i64, margin: i64) -> bool {
    x >= 0
        && y >= 0
        && x + w - y <= AUTOMAP_MAX_DIFF - margin
        && y + h - x <= AUTOMAP_MAX_DIFF - margin
        && x + w + y + h <= AUTOMAP_MAX_SUM - margin
}

struct LevelBox {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// A level's fixed place in its act's world, or `None` when the game places it.
fn level_box(levels: &TxtDoc, row: usize) -> Option<LevelBox> {
    let (x, y) = (num(levels, row, "OffsetX"), num(levels, row, "OffsetY"));
    let w = ["SizeX", "SizeX(N)", "SizeX(H)"]
        .iter()
        .map(|c| num(levels, row, c))
        .max()?;
    let h = ["SizeY", "SizeY(N)", "SizeY(H)"]
        .iter()
        .map(|c| num(levels, row, c))
        .max()?;
    (x >= 0 && y >= 0 && num(levels, row, "Depend") == 0 && w > 0).then_some(LevelBox {
        x,
        y,
        w,
        h,
    })
}

/// A world position for a level of `size` in `act`: clear of every other
/// fixed level there by 100 tiles and 100 tiles inside the automap's range, as
/// far out as that allows. `except` is the level's own row when moving it.
pub fn free_offset(
    levels: &TxtDoc,
    act: i64,
    size: (i64, i64),
    except: Option<usize>,
) -> (i64, i64) {
    let gap = 100;
    let boxes: Vec<LevelBox> = data_rows(levels)
        .into_iter()
        .filter(|&r| Some(r) != except && level_act(num(levels, r, "Id")) == act)
        .filter_map(|r| level_box(levels, r))
        .collect();
    let mut best: Option<(i64, i64)> = None;
    let mut y = 0;
    while y <= AUTOMAP_MAX_SUM {
        let mut x = 0;
        while x <= AUTOMAP_MAX_SUM {
            let free = automap_fits(x, y, size.0, size.1, 100)
                && !boxes.iter().any(|b| {
                    x < b.x + b.w + gap
                        && b.x < x + size.0 + gap
                        && y < b.y + b.h + gap
                        && b.y < y + size.1 + gap
                });
            if free && best.is_none_or(|(bx, by)| x + y > bx + by || (x + y == bx + by && x > bx)) {
                best = Some((x, y));
            }
            x += 50;
        }
        y += 50;
    }
    best.unwrap_or((1000, 1000))
}

/// The most common `Layer` among levels the mod added (preset levels past the
/// game's own, which share one), else 98.
fn safe_layer(levels: &TxtDoc) -> i64 {
    let mut count: Vec<(i64, usize)> = Vec::new();
    for r in data_rows(levels) {
        let l = num(levels, r, "Layer");
        if num(levels, r, "Id") >= 137 && num(levels, r, "DrlgType") == 2 && l <= MAX_LAYER {
            match count.iter_mut().find(|(v, _)| *v == l) {
                Some(e) => e.1 += 1,
                None => count.push((l, 1)),
            }
        }
    }
    let mut best: Option<(i64, usize)> = None;
    for (v, n) in count {
        if best.is_none_or(|b| n > b.1) {
            best = Some((v, n));
        }
    }
    best.map_or(98, |b| b.0)
}

/// The act (0-4) a level type's tiles belong to (LvlTypes `Act` is 1-5).
fn type_act(types: &TxtDoc, type_id: i64) -> Option<i64> {
    let row = row_of_record(types, type_id)?;
    let act = num(types, row, "Act");
    (1..=5).contains(&act).then_some(act - 1)
}

/// A tile path the game can hold; `None` when fine.
pub fn tile_path_problem(rel: &str) -> Option<String> {
    (rel.len() > MAX_TILE_PATH).then(|| {
        format!(
            "\"{rel}\" is {} characters. The game copies tile paths into a 60-byte buffer after DATA\\GLOBAL\\TILES\\, so it holds at most {MAX_TILE_PATH}: a longer one can crash the game or fail to load. Use shorter folder or file names.",
            rel.len()
        )
    })
}

/// A fix that sets cells of one table; only cells that differ are included.
/// `None` when nothing would change.
fn cell_fix(
    table: TableName,
    doc: &TxtDoc,
    label: impl Into<String>,
    cells: &[(usize, String, String)],
) -> Option<Fix> {
    let edits: Vec<Edit> = cells
        .iter()
        .filter(|(row, col, value)| doc.column(col).is_some() && get(doc, *row, col) != *value)
        .map(|(row, col, value)| Edit::SetCell {
            table,
            row: *row,
            column: col.clone(),
            value: value.clone(),
        })
        .collect();
    (!edits.is_empty()).then(|| Fix::edits(label, edits))
}

fn cell(row: usize, col: &str, value: impl ToString) -> (usize, String, String) {
    (row, col.to_string(), value.to_string())
}

/// Puts the records of `doc` back in `col` order; the Id values stay. Other
/// lines (the "Expansion" separator, blank lines) stay just before the record
/// that followed them. Returns how many records moved. Errors when the ids
/// are not simply a shuffled 0..n (then rows cannot be fixed by moving).
pub fn reorder_records(doc: &mut TxtDoc, col: &str) -> Result<usize, String> {
    let rows = data_rows(doc);
    let Some(c) = doc.column(col) else {
        return Err(format!("no {col} column"));
    };
    let ids: Vec<Option<usize>> = rows
        .iter()
        .map(|&r| doc.cell(r, c).and_then(|s| s.trim().parse().ok()))
        .collect();
    let n = rows.len();
    let mut seen = vec![false; n];
    for id in &ids {
        match id {
            Some(i) if *i < n && !seen[*i] => seen[*i] = true,
            _ => return Err(format!("the {col}s aren't simply out of order")),
        }
    }
    let moved = ids
        .iter()
        .enumerate()
        .filter(|(i, id)| **id != Some(*i))
        .count();
    if moved == 0 {
        return Ok(0);
    }
    let bytes = doc.to_bytes();
    let lines: Vec<&[u8]> = bytes.split_inclusive(|&b| b == b'\n').collect();
    if lines.len() != doc.row_count() + 1 {
        return Err("unusual line endings".into());
    }
    let eol: &[u8] = if lines[0].ends_with(b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let had_final = bytes.ends_with(b"\n");
    let line = |r: usize| -> Vec<u8> {
        let mut l = lines[r + 1].to_vec();
        if !l.ends_with(b"\n") {
            l.extend_from_slice(eol);
        }
        l
    };
    let mut before: Vec<Vec<Vec<u8>>> = vec![Vec::new(); n];
    let mut tail: Vec<Vec<u8>> = Vec::new();
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut by_id: Vec<Vec<u8>> = vec![Vec::new(); n];
    let mut k = 0;
    for r in 0..doc.row_count() {
        if rows.get(k) == Some(&r) {
            let id = ids[k].unwrap_or(0);
            before[id] = std::mem::take(&mut pending);
            by_id[id] = line(r);
            k += 1;
        } else {
            pending.push(line(r));
        }
    }
    tail.extend(pending);
    let mut out: Vec<u8> = lines[0].to_vec();
    if !out.ends_with(b"\n") {
        out.extend_from_slice(eol);
    }
    for i in 0..n {
        for l in &before[i] {
            out.extend_from_slice(l);
        }
        out.extend_from_slice(&by_id[i]);
    }
    for l in &tail {
        out.extend_from_slice(l);
    }
    if !had_final {
        out.truncate(out.len() - eol.len());
    }
    *doc = TxtDoc::parse(&out);
    Ok(moved)
}

fn issue(severity: Severity, title: impl Into<String>) -> Finding {
    Finding::new(severity, Area::Tables, title)
}

/// Checks that the tables load `map_rel` the way the game's rules need
/// (`size` is the DS1's width and height in tiles). Findings come in
/// reading order; each may carry fixes as [`Edit`]s.
pub fn verify_in_game(
    ctx: &Ctx<'_>,
    map_rel: &str,
    size: (usize, usize),
    env: &Env<'_>,
) -> Vec<Finding> {
    let Ctx {
        prest,
        levels,
        types,
    } = *ctx;
    let mut out = Vec::new();
    for (name, table, doc, col) in [
        (TableName::Levels, "Levels", levels, "Id"),
        (TableName::LvlPrest, "LvlPrest", prest, "Def"),
        (TableName::LvlTypes, "LvlTypes", types, "Id"),
    ] {
        let rows = data_rows(doc);
        if let Some(bad) = rows
            .iter()
            .enumerate()
            .position(|(i, &r)| get(doc, r, col) != i.to_string())
        {
            let have = get(doc, rows[bad], col);
            let mut f = issue(
                Severity::Error,
                format!("{table}.txt: row {bad} has {col} {}, not {bad}", if have.is_empty() { "(empty)" } else { &have }),
            )
            .detail(format!("The game reads {table}.txt by row position (the \"Expansion\" line and blank lines don't count), so {col} must count up 0, 1, 2... A row inserted or removed shifts every one after it, so the game uses the wrong row for everything after it."))
            .cols(&[(name, col)]);
            let mut probe = doc.clone();
            if reorder_records(&mut probe, col).is_ok() {
                f = f.fix(Fix::edits(
                    format!("Put {table}'s rows back in {col} order (no {col} changes)"),
                    vec![Edit::ReorderRecords {
                        table: name,
                        column: col.to_string(),
                    }],
                ));
            }
            out.push(f);
        }
    }
    // Until the rows are in order, every lookup below would read the wrong row.
    if !out.is_empty() {
        return out;
    }
    let want = norm_path(map_rel);
    let p_rows: Vec<usize> = data_rows(prest)
        .into_iter()
        .filter(|&r| (1..=6).any(|i| norm_path(&get(prest, r, &format!("File{i}"))) == want))
        .collect();
    let mut long_paths: Vec<String> = Vec::new();
    for &r in &p_rows {
        let label = format!(
            "Preset {} \"{}\"",
            get(prest, r, "Def"),
            get(prest, r, "Name")
        );
        for i in 1..=6 {
            let f = get(prest, r, &format!("File{i}"));
            if !f.is_empty()
                && f != "0"
                && let Some(p) = tile_path_problem(&f)
            {
                if !long_paths.iter().any(|x| norm_path(x) == norm_path(&f)) {
                    long_paths.push(f.clone());
                }
                out.push(
                    issue(Severity::Error, format!("{label}: File{i} path too long"))
                        .detail(p)
                        .cols(&[(TableName::LvlPrest, &format!("File{i}"))]),
                );
            }
        }
        let files = (1..=6)
            .filter(|i| {
                let f = get(prest, r, &format!("File{i}"));
                !f.is_empty() && f != "0"
            })
            .count() as i64;
        let files_col = num(prest, r, "Files");
        if files_col > files {
            let mut f = issue(
                Severity::Error,
                format!(
                    "{label}: Files is {files_col} but only {files} File column{} set",
                    if files == 1 { " is" } else { "s are" }
                ),
            )
            .detail(
                "The game picks one of the first Files maps at random; an empty one fails to load.",
            )
            .cols(&[(TableName::LvlPrest, "Files")]);
            if let Some(fx) = cell_fix(
                TableName::LvlPrest,
                prest,
                format!("Set Files to {files}"),
                &[cell(r, "Files", files)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        let level_id = num(prest, r, "LevelId");
        if level_id == 0 {
            continue;
        }
        // Every whole-level preset of the game has FillBlanks 1.
        if prest.column("FillBlanks").is_some() && get(prest, r, "FillBlanks") != "1" {
            let have = get(prest, r, "FillBlanks");
            let mut f = issue(
                Severity::Warning,
                format!("{label}: FillBlanks is {}", if have.is_empty() { "empty" } else { &have }),
            )
            .detail("Every preset level of the game has FillBlanks 1 (empty cells of the map are filled in).")
            .cols(&[(TableName::LvlPrest, "FillBlanks")]);
            if let Some(fx) = cell_fix(
                TableName::LvlPrest,
                prest,
                "Set FillBlanks to 1",
                &[cell(r, "FillBlanks", 1)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        // AutoMap 1 reveals the whole automap when the level is built; only for towns.
        if !TOWNS.contains(&(level_id as u32)) && get(prest, r, "AutoMap") == "1" {
            let mut f = issue(
                Severity::Error,
                format!("{label}: AutoMap is 1, so the game crashes when a player enters"),
            )
            .detail("AutoMap 1 reveals the whole automap as the level is built. The game does that only for its five towns; for any other level it halts while entering (D2Client, while revealing the automap).")
            .cols(&[(TableName::LvlPrest, "AutoMap")]);
            if let Some(fx) = cell_fix(
                TableName::LvlPrest,
                prest,
                "Set AutoMap to 0",
                &[cell(r, "AutoMap", 0)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        let Some(lrow) = row_of_record(levels, level_id) else {
            let pname = get(prest, r, "Name").to_lowercase();
            let by_name = data_rows(levels).into_iter().find(|&x| {
                [get(levels, x, "Name"), get(levels, x, "LevelName")]
                    .iter()
                    .any(|n| n.to_lowercase() == pname)
            });
            let mut f = issue(
                Severity::Error,
                format!("{label} builds level {level_id}, which Levels.txt doesn't have"),
            )
            .detail(format!(
                "Levels.txt has {} levels (0-{}).{}",
                data_rows(levels).len(),
                data_rows(levels).len().saturating_sub(1),
                by_name.map_or(String::new(), |b| format!(
                    " Level {} \"{}\" has this preset's name.",
                    get(levels, b, "Id"),
                    get(levels, b, "Name")
                ))
            ))
            .cols(&[(TableName::LvlPrest, "LevelId")]);
            if let Some(b) = by_name {
                let id = get(levels, b, "Id");
                if let Some(fx) = cell_fix(
                    TableName::LvlPrest,
                    prest,
                    format!("Point it at level {id} \"{}\"", get(levels, b, "Name")),
                    &[cell(r, "LevelId", id)],
                ) {
                    f = f.fix(fx);
                }
            }
            out.push(f);
            continue;
        };
        let lname = get(levels, lrow, "Name");
        let layer = num(levels, lrow, "Layer");
        if layer > MAX_LAYER {
            let to = safe_layer(levels);
            let mut f = Finding::new(
                Severity::Error,
                Area::Level,
                format!("Level {level_id} \"{lname}\" has automap Layer {layer}: the game crashes as players arrive"),
            )
            .detail(format!("Levels.txt Layer is the level's automap layer; the game takes 0-{MAX_LAYER} (its own levels use 0-99, PD2's maps all share 98). A higher one crashes the game the moment a player enters the level."))
            .cols(&[(TableName::Levels, "Layer")]);
            if let Some(fx) = cell_fix(
                TableName::Levels,
                levels,
                format!("Set Layer to {to} (as the other added levels)"),
                &[cell(lrow, "Layer", to)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        let drlg = num(levels, lrow, "DrlgType");
        if drlg != 2 {
            out.push(
                Finding::new(
                    Severity::Warning,
                    Area::Level,
                    format!("{label}: level {level_id} \"{lname}\" is built at random (DrlgType {drlg})"),
                )
                .detail("Only preset levels (DrlgType 2) load a map from their LvlPrest row; this row is probably not used as a whole level.")
                .cols(&[(TableName::Levels, "DrlgType")]),
            );
            continue;
        }
        let first = data_rows(prest)
            .into_iter()
            .find(|&x| num(prest, x, "LevelId") == level_id);
        if let Some(fr) = first.filter(|&fr| fr != r) {
            out.push(
                issue(
                    Severity::Error,
                    format!("{label} is ignored: level {level_id} \"{lname}\" uses preset {} \"{}\"", get(prest, fr, "Def"), get(prest, fr, "Name")),
                )
                .detail("The game builds a preset level from the first LvlPrest row with its LevelId. Point that row at this map, or give this map its own level (Add to game, new level).")
                .cols(&[(TableName::LvlPrest, "LevelId")]),
            );
        }
        let (w1, h1) = (size.0 as i64 - 1, size.1 as i64 - 1);
        if num(prest, r, "SizeX") == 0 || num(prest, r, "SizeY") == 0 {
            let sizes: Vec<String> = ["", "(N)", "(H)"]
                .iter()
                .map(|s| {
                    format!(
                        "{}x{}",
                        num(levels, lrow, &format!("SizeX{s}")),
                        num(levels, lrow, &format!("SizeY{s}"))
                    )
                })
                .collect();
            let need = format!("{w1}x{h1}");
            if sizes.iter().any(|s| *s != need) {
                let mut uniq = sizes.clone();
                uniq.dedup();
                let mut cells = Vec::new();
                for s in ["", "(N)", "(H)"] {
                    cells.push(cell(lrow, &format!("SizeX{s}"), w1));
                    cells.push(cell(lrow, &format!("SizeY{s}"), h1));
                }
                let mut f = Finding::new(
                    Severity::Error,
                    Area::Level,
                    format!("Level {level_id} size {} doesn't match the map ({need}): the game crashes building it", uniq.join(" / ")),
                )
                .detail("The level size (Levels SizeX/SizeY, per difficulty) must be the DS1 size minus one, as in every game preset level. D2Common checks it when it builds the level (entering it, or opening a portal to it) and stops there.")
                .cols(&[(TableName::Levels, "SizeX")]);
                if let Some(fx) = cell_fix(
                    TableName::Levels,
                    levels,
                    format!("Set the level size to {need} (all difficulties)"),
                    &cells,
                ) {
                    f = f.fix(fx);
                }
                out.push(f);
            }
        }
        let act = num(levels, lrow, "Act");
        if act != level_act(level_id) {
            let la = level_act(level_id);
            let mut f = Finding::new(
                Severity::Warning,
                Area::Level,
                format!("Level {level_id} says Act {}, but the game treats it as Act {}", act + 1, la + 1),
            )
            .detail("The game takes the act from the level number (1-39 Act 1, 40-74 Act 2, 75-102 Act 3, 103-108 Act 4, 109 and up Act 5), not from the Act column.")
            .cols(&[(TableName::Levels, "Act")]);
            if let Some(fx) = cell_fix(
                TableName::Levels,
                levels,
                format!("Set Act to {la} (Act {})", la + 1),
                &[cell(lrow, "Act", la)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        // Palette.
        let tiles_act: Option<i64> = match env.tiles_act {
            Some(v) => v.map(i64::from),
            None => type_act(types, num(levels, lrow, "LevelType")),
        };
        let pal = num(levels, lrow, "Pal");
        let new_level = env.new_level.is_some_and(|f| f(level_id as u32));
        if level_act(level_id) == 4 && new_level {
            if let Some(Some(art)) = env.tiles_act
                && art != 4
            {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        Area::Level,
                        format!("Level {level_id} is a new level in the Act 5 slot, but its tiles are Act {} tiles", art + 1),
                    )
                    .detail("Seen in PD2: new levels in the Act 5 slot were drawn with the Act 5 palette whatever Pal said, so tiles drawn for another act show wrong colours there. Use tiles drawn for Act 5 or in the colours every act shares.")
                    .cols(&[(TableName::Levels, "Pal")]),
                );
            }
        } else if let Some(ta) = tiles_act
            && pal != ta
        {
            let mut f = Finding::new(
                Severity::Warning,
                Area::Level,
                format!("Level {level_id} uses the Act {} palette but its tiles are Act {} tiles", pal + 1, ta + 1),
            )
            .detail(format!("Pal picks the colours. Tiles drawn for another act show wrong colours (often red or purple). Set Pal to {ta}, as the game's own Act 5 levels that reuse other acts' tiles do."))
            .cols(&[(TableName::Levels, "Pal")]);
            if let Some(fx) = cell_fix(
                TableName::Levels,
                levels,
                format!("Set Pal to {ta} (Act {} colours)", ta + 1),
                &[cell(lrow, "Pal", ta)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        // A quest the level is locked behind: a question asked once.
        let quest = num(levels, lrow, "QuestFlag");
        if quest != 0 {
            let key = format!("quest:{level_id}:{quest}");
            let name = quest_name(quest);
            let remove = cell_fix(
                TableName::Levels,
                levels,
                "No, remove it (special areas only)",
                &[cell(lrow, "QuestFlag", ""), cell(lrow, "QuestFlagEx", "")],
            );
            if env.kept.is_some_and(|k| k(&key)) {
                let mut f = Finding::new(
                    Severity::Info,
                    Area::Level,
                    format!("Players need {name} done to enter (you chose to keep this)"),
                )
                .cols(&[(TableName::Levels, "QuestFlag")]);
                f.fixes.extend(remove);
                out.push(f);
            } else {
                let mut f = Finding::new(
                    Severity::Info,
                    Area::Level,
                    format!("Should players have to finish {name} before entering this map?"),
                )
                .detail(if quest == 39 {
                    "The level is locked until a character has beaten the three Ancients on Mount Arreat (the Rite of Passage quest) in that difficulty. The game's own Worldstone Keep and every PD2 map work this way, so for a typical map keep it."
                } else {
                    "The level is locked until a character has finished the quest in that difficulty (a level copied from another one keeps its requirement). Keep it if that's intended; remove it for an area everyone should reach."
                })
                .cols(&[(TableName::Levels, "QuestFlag")])
                .fix(Fix {
                    label: if quest == 39 { "Yes, keep it (usual for maps)".into() } else { "Yes, keep it".into() },
                    action: FixAction::Keep { key },
                });
                f.fixes.extend(remove);
                out.push(f);
            }
        }
        // The loading screen image: an Act 5 level whose image the game can't open crashes while loading.
        let entry = get(levels, lrow, "EntryFile");
        let no_image = if entry.is_empty() {
            Some("empty")
        } else if env.entry_image_exists.is_some_and(|f| !f(&entry)) {
            Some("missing")
        } else {
            None
        };
        if level_act(level_id) == 4
            && levels.column("EntryFile").is_some()
            && let Some(why) = no_image
        {
            let mut f = Finding::new(
                Severity::Error,
                Area::Level,
                if why == "empty" {
                    format!("Level {level_id}: EntryFile is empty, so the game crashes on the loading screen")
                } else {
                    format!("Level {level_id}: its loading-screen image {entry}.dc6 doesn't exist, so the game crashes on the loading screen")
                },
            )
            .detail(format!("EntryFile is the image shown while the level loads: {ENTRY_IMAGE_DIR}<EntryFile>.dc6 (not a text string). The game halts when it can't open it. Use an image the game has (Harrogath's is {DEFAULT_ENTRY_FILE})."))
            .cols(&[(TableName::Levels, "EntryFile")]);
            if let Some(fx) = cell_fix(
                TableName::Levels,
                levels,
                format!("Use Harrogath's loading screen ({DEFAULT_ENTRY_FILE})"),
                &[cell(lrow, "EntryFile", DEFAULT_ENTRY_FILE)],
            ) {
                f = f.fix(fx);
            }
            out.push(f);
        }
        for c in ["LevelName", "LevelWarp", "EntryFile"] {
            if get(levels, lrow, c).len() > MAX_LEVEL_STRING {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        Area::Level,
                        format!("Level {level_id}: {c} is longer than {MAX_LEVEL_STRING} characters (the game cuts it)"),
                    )
                    .cols(&[(TableName::Levels, c)]),
                );
            }
        }
        // World position.
        let me = level_box(levels, lrow);
        let move_to = |me: &Option<LevelBox>| {
            let size = me.as_ref().map_or((200, 200), |b| (b.w, b.h));
            free_offset(levels, level_act(level_id), size, Some(lrow))
        };
        if let Some(b) = &me {
            let (mx, my) = (b.x + b.w / 2, b.y + b.h / 2);
            if !automap_fits(mx, my, 0, 0, 0) {
                let off = move_to(&me);
                let mut f = Finding::new(
                    Severity::Error,
                    Area::Level,
                    format!("Level {level_id} is too far out in the act's world ({}, {}), so the game crashes after a few steps", b.x, b.y),
                )
                .detail(format!("The automap stores positions in 16 bits: OffsetX - OffsetY must stay within +-{AUTOMAP_MAX_DIFF} tiles and OffsetX + OffsetY up to {AUTOMAP_MAX_SUM} (at this level's middle: {} and {}).", mx - my, mx + my))
                .cols(&[(TableName::Levels, "OffsetX")]);
                if let Some(fx) = cell_fix(
                    TableName::Levels,
                    levels,
                    format!(
                        "Move it to a free spot inside the range ({}, {})",
                        off.0, off.1
                    ),
                    &[cell(lrow, "OffsetX", off.0), cell(lrow, "OffsetY", off.1)],
                ) {
                    f = f.fix(fx);
                }
                out.push(f);
            }
            for other in data_rows(levels) {
                let id = num(levels, other, "Id");
                if other == lrow || level_act(id) != level_act(level_id) {
                    continue;
                }
                let Some(o) = level_box(levels, other) else {
                    continue;
                };
                if b.x < o.x + o.w && o.x < b.x + b.w && b.y < o.y + o.h && o.y < b.y + b.h {
                    let off = move_to(&me);
                    let mut f = Finding::new(
                        Severity::Warning,
                        Area::Level,
                        format!(
                            "Level {level_id} overlaps level {id} \"{}\" in the act's world",
                            get(levels, other, "Name")
                        ),
                    )
                    .detail(
                        "Levels of one act share one world; OffsetX/OffsetY must keep them apart.",
                    )
                    .cols(&[(TableName::Levels, "OffsetX")]);
                    if let Some(fx) = cell_fix(
                        TableName::Levels,
                        levels,
                        format!("Move it to a free spot ({}, {})", off.0, off.1),
                        &[cell(lrow, "OffsetX", off.0), cell(lrow, "OffsetY", off.1)],
                    ) {
                        f = f.fix(fx);
                    }
                    out.push(f);
                    break;
                }
            }
        }
        // Level type files this preset loads.
        if let Some(trow) = row_of_record(types, num(levels, lrow, "LevelType")) {
            let mask = num(prest, r, "Dt1Mask") as u64 as u32;
            for i in 1..=32u32 {
                let f = get(types, trow, &format!("File {i}"));
                if (mask >> (i - 1)) & 1 == 1
                    && !f.is_empty()
                    && f != "0"
                    && let Some(p) = tile_path_problem(&f)
                {
                    if !long_paths.iter().any(|x| norm_path(x) == norm_path(&f)) {
                        long_paths.push(f.clone());
                    }
                    out.push(
                        issue(
                            Severity::Error,
                            format!(
                                "LvlTypes \"{}\" File {i} path too long",
                                get(types, trow, "Name")
                            ),
                        )
                        .detail(p)
                        .cols(&[(TableName::LvlTypes, &format!("File {i}"))]),
                    );
                }
            }
        }
    }
    // Every too-long path together, so one dialog renames them all.
    if !long_paths.is_empty() {
        let label = if long_paths.len() > 1 {
            format!("Rename the {} long paths", long_paths.len())
        } else {
            "Rename it".to_string()
        };
        for f in out.iter_mut().filter(|f| f.title.contains("path too long")) {
            f.fixes.insert(
                0,
                Fix {
                    label: label.clone(),
                    action: FixAction::ShortenPaths {
                        paths: long_paths.clone(),
                    },
                },
            );
        }
    }
    for f in &mut out {
        if f.area == Area::Tables && f.columns.first().is_some_and(|c| c.0 == TableName::Levels) {
            f.area = Area::Level;
        }
        let table = f.columns.first().map_or(TableName::LvlPrest, |c| c.0);
        f.fixes.push(Fix::open(table, None));
    }
    out
}

/// Quest names by the number Levels.txt's `QuestFlag` uses.
pub fn quest_name(n: i64) -> String {
    match n {
        1 => "Den of Evil",
        2 => "Sisters' Burial Grounds",
        3 => "Tools of the Trade",
        4 => "The Search for Cain",
        5 => "The Forgotten Tower",
        6 => "Sisters to the Slaughter (Andariel)",
        9 => "Radament's Lair",
        10 => "The Horadric Staff",
        11 => "The Tainted Sun",
        12 => "The Arcane Sanctuary",
        13 => "The Summoner",
        14 => "The Seven Tombs (Duriel)",
        17 => "Lam Esen's Tome",
        18 => "Khalim's Will",
        19 => "The Blade of the Old Religion",
        20 => "The Golden Bird",
        21 => "The Blackened Temple",
        22 => "The Guardian (Mephisto)",
        25 => "The Fallen Angel",
        26 => "Terror's End (Diablo)",
        27 => "Hell's Forge",
        35 => "Siege on Harrogath",
        36 => "Rescue on Mount Arreat",
        37 => "Prison of Ice",
        38 => "Betrayal of Harrogath",
        39 => "Rite of Passage (the Ancients)",
        40 => "Eve of Destruction (Baal)",
        _ => return format!("quest {n}"),
    }
    .to_string()
}
