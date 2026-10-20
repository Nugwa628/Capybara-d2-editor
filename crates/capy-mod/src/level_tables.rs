//! Level tables (Levels, LvlPrest, LvlTypes): shared primitives for the map
//! registration, level-type and type-package tools.
//!
//! The game reads these tables by row position: a level's `Id`, a preset's
//! `Def` and a level type's `Id` equal the row's index counting data rows only
//! (the `Expansion` separator and blank lines do not count). Rows are therefore
//! only ever appended.

use std::collections::HashMap;

use d2_formats::txt::TxtDoc;

use crate::lvl_error::{Result, refuse};
use crate::txt_util::{
    append_values, cell, cell_num, data_rows, file_value, has, mask, rel_key, row_values, set,
    tiles_rel,
};

/// A tile path the game can hold: `DATA\GLOBAL\TILES\` + path in a 60-byte buffer.
pub const MAX_TILE_PATH: usize = 41;
/// The longest LevelName / LevelWarp the game takes.
pub const MAX_LEVEL_STRING: usize = 39;
/// Levels.txt `Layer` (automap layer): 100 is the highest the game takes.
pub const MAX_LAYER: i64 = 100;
/// Loading-screen image of a new level (Harrogath's), under `data/local/ui/eng/expansion`.
pub const DEFAULT_ENTRY_FILE: &str = "A5L1";
/// The game's towns (the only levels whose LvlPrest `AutoMap` is 1).
pub const TOWNS: [u32; 5] = [1, 40, 75, 103, 109];
/// Automap positions are 16-bit: `|x - y| <= 4095` and `x + y <= 8191` for every corner.
pub const AUTOMAP_MAX_DIFF: i64 = 4095;
/// See [`AUTOMAP_MAX_DIFF`].
pub const AUTOMAP_MAX_SUM: i64 = 8191;

/// The three tables that make the game load a map, plus the optional LvlSub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelTables {
    pub prest: TxtDoc,
    pub levels: TxtDoc,
    pub types: TxtDoc,
    pub lvl_sub: Option<TxtDoc>,
}

/// Which table a change was made in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TableName {
    Levels,
    LvlPrest,
    LvlTypes,
}

impl TableName {
    /// File name under `data\global\excel`.
    pub fn file_name(self) -> &'static str {
        match self {
            TableName::Levels => "Levels.txt",
            TableName::LvlPrest => "LvlPrest.txt",
            TableName::LvlTypes => "LvlTypes.txt",
        }
    }
}

/// One cell (or whole-row) change, for the change list shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange {
    pub table: TableName,
    /// Row label, e.g. `Level 137 "My Map"`.
    pub row: String,
    /// Column, or `(new row)`.
    pub column: String,
    pub from: String,
    pub to: String,
    pub why: Option<String>,
}

impl FieldChange {
    /// `row: column from -> to (why)`.
    pub fn summary(&self) -> String {
        let from = if self.from.is_empty() {
            "(empty)"
        } else {
            &self.from
        };
        let why = self
            .why
            .as_ref()
            .map_or(String::new(), |w| format!(" ({w})"));
        format!(
            "{}: {} {} -> {}{}",
            self.row, self.column, from, self.to, why
        )
    }
}

/// Act (0-based) of a level, from its number: 109 on is Act 5.
pub fn level_act(id: u32) -> u8 {
    match id {
        109.. => 4,
        103.. => 3,
        75.. => 2,
        40.. => 1,
        _ => 0,
    }
}

/// A tile path the game cannot hold, with the reason; None when fine.
pub fn tile_path_problem(rel: &str) -> Option<String> {
    (rel.chars().count() > MAX_TILE_PATH).then(|| {
        format!(
            "\"{rel}\" is {} characters. The game copies tile paths into a 60-byte buffer after DATA\\GLOBAL\\TILES\\, so it holds at most {MAX_TILE_PATH}: a longer one can crash the game or fail to load. Use shorter folder or file names.",
            rel.chars().count()
        )
    })
}

/// Rows of LvlPrest that list `map` (any path form) in File1..File6.
pub fn preset_rows_for(prest: &TxtDoc, map: &str) -> Vec<usize> {
    let want = rel_key(map);
    (0..prest.row_count())
        .filter(|&r| (1..=6).any(|f| rel_key(&cell(prest, r, &format!("File{f}"))) == want))
        .collect()
}

/// The act (0-4) a level type's tiles belong to (LvlTypes `Act` is 1-5).
pub fn type_act(types: &TxtDoc, type_id: u32) -> Option<u8> {
    let row = crate::txt_util::row_of_record(types, type_id as usize)?;
    let act = cell_num(types, row, "Act");
    (1..=5).contains(&act).then(|| (act - 1) as u8)
}

/// Rows of the levels other than `except` whose LevelType is `type_id`.
pub fn levels_of_type(levels: &TxtDoc, type_id: u32, except: Option<u32>) -> Vec<usize> {
    data_rows(levels)
        .into_iter()
        .filter(|&r| {
            cell_num(levels, r, "LevelType") == i64::from(type_id)
                && except.map(i64::from) != Some(cell_num(levels, r, "Id"))
        })
        .collect()
}

/// The level and level type a map's LvlPrest row belongs to (`fallback_type`
/// for shared presets, LevelId 0).
pub fn map_level_type(tables: &LevelTables, map: &str, fallback_type: Option<u32>) -> (u32, u32) {
    let rows = preset_rows_for(&tables.prest, map);
    let level_id = rows
        .first()
        .map_or(0, |&r| cell_num(&tables.prest, r, "LevelId")) as u32;
    let mut type_id = fallback_type.unwrap_or(0);
    if level_id != 0
        && let Some(lv) = data_rows(&tables.levels)
            .into_iter()
            .find(|&r| cell_num(&tables.levels, r, "Id") == i64::from(level_id))
    {
        type_id = cell_num(&tables.levels, lv, "LevelType") as u32;
    }
    (level_id, type_id)
}

/// The path of File slot `i` (1-32) of a LvlTypes row, normalised; None if empty.
pub fn slot_key(types: &TxtDoc, row: usize, i: usize) -> Option<String> {
    file_value(&cell(types, row, &format!("File {i}"))).map(|v| rel_key(&v))
}

/// Slots (1-32) of a LvlTypes row that hold a file.
pub fn filled_slots(types: &TxtDoc, row: usize) -> Vec<usize> {
    (1..=32)
        .filter(|&i| file_value(&cell(types, row, &format!("File {i}"))).is_some())
        .collect()
}

/// A DT1 path the way LvlTypes stores it: relative to data/global/tiles, case
/// kept, in the row's slash style.
fn as_slot_value(types: &TxtDoc, row: usize, path: &str) -> String {
    let rel = tiles_rel(path);
    let backslash = (1..=32).any(|i| cell(types, row, &format!("File {i}")).contains('\\'));
    if backslash {
        rel.replace('/', "\\")
    } else {
        rel
    }
}

/// A DT1 put into a free File slot by [`ensure_type_slots`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotAdd {
    pub slot: usize,
    pub value: String,
}

/// Slot (1-based) of every DT1 (key: [`rel_key`] form) and the slots newly filled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    pub slots: HashMap<String, usize>,
    pub added: Vec<SlotAdd>,
}

/// Makes a LvlTypes row load `dt1s`: each DT1 already in a File slot is reused,
/// others go into free slots. Refuses (leaving `types` as it was) when the row
/// runs out of slots.
pub fn ensure_type_slots(types: &mut TxtDoc, type_row: usize, dt1s: &[String]) -> Result<Slots> {
    let mut work = types.clone();
    let mut out = Slots::default();
    for dt1 in dt1s {
        let key = rel_key(dt1);
        let mut slot = (1..=32).find(|&i| slot_key(&work, type_row, i).as_deref() == Some(&key));
        if slot.is_none() {
            slot = (1..=32).find(|&i| {
                has(&work, &format!("File {i}")) && slot_key(&work, type_row, i).is_none()
            });
            let Some(s) = slot else {
                return refuse(format!(
                    "LvlTypes \"{}\" has no free File slot for {} (all 32 are used)",
                    cell(&work, type_row, "Name"),
                    tiles_rel(dt1)
                ));
            };
            let value = as_slot_value(&work, type_row, dt1);
            set(&mut work, type_row, &format!("File {s}"), &value)?;
            out.added.push(SlotAdd { slot: s, value });
        }
        out.slots.insert(key, slot.unwrap_or(0));
    }
    *types = work;
    Ok(out)
}

/// The Dt1Mask bits of a set of slots.
pub fn mask_of(slots: &HashMap<String, usize>) -> u32 {
    slots.values().fold(0u32, |m, &s| m | (1u32 << (s - 1)))
}

/// The Dt1Mask a preset needs for `dt1s`, starting from its current mask: bits
/// of slots holding one of the DT1s stay on, a DT1 not yet selected turns on
/// its first slot, bits of slots holding other DT1s turn off, bits of empty
/// slots are left alone. An unchanged mask comes back unchanged.
pub fn mask_for(types: &TxtDoc, type_row: usize, dt1s: &[String], old: u32) -> u32 {
    let want: std::collections::HashSet<String> = dt1s.iter().map(|d| rel_key(d)).collect();
    let mut covered = std::collections::HashSet::new();
    let mut out = 0u32;
    for i in 1..=32usize {
        let bit = 1u32 << (i - 1);
        match slot_key(types, type_row, i) {
            None => out |= old & bit,
            Some(k) => {
                if want.contains(&k) && old & bit != 0 {
                    out |= bit;
                    covered.insert(k);
                }
            }
        }
    }
    for i in 1..=32usize {
        if let Some(k) = slot_key(types, type_row, i)
            && want.contains(&k)
            && !covered.contains(&k)
        {
            out |= 1 << (i - 1);
            covered.insert(k);
        }
    }
    out
}

/// A level type of the map's own, appended as the next record: File 1... are
/// `dt1s`, every other column (Act, Expansion...) is copied from `from_row`.
pub struct OwnType {
    pub type_row: usize,
    pub type_id: u32,
    pub slots: HashMap<String, usize>,
}

/// See [`OwnType`].
pub fn append_own_type(
    types: &mut TxtDoc,
    from_row: usize,
    name: &str,
    dt1s: &[String],
) -> Result<OwnType> {
    let mut files: Vec<String> = Vec::new();
    for p in dt1s {
        if !files.iter().any(|f| rel_key(f) == rel_key(p)) {
            files.push(tiles_rel(p));
        }
    }
    if files.len() > 32 {
        return refuse(format!(
            "The map uses {} tile libraries; a level type holds at most 32.",
            files.len()
        ));
    }
    let type_id = data_rows(types).len() as u32;
    let mut values = row_values(types, from_row);
    let mut put = |k: &str, v: String| {
        if let Some(e) = values.iter_mut().find(|(c, _)| c.eq_ignore_ascii_case(k)) {
            e.1 = v;
        }
    };
    put("Name", name.to_owned());
    put("Id", type_id.to_string());
    for i in 1..=32usize {
        put(
            &format!("File {i}"),
            files.get(i - 1).cloned().unwrap_or_else(|| "0".into()),
        );
    }
    let type_row = append_values(types, &values)?;
    let slots = files
        .iter()
        .enumerate()
        .map(|(i, f)| (rel_key(f), i + 1))
        .collect();
    Ok(OwnType {
        type_row,
        type_id,
        slots,
    })
}

/// Whether a level at (x, y) sized w*h tiles stays inside the automap's range
/// (less `margin` tiles).
pub fn automap_fits(x: i64, y: i64, w: i64, h: i64, margin: i64) -> bool {
    x >= 0
        && y >= 0
        && x + w - y <= AUTOMAP_MAX_DIFF - margin
        && y + h - x <= AUTOMAP_MAX_DIFF - margin
        && x + w + y + h <= AUTOMAP_MAX_SUM - margin
}

fn level_box(levels: &TxtDoc, row: usize) -> Option<(i64, i64, i64, i64)> {
    let x = cell_num(levels, row, "OffsetX");
    let y = cell_num(levels, row, "OffsetY");
    let w = ["SizeX", "SizeX(N)", "SizeX(H)"]
        .iter()
        .map(|c| cell_num(levels, row, c))
        .max()?;
    let h = ["SizeY", "SizeY(N)", "SizeY(H)"]
        .iter()
        .map(|c| cell_num(levels, row, c))
        .max()?;
    (x >= 0 && y >= 0 && cell_num(levels, row, "Depend") == 0 && w > 0).then_some((x, y, w, h))
}

/// Suggested world position for a level of `size` (w, h) in an act: clear of
/// every other level there by 100 tiles, 100 tiles inside the automap's range,
/// as far out as that allows. `except` is the level's own row when moving it.
pub fn free_offset(
    levels: &TxtDoc,
    act: u8,
    size: (i64, i64),
    except: Option<usize>,
) -> (i64, i64) {
    let gap = 100;
    let boxes: Vec<_> = data_rows(levels)
        .into_iter()
        .filter(|&r| Some(r) != except && level_act(cell_num(levels, r, "Id") as u32) == act)
        .filter_map(|r| level_box(levels, r))
        .collect();
    let mut best: Option<(i64, i64)> = None;
    let mut y = 0;
    while y <= AUTOMAP_MAX_SUM {
        let mut x = 0;
        while x <= AUTOMAP_MAX_SUM {
            let ok = automap_fits(x, y, size.0, size.1, 100)
                && !boxes.iter().any(|&(bx, by, bw, bh)| {
                    x < bx + bw + gap
                        && bx < x + size.0 + gap
                        && y < by + bh + gap
                        && by < y + size.1 + gap
                });
            if ok && best.is_none_or(|(bx, by)| x + y > bx + by || (x + y == bx + by && x > bx)) {
                best = Some((x, y));
            }
            x += 50;
        }
        y += 50;
    }
    best.unwrap_or((1000, 1000))
}

/// A layer for a new level: the one most mod-added preset levels (Id >= 137,
/// DrlgType 2) share, else 98 (PD2's maps).
pub fn safe_layer(levels: &TxtDoc) -> i64 {
    let mut count: std::collections::BTreeMap<i64, usize> = Default::default();
    for r in data_rows(levels) {
        let l = cell_num(levels, r, "Layer");
        if cell_num(levels, r, "Id") >= 137
            && cell_num(levels, r, "DrlgType") == 2
            && l <= MAX_LAYER
        {
            *count.entry(l).or_default() += 1;
        }
    }
    count
        .into_iter()
        .max_by_key(|&(l, n)| (n, std::cmp::Reverse(l)))
        .map_or(98, |(l, _)| l)
}

/// Dt1Mask of a preset row.
pub(crate) fn preset_mask(prest: &TxtDoc, row: usize) -> u32 {
    mask(&cell(prest, row, "Dt1Mask"))
}
