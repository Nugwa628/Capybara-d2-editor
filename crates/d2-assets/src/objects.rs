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
//! `MonPreset`, `MonStats`, `MonStats2` and `SuperUniques` are loaded through
//! [`crate::tables::load_excel`], so 1.13c profiles (Median XL) read the
//! compiled `.bin`. That form has no names: every link between the tables is a
//! row number (`MonPreset` -> `PlaceKind` + `PlaceIndex`, `SuperUniques` ->
//! `ClassRow`, `MonStats` -> `MonStatsExRow`) and `NameStr` is a string id, so
//! the catalog follows rows where the `.txt` form is followed by `Id` text. The
//! chain is the same either way: MonPreset, then MonStats or SuperUniques (which
//! points at a MonStats row), then MonStats2.

use std::collections::HashMap;

use d2_formats::ds1::Object;
use d2_formats::excel::{ExcelTableKind, monplace_code};
use d2_formats::tbl::StringTable;
use d2_formats::txt::{Row, TxtTable};

use crate::error::{AssetError, Result};
use crate::overlay::Overlay;
use crate::path::GamePath;
use crate::profile::GameVersion;
use crate::tables::load_excel;

/// DS1Edit's object table, MIT licensed (see `NOTICE`). Latin-1 text.
const EMBEDDED_OBJ_TXT: &[u8] = include_bytes!("../../../assets/editor/obj.txt");

const LNG_DIR: &str = r"data\local\lng\eng\";
/// String tables in lookup priority order, with their compiled-id offsets.
const STRING_TABLES: [(&str, u32); 3] = [
    ("patchstring.tbl", 10000),
    ("expansionstring.tbl", 20000),
    ("string.tbl", 0),
];

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

/// A palette shift for one unique or special monster, from the `Colormap`
/// and `Index` columns of DS1Edit's obj.txt (`cof.c:216-242`).
///
/// The file is a run of 256-byte rows (`Data\Global\Monsters\RandTransforms.dat`
/// has 30, a monster's own `COF\Palshift.dat` 8). Each row maps a palette
/// index to another one; it applies to the COF layers with `trans_a == 0`
/// only, before the palette lookup (`cof.c:343-361`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaletteShift {
    /// The colormap file exactly as obj.txt names it, e.g.
    /// `Data\Global\Monsters\CR\COF\Palshift.dat` (an archive path).
    pub path: String,
    /// The 0-based row: obj.txt `Index` minus one. (`Index` 0 or empty means
    /// no palette shift, so it never appears here.)
    pub row: u32,
}

impl PaletteShift {
    /// The 256-byte row of this shift in the colormap file `bytes`; `None`
    /// when the file is too short for it (the reference then draws the
    /// monster without a shift, `cof.c:236-241`).
    pub fn row_in<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8; 256]> {
        let start = usize::try_from(self.row).ok()?.checked_mul(256)?;
        let end = start.checked_add(256)?;
        bytes.get(start..end)?.try_into().ok()
    }
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
    /// Per-component skin/armor token for DCC file names, e.g. `("TR", "LIT")`,
    /// from DS1Edit's obj.txt. Empty, or a component missing here, means `LIT`.
    /// Type 1 objects resolved through MonPreset take them from the obj.txt
    /// row `(act, 1, id)` when it draws the same token (DS1 v5+).
    pub skins: Vec<(String, String)>,
    /// Default facing from DS1Edit's obj.txt `Direction` column (e.g. rogues
    /// 7, chickens 6), which the reference uses as the user direction. None
    /// when no obj.txt row applies; renderers then use direction 0. Type 1
    /// objects get it like `skins`.
    pub direction: Option<u8>,
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
    /// Palette shift of a unique or special monster, from DS1Edit's obj.txt
    /// `Colormap`/`Index` (row `(act, type, id)`). Set only when that row
    /// draws the same token as this object, so a mod whose presets differ
    /// from vanilla's does not inherit vanilla's colours. Renderers apply it
    /// to layers with `trans_a == 0` (`cof.c:343-361`).
    pub colormap: Option<PaletteShift>,
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
            skins: Vec::new(),
            direction: None,
            size: (0, 0),
            offset: (0, 0),
            draw_effect: None,
            frame_count: None,
            frame_delta: None,
            colormap: None,
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

/// A display-name reference: the `.txt` key, or the string id the `.bin` keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NameRef {
    Key(String),
    Id(u16),
}

impl Default for NameRef {
    fn default() -> Self {
        NameRef::Key(String::new())
    }
}

/// One `MonStats` row.
#[derive(Clone, Debug, Default)]
struct MonRow {
    /// The `Id` text; empty when the table came from `.bin`.
    id: String,
    name: NameRef,
    code: String,
    /// The MonStats2 row (`MonStatsEx`, or the row's own `Id` when blank).
    stats2: Option<usize>,
}

/// One `MonStats2` row.
#[derive(Clone, Debug, Default)]
struct Mon2Row {
    parts: Vec<String>,
    base_weapon: String,
    size: (i32, i32),
}

/// One `SuperUniques` row.
#[derive(Clone, Debug, Default)]
struct SuperRow {
    name: NameRef,
    /// The MonStats row of `Class`.
    class_row: Option<usize>,
}

/// What a MonPreset row places.
#[derive(Clone, Debug)]
enum Place {
    /// A SuperUniques row (whose class is a MonStats row).
    Super(usize),
    /// A MonStats row.
    Stats(usize),
    /// A MonPlace marker (group, nothing, ...), with its text.
    Marker(String),
    /// An index past its table.
    Missing,
}

/// `MonPreset`, `MonStats`, `MonStats2` and `SuperUniques`, with every link
/// between them resolved to row numbers.
#[derive(Debug, Default)]
struct MonTables {
    /// All MonPreset rows in file order.
    preset: Vec<Place>,
    /// First flat index of each act's block, and its row count.
    act_base: [usize; ACTS],
    act_len: [usize; ACTS],
    stats: Vec<MonRow>,
    stats2: Vec<Mon2Row>,
    supers: Vec<SuperRow>,
}

/// One row of the embedded DS1Edit `obj.txt`.
#[derive(Clone, Debug, Default)]
struct EditRow {
    description: String,
    objects_id: Option<u32>,
    /// All of Base, Token, Mode and Class were given (what DS1Edit needs).
    visual: Option<EditVisual>,
    /// `Colormap` and a non-zero `Index`.
    colormap: Option<PaletteShift>,
}

#[derive(Clone, Debug)]
struct EditVisual {
    base: SpriteBase,
    token: String,
    mode: String,
    class: String,
    parts: Vec<String>,
    skins: Vec<(String, String)>,
    direction: Option<u8>,
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

/// The game skips every row whose first cell is `Expansion`, whatever its other
/// cells hold (`monstats2.txt` has an `EOL` cell on it), and row numbers count
/// the other rows only.
fn mon_data_rows(table: &TxtTable) -> impl Iterator<Item = Row<'_>> {
    table.rows().filter(|row| {
        !row.cells()
            .first()
            .is_some_and(|c| c.trim().eq_ignore_ascii_case("expansion"))
    })
}

/// A compiled row reference column: `-1` (blank) and out-of-range values are
/// "no row".
fn row_ref(row: Row<'_>, column: &str, limit: usize) -> Option<usize> {
    row.get_i64(column)
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v < limit)
}

fn parse_mon_tables(
    preset: &TxtTable,
    stats: &TxtTable,
    stats2: Option<&TxtTable>,
    supers: Option<&TxtTable>,
) -> MonTables {
    let mut tables = MonTables::default();

    // MonStats2: every compiled row; the `.txt` form is also indexed by `Id`.
    let mut stats2_by_id: HashMap<String, usize> = HashMap::new();
    if let Some(stats2) = stats2 {
        for row in mon_data_rows(stats2) {
            stats2_by_id
                .entry(trimmed(row, "Id").to_ascii_lowercase())
                .or_insert(tables.stats2.len());
            tables.stats2.push(Mon2Row {
                parts: flagged_parts(row),
                base_weapon: trimmed(row, "BaseW").to_owned(),
                size: (to_i32(num(row, "SizeX")), to_i32(num(row, "SizeY"))),
            });
        }
    }

    // MonStats: `.bin` rows carry a string id, row numbers and no `Id`.
    let stats_is_bin = stats.column("NameStrId").is_some();
    let mut stats_by_id: HashMap<String, usize> = HashMap::new();
    for row in mon_data_rows(stats) {
        let (id, name, ex) = if stats_is_bin {
            let name = NameRef::Id(u16::try_from(num(row, "NameStrId")).unwrap_or(0));
            (
                String::new(),
                name,
                row_ref(row, "MonStatsExRow", tables.stats2.len()),
            )
        } else {
            let id = trimmed(row, "Id").to_owned();
            stats_by_id
                .entry(id.to_ascii_lowercase())
                .or_insert(tables.stats.len());
            let ex_text = match trimmed(row, "MonStatsEx") {
                "" => id.as_str(),
                text => text,
            };
            let ex = stats2_by_id.get(&ex_text.to_ascii_lowercase()).copied();
            (
                id.clone(),
                NameRef::Key(trimmed(row, "NameStr").to_owned()),
                ex,
            )
        };
        tables.stats.push(MonRow {
            id,
            name,
            code: trimmed(row, "Code").to_owned(),
            stats2: ex,
        });
    }

    // SuperUniques: `Class` is a MonStats row.
    let mut supers_by_name: HashMap<String, usize> = HashMap::new();
    if let Some(supers) = supers {
        let is_bin = supers.column("ClassRow").is_some();
        for row in mon_data_rows(supers) {
            let (name, class_row) = if is_bin {
                (
                    NameRef::Id(u16::try_from(num(row, "NameStrId")).unwrap_or(0)),
                    row_ref(row, "ClassRow", tables.stats.len()),
                )
            } else {
                supers_by_name
                    .entry(trimmed(row, "Superunique").to_ascii_lowercase())
                    .or_insert(tables.supers.len());
                (
                    NameRef::Key(trimmed(row, "Name").to_owned()),
                    stats_by_id
                        .get(&trimmed(row, "Class").to_ascii_lowercase())
                        .copied(),
                )
            };
            tables.supers.push(SuperRow { name, class_row });
        }
    }

    // MonPreset: acts are blocks of consecutive rows; `Place` names a row.
    let preset_is_bin = preset.column("PlaceKind").is_some();
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
        let place = if preset_is_bin {
            let index = num(row, "PlaceIndex");
            match num(row, "PlaceKind") {
                0 => Place::Marker(
                    usize::try_from(index)
                        .ok()
                        .and_then(monplace_code)
                        .map_or_else(|| format!("place {index}"), str::to_owned),
                ),
                1 => row_ref(row, "PlaceIndex", tables.stats.len())
                    .map_or(Place::Missing, Place::Stats),
                2 => row_ref(row, "PlaceIndex", tables.supers.len())
                    .filter(|s| tables.supers.get(*s).is_some_and(|r| r.class_row.is_some()))
                    .map_or(Place::Missing, Place::Super),
                _ => Place::Missing,
            }
        } else {
            let text = trimmed(row, "Place");
            let key = text.to_ascii_lowercase();
            // Search order: SuperUniques (when its class exists), MonStats, MonPlace.
            if let Some(&s) = supers_by_name.get(&key)
                && tables.supers.get(s).is_some_and(|r| r.class_row.is_some())
            {
                Place::Super(s)
            } else if let Some(&row) = stats_by_id.get(&key) {
                Place::Stats(row)
            } else {
                Place::Marker(text.to_owned())
            }
        };
        tables.preset.push(place);
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
                    skins: PART_CODES
                        .iter()
                        .filter_map(|code| {
                            let skin = trimmed(row, code);
                            (!skin.is_empty())
                                .then(|| ((*code).to_owned(), skin.to_ascii_uppercase()))
                        })
                        .collect(),
                    direction: trimmed(row, "Direction").parse::<u8>().ok(),
                })
            }
            _ => None,
        };
        // cof.c:218-226: both cells set and Index non-zero.
        let colormap = match (trimmed(row, "Colormap"), num(row, "Index")) {
            (path, index) if !path.is_empty() && index > 0 => {
                u32::try_from(index - 1).ok().map(|row| PaletteShift {
                    path: path.to_owned(),
                    row,
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
            colormap,
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

/// Loads a monster table through `load_excel` (`.bin` first on 1.13c).
/// `None` when it is missing or unusable, with the reason pushed to `warnings`.
fn load_mon_table(
    overlay: &Overlay,
    kind: ExcelTableKind,
    version: GameVersion,
    warnings: &mut Vec<String>,
) -> Option<TxtTable> {
    match load_excel(overlay, kind, version) {
        Ok(table) => Some(table),
        Err(err) if is_absent(&err) => {
            warnings.push(format!("{} is not in the overlay", kind.file_stem()));
            None
        }
        Err(err) => {
            warnings.push(format!("{} cannot be loaded: {err}", kind.file_stem()));
            None
        }
    }
}

/// The string tables in lookup priority order, with the id offset the compiled
/// excel tables use for each (`string.tbl` 0, `patchstring.tbl` 10000,
/// `expansionstring.tbl` 20000).
#[derive(Debug, Default)]
struct Strings {
    tables: Vec<(StringTable, u32)>,
}

impl Strings {
    /// The text for `key` from the first table that has it.
    fn get(&self, key: &str) -> Option<&str> {
        self.tables.iter().find_map(|(table, _)| table.get(key))
    }

    /// The key a compiled string id stands for.
    fn key_for_id(&self, id: u16) -> Option<&str> {
        let id = u32::from(id);
        let base = if id >= 20000 {
            20000
        } else if id >= 10000 {
            10000
        } else {
            0
        };
        let local = u16::try_from(id - base).ok()?;
        self.tables
            .iter()
            .find(|(_, table_base)| *table_base == base)
            .and_then(|(table, _)| table.entry_by_id(local))
            .map(|(key, _)| key)
    }

    fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

fn load_strings(overlay: &Overlay, warnings: &mut Vec<String>) -> Strings {
    let mut out = Strings::default();
    for (name, base) in STRING_TABLES {
        let path = GamePath::new(&format!("{LNG_DIR}{name}"));
        match overlay.read_bytes(&path) {
            Ok(Some(bytes)) => match StringTable::parse(&bytes) {
                Ok(table) => out.tables.push((table, base)),
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
        /// Set when the row came through a SuperUniques entry: its name.
        super_name: Option<NameRef>,
    },
    /// A MonPreset `Place` that is neither a superunique nor a monster
    /// (a MonPlace group marker), with the text of the place.
    Place(String),
    /// A reference past the end of its table.
    Missing,
}

/// Resolves DS1 objects against the loaded game tables.
pub struct ObjectCatalog {
    objects: Option<Vec<ObjRow>>,
    /// `table[act * 150 + id]` when the profile ships an ObjPreset table.
    objpreset: Option<Vec<u32>>,
    mon: Option<MonTables>,
    edit: EditTable,
    strings: Strings,
}

impl ObjectCatalog {
    /// Loads Objects, ObjPreset (when the profile has it; else the embedded
    /// MIT `obj.txt` from DS1Edit, credited in NOTICE), MonPreset/MonStats/
    /// MonStats2/SuperUniques (`.bin` or `.txt`, as `load_excel` prefers for the
    /// version), and string tables.
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

        let preset = load_mon_table(overlay, ExcelTableKind::MonPreset, version, &mut warnings);
        let stats = load_mon_table(overlay, ExcelTableKind::MonStats, version, &mut warnings);
        let stats2 = load_mon_table(overlay, ExcelTableKind::MonStats2, version, &mut warnings);
        let supers = load_mon_table(
            overlay,
            ExcelTableKind::SuperUniques,
            version,
            &mut warnings,
        );
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
        let mut info = match object.kind {
            1 => self.resolve_monster(act, ds1_version, object.id),
            2 => self.resolve_object(act, ds1_version, object.id),
            kind => return ObjectInfo::unresolved(kind, object.id),
        };
        if let Some((row, visual)) = self.matching_edit_row(act, ds1_version, &info) {
            info.colormap.clone_from(&row.colormap);
            if info.ds1_kind == 1 && info.skins.is_empty() {
                info.skins.clone_from(&visual.skins);
            }
            if info.ds1_kind == 1 && info.direction.is_none() {
                info.direction = visual.direction;
            }
        }
        info
    }

    /// DS1Edit's obj.txt row `(act, type, id)` and its visual, when that row
    /// draws the same token as `info` (type 1 needs DS1 v5+, where ids are
    /// presets as in obj.txt).
    fn matching_edit_row(
        &self,
        act: usize,
        ds1_version: i32,
        info: &ObjectInfo,
    ) -> Option<(&EditRow, &EditVisual)> {
        if info.ds1_kind == 1 && ds1_version < 5 {
            return None;
        }
        let act = i32::try_from(act).ok()? + 1;
        let row = self
            .edit
            .by_key
            .get(&(act, info.ds1_kind, info.ds1_id))
            .and_then(|i| self.edit.rows.get(*i))?;
        let token = info.token.as_deref()?;
        let visual = row
            .visual
            .as_ref()
            .filter(|v| v.token.eq_ignore_ascii_case(token))?;
        Some((row, visual))
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
            .get(key)
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

    /// The display name for a `NameStr`/`Name` reference.
    fn name_of(&self, name: &NameRef, fallback: &str) -> String {
        match name {
            NameRef::Key(key) => self.pick_name(key, fallback),
            NameRef::Id(id) => match self.strings.key_for_id(*id) {
                Some(key) => self.pick_name(key, fallback),
                None => fallback.to_owned(),
            },
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
            info.skins.clone_from(&visual.skins);
            info.direction = visual.direction;
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
            Some(Self::resolve_place(mon, row))
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
            Some(MonTarget::Missing) | None => ObjectInfo::unresolved(1, id),
        }
    }

    /// What a MonPreset row places (its `Place` was resolved at load time in the
    /// game's search order: SuperUniques, MonStats, then MonPlace).
    fn resolve_place(mon: &MonTables, place: &Place) -> MonTarget {
        match place {
            Place::Super(index) => {
                let sup = mon.supers.get(*index);
                match sup.and_then(|s| s.class_row.map(|row| (row, s.name.clone()))) {
                    Some((row, name)) => MonTarget::Stats {
                        row,
                        super_name: Some(name),
                    },
                    None => MonTarget::Missing,
                }
            }
            Place::Stats(row) => MonTarget::Stats {
                row: *row,
                super_name: None,
            },
            Place::Marker(text) => MonTarget::Place(text.clone()),
            Place::Missing => MonTarget::Missing,
        }
    }

    fn monster_info(
        &self,
        mon: &MonTables,
        id: i32,
        row: usize,
        super_name: Option<NameRef>,
    ) -> ObjectInfo {
        let mut info = ObjectInfo::blank(1, id, SOURCE_MONPRESET);
        let Some(stats) = mon.stats.get(row) else {
            return ObjectInfo::unresolved(1, id);
        };
        info.row = u32::try_from(row).ok();
        // The compiled table has no `Id` text to fall back on.
        let fallback = if stats.id.is_empty() {
            format!("monster {row}")
        } else {
            stats.id.clone()
        };
        info.name = self.name_of(super_name.as_ref().unwrap_or(&stats.name), &fallback);
        if !stats.code.is_empty() {
            info.token = Some(stats.code.clone());
            info.sprite_base = Some(SpriteBase::Monsters);
        }
        if let Some(stats2) = stats.stats2.and_then(|ex| mon.stats2.get(ex)) {
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
    info.skins.clone_from(&visual.skins);
    info.direction = visual.direction;
}
