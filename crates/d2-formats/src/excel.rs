//! Precompiled excel tables (`data\global\excel\*.bin`).
//!
//! The retail 1.13c loader reads `<table>.bin` by default: a `u32` record
//! count followed by that many fixed-size records, laid out exactly as the
//! game's in-memory table (see `docs/guides/excel-tables-and-data-loading.md`).
//! [`decode_bin`] turns such a file back into a [`TxtTable`] with the column
//! names of the matching `.txt`, so the rest of the editor reads one shape.
//!
//! # What was verified, and how
//!
//! Record sizes and field offsets were derived by aligning the vanilla `.bin`
//! files against the `.txt` of the same table (every row, every candidate
//! offset and width; blank numeric cells compare equal to 0, which is what the
//! compiler writes), then checked for plausibility against Median XL's 1.13c
//! `.bin` files, which have no `.txt`. A test in this module repeats the
//! alignment on every run when the private corpus is present.
//!
//! | table | record size | notes |
//! |---|---|---|
//! | `lvltypes` | `0x788` | `File 1..32` (60-byte strings), `Act`, `Expansion`; `Id` is the record index; `Name` and `Beta` are not stored |
//! | `lvlprest` | `0x1B0` | everything but `Name` and `Beta`; `Def` is stored |
//! | `lvlwarp` | `0x30` | everything but `Name` and `Beta`; `Direction` is a one-char string |
//! | `objects` | `0x1C0` | see [`ExcelTableKind::Objects`]; `Id` is the record index |
//! | `levels` | `0x220` | `levels.bin`: `Id`, `Pal`, `Act`, flags, spawn data, names; the DRLG half lives in `leveldefs.bin` (`0x9C`), see [`decode_levels_bin`] |
//! | `objpreset` | `4` | Median XL only; 750 `i32`: `table[act * 150 + id]` = `Objects.txt` id |
//! | `monpreset` | `4` | `Act`, then which table `Place` names (MonPlace / MonStats / SuperUniques) and the row in it |
//! | `monstats` | `0x1A8` | no `Id` text: links are row numbers, `NameStr` is a string id; see [`ExcelTableKind::MonStats`] |
//! | `monstats2` | `0x134` | no `Id` text; part and mode flags are bit words; see [`ExcelTableKind::MonStats2`] |
//! | `superuniques` | `0x34` | no `Superunique` text; `Class` is a MonStats row, `Name` a string id |
//!
//! The monster tables store no names: the compiler replaces every text that
//! names a row of another table with that row's number (and a `NameStr` /
//! `Name` key with its `.tbl` string id), so the decoded columns for those are
//! called `...Row` / `...Id` instead of reusing the `.txt` name. Vanilla's own
//! `.bin` files are slightly stale against the shipped `.txt` in a few places
//! (`monstats.bin` record words 723..733, the `NameStr` of `uberandariel`);
//! the comparison test lists them.
//!
//! **1.13c versus 1.14d.** Every table where both a 1.13c-era file (Median XL)
//! and the 1.14d install's file exist has the same record size, and the
//! vanilla 1.14d files are byte-identical in size to the 1.13c archives'
//! (`objects.bin` is 256,708 bytes in both). The 1.14d game reads `.txt`; its
//! `.bin` files are the unchanged 1.13c-era ones. Both [`ExcelVersion`]s
//! therefore share one layout, and [`detect_version`] reports [`ExcelVersion::V113c`]
//! for any file that fits it.

use crate::txt::TxtTable;
use crate::{FormatError, Result};

const FORMAT: &str = "excel";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExcelVersion {
    V113c,
    V114d,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExcelTableKind {
    LvlTypes,
    LvlPrest,
    LvlWarp,
    /// Decoded columns: `Name`, `Id` (record index; the `.txt` `Id` differs in
    /// three vanilla rows and the game ignores it), `Token` (stored as at most
    /// two characters), `SpawnMax`, `Selectable0..7`, `TrapProb`, `SizeX/Y`,
    /// `FrameCnt0..7`, `FrameDelta0..7`, `CycleAnim0..7`, `Lit0..7`,
    /// `BlocksLight0..7`, `HasCollision0..7`, `IsAttackable0`, `Start0..7`,
    /// `OrderFlag0..7`, `EnvEffect`, `IsDoor`, `BlocksVis`, `Orientation`,
    /// `PreOperate`, `Trans`, `Mode0..7`, `Xoffset`, `Yoffset`, `Draw`, the
    /// body-part columns `HD..S8`, `TotalPieces`, `Xspace`, `Yspace`,
    /// `Red/Green/Blue`, `SubClass`, `NameOffset`, `MonsterOK`,
    /// `OperateRange`, `Act`, `Lockable`, `Gore`, `Restore`,
    /// `RestoreVirgins`, `Sync`, `Parm0..7`, `nTgtFX/FY/BX/BY`, `Damage`,
    /// `CollisionSubst`, `Left/Top/Width/Height`, `Beta`, `InitFn`,
    /// `PopulateFn`, `OperateFn`, `ClientFn`, `Overlay`, `BlockMissile`,
    /// `DrawUnder`, `OpenWarp`, `AutoMap`. Not decoded: `description - not
    /// loaded` (not stored), `Flicker` and `ShrineFunction` (no offset could
    /// be established). Array columns whose values are zero in every record of
    /// both corpora (`Selectable4..7`, `Start6/7`, `S5..S8`, `LG..LH`, ...)
    /// cannot be located by value; they sit in the slots left over between
    /// their verified siblings, in column order.
    Objects,
    /// Not an excel table in the vanilla archives: Median XL ships the
    /// `[5 acts][150]` object remap table that vanilla compiles into
    /// `D2Common.dll` as `objpreset.bin`. Decoded to columns `Act`, `Id`
    /// (0..149 within the act) and `ObjectsId`.
    ObjPreset,
    /// `levels.bin` alone; see [`decode_levels_bin`] for the full table.
    Levels,
    /// `monpreset.bin`: the per-act placement table DS1 monster ids index.
    /// Four bytes per record: `Act` (`1..=5`), `PlaceKind` (`0` = a MonPlace
    /// row, `1` = a MonStats row, `2` = a SuperUniques row) and `PlaceIndex`
    /// (the row in that table). The `.txt` `Place` text is not stored: it is
    /// the code/Id/Superunique of the row that `PlaceKind` + `PlaceIndex` name,
    /// and the MonStats / SuperUniques `.bin` files hold no Id text either, so
    /// the decoder keeps the indices. [`monplace_code`] names the MonPlace rows.
    MonPreset,
    /// `monstats.bin`. The `Id` text and the text links (`BaseId`, `MonStatsEx`,
    /// ...) are not stored; the compiled form keeps row numbers. Decoded
    /// columns: `hcIdx` (the record index, see below), `BaseIdRow`,
    /// `NextInClassRow`, `spawnRow`, `minion1Row`, `minion2Row` (MonStats rows,
    /// `-1` = blank), `MonStatsExRow` (a **MonStats2** row, `-1` = blank),
    /// `NameStrId` (a string id, see below), `Code` (4 characters, trailing
    /// spaces trimmed), the 29 flag columns `isSpawn..rangedtype` (bits of the
    /// word at `0xC`), `TransLvl`, `Align`, `threat`, `spawnx/y`,
    /// `PartyMin/Max`, `Rarity`, `MinGrp/MaxGrp`, `sparsePopulate`, `Level`,
    /// `Level(N)`, `Level(H)`, `TCQuestId` and `TCQuestCP`.
    ///
    /// `NameStrId` is the string id of the `NameStr` key: ids below 10000 are in
    /// `string.tbl`, `10000..20000` in `patchstring.tbl` (id - 10000) and
    /// `20000..` in `expansionstring.tbl` (id - 20000); a blank or unknown key
    /// is compiled as the id of `dummy`. The word at offset 0 equals the record
    /// index in every Median XL record, but the last 11 vanilla records hold a
    /// stale value (index - 1); `hcIdx` is therefore decoded as the record
    /// index, which is also what the `.txt` says.
    MonStats,
    /// `monstats2.bin` (rows are addressed by `MonStatsExRow`; the `Id` text is
    /// not stored, the word at offset 0 is the record index). Decoded columns:
    /// `SizeX`, `SizeY`, `spawnCol`, `Height`, `OverlayHeight`, `pixHeight`,
    /// `MeleeRng`, `BaseW` (4 characters, trailing spaces trimmed), `HitClass`,
    /// the part flags `HD TR LG RA LA RH LH SH S1..S8`, `TotalPieces`, the mode
    /// flags `mDT..mRN`, the counts `dDT..dRN`, `A2mv S1mv S2mv`, the flags
    /// `noGfxHitTest .. unflatDead`, `htTop/Left/Width/Height`, `localBlood`,
    /// `Bleed`, `Light`, `light-r/g/b`, `Utrans(N/H)` and
    /// `InfernoLen/Anim/Rollback`. `SizeX`/`SizeY` are equal in every vanilla
    /// row, so their order (offset 8 then 9) follows the `.txt` column order.
    MonStats2,
    /// `superuniques.bin` (the `Superunique` and `Name` texts are not stored).
    /// Decoded columns: `NameStrId` (string id of `Name`, see
    /// [`ExcelTableKind::MonStats`]), `ClassRow` (a MonStats row), `hcIdx`,
    /// `Mod1..Mod3`, `MinGrp`, `MaxGrp`, `AutoPos`, `EClass`, `Stacks`,
    /// `Replaceable`, `Utrans`, `Utrans(N)`, `Utrans(H)`. The three `Utrans`
    /// bytes are equal in every vanilla row, so their order follows the
    /// `.txt` column order.
    SuperUniques,
}

impl ExcelTableKind {
    /// The file name without extension, e.g. `"lvltypes"`.
    pub fn file_stem(self) -> &'static str {
        match self {
            ExcelTableKind::LvlTypes => "lvltypes",
            ExcelTableKind::LvlPrest => "lvlprest",
            ExcelTableKind::LvlWarp => "lvlwarp",
            ExcelTableKind::Objects => "objects",
            ExcelTableKind::ObjPreset => "objpreset",
            ExcelTableKind::Levels => "levels",
            ExcelTableKind::MonPreset => "monpreset",
            ExcelTableKind::MonStats => "monstats",
            ExcelTableKind::MonStats2 => "monstats2",
            ExcelTableKind::SuperUniques => "superuniques",
        }
    }

    pub fn all() -> &'static [ExcelTableKind] {
        &[
            ExcelTableKind::LvlTypes,
            ExcelTableKind::LvlPrest,
            ExcelTableKind::LvlWarp,
            ExcelTableKind::Objects,
            ExcelTableKind::ObjPreset,
            ExcelTableKind::Levels,
            ExcelTableKind::MonPreset,
            ExcelTableKind::MonStats,
            ExcelTableKind::MonStats2,
            ExcelTableKind::SuperUniques,
        ]
    }
}

// --------------------------------------------------------------------------
// layouts

#[derive(Clone, Copy, Debug)]
enum Field {
    /// Zero-based record index, written as the column value.
    Index,
    U8,
    I8,
    U16,
    I32,
    U32,
    /// NUL-terminated Latin-1 string in a fixed field of this many bytes.
    Str(usize),
    /// Like `Str`, with trailing spaces removed (the 4-byte codes of
    /// `monstats` are space padded, e.g. `"SK  "`).
    StrPad(usize),
    I16,
    /// One bit (`0..32`) of the little-endian `u32` at the column offset,
    /// written as `0` or `1`. Several `Bit` columns share a word, so they do
    /// not count towards [`Field::width`].
    Bit(u8),
}

impl Field {
    fn width(self) -> usize {
        match self {
            Field::Index => 0,
            Field::U8 | Field::I8 => 1,
            Field::U16 | Field::I16 => 2,
            Field::I32 | Field::U32 => 4,
            Field::Str(n) | Field::StrPad(n) => n,
            Field::Bit(_) => 0,
        }
    }

    /// Bytes read from the record, including the word a `Bit` lives in.
    #[cfg(test)]
    fn reach(self) -> usize {
        match self {
            Field::Bit(_) => 4,
            other => other.width(),
        }
    }
}

struct Column {
    name: String,
    offset: usize,
    field: Field,
}

struct Layout {
    record_size: usize,
    columns: Vec<Column>,
}

impl Layout {
    fn new(record_size: usize) -> Self {
        Layout {
            record_size,
            columns: Vec::new(),
        }
    }

    fn col(&mut self, name: &str, offset: usize, field: Field) -> &mut Self {
        self.columns.push(Column {
            name: name.to_string(),
            offset,
            field,
        });
        self
    }

    /// One `Bit` column per name: `names[i]` is bit `first_bit + i` of the word
    /// at `offset`.
    fn bits(&mut self, names: &[&str], offset: usize, first_bit: u8) -> &mut Self {
        for (bit, name) in (first_bit..).zip(names) {
            self.col(name, offset, Field::Bit(bit));
        }
        self
    }

    /// `count` columns `prefix{first}..` laid out back to back from `offset`.
    fn array(
        &mut self,
        prefix: &str,
        first: usize,
        count: usize,
        offset: usize,
        field: Field,
    ) -> &mut Self {
        for i in 0..count {
            let name = format!("{prefix}{}", first + i);
            self.col(&name, offset + i * field.width(), field);
        }
        self
    }

    fn decode(&self, bytes: &[u8]) -> Result<TxtTable> {
        let rec = self.record_size;
        let count = record_count(bytes, rec)?;
        let headers: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let mut rows = Vec::with_capacity(count);
        for (index, record) in bytes.get(4..).unwrap_or(&[]).chunks_exact(rec).enumerate() {
            let mut row = Vec::with_capacity(self.columns.len());
            for c in &self.columns {
                row.push(read_field(record, c, index)?);
            }
            rows.push(row);
        }
        Ok(TxtTable::from_parts(headers, rows))
    }
}

fn invalid(what: &'static str, detail: String) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset: 0,
        detail,
    }
}

fn le(record: &[u8], offset: usize, width: usize) -> Option<u32> {
    let b = record.get(offset..offset.checked_add(width)?)?;
    let mut v: u32 = 0;
    for (i, &byte) in b.iter().enumerate() {
        v |= u32::from(byte) << (8 * i);
    }
    Some(v)
}

fn read_field(record: &[u8], c: &Column, index: usize) -> Result<String> {
    let bad = || {
        invalid(
            "column",
            format!("{} at {:#x} lies outside the record", c.name, c.offset),
        )
    };
    Ok(match c.field {
        Field::Index => index.to_string(),
        Field::U8 | Field::U16 | Field::U32 => le(record, c.offset, c.field.width())
            .ok_or_else(bad)?
            .to_string(),
        Field::I8 => (le(record, c.offset, 1).ok_or_else(bad)? as u8 as i8).to_string(),
        Field::I16 => (le(record, c.offset, 2).ok_or_else(bad)? as u16 as i16).to_string(),
        Field::Bit(bit) => {
            ((le(record, c.offset, 4).ok_or_else(bad)? >> u32::from(bit)) & 1).to_string()
        }
        Field::I32 => (le(record, c.offset, 4).ok_or_else(bad)? as i32).to_string(),
        Field::Str(n) | Field::StrPad(n) => {
            let b = record
                .get(c.offset..c.offset.checked_add(n).ok_or_else(bad)?)
                .ok_or_else(bad)?;
            let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
            let text: String = b
                .get(..end)
                .unwrap_or(&[])
                .iter()
                .map(|&x| char::from(x))
                .collect();
            if matches!(c.field, Field::StrPad(_)) {
                text.trim_end_matches(' ').to_owned()
            } else {
                text
            }
        }
    })
}

/// Validates `u32 count + count * record_size == len` and returns the count.
fn record_count(bytes: &[u8], record_size: usize) -> Result<usize> {
    let head: [u8; 4] =
        bytes
            .get(..4)
            .and_then(|h| h.try_into().ok())
            .ok_or(FormatError::Truncated {
                format: FORMAT,
                offset: 0,
                needed: 4usize.saturating_sub(bytes.len()),
            })?;
    let count = u64::from(u32::from_le_bytes(head));
    let expected = count
        .checked_mul(record_size as u64)
        .and_then(|n| n.checked_add(4));
    if expected != Some(bytes.len() as u64) {
        return Err(invalid(
            "record count",
            format!(
                "{count} records of {record_size} bytes do not fit a {}-byte file",
                bytes.len()
            ),
        ));
    }
    Ok(count as usize)
}

const LVLTYPES_SIZE: usize = 0x788;
const LVLPREST_SIZE: usize = 0x1B0;
const LVLWARP_SIZE: usize = 0x30;
const OBJECTS_SIZE: usize = 0x1C0;
const OBJPRESET_SIZE: usize = 4;
const LEVELS_SIZE: usize = 0x220;
const LEVELDEFS_SIZE: usize = 0x9C;
const MONPRESET_SIZE: usize = 4;
const MONSTATS_SIZE: usize = 0x1A8;
const MONSTATS2_SIZE: usize = 0x134;
const SUPERUNIQUES_SIZE: usize = 0x34;

/// Size in bytes of one record. Identical for every [`ExcelVersion`] (see the
/// module docs).
pub fn record_size(kind: ExcelTableKind, _version: ExcelVersion) -> usize {
    match kind {
        ExcelTableKind::LvlTypes => LVLTYPES_SIZE,
        ExcelTableKind::LvlPrest => LVLPREST_SIZE,
        ExcelTableKind::LvlWarp => LVLWARP_SIZE,
        ExcelTableKind::Objects => OBJECTS_SIZE,
        ExcelTableKind::ObjPreset => OBJPRESET_SIZE,
        ExcelTableKind::Levels => LEVELS_SIZE,
        ExcelTableKind::MonPreset => MONPRESET_SIZE,
        ExcelTableKind::MonStats => MONSTATS_SIZE,
        ExcelTableKind::MonStats2 => MONSTATS2_SIZE,
        ExcelTableKind::SuperUniques => SUPERUNIQUES_SIZE,
    }
}

/// Size of one `leveldefs.bin` record (the DRLG half of `Levels`).
pub fn leveldefs_record_size(_version: ExcelVersion) -> usize {
    LEVELDEFS_SIZE
}

fn lvltypes_layout() -> Layout {
    let mut l = Layout::new(LVLTYPES_SIZE);
    l.col("Id", 0, Field::Index);
    for i in 0..32 {
        l.col(&format!("File {}", i + 1), i * 60, Field::Str(60));
    }
    l.col("Act", 0x780, Field::I32);
    l.col("Expansion", 0x784, Field::I32);
    l
}

fn lvlprest_layout() -> Layout {
    let mut l = Layout::new(LVLPREST_SIZE);
    l.col("Def", 0, Field::I32)
        .col("LevelId", 4, Field::I32)
        .col("Populate", 8, Field::I32)
        .col("Logicals", 12, Field::I32)
        .col("Outdoors", 16, Field::I32)
        .col("Animate", 20, Field::I32)
        .col("KillEdge", 24, Field::I32)
        .col("FillBlanks", 28, Field::I32)
        .col("SizeX", 40, Field::I32)
        .col("SizeY", 44, Field::I32)
        .col("AutoMap", 48, Field::I32)
        .col("Scan", 52, Field::I32)
        .col("Pops", 56, Field::I32)
        .col("PopPad", 60, Field::I32)
        .col("Files", 64, Field::I32);
    for i in 0..6 {
        l.col(&format!("File{}", i + 1), 68 + i * 60, Field::Str(60));
    }
    l.col("Dt1Mask", 428, Field::U32);
    l.col("Expansion", 32, Field::I32);
    l
}

fn lvlwarp_layout() -> Layout {
    let mut l = Layout::new(LVLWARP_SIZE);
    l.col("Id", 0, Field::I32)
        .col("SelectX", 4, Field::I32)
        .col("SelectY", 8, Field::I32)
        .col("SelectDX", 12, Field::I32)
        .col("SelectDY", 16, Field::I32)
        .col("ExitWalkX", 20, Field::I32)
        .col("ExitWalkY", 24, Field::I32)
        .col("OffsetX", 28, Field::I32)
        .col("OffsetY", 32, Field::I32)
        .col("LitVersion", 36, Field::I32)
        .col("Tiles", 40, Field::I32)
        .col("Direction", 44, Field::Str(4));
    l
}

fn objects_layout() -> Layout {
    use Field::{I32, Index, U8, U16};
    let mut l = Layout::new(OBJECTS_SIZE);
    l.col("Name", 0, Field::Str(64))
        .col("Id", 0, Index)
        .col("Token", 192, Field::Str(3))
        .col("SpawnMax", 195, U8)
        .array("Selectable", 0, 8, 196, U8)
        .col("TrapProb", 204, I32)
        .col("SizeX", 208, I32)
        .col("SizeY", 212, I32)
        .col("nTgtFX", 408, U8)
        .col("nTgtFY", 409, U8)
        .col("nTgtBX", 410, U8)
        .col("nTgtBY", 411, U8)
        .array("FrameCnt", 0, 8, 216, I32)
        .array("FrameDelta", 0, 8, 248, U16)
        .array("CycleAnim", 0, 8, 264, U8)
        .array("Lit", 0, 8, 272, U8)
        .array("BlocksLight", 0, 8, 280, U8)
        .array("HasCollision", 0, 8, 288, U8)
        .col("IsAttackable0", 296, U8)
        .array("Start", 0, 8, 297, U8)
        .col("EnvEffect", 313, U8)
        .col("IsDoor", 314, U8)
        .col("BlocksVis", 315, U8)
        .col("Orientation", 316, U8)
        .col("Trans", 318, U8)
        .array("OrderFlag", 0, 8, 305, U8)
        .col("PreOperate", 317, U8)
        .array("Mode", 0, 8, 319, U8)
        .col("Yoffset", 332, I32)
        .col("Xoffset", 328, I32)
        .col("Draw", 336, U8)
        .col("Red", 356, U8)
        .col("Green", 357, U8)
        .col("Blue", 358, U8);
    for (i, part) in ["HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH"]
        .iter()
        .enumerate()
    {
        l.col(part, 337 + i, U8);
    }
    l.array("S", 1, 8, 345, U8)
        .col("TotalPieces", 353, U8)
        .col("SubClass", 359, U8)
        .col("Xspace", 354, Field::I8)
        .col("Yspace", 355, Field::I8)
        .col("NameOffset", 360, I32)
        .col("MonsterOK", 365, U8)
        .col("OperateRange", 366, U8)
        .col("Restore", 371, U8)
        .array("Parm", 0, 8, 376, I32)
        .col("Act", 368, U8)
        .col("Lockable", 369, U8)
        .col("Gore", 370, U8)
        .col("Sync", 373, U8)
        .col("Damage", 412, U8)
        .col("Beta", 432, U8)
        .col("Overlay", 437, U8)
        .col("CollisionSubst", 413, U8)
        .col("Left", 416, I32)
        .col("Top", 420, I32)
        .col("Width", 424, I32)
        .col("Height", 428, I32)
        .col("OperateFn", 435, U8)
        .col("PopulateFn", 434, U8)
        .col("InitFn", 433, U8)
        .col("ClientFn", 436, U8)
        .col("RestoreVirgins", 372, U8)
        .col("BlockMissile", 438, U8)
        .col("DrawUnder", 439, U8)
        .col("OpenWarp", 440, I32)
        .col("AutoMap", 444, I32);
    l
}

fn levels_layout() -> Layout {
    use Field::{I32, U8, U16, U32};
    let mut l = Layout::new(LEVELS_SIZE);
    l.col("Id", 0, U16)
        .col("Pal", 2, U8)
        .col("Act", 3, U8)
        .col("Teleport", 4, U8)
        .col("Rain", 5, U8)
        .col("Mud", 6, U8)
        .col("NoPer", 7, U8)
        .col("IsInside", 8, U8)
        .col("DrawEdges", 9, U8)
        .col("WarpDist", 12, U32)
        .col("MonLvl1", 16, U16)
        .col("MonLvl2", 18, U16)
        .col("MonLvl3", 20, U16)
        .col("MonLvl1Ex", 22, U16)
        .col("MonLvl2Ex", 24, U16)
        .col("MonLvl3Ex", 26, U16)
        .col("MonDen", 28, U32)
        .col("MonDen(N)", 32, U32)
        .col("MonDen(H)", 36, U32)
        .col("MonUMin", 40, U8)
        .col("MonUMin(N)", 41, U8)
        .col("MonUMin(H)", 42, U8)
        .col("MonUMax", 43, U8)
        .col("MonUMax(N)", 44, U8)
        .col("MonUMax(H)", 45, U8)
        .col("MonWndr", 46, U8)
        .col("MonSpcWalk", 47, U8)
        .col("Quest", 48, U8)
        .col("rangedspawn", 49, U8)
        .col("NumMon", 50, U8)
        .col("Waypoint", 228, U8)
        .array("ObjGrp", 0, 8, 229, U8)
        .array("ObjPrb", 0, 8, 237, U8)
        .col("LevelName", 245, Field::Str(40))
        .col("LevelWarp", 285, Field::Str(40))
        .col("EntryFile", 325, Field::Str(40))
        .col("Themes", 528, I32)
        .col("FloorFilter", 532, I32)
        .col("BlankScreen", 536, I32)
        .col("SoundEnv", 540, I32);
    l
}

fn leveldefs_layout() -> Layout {
    use Field::{I32, U8};
    let mut l = Layout::new(LEVELDEFS_SIZE);
    l.col("QuestFlag", 0, I32)
        .col("QuestFlagEx", 4, I32)
        .col("Layer", 8, I32)
        .col("SizeX", 12, I32)
        .col("SizeX(N)", 16, I32)
        .col("SizeX(H)", 20, I32)
        .col("SizeY", 24, I32)
        .col("SizeY(N)", 28, I32)
        .col("SizeY(H)", 32, I32)
        .col("OffsetX", 36, I32)
        .col("OffsetY", 40, I32)
        .col("Depend", 44, I32)
        .col("LOSDraw", 152, I32)
        .col("DrlgType", 48, I32)
        .col("LevelType", 52, I32)
        .col("SubType", 56, I32)
        .col("SubTheme", 60, I32)
        .col("SubWaypoint", 64, I32)
        .col("SubShrine", 68, I32)
        .array("Vis", 0, 8, 72, I32)
        .array("Warp", 0, 8, 104, I32)
        .col("Intensity", 136, U8)
        .col("Red", 137, U8)
        .col("Green", 138, U8)
        .col("Blue", 139, U8)
        .col("Portal", 140, I32)
        .col("Position", 144, I32)
        .col("SaveMonsters", 148, I32);
    l
}

fn monpreset_layout() -> Layout {
    let mut l = Layout::new(MONPRESET_SIZE);
    l.col("Act", 0, Field::U8)
        .col("PlaceKind", 1, Field::U8)
        .col("PlaceIndex", 2, Field::U16);
    l
}

/// Flag columns of `monstats`, in bit order of the word at `0xC`.
const MONSTATS_FLAGS: [&str; 29] = [
    "isSpawn",
    "isMelee",
    "noRatio",
    "opendoors",
    "SetBoss",
    "BossXfer",
    "boss",
    "primeevil",
    "npc",
    "interact",
    "inTown",
    "lUndead",
    "hUndead",
    "demon",
    "flying",
    "killable",
    "switchai",
    "nomultishot",
    "neverCount",
    "petIgnore",
    "deathDmg",
    "genericSpawn",
    "zoo",
    "placespawn",
    "inventory",
    "enabled",
    "NoShldBlock",
    "noAura",
    "rangedtype",
];

fn monstats_layout() -> Layout {
    use Field::{I8, I16, Index, U8, U16};
    let mut l = Layout::new(MONSTATS_SIZE);
    l.col("hcIdx", 0, Index)
        .col("BaseIdRow", 2, I16)
        .col("NextInClassRow", 4, I16)
        .col("NameStrId", 6, U16)
        .bits(&MONSTATS_FLAGS, 12, 0)
        .col("Code", 16, Field::StrPad(4))
        .col("MonStatsExRow", 24, I16)
        .col("spawnRow", 32, I16)
        .col("spawnx", 34, I8)
        .col("spawny", 35, I8)
        .col("minion1Row", 38, I16)
        .col("minion2Row", 40, I16)
        .col("PartyMin", 44, U8)
        .col("PartyMax", 45, U8)
        .col("Rarity", 46, U8)
        .col("MinGrp", 47, U8)
        .col("MaxGrp", 48, U8)
        .col("sparsePopulate", 49, U8)
        .col("Align", 76, U8)
        .col("TransLvl", 77, U8)
        .col("threat", 78, U8)
        .col("TCQuestId", 158, U8)
        .col("TCQuestCP", 159, U8)
        .col("Level", 170, U16)
        .col("Level(N)", 172, U16)
        .col("Level(H)", 174, U16);
    l
}

fn monstats2_layout() -> Layout {
    use Field::{Bit, I16, U8, U32};
    let mut l = Layout::new(MONSTATS2_SIZE);
    l.bits(
        &[
            "noGfxHitTest",
            "noMap",
            "noOvly",
            "isSel",
            "alSel",
            "noSel",
            "shiftSel",
            "corpseSel",
            "revive",
            "isAtt",
            "small",
            "large",
            "soft",
            "critter",
            "Shadow",
            "noUniqueShift",
            "compositeDeath",
            "inert",
            "objCol",
            "deadCol",
            "unflatDead",
        ],
        4,
        0,
    )
    .col("SizeX", 8, U8)
    .col("SizeY", 9, U8)
    .col("spawnCol", 10, U8)
    .col("Height", 11, U8)
    .col("OverlayHeight", 12, U8)
    .col("pixHeight", 13, U8)
    .col("MeleeRng", 14, U8)
    .col("BaseW", 16, Field::StrPad(4))
    .col("HitClass", 20, U8)
    .bits(
        &[
            "HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH", "S1", "S2", "S3", "S4", "S5", "S6",
            "S7", "S8",
        ],
        232,
        0,
    )
    .col("TotalPieces", 236, U32)
    .bits(
        &[
            "mDT", "mNU", "mWL", "mGH", "mA1", "mA2", "mBL", "mSC", "mS1", "mS2", "mS3", "mS4",
            "mDD", "mKB", "mSQ", "mRN",
        ],
        240,
        0,
    );
    for (i, name) in [
        "dDT", "dNU", "dWL", "dGH", "dA1", "dA2", "dBL", "dSC", "dS1", "dS2", "dS3", "dS4", "dDD",
        "dKB", "dSQ", "dRN",
    ]
    .iter()
    .enumerate()
    {
        l.col(name, 244 + i, U8);
    }
    l.col("A2mv", 260, Bit(5))
        .col("S1mv", 260, Bit(8))
        .col("S2mv", 260, Bit(9))
        .col("InfernoLen", 264, U8)
        .col("InfernoAnim", 265, U8)
        .col("InfernoRollback", 266, U8)
        .col("htTop", 270, I16)
        .col("htLeft", 272, I16)
        .col("htWidth", 274, I16)
        .col("htHeight", 276, I16)
        .col("localBlood", 284, U8)
        .col("Bleed", 285, U8)
        .col("Light", 286, U8)
        .col("light-r", 287, U8)
        .col("light-g", 288, U8)
        .col("light-b", 289, U8)
        .col("Utrans", 290, U8)
        .col("Utrans(N)", 291, U8)
        .col("Utrans(H)", 292, U8);
    l
}

fn superuniques_layout() -> Layout {
    use Field::{I16, U8, U16, U32};
    let mut l = Layout::new(SUPERUNIQUES_SIZE);
    l.col("NameStrId", 2, U16)
        .col("ClassRow", 4, I16)
        .col("hcIdx", 8, U32)
        .col("Mod1", 12, U32)
        .col("Mod2", 16, U32)
        .col("Mod3", 20, U32)
        .col("MinGrp", 28, U32)
        .col("MaxGrp", 32, U32)
        .col("AutoPos", 36, U8)
        .col("EClass", 37, U8)
        .col("Stacks", 38, U8)
        .col("Replaceable", 39, U8)
        .col("Utrans", 40, U8)
        .col("Utrans(N)", 41, U8)
        .col("Utrans(H)", 42, U8);
    l
}

/// The MonPlace rows: `monplace.txt`'s `code` column in file order.
/// `monplace.bin` is just `0..37` as `u16` and is byte-identical in vanilla and
/// Median XL, so the names exist only in the vanilla `.txt`.
const MONPLACE_CODES: [&str; 37] = [
    "place_nothing",
    "place_npc_pack",
    "place_unique_pack",
    "place_champion",
    "place_rogue_warner",
    "place_bloodraven",
    "place_rivermonster_right",
    "place_rivermonster_left",
    "place_tightspotboss",
    "place_amphibian",
    "place_tentacle_ns",
    "place_tentacle_ew",
    "place_fallennest",
    "place_fetishnest",
    "place_talkingrogue",
    "place_talkingguard",
    "place_dumbguard",
    "place_fallen",
    "place_fallenshaman",
    "place_maggot",
    "place_maggotegg",
    "place_mosquitonest",
    "place_fetish",
    "place_fetishshaman",
    "place_impgroup",
    "place_imp",
    "place_miniongroup",
    "place_minion",
    "place_bloodlord",
    "place_deadminion",
    "place_deadimp",
    "place_deadbarb",
    "place_reanimateddead",
    "place_group25",
    "place_group50",
    "place_group75",
    "place_group100",
];

/// The `monplace.txt` code of MonPlace row `index` (`PlaceKind` 0 of
/// [`ExcelTableKind::MonPreset`]).
pub fn monplace_code(index: usize) -> Option<&'static str> {
    MONPLACE_CODES.get(index).copied()
}

fn layout(kind: ExcelTableKind) -> Layout {
    match kind {
        ExcelTableKind::LvlTypes => lvltypes_layout(),
        ExcelTableKind::LvlPrest => lvlprest_layout(),
        ExcelTableKind::LvlWarp => lvlwarp_layout(),
        ExcelTableKind::Objects => objects_layout(),
        ExcelTableKind::ObjPreset => Layout::new(OBJPRESET_SIZE),
        ExcelTableKind::Levels => levels_layout(),
        ExcelTableKind::MonPreset => monpreset_layout(),
        ExcelTableKind::MonStats => monstats_layout(),
        ExcelTableKind::MonStats2 => monstats2_layout(),
        ExcelTableKind::SuperUniques => superuniques_layout(),
    }
}

/// Decodes a `.bin` into a table using the `.txt` column names; columns the
/// record does not store are omitted (see the module docs per table).
///
/// `Levels` decodes `levels.bin` only; the game keeps `DrlgType`, `LevelType`
/// and the other DRLG columns in `leveldefs.bin`, which
/// [`decode_levels_bin`] merges in.
///
/// Fails with `Invalid` unless the length is exactly `4 + count * record_size`.
pub fn decode_bin(kind: ExcelTableKind, _version: ExcelVersion, bytes: &[u8]) -> Result<TxtTable> {
    if kind == ExcelTableKind::ObjPreset {
        return decode_objpreset(bytes);
    }
    layout(kind).decode(bytes)
}

fn decode_objpreset(bytes: &[u8]) -> Result<TxtTable> {
    let count = record_count(bytes, OBJPRESET_SIZE)?;
    let headers = vec!["Act".to_string(), "Id".to_string(), "ObjectsId".to_string()];
    let mut rows = Vec::with_capacity(count);
    let (chunks, _) = bytes.get(4..).unwrap_or(&[]).as_chunks::<4>();
    for (i, chunk) in chunks.iter().enumerate() {
        let value = i32::from_le_bytes(*chunk);
        rows.push(vec![
            (i / 150).to_string(),
            (i % 150).to_string(),
            value.to_string(),
        ]);
    }
    Ok(TxtTable::from_parts(headers, rows))
}

/// Changes one field of one record of a compiled table and returns the new
/// bytes; every byte outside that field is copied unchanged. `column` is a
/// name [`decode_bin`] yields (`"File 3"` of LvlTypes, `"Dt1Mask"` of
/// LvlPrest...), `record` the zero-based record, `value` the text the column
/// shows. Strings need room for their NUL (`len < field size`) and must be
/// Latin-1; numbers must fit the field; the `Index` column cannot be set.
pub fn patch_bin_field(
    kind: ExcelTableKind,
    bytes: &[u8],
    record: usize,
    column: &str,
    value: &str,
) -> Result<Vec<u8>> {
    let l = layout(kind);
    let count = record_count(bytes, l.record_size)?;
    if record >= count {
        return Err(invalid(
            "record",
            format!("record {record} of {count} does not exist"),
        ));
    }
    let col = l
        .columns
        .iter()
        .find(|c| c.name == column)
        .ok_or_else(|| invalid("column", format!("no column {column}")))?;
    let at = 4 + record * l.record_size + col.offset;
    let mut out = bytes.to_vec();
    let num = |lo: i64, hi: i64| -> Result<i64> {
        value
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|v| (lo..=hi).contains(v))
            .ok_or_else(|| invalid("value", format!("{value:?} does not fit {column}")))
    };
    let put = |out: &mut Vec<u8>, n: u32, w: usize| {
        for (i, b) in n.to_le_bytes().iter().take(w).enumerate() {
            out[at + i] = *b;
        }
    };
    match col.field {
        Field::Index => return Err(invalid("column", format!("{column} cannot be set"))),
        Field::U8 => put(&mut out, num(0, 255)? as u32, 1),
        Field::I8 => put(&mut out, num(-128, 127)? as i8 as u8 as u32, 1),
        Field::U16 => put(&mut out, num(0, 65535)? as u32, 2),
        Field::I16 => put(&mut out, num(-32768, 32767)? as i16 as u16 as u32, 2),
        Field::I32 => put(
            &mut out,
            num(i64::from(i32::MIN), i64::from(i32::MAX))? as i32 as u32,
            4,
        ),
        Field::U32 => put(&mut out, num(0, i64::from(u32::MAX))? as u32, 4),
        Field::Bit(bit) => {
            let word = u32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
            let new = match num(0, 1)? {
                0 => word & !(1 << bit),
                _ => word | (1 << bit),
            };
            put(&mut out, new, 4);
        }
        Field::Str(n) | Field::StrPad(n) => {
            let latin: Option<Vec<u8>> = value
                .chars()
                .map(|c| u8::try_from(u32::from(c)).ok())
                .collect();
            let latin = latin
                .filter(|b| b.len() < n && !b.contains(&0))
                .ok_or_else(|| {
                    invalid(
                        "value",
                        format!("{value:?} does not fit {column} ({} bytes with its NUL)", n),
                    )
                })?;
            out[at..at + n].fill(0);
            out[at..at + latin.len()].copy_from_slice(&latin);
        }
    }
    Ok(out)
}

/// Decodes `levels.bin` and `leveldefs.bin` together into one table with the
/// columns of `Levels.txt` that the two records store: everything
/// [`ExcelTableKind::Levels`] yields plus `QuestFlag`, `Layer`, the sizes,
/// `Depend`, `DrlgType`, `LevelType`, `Sub*`, `Vis0..7`, `Warp0..7`, light
/// colour, `Portal`, `Position`, `SaveMonsters` and `LOSDraw`. Both files must
/// hold the same number of records.
pub fn decode_levels_bin(
    version: ExcelVersion,
    levels: &[u8],
    leveldefs: &[u8],
) -> Result<TxtTable> {
    let main = decode_bin(ExcelTableKind::Levels, version, levels)?;
    let defs = leveldefs_layout().decode(leveldefs)?;
    if main.len() != defs.len() {
        return Err(invalid(
            "record count",
            format!(
                "levels.bin has {} records but leveldefs.bin has {}",
                main.len(),
                defs.len()
            ),
        ));
    }
    let mut headers: Vec<String> = main.headers().to_vec();
    headers.extend(defs.headers().iter().cloned());
    let rows = main
        .rows()
        .zip(defs.rows())
        .map(|(a, b)| {
            let mut row = a.cells().to_vec();
            row.extend(b.cells().iter().cloned());
            row
        })
        .collect();
    Ok(TxtTable::from_parts(headers, rows))
}

/// The version whose record size fits the file.
///
/// The two versions share one layout, so a file that fits is reported as
/// [`ExcelVersion::V113c`]; `None` when the length does not fit the record
/// size at all.
pub fn detect_version(kind: ExcelTableKind, bytes: &[u8]) -> Option<ExcelVersion> {
    let fits = |v: ExcelVersion| record_count(bytes, record_size(kind, v)).is_ok();
    [ExcelVersion::V113c, ExcelVersion::V114d]
        .into_iter()
        .find(|&v| fits(v))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
