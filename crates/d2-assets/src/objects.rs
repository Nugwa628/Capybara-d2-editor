//! Resolving DS1 objects to what is needed to draw and name them.
//!
//! A DS1 object record is `(type, id)`. The game turns it into a unit like so
//! (see `docs/specs/ds1.md` section 8 and the DS1Edit guide
//! `monsters-and-objects.md`):
//!
//! * **Type 2 (objects).** DS1 version 6 and later: `id < 150` selects
//!   `table[act * 150 + id]` (the hardcoded table of `D2Common.dll`, shipped as
//!   `objpreset.bin` by Median XL), `id >= 150` is the `Objects.txt` row
//!   `id - 150`. Below version 6 the id is the `Objects.txt` row itself.
//! * **Type 1 (monsters and NPCs).** DS1 version 5 and later: `id` is a row of
//!   the act's block of `MonPreset.txt` when it is smaller than the block's row
//!   count, otherwise it is a direct `MonStats.txt` row. Below version 5 the id
//!   is used as a `MonStats.txt` row. A few resulting monster rows are
//!   rewritten into objects by act (natalya, the compelling orb, nihlathak and
//!   the ancient statues).
//!
//! Vanilla has no ObjPreset table, so the hardcoded one is reproduced by the
//! DS1Edit `obj.txt` (MIT, credited in `NOTICE`), which is embedded here. Its
//! rows also carry the sprite tokens the editor draws.
//!
//! The `MonPreset`, `MonStats` and `MonStats2` tables are read from `.txt`
//! only; the `.bin` forms have no decoder yet.
// TODO(excel-mon): decode monpreset/monstats/monstats2 .bin in d2-formats and
// load them here through `tables::load_excel`; until then profiles that ship
// only `.bin` (Median XL) resolve type 1 from the embedded `obj.txt`.

use std::collections::HashMap;

use d2_formats::ds1::Object;
use d2_formats::excel::ExcelTableKind;
use d2_formats::tbl::StringTable;
use d2_formats::txt::{Row, TxtTable};

use crate::error::{AssetError, Result};
use crate::overlay::Overlay;
use crate::path::GamePath;
use crate::profile::GameVersion;
use crate::tables::load_excel;

/// DS1Edit's object table, MIT licensed (see `NOTICE`). Latin-1 text.
const EMBEDDED_OBJ_TXT: &[u8] = include_bytes!("../../../assets/editor/obj.txt");

const EXCEL_DIR: &str = r"data\global\excel\";
const LNG_DIR: &str = r"data\local\lng\eng\";
/// String tables in lookup priority order.
const STRING_TABLES: [&str; 3] = ["patchstring.tbl", "expansionstring.tbl", "string.tbl"];

/// Component codes in COF layer order.
const PART_CODES: [&str; 16] = [
    "HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8",
];
/// Object modes by `Objects.txt` column index.
const MODE_CODES: [&str; 8] = ["NU", "OP", "ON", "S1", "S2", "S3", "S4", "S5"];

const OBJECTS_PER_ACT: i64 = 150;
const ACTS: usize = 5;
const SOURCE_OBJPRESET: &str = "objpreset";
const SOURCE_OBJ_TXT: &str = "obj.txt";
const SOURCE_MONPRESET: &str = "monpreset";
const SOURCE_UNRESOLVED: &str = "unresolved";

/// Monster rows that the game turns into objects, by DS1 act (0-based):
/// `(act, MonStats row, Objects row)`.
const MONSTER_TO_OBJECT: [(usize, usize, usize); 6] = [
    (2, 297, 382),
    (2, 366, 404),
    (4, 514, 461),
    (4, 537, 476),
    (4, 538, 475),
    (4, 539, 474),
];

/// What kind of thing a DS1 object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectClass {
    /// DS1 type 1 (monsters and NPCs).
    Npc,
    /// DS1 type 2 (objects), or a monster the game rewrites into an object.
    Object,
    /// Any other DS1 type.
    Other(i32),
}

/// Which sprite tree an object's token lives under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpriteBase {
    /// `data\global\objects`
    Objects,
    /// `data\global\monsters`
    Monsters,
}

/// Everything known about one DS1 object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub class: ObjectClass,
    pub ds1_kind: i32,
    pub ds1_id: i32,
    /// Objects.txt row (type 2) or MonStats row (type 1) when known.
    pub row: Option<u32>,
    /// Display name: string-table lookup of the row's name key when possible,
    /// else the table's Name column, else "type N id M".
    pub name: String,
    pub sprite_base: Option<SpriteBase>,
    /// COF token, e.g. "TO", "AF".
    pub token: Option<String>,
    /// Mode token used for drawing (objects: per Objects.txt/obj.txt; monsters "NU").
    pub mode: String,
    /// "HTH" unless the data says otherwise.
    pub weapon_class: String,
    /// Objects.txt OrderFlag of that mode (0/1/2); monsters 0.
    pub order_flag: u8,
    /// Enabled components (HD TR LG RA LA RH LH SH S1..S8).
    pub parts: Vec<String>,
    /// SizeX/SizeY.
    pub size: (i32, i32),
    /// Xoffset/Yoffset.
    pub offset: (i32, i32),
    /// Object-level transparency/blend if the table has one.
    pub draw_effect: Option<u8>,
    /// FrameCnt of the mode.
    pub frame_count: Option<u32>,
    /// FrameDelta of the mode (animation speed, 256 = 1 frame per tick).
    pub frame_delta: Option<u32>,
    /// `"objpreset"`, `"obj.txt"`, `"monpreset"` or `"unresolved"`.
    pub source: &'static str,
}

impl ObjectInfo {
    fn blank(kind: i32, id: i32, source: &'static str) -> ObjectInfo {
        ObjectInfo {
            class: match kind {
                1 => ObjectClass::Npc,
                2 => ObjectClass::Object,
                other => ObjectClass::Other(other),
            },
            ds1_kind: kind,
            ds1_id: id,
            row: None,
            name: format!("type {kind} id {id}"),
            sprite_base: None,
            token: None,
            mode: "NU".to_owned(),
            weapon_class: "HTH".to_owned(),
            order_flag: 0,
            parts: Vec::new(),
            size: (0, 0),
            offset: (0, 0),
            draw_effect: None,
            frame_count: None,
            frame_delta: None,
            source,
        }
    }

    fn unresolved(kind: i32, id: i32) -> ObjectInfo {
        ObjectInfo::blank(kind, id, SOURCE_UNRESOLVED)
    }
}

// ---------------------------------------------------------------------------
// parsed tables

/// One `Objects.txt` row, reduced to what drawing needs.
#[derive(Clone, Debug, Default)]
struct ObjRow {
    name_key: String,
    description: String,
    token: String,
    size: (i32, i32),
    offset: (i32, i32),
    frame_count: [u32; 8],
    frame_delta: [u32; 8],
    order_flag: [u8; 8],
    trans: u8,
    parts: Vec<String>,
}

/// One `MonStats.txt` row.
#[derive(Clone, Debug, Default)]
struct MonRow {
    id: String,
    name_key: String,
    code: String,
    stats_ex: String,
}

/// One `MonStats2.txt` row.
#[derive(Clone, Debug, Default)]
struct Mon2Row {
    parts: Vec<String>,
    base_weapon: String,
    size: (i32, i32),
}

/// One `SuperUniques.txt` row.
#[derive(Clone, Debug, Default)]
struct SuperRow {
    name_key: String,
    class: String,
}

#[derive(Clone, Debug)]
struct PresetRow {
    place: String,
}

/// `MonPreset`, `MonStats`, `MonStats2` and `SuperUniques` as `.txt`.
#[derive(Debug, Default)]
struct MonTables {
    /// All MonPreset rows in file order.
    preset: Vec<PresetRow>,
    /// First flat index of each act's block, and its row count.
    act_base: [usize; ACTS],
    act_len: [usize; ACTS],
    stats: Vec<MonRow>,
    /// Lower-cased `Id` to row (first match wins).
    stats_by_id: HashMap<String, usize>,
    /// Lower-cased `Id` to MonStats2 row data.
    stats2: HashMap<String, Mon2Row>,
    /// Lower-cased `Superunique` name to its row.
    supers: HashMap<String, SuperRow>,
}

/// One row of the embedded DS1Edit `obj.txt`.
#[derive(Clone, Debug, Default)]
struct EditRow {
    description: String,
    objects_id: Option<u32>,
    /// All of Base, Token, Mode and Class were given (what DS1Edit needs).
    visual: Option<EditVisual>,
}

#[derive(Clone, Debug)]
struct EditVisual {
    base: SpriteBase,
    token: String,
    mode: String,
    class: String,
    parts: Vec<String>,
}

#[derive(Debug, Default)]
struct EditTable {
    rows: Vec<EditRow>,
    /// `(act 1..=5, type, id)` to row (first match wins).
    by_key: HashMap<(i32, i32, i32), usize>,
    /// Objects row to the first type-2 row that has a visual for it.
    by_object: HashMap<u32, usize>,
}

// ---------------------------------------------------------------------------
// table parsing helpers

fn trimmed<'a>(row: Row<'a>, column: &str) -> &'a str {
    row.get(column).map_or("", str::trim)
}

fn num(row: Row<'_>, column: &str) -> i64 {
    row.get_i64(column).unwrap_or(0)
}

fn to_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(0)
}

fn to_u32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn to_u8(value: i64) -> u8 {
    u8::try_from(value).unwrap_or(0)
}

/// The vanilla `.txt` tables carry a separator row named "Expansion" that the
/// game skips; row indices count the other rows only.
fn is_expansion_marker(row: Row<'_>) -> bool {
    let cells = row.cells();
    cells
        .first()
        .is_some_and(|c| c.trim().eq_ignore_ascii_case("expansion"))
        && cells.iter().skip(1).all(|c| c.trim().is_empty())
}

fn data_rows(table: &TxtTable) -> impl Iterator<Item = Row<'_>> {
    table.rows().filter(|row| !is_expansion_marker(*row))
}

fn flagged_parts(row: Row<'_>) -> Vec<String> {
    PART_CODES
        .iter()
        .filter(|code| num(row, code) != 0)
        .map(|code| (*code).to_owned())
        .collect()
}

fn indexed<T: Copy + Default, const N: usize>(
    row: Row<'_>,
    prefix: &str,
    convert: fn(i64) -> T,
) -> [T; N] {
    let mut out = [T::default(); N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = convert(num(row, &format!("{prefix}{i}")));
    }
    out
}

fn parse_objects(table: &TxtTable) -> Vec<ObjRow> {
    data_rows(table)
        .map(|row| ObjRow {
            name_key: trimmed(row, "Name").to_owned(),
            description: trimmed(row, "description - not loaded").to_owned(),
            token: trimmed(row, "Token").to_owned(),
            size: (to_i32(num(row, "SizeX")), to_i32(num(row, "SizeY"))),
            offset: (to_i32(num(row, "Xoffset")), to_i32(num(row, "Yoffset"))),
            frame_count: indexed(row, "FrameCnt", to_u32),
            frame_delta: indexed(row, "FrameDelta", to_u32),
            order_flag: indexed(row, "OrderFlag", to_u8),
            trans: to_u8(num(row, "Trans")),
            parts: flagged_parts(row),
        })
        .collect()
}

fn parse_objpreset(table: &TxtTable) -> Vec<u32> {
    let total = ACTS * OBJECTS_PER_ACT as usize;
    let mut out = vec![0u32; total];
    for row in table.rows() {
        let index = match (row.get_i64("Act"), row.get_i64("Id")) {
            (Some(act), Some(id)) => act * OBJECTS_PER_ACT + id,
            _ => i64::try_from(row.index()).unwrap_or(-1),
        };
        if let Some(slot) = usize::try_from(index).ok().and_then(|i| out.get_mut(i)) {
            *slot = to_u32(num(row, "ObjectsId"));
        }
    }
    out
}

fn parse_mon_tables(
    preset: &TxtTable,
    stats: &TxtTable,
    stats2: Option<&TxtTable>,
    supers: Option<&TxtTable>,
) -> MonTables {
    let mut tables = MonTables::default();
    let mut act_seen = [false; ACTS];
    for (flat, row) in preset.rows().enumerate() {
        let act = row
            .get_i64("Act")
            .and_then(|a| usize::try_from(a - 1).ok())
            .filter(|a| *a < ACTS);
        if let Some(act) = act {
            if let Some(seen) = act_seen.get_mut(act)
                && !*seen
            {
                *seen = true;
                if let Some(base) = tables.act_base.get_mut(act) {
                    *base = flat;
                }
            }
            if let Some(len) = tables.act_len.get_mut(act) {
                *len += 1;
            }
        }
        tables.preset.push(PresetRow {
            place: trimmed(row, "Place").to_owned(),
        });
    }
    for row in data_rows(stats) {
        let id = trimmed(row, "Id").to_owned();
        tables
            .stats_by_id
            .entry(id.to_ascii_lowercase())
            .or_insert(tables.stats.len());
        tables.stats.push(MonRow {
            id,
            name_key: trimmed(row, "NameStr").to_owned(),
            code: trimmed(row, "Code").to_owned(),
            stats_ex: trimmed(row, "MonStatsEx").to_owned(),
        });
    }
    if let Some(stats2) = stats2 {
        for row in data_rows(stats2) {
            tables
                .stats2
                .entry(trimmed(row, "Id").to_ascii_lowercase())
                .or_insert_with(|| Mon2Row {
                    parts: flagged_parts(row),
                    base_weapon: trimmed(row, "BaseW").to_owned(),
                    size: (to_i32(num(row, "SizeX")), to_i32(num(row, "SizeY"))),
                });
        }
    }
    if let Some(supers) = supers {
        for row in data_rows(supers) {
            tables
                .supers
                .entry(trimmed(row, "Superunique").to_ascii_lowercase())
                .or_insert_with(|| SuperRow {
                    name_key: trimmed(row, "Name").to_owned(),
                    class: trimmed(row, "Class").to_owned(),
                });
        }
    }
    tables
}

fn parse_edit_table(bytes: &[u8]) -> EditTable {
    let mut out = EditTable::default();
    let Ok(table) = TxtTable::parse(bytes) else {
        return out;
    };
    for row in table.rows() {
        let (act, kind, id) = (
            to_i32(num(row, "Act")),
            to_i32(num(row, "Type")),
            to_i32(num(row, "Id")),
        );
        if act == 0 && kind == 0 {
            continue;
        }
        let objects_id = row
            .get_i64("Objects.txt_ID")
            .filter(|v| *v > 0)
            .and_then(|v| u32::try_from(v).ok());
        let base = match trimmed(row, "Base").to_ascii_lowercase().as_str() {
            b if b.ends_with("monsters") => Some(SpriteBase::Monsters),
            b if b.ends_with("objects") => Some(SpriteBase::Objects),
            _ => None,
        };
        let (token, mode, class) = (
            trimmed(row, "Token"),
            trimmed(row, "Mode"),
            trimmed(row, "Class"),
        );
        let visual = match base {
            Some(base) if !token.is_empty() && !mode.is_empty() && !class.is_empty() => {
                Some(EditVisual {
                    base,
                    token: token.to_owned(),
                    mode: mode.to_ascii_uppercase(),
                    class: class.to_owned(),
                    parts: PART_CODES
                        .iter()
                        .filter(|code| !trimmed(row, code).is_empty())
                        .map(|code| (*code).to_owned())
                        .collect(),
                })
            }
            _ => None,
        };
        let index = out.rows.len();
        out.by_key.entry((act, kind, id)).or_insert(index);
        if kind == 2
            && visual.is_some()
            && let Some(object) = objects_id
        {
            out.by_object.entry(object).or_insert(index);
        }
        out.rows.push(EditRow {
            description: trimmed(row, "Description")
                .trim_start_matches('#')
                .trim()
                .to_owned(),
            objects_id,
            visual,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// table loading

/// Whether `err` just means that the table is not in the overlay.
fn is_absent(err: &AssetError) -> bool {
    matches!(err, AssetError::Profile { .. })
}

/// Reads a table that only exists as `.txt` for us. `Ok(None)` when it is not
/// usable, with the reason pushed to `warnings`.
fn load_txt_only(
    overlay: &Overlay,
    stem: &str,
    version: GameVersion,
    warnings: &mut Vec<String>,
) -> Option<TxtTable> {
    let txt_path = GamePath::new(&format!("{EXCEL_DIR}{stem}.txt"));
    let bin_path = GamePath::new(&format!("{EXCEL_DIR}{stem}.bin"));
    let has_txt = overlay.resolve(&txt_path).is_some();
    let has_bin = version.excel().is_some() && overlay.resolve(&bin_path).is_some();
    // Same preference as `tables::load_excel`: 1.13c reads `.bin` first.
    let use_txt = match version {
        GameVersion::V113c => has_txt && !has_bin,
        GameVersion::V114d | GameVersion::D2r => has_txt,
    };
    if use_txt {
        return match overlay.read_bytes(&txt_path) {
            Ok(Some(bytes)) => match TxtTable::parse(&bytes) {
                Ok(table) => Some(table),
                Err(err) => {
                    warnings.push(format!("{stem}.txt cannot be parsed: {err}"));
                    None
                }
            },
            Ok(None) => None,
            Err(err) => {
                warnings.push(format!("{stem}.txt cannot be read: {err}"));
                None
            }
        };
    }
    if has_bin {
        warnings.push(format!(
            "{stem}.bin cannot be decoded yet (TODO(excel-mon))"
        ));
    } else {
        warnings.push(format!("{stem} is not in the overlay"));
    }
    None
}

fn load_strings(overlay: &Overlay, warnings: &mut Vec<String>) -> Vec<StringTable> {
    let mut out = Vec::new();
    for name in STRING_TABLES {
        let path = GamePath::new(&format!("{LNG_DIR}{name}"));
        match overlay.read_bytes(&path) {
            Ok(Some(bytes)) => match StringTable::parse(&bytes) {
                Ok(table) => out.push(table),
                Err(err) => warnings.push(format!("{name} cannot be parsed: {err}")),
            },
            Ok(None) => {}
            Err(err) => warnings.push(format!("{name} cannot be read: {err}")),
        }
    }
    if out.is_empty() {
        warnings.push("no string tables found; names fall back to table columns".to_owned());
    }
    out
}

// ---------------------------------------------------------------------------
// the catalog

/// Result of the monster lookup before sprite data is attached.
enum MonTarget {
    /// A MonStats row.
    Stats {
        row: usize,
        /// Set when the row came through a SuperUniques entry: its name key.
        super_name: Option<String>,
    },
    /// A MonPreset `Place` that is neither a superunique nor a monster
    /// (a MonPlace group marker), with the text of the place.
    Place(String),
}

/// Resolves DS1 objects against the loaded game tables.
pub struct ObjectCatalog {
    objects: Option<Vec<ObjRow>>,
    /// `table[act * 150 + id]` when the profile ships an ObjPreset table.
    objpreset: Option<Vec<u32>>,
    mon: Option<MonTables>,
    edit: EditTable,
    strings: Vec<StringTable>,
}

impl ObjectCatalog {
    /// Loads Objects, ObjPreset (when the profile has it; else the embedded
    /// MIT `obj.txt` from DS1Edit, credited in NOTICE), MonPreset/MonStats/
    /// MonStats2 when available as .txt or decodable .bin, and string tables.
    /// Missing tables become warnings, never errors.
    pub fn build(overlay: &Overlay, version: GameVersion) -> Result<(ObjectCatalog, Vec<String>)> {
        let mut warnings = Vec::new();

        let objects = match load_excel(overlay, ExcelTableKind::Objects, version) {
            Ok(table) => Some(parse_objects(&table)),
            Err(err) => {
                warnings.push(format!("Objects table unavailable: {err}"));
                None
            }
        };
        let objpreset = match load_excel(overlay, ExcelTableKind::ObjPreset, version) {
            Ok(table) => Some(parse_objpreset(&table)),
            Err(err) => {
                if !is_absent(&err) {
                    warnings.push(format!("ObjPreset table unusable: {err}"));
                }
                None
            }
        };
        if objpreset.is_none() {
            warnings
                .push("no ObjPreset table; type 2 ids use the embedded DS1Edit obj.txt".to_owned());
        }

        let preset = load_txt_only(overlay, "monpreset", version, &mut warnings);
        let stats = load_txt_only(overlay, "monstats", version, &mut warnings);
        let stats2 = load_txt_only(overlay, "monstats2", version, &mut warnings);
        let supers = load_txt_only(overlay, "superuniques", version, &mut warnings);
        let mon = match (preset, stats) {
            (Some(preset), Some(stats)) => Some(parse_mon_tables(
                &preset,
                &stats,
                stats2.as_ref(),
                supers.as_ref(),
            )),
            _ => {
                warnings.push(
                    "MonPreset/MonStats unavailable; type 1 resolves from the embedded obj.txt \
                     only, ids without a token there are unresolved"
                        .to_owned(),
                );
                None
            }
        };

        let strings = load_strings(overlay, &mut warnings);
        let catalog = ObjectCatalog {
            objects,
            objpreset,
            mon,
            edit: parse_edit_table(EMBEDDED_OBJ_TXT),
            strings,
        };
        Ok((catalog, warnings))
    }

    /// `act` is the DS1's effective act (0..=4); `ds1_version` selects the
    /// pre-v5/pre-v6 id rules from docs/specs/ds1.md.
    pub fn resolve(&self, act: u8, ds1_version: i32, object: &Object) -> ObjectInfo {
        let act = usize::from(act).min(ACTS - 1);
        match object.kind {
            1 => self.resolve_monster(act, ds1_version, object.id),
            2 => self.resolve_object(act, ds1_version, object.id),
            kind => ObjectInfo::unresolved(kind, object.id),
        }
    }

    /// Every placeable (kind, id) for an act, for the object picker.
    pub fn placeable(&self, act: u8) -> Vec<ObjectInfo> {
        let act = act.min(ACTS as u8 - 1);
        const CURRENT: i32 = 18;
        let mut out = Vec::new();
        let push = |kind: i32, id: i32, out: &mut Vec<ObjectInfo>| {
            let info = self.resolve(
                act,
                CURRENT,
                &Object {
                    kind,
                    id,
                    x: 0,
                    y: 0,
                    flags: 0,
                },
            );
            if info.source != SOURCE_UNRESOLVED {
                out.push(info);
            }
        };

        // Type 1: the act's preset block, then every MonStats row.
        if let Some(mon) = &self.mon {
            let ids = mon
                .act_len
                .get(usize::from(act))
                .copied()
                .unwrap_or(0)
                .max(mon.stats.len());
            for id in 0..ids {
                push(1, i32::try_from(id).unwrap_or(i32::MAX), &mut out);
            }
        } else {
            let mut ids: Vec<i32> = self
                .edit
                .by_key
                .keys()
                .filter(|(a, kind, _)| *a == i32::from(act) + 1 && *kind == 1)
                .map(|(_, _, id)| *id)
                .collect();
            ids.sort_unstable();
            for id in ids {
                push(1, id, &mut out);
            }
        }

        // Type 2: the act's 150 slots, then the rest of Objects.txt by direct index.
        let mut covered = std::collections::HashSet::new();
        for id in 0..OBJECTS_PER_ACT as i32 {
            let before = out.len();
            push(2, id, &mut out);
            if out.len() > before
                && let Some(row) = out.last().and_then(|i| i.row)
            {
                covered.insert(row);
            }
        }
        if let Some(objects) = &self.objects {
            for row in 1..objects.len() {
                let Ok(row) = u32::try_from(row) else { break };
                if !covered.contains(&row) {
                    let id = i32::try_from(i64::from(row) + OBJECTS_PER_ACT).unwrap_or(i32::MAX);
                    push(2, id, &mut out);
                }
            }
        }
        out
    }

    // ----- names

    fn text(&self, key: &str) -> Option<&str> {
        if key.is_empty() {
            return None;
        }
        self.strings
            .iter()
            .find_map(|table| table.get(key))
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }

    /// The key "dummy" is a placeholder (its string is "an evil force");
    /// prefer `fallback` for it.
    fn pick_name(&self, key: &str, fallback: &str) -> String {
        if !key.eq_ignore_ascii_case("dummy") {
            let candidates = [self.text(key), (!key.is_empty()).then_some(key)];
            if let Some(name) = candidates.into_iter().flatten().next() {
                return name.to_owned();
            }
        }
        if fallback.is_empty() {
            key.to_owned()
        } else {
            fallback.to_owned()
        }
    }

    // ----- type 2

    fn resolve_object(&self, act: usize, ds1_version: i32, id: i32) -> ObjectInfo {
        let unresolved = || ObjectInfo::unresolved(2, id);
        let source = if self.objpreset.is_some() {
            SOURCE_OBJPRESET
        } else {
            SOURCE_OBJ_TXT
        };
        let remap_source;
        let objects_id: i64;
        let mut edit_row: Option<&EditRow> = None;

        if ds1_version < 6 {
            // No remap below version 6: the id is the Objects.txt row.
            remap_source = source;
            objects_id = i64::from(id);
        } else if id >= OBJECTS_PER_ACT as i32 {
            remap_source = source;
            objects_id = i64::from(id) - OBJECTS_PER_ACT;
        } else {
            // A negative id reads backwards into the previous acts' slots.
            let slot = act as i64 * OBJECTS_PER_ACT + i64::from(id);
            let Ok(slot_index) = usize::try_from(slot) else {
                return unresolved();
            };
            if let Some(table) = &self.objpreset {
                let Some(value) = table.get(slot_index) else {
                    return unresolved();
                };
                remap_source = SOURCE_OBJPRESET;
                objects_id = i64::from(*value);
            } else {
                let slot_act = slot.div_euclid(OBJECTS_PER_ACT) + 1;
                let slot_id = slot.rem_euclid(OBJECTS_PER_ACT);
                let key = (
                    i32::try_from(slot_act).unwrap_or(0),
                    2,
                    i32::try_from(slot_id).unwrap_or(0),
                );
                let Some(row) = self
                    .edit
                    .by_key
                    .get(&key)
                    .and_then(|i| self.edit.rows.get(*i))
                else {
                    return unresolved();
                };
                let Some(object) = row.objects_id else {
                    return unresolved();
                };
                remap_source = SOURCE_OBJ_TXT;
                objects_id = i64::from(object);
                edit_row = Some(row);
            }
        }
        let Ok(row_index) = u32::try_from(objects_id) else {
            return unresolved();
        };
        if row_index == 0 {
            // Objects.txt row 0 is test data and marks unused slots.
            return unresolved();
        }
        self.object_info(2, id, row_index, remap_source, edit_row)
            .unwrap_or_else(unresolved)
    }

    /// Builds the info for Objects.txt row `row_index`. `edit_row` is the
    /// DS1Edit row the remap came through (its visuals are preferred).
    fn object_info(
        &self,
        kind: i32,
        id: i32,
        row_index: u32,
        source: &'static str,
        edit_row: Option<&EditRow>,
    ) -> Option<ObjectInfo> {
        let table_row = self
            .objects
            .as_ref()
            .and_then(|rows| rows.get(row_index as usize));
        if self.objects.is_some() && table_row.is_none() {
            return None;
        }
        let mut info = ObjectInfo::blank(kind, id, source);
        info.class = ObjectClass::Object;
        info.row = Some(row_index);

        let visual = edit_row.and_then(|r| r.visual.as_ref());
        let mut name_fallback = edit_row.map_or("", |r| r.description.as_str());

        if let Some(row) = table_row {
            if name_fallback.is_empty() {
                name_fallback = row.description.as_str();
            }
            info.name = self.pick_name(&row.name_key, name_fallback);
            info.size = row.size;
            info.offset = row.offset;
            info.draw_effect = (row.trans != 0).then_some(row.trans);
            info.parts = row.parts.clone();
            if !row.token.is_empty() {
                info.token = Some(row.token.clone());
                info.sprite_base = Some(SpriteBase::Objects);
            }
            // A curated mode from DS1Edit's table, only where it describes the
            // same sprite.
            let hint = self
                .edit
                .by_object
                .get(&row_index)
                .and_then(|i| self.edit.rows.get(*i))
                .and_then(|r| r.visual.as_ref())
                .filter(|v| v.token.eq_ignore_ascii_case(&row.token));
            if let Some(hint) = hint {
                info.mode = hint.mode.clone();
                info.weapon_class = hint.class.clone();
            }
        } else if !name_fallback.is_empty() {
            name_fallback.clone_into(&mut info.name);
        }

        if let Some(visual) = visual {
            info.sprite_base = Some(visual.base);
            info.token = Some(visual.token.clone());
            info.mode = visual.mode.clone();
            info.weapon_class = visual.class.clone();
            if !visual.parts.is_empty() {
                info.parts = visual.parts.clone();
            }
        }

        if let Some(row) = table_row {
            let mode = MODE_CODES
                .iter()
                .position(|m| m.eq_ignore_ascii_case(&info.mode))
                .unwrap_or(0);
            info.frame_count = row.frame_count.get(mode).copied();
            info.frame_delta = row
                .frame_delta
                .get(mode)
                .map(|d| if *d == 0 { 256 } else { *d });
            let flag = row.order_flag.get(mode).copied().unwrap_or(0);
            info.order_flag = if flag == 0 {
                row.order_flag.first().copied().unwrap_or(0)
            } else {
                flag
            };
        }
        Some(info)
    }

    // ----- type 1

    fn resolve_monster(&self, act: usize, ds1_version: i32, id: i32) -> ObjectInfo {
        let Some(mon) = &self.mon else {
            return self.monster_from_edit(act, ds1_version, id);
        };
        let target = if ds1_version >= 5
            && let Some(count) = mon.act_len.get(act).copied()
            && i64::from(id) < count as i64
        {
            let base = mon.act_base.get(act).copied().unwrap_or(0) as i64;
            // A negative id reads backwards into the previous acts' blocks.
            let Some(row) = usize::try_from(base + i64::from(id))
                .ok()
                .and_then(|flat| mon.preset.get(flat))
            else {
                return ObjectInfo::unresolved(1, id);
            };
            Some(self.resolve_place(mon, &row.place))
        } else {
            match usize::try_from(id) {
                Ok(row) if row < mon.stats.len() => Some(MonTarget::Stats {
                    row,
                    super_name: None,
                }),
                _ => None,
            }
        };
        match target {
            Some(MonTarget::Stats { row, super_name }) => {
                if let Some(&(_, _, object)) = MONSTER_TO_OBJECT
                    .iter()
                    .find(|(a, monster, _)| *a == act && *monster == row)
                {
                    return u32::try_from(object)
                        .ok()
                        .and_then(|o| self.object_info(1, id, o, SOURCE_MONPRESET, None))
                        .unwrap_or_else(|| ObjectInfo::unresolved(1, id));
                }
                self.monster_info(mon, id, row, super_name)
            }
            Some(MonTarget::Place(place)) => self.place_info(act, id, &place),
            None => ObjectInfo::unresolved(1, id),
        }
    }

    /// MonPreset `Place` search order: SuperUniques, MonStats, then MonPlace.
    fn resolve_place(&self, mon: &MonTables, place: &str) -> MonTarget {
        let key = place.to_ascii_lowercase();
        if let Some(sup) = mon.supers.get(&key)
            && let Some(&row) = mon.stats_by_id.get(&sup.class.to_ascii_lowercase())
        {
            return MonTarget::Stats {
                row,
                super_name: Some(sup.name_key.clone()),
            };
        }
        if let Some(&row) = mon.stats_by_id.get(&key) {
            return MonTarget::Stats {
                row,
                super_name: None,
            };
        }
        MonTarget::Place(place.to_owned())
    }

    fn monster_info(
        &self,
        mon: &MonTables,
        id: i32,
        row: usize,
        super_name: Option<String>,
    ) -> ObjectInfo {
        let mut info = ObjectInfo::blank(1, id, SOURCE_MONPRESET);
        let Some(stats) = mon.stats.get(row) else {
            return ObjectInfo::unresolved(1, id);
        };
        info.row = u32::try_from(row).ok();
        let key = super_name.as_deref().unwrap_or(&stats.name_key);
        info.name = self.pick_name(key, &stats.id);
        if !stats.code.is_empty() {
            info.token = Some(stats.code.clone());
            info.sprite_base = Some(SpriteBase::Monsters);
        }
        let ex = if stats.stats_ex.is_empty() {
            &stats.id
        } else {
            &stats.stats_ex
        };
        if let Some(stats2) = mon.stats2.get(&ex.to_ascii_lowercase()) {
            info.parts = stats2.parts.clone();
            info.size = stats2.size;
            if !stats2.base_weapon.is_empty() {
                info.weapon_class = stats2.base_weapon.to_ascii_uppercase();
            }
        }
        info
    }

    /// A MonPreset row whose `Place` is a MonPlace marker: only DS1Edit's
    /// table can draw it.
    fn place_info(&self, act: usize, id: i32, place: &str) -> ObjectInfo {
        let mut info = ObjectInfo::blank(1, id, SOURCE_MONPRESET);
        info.name = place.to_owned();
        let edit = self
            .edit
            .by_key
            .get(&(act as i32 + 1, 1, id))
            .and_then(|i| self.edit.rows.get(*i));
        if let Some(edit) = edit {
            if !edit.description.is_empty() {
                info.name.clone_from(&edit.description);
            }
            if let Some(visual) = &edit.visual {
                apply_visual(&mut info, visual);
            }
        }
        info
    }

    /// Type 1 without MonPreset/MonStats: the embedded table alone.
    fn monster_from_edit(&self, act: usize, ds1_version: i32, id: i32) -> ObjectInfo {
        if ds1_version < 5 {
            return ObjectInfo::unresolved(1, id);
        }
        let Some(edit) = self
            .edit
            .by_key
            .get(&(act as i32 + 1, 1, id))
            .and_then(|i| self.edit.rows.get(*i))
        else {
            return ObjectInfo::unresolved(1, id);
        };
        let Some(visual) = &edit.visual else {
            return ObjectInfo::unresolved(1, id);
        };
        let mut info = ObjectInfo::blank(1, id, SOURCE_OBJ_TXT);
        if !edit.description.is_empty() {
            info.name.clone_from(&edit.description);
        }
        apply_visual(&mut info, visual);
        info
    }
}

fn apply_visual(info: &mut ObjectInfo, visual: &EditVisual) {
    info.sprite_base = Some(visual.base);
    info.token = Some(visual.token.clone());
    info.mode.clone_from(&visual.mode);
    info.weapon_class.clone_from(&visual.class);
    info.parts.clone_from(&visual.parts);
}
