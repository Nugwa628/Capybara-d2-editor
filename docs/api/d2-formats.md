# `d2-formats` public API (owner: lead)

Pure parsing. No Bevy, no I/O beyond `&[u8]`, no `unsafe`, no panics on bad data.
Changing anything here needs the lead's approval. Private helpers are free.

```rust
// ───────────── lib.rs ─────────────
pub mod error;    pub use error::{FormatError, Result};
pub mod grid;     pub use grid::Grid;
pub mod image;    pub use image::IndexedImage;
pub mod ds1;      // wave 1
pub mod txt;      // wave 1
pub mod excel;    // wave 1 (spike)
pub mod tables;   // wave 1
pub mod tbl;      // wave 1
pub mod palette;  // wave 2
pub mod dt1;      // wave 2
pub mod dc6;      // wave 2
pub mod dcc;      // wave 2
pub mod cof;      // wave 2
pub mod animdata; // wave 2
pub mod blend;    // sprite blend modes (moved from capy-render, lead request 2026-10-19)

// ───────────── error.rs ─────────────
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    #[error("{format}: unexpected end of data at offset {offset:#x} (needed {needed} more bytes)")]
    Truncated { format: &'static str, offset: usize, needed: usize },
    #[error("{format}: invalid {what} at offset {offset:#x}: {detail}")]
    Invalid { format: &'static str, what: &'static str, offset: usize, detail: String },
    #[error("{format}: unsupported version {version}")]
    UnsupportedVersion { format: &'static str, version: i64 },
    #[error("{format}: missing column {column:?}")]
    MissingColumn { format: &'static str, column: String },
    /// The model cannot be written in the requested file version
    /// (e.g. two wall layers in a v3 DS1).
    #[error("{format}: cannot write version {version}: {detail}")]
    NotRepresentable { format: &'static str, version: i64, detail: String },
}
pub type Result<T, E = FormatError> = std::result::Result<T, E>;

// ───────────── grid.rs ─────────────
/// Row-major, x fastest.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Grid<T> { /* width: u32, height: u32, cells: Vec<T> */ }
impl<T: Copy + Default> Grid<T> {
    pub fn new(width: u32, height: u32) -> Self;
    pub fn from_vec(width: u32, height: u32, cells: Vec<T>) -> Option<Self>; // None on len mismatch
    pub fn width(&self) -> u32;
    pub fn height(&self) -> u32;
    pub fn get(&self, x: u32, y: u32) -> Option<T>;
    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut T>;
    pub fn set(&mut self, x: u32, y: u32, value: T) -> bool; // false when out of bounds
    pub fn as_slice(&self) -> &[T];
    pub fn as_mut_slice(&mut self) -> &mut [T];
    /// New size, keeping the top-left overlap; new cells are `fill`.
    pub fn resized(&self, width: u32, height: u32, fill: T) -> Self;
}

// ───────────── image.rs ─────────────
/// Palette-indexed image. `alpha[i] == 0` is transparent, 255 opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    /// Position of pixel (0,0) relative to the tile/sprite anchor.
    pub offset_x: i32,
    pub offset_y: i32,
    pub pixels: Vec<u8>,
    pub alpha: Vec<u8>,
}
impl IndexedImage {
    pub fn new(width: u32, height: u32, offset_x: i32, offset_y: i32) -> Self; // all transparent
    pub fn to_rgba(&self, palette: &crate::palette::Palette) -> Vec<u8>;       // wave 2
}

// ───────────── ds1.rs ─────────────
pub const MAX_WALL_LAYERS: usize = 4;
pub const MAX_FLOOR_LAYERS: usize = 2;
pub const WORKSPACE_PREFIX: &[u8] = b"DS1EDIT_WRKSPC_";
/// Orientation remap applied to raw values when version < 7.
pub const DIR_LOOKUP: [u8; 25];

/// One cell dword of a wall/floor/shadow layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Cell(pub u32);
impl Cell {
    pub const EMPTY: Cell = Cell(0);
    pub fn prop1(self) -> u8;          // bits 0-7
    pub fn sub_index(self) -> u8;      // bits 8-15
    pub fn main_index(self) -> u8;     // bits 20-25
    pub fn unwalkable(self) -> bool;   // bit 17
    pub fn hidden(self) -> bool;       // bit 31
    pub fn is_empty(self) -> bool;     // rule confirmed against reference ds1.c / preview.c
    pub fn with_indices(self, main: u8, sub: u8) -> Cell; // keeps all other bits
    pub fn with_hidden(self, hidden: bool) -> Cell;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallLayer {
    pub tiles: Grid<Cell>,
    /// Raw orientation dwords exactly as stored.
    pub orientations: Grid<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Object { pub kind: i32, pub id: i32, pub x: i32, pub y: i32, pub flags: i32 } // flags: v>=6, else 0
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Group { pub x: i32, pub y: i32, pub width: i32, pub height: i32, pub unknown: i32 } // unknown: v>=13
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathPoint { pub x: i32, pub y: i32, pub action: i32 } // action: v>=15, else 1
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcPath { pub x: i32, pub y: i32, pub points: Vec<PathPoint> }

/// Bytes that are not part of the editable model but must survive a round trip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preserved {
    pub unknown_v9_13: [i32; 2],   // v9..=13 only
    pub pre_group_dword: i32,      // v>=18 with a group block
    /// Bytes after the last block the version defines (e.g. the NPC count
    /// v12/13 authoring tools wrote, swamp2.ds1's extra groups).
    pub trailing: Vec<u8>,
    /// Bytes the declared structure needed past EOF; they were read as zero.
    /// `to_bytes` drops them again if they are still all zero.
    pub missing: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ds1 {
    pub version: i32,
    pub width: u32,           // real width  (stored value + 1)
    pub height: u32,          // real height (stored value + 1)
    pub act: i32,             // raw stored value, 0-based; 0 when v<8
    pub tag_type: i32,        // 0 when v<10
    /// Raw file-list strings without their NUL, in order, including
    /// DS1EDIT_WRKSPC_* entries. Empty when v<3.
    pub files: Vec<Vec<u8>>,
    pub walls: Vec<WallLayer>,
    pub floors: Vec<Grid<Cell>>,
    pub shadow: Grid<Cell>,
    pub tag: Option<Grid<u32>>, // Some when v<4, or 1<=tag_type<=2
    pub objects: Vec<Object>,
    pub groups: Vec<Group>,
    pub npc_paths: Vec<NpcPath>,
    pub preserved: Preserved,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Report { pub missing_bytes: usize, pub trailing_bytes: usize, pub warnings: Vec<String> }

/// Zero-padding past EOF is tolerated only in the object/group/NPC blocks, up to this many bytes.
pub const MAX_MISSING_BYTES: usize = 256;
impl Ds1 {
    pub fn parse(bytes: &[u8]) -> Result<Ds1>;   // header and cell layers must be complete
    pub fn parse_with_report(bytes: &[u8]) -> Result<(Ds1, Ds1Report)>;
    /// Writes `self.version`'s layout. parse→to_bytes is byte-identical.
    pub fn to_bytes(&self) -> Result<Vec<u8>>;
    pub fn new(width: u32, height: u32, act: i32) -> Ds1; // v18, 1 wall, 1 floor, no tag
    pub fn effective_act(&self) -> u8;                    // act clamped to 0..=4
    /// Effective orientation (DIR_LOOKUP applied when version < 7).
    pub fn orientation(&self, wall: usize, x: u32, y: u32) -> Option<u8>; // None for v<7 raw values >= 25
    pub fn workspace_entries(&self) -> impl Iterator<Item = &[u8]>; // files starting with WORKSPACE_PREFIX
    pub fn tileset_refs(&self) -> impl Iterator<Item = &[u8]>;      // the other files
    pub fn add_wall_layer(&mut self) -> Result<usize>;  // NotRepresentable past MAX or for v<4
    pub fn add_floor_layer(&mut self) -> Result<usize>; // NotRepresentable past MAX or for v<16
    /// Smallest version >= self.version that can hold the current model.
    pub fn min_version_for_model(&self) -> i32;
}

// ───────────── txt.rs ─────────────
/// Tab-separated excel table. Latin-1 decoded, CRLF or LF, ragged rows padded with "".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxtTable { /* headers: Vec<String>, rows: Vec<Vec<String>> */ }
impl TxtTable {
    pub fn parse(bytes: &[u8]) -> Result<TxtTable>;
    pub fn from_parts(headers: Vec<String>, rows: Vec<Vec<String>>) -> TxtTable;
    pub fn headers(&self) -> &[String];
    pub fn column(&self, name: &str) -> Option<usize>; // ASCII case-insensitive, first match
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn row(&self, index: usize) -> Option<Row<'_>>;
    pub fn rows(&self) -> impl Iterator<Item = Row<'_>>;
}
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> { /* table: &'a TxtTable, index: usize */ }
impl<'a> Row<'a> {
    pub fn index(&self) -> usize;
    pub fn get(&self, column: &str) -> Option<&'a str>;
    pub fn get_i64(&self, column: &str) -> Option<i64>; // None when missing or empty
    pub fn cells(&self) -> &'a [String];
}

// ───────────── txt.rs: lossless writer (txt/edit.rs) ─────────────
/// Byte-exact editable table. Unedited lines (and everything outside an edited
/// cell) keep their bytes: line endings, BOM, blank/comment rows, ragged rows,
/// missing final newline. Row r is line r+1; the header is line 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxtDoc { /* bom, lines: raw content + own terminator */ }
impl TxtDoc {
    pub fn parse(bytes: &[u8]) -> TxtDoc;          // infallible
    pub fn to_bytes(&self) -> Vec<u8>;             // == input when unedited
    pub fn headers(&self) -> Vec<String>;
    pub fn column(&self, name: &str) -> Option<usize>; // ASCII case-insensitive, first
    pub fn row_count(&self) -> usize;              // includes blank/trailing rows
    pub fn row_cells(&self, row: usize) -> Option<Vec<&[u8]>>; // as stored, unpadded
    pub fn cell_bytes(&self, row: usize, col: usize) -> Option<&[u8]>;
    pub fn cell(&self, row: usize, col: usize) -> Option<String>; // Latin-1
    /// Err (Invalid) for a missing row, chars > U+00FF, or tab/CR/LF in the value.
    pub fn set_cell(&mut self, row: usize, col: usize, value: &str) -> Result<()>;
    pub fn set_cell_bytes(&mut self, row: usize, col: usize, value: &[u8]) -> Result<()>;
    pub fn insert_row(&mut self, at: usize, cells: &[&str]) -> Result<()>; // at <= row_count()
    pub fn duplicate_row(&mut self, row: usize) -> Result<()>;  // copy goes after `row`
    pub fn delete_row(&mut self, row: usize) -> Result<()>;
    pub fn add_column(&mut self, name: &str, default: &str) -> Result<usize>; // new index
}

// ───────────── excel.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExcelVersion { V113c, V114d }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExcelTableKind { LvlTypes, LvlPrest, LvlWarp, Objects, ObjPreset, Levels, MonPreset, MonStats, MonStats2, SuperUniques }
impl ExcelTableKind {
    pub fn file_stem(self) -> &'static str;       // "lvltypes", ...
    pub fn all() -> &'static [ExcelTableKind];
}
pub fn record_size(kind: ExcelTableKind, version: ExcelVersion) -> usize; // identical for 1.13c and 1.14d
/// Levels is split across two files: levels.bin (0x220) and leveldefs.bin
/// (0x9C, no .txt of its own; holds DrlgType, LevelType, sizes, Vis, Warp...).
pub fn decode_levels_bin(version: ExcelVersion, levels_bin: &[u8], leveldefs_bin: &[u8]) -> Result<TxtTable>;
pub fn leveldefs_record_size(version: ExcelVersion) -> usize;
/// `.bin` = u32 record count + fixed-size records. Output uses the `.txt`
/// column names; columns the record does not store are omitted. A layout may
/// decode only a subset of columns (documented per table).
pub fn decode_bin(kind: ExcelTableKind, version: ExcelVersion, bytes: &[u8]) -> Result<TxtTable>;
/// V113c for any file whose size fits (1.13c and 1.14d layouts are identical,
/// so V114d is never returned); None otherwise.
pub fn detect_version(kind: ExcelTableKind, bytes: &[u8]) -> Option<ExcelVersion>;
// Not stored in .bin, so absent from decoded tables: Name (LvlTypes, LvlPrest,
// LvlWarp), Beta (LvlTypes, LvlPrest, LvlWarp, Levels), Objects
// "description - not loaded", Flicker, ShrineFunction. ObjPreset (Median XL
// only) decodes to Act, Id, ObjectsId: table[act*150 + id].
//
// Monster tables (record sizes 4, 0x1A8, 0x134, 0x34; vanilla 229/734/609/66
// rows, Median XL 350/3901/1634/90). The compiler stores NO names: every text
// that names a row of another table became that row's number, and NameStr /
// Name became a .tbl string id. So Id (MonStats, MonStats2), Superunique and
// Place are not decoded; the links decode under `...Row` / `...Id` names
// instead (-1 = blank). Every column below was compared with the vanilla .txt
// on every row (a test repeats it when the private corpus is present).
//  * MonPreset: Act, PlaceKind (0 MonPlace row, 1 MonStats row, 2 SuperUniques
//    row), PlaceIndex. `monplace_code(index)` names the 37 MonPlace rows
//    (monplace.bin holds only 0..37; the names exist only in monplace.txt).
//  * MonStats: hcIdx (= record index; vanilla's stored word is stale for the
//    last 11 rows), BaseIdRow, NextInClassRow, spawnRow, minion1Row, minion2Row
//    (MonStats rows), MonStatsExRow (a MONSTATS2 row), NameStrId, Code (trimmed
//    to the .txt value), the 29 flags isSpawn..rangedtype, TransLvl, Align,
//    threat, spawnx, spawny, PartyMin/Max, Rarity, MinGrp/MaxGrp,
//    sparsePopulate, Level, Level(N), Level(H), TCQuestId, TCQuestCP.
//  * MonStats2 (row = MonStatsExRow; offset 0 is the record index): SizeX,
//    SizeY, spawnCol, Height, OverlayHeight, pixHeight, MeleeRng, BaseW,
//    HitClass, HD TR LG RA LA RH LH SH S1..S8, TotalPieces, mDT..mRN, dDT..dRN,
//    A2mv, S1mv, S2mv, noGfxHitTest..unflatDead, htTop/Left/Width/Height,
//    localBlood, Bleed, Light, light-r/g/b, Utrans, Utrans(N), Utrans(H),
//    InfernoLen/Anim/Rollback.
//  * SuperUniques: NameStrId, ClassRow (MonStats row), hcIdx, Mod1..Mod3,
//    MinGrp, MaxGrp, AutoPos, EClass, Stacks, Replaceable, Utrans, Utrans(N),
//    Utrans(H).
// NameStrId: < 10000 string.tbl, 10000.. patchstring.tbl (id - 10000),
// 20000.. expansionstring.tbl (id - 20000); a blank/unknown key is the id of
// "dummy". Resolve with StringTable::entry_by_id. Not decoded (offsets not
// established): the many MonStats stat columns beyond the list above (HP,
// resistances, damage, treasure classes, AI params), the text-valued columns
// (MonProp, AI, sounds, skills, missiles, TreasureClass*), MonStats2 variant
// lists (HDv..S8v). Unverified order: SizeX/SizeY and the three SuperUniques
// Utrans bytes are equal in every vanilla row, so their order is the .txt's.
pub fn monplace_code(index: usize) -> Option<&'static str>;

// ───────────── tables.rs ─────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LvlType {
    pub row: usize,
    pub id: u32,
    pub name: String,
    pub act: u8,
    /// File1..File32 by slot; None for empty or "0".
    pub files: Vec<Option<String>>,
    pub expansion: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LvlPrest {
    pub row: usize,
    pub def: u32,
    pub level_id: i32,
    pub name: String,
    pub size_x: i32,
    pub size_y: i32,
    pub populate: bool,
    pub logicals: bool,
    pub outdoors: bool,
    pub animate: bool,
    pub kill_edge: bool,
    pub fill_blanks: bool,
    pub auto_map: bool,
    pub scan: bool,
    pub pops: i32,
    pub pop_pad: i32,
    /// File1..File6 by slot; None for empty or "0". Paths relative to data\global\tiles.
    pub files: Vec<Option<String>>,
    pub dt1_mask: u32,
    pub expansion: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// `name` is LevelName (the only name a .bin stores), falling back to Name.
/// `LvlType.act` and `Level.act` are both 0-based (Act 1 = 0). Names are empty for decoded bins.
pub struct Level { pub row: usize, pub id: u32, pub name: String, pub act: u8, pub pal: u8, pub level_type: u32, pub drlg_type: u32 }
pub fn lvl_types(table: &TxtTable) -> Result<Vec<LvlType>>;
pub fn lvl_prests(table: &TxtTable) -> Result<Vec<LvlPrest>>;
pub fn levels(table: &TxtTable) -> Result<Vec<Level>>;
impl LvlPrest {
    /// DT1 files of `lvl_type` selected by `dt1_mask` (bit i = File(i+1)), in slot order.
    pub fn dt1_files<'a>(&self, lvl_type: &'a LvlType) -> impl Iterator<Item = &'a str> + use<'a>;
}

// ───────────── tbl.rs ─────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringTable { /* entries: Vec<(String, String)> */ }
impl StringTable {
    pub fn parse(bytes: &[u8]) -> Result<StringTable>;
    pub fn get(&self, key: &str) -> Option<&str>;
    pub fn entries(&self) -> &[(String, String)];
    /// (key, text) of the node whose string id is `id` (the number compiled
    /// excel tables store; see ExcelTableKind::MonStats for the 10000/20000 bases).
    pub fn entry_by_id(&self, id: u16) -> Option<(&str, &str)>;
    pub fn entry_ids(&self) -> &[u16]; // parallel to entries()
}

// ───────────── tbl.rs: writer (tbl/write.rs) ─────────────
/// Strings by id (entry i has id i), plus the hash size and version byte of the
/// source. An unedited loaded table writes back its original bytes; after any
/// edit the layout is regenerated (see docs/specs/tbl.md section 7), which
/// reproduces all 40 corpus tables byte for byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TblDoc { /* entries, hash_size, version, original: Option<Vec<u8>> */ }
impl TblDoc {
    pub fn new() -> TblDoc;
    /// Err if StringTable::parse fails or active ids are not exactly 0..n.
    pub fn parse(bytes: &[u8]) -> Result<TblDoc>;
    pub fn entries(&self) -> &[(String, String)];   // by id
    pub fn get(&self, key: &str) -> Option<&str>;   // lowest id with `key`
    pub fn is_modified(&self) -> bool;
    /// Replace the text of `key` (id kept) or append it; returns the id.
    pub fn set(&mut self, key: &str, text: &str) -> Result<u16>;
    /// Append even if `key` exists; returns the id.
    pub fn add(&mut self, key: &str, text: &str) -> Result<u16>;
    /// Removes the first `key`; later ids shift down. false if absent.
    pub fn remove(&mut self, key: &str) -> bool;
    pub fn to_bytes(&self) -> Result<Vec<u8>>;      // original if unedited
    pub fn layout(&self) -> Result<Vec<u8>>;        // always regenerated
}
// Strings are Latin-1; chars > U+00FF, NUL, empty keys, texts >= 65535 bytes,
// > 65535 strings or > 65536 slots are Invalid.

// ───────────── palette.rs (wave 2) ─────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette { pub colors: [[u8; 3]; 256] } // RGB
impl Palette {
    pub fn parse_dat(bytes: &[u8]) -> Result<Palette>; // 768 bytes, BGR on disk
    pub fn with_gamma(&self, gamma: f32) -> Palette;  // 255*(i/255)^(1/g) per channel
}
/// "data\\global\\palette\\act{n}\\pal.dat" for act 0..=4.
pub fn act_palette_path(act: u8) -> &'static str;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pl2 {
    pub base: Palette,
    pub light_levels: Vec<[u8; 256]>,       // 32
    pub alpha_blend: Vec<Vec<[u8; 256]>>,   // [25%,50%,75%][src] -> dst map
    pub additive: Vec<[u8; 256]>,
    pub multiply: Vec<[u8; 256]>,
    // remaining tables per docs/guides/palettes-and-colour.md, pub, named
}
impl Pl2 { pub fn parse(bytes: &[u8]) -> Result<Pl2>; }

// ───────────── dt1.rs (wave 2) ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey { pub orientation: u8, pub main: u8, pub sub: u8 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFormat { Isometric, Rle }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block { pub x: i16, pub y: i16, pub grid_x: u8, pub grid_y: u8, pub raw_format: i16, pub format: BlockFormat, pub data: Vec<u8> }
impl Block {
    pub fn render(&self) -> Result<IndexedImage>;  // this block alone, block x/y space
    /// 32x15 diamond at (x, y) from the opaque pixels of `image` (block space); 0 elsewhere.
    pub fn isometric(x: i16, y: i16, grid_x: u8, grid_y: u8, image: &IndexedImage) -> Block;
    /// RLE block of the opaque pixels of `image`; at least `min_rows` rows terminated.
    /// NotRepresentable: raw_format 1, a pixel left of x / above y, data > 65535 bytes.
    pub fn rle(x: i16, y: i16, grid_x: u8, grid_y: u8, raw_format: i16, image: &IndexedImage, min_rows: usize) -> Result<Block>;
    /// Same position/grid/format, pixels re-encoded from `image` (RLE keeps its row count).
    pub fn with_pixels(&self, image: &IndexedImage) -> Result<Block>;
}
/// Tile-header bytes not interpreted, kept for byte-identical write-back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TilePreserved { pub unknown_16: [u8; 4], pub unknown_36: [u8; 4], pub unknown_65: [u8; 7], pub runtime_84: [u8; 12] }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tile {
    pub direction: i32, pub roof_height: i16, pub sound: u8, pub animated: u8,
    pub height: i32, pub width: i32,
    pub orientation: i32, pub main_index: i32, pub sub_index: i32,
    pub rarity: i32, pub subtile_flags: [u8; 25],
    pub blocks: Vec<Block>,
    pub preserved: TilePreserved,
}
impl Tile {
    pub fn key(&self) -> Option<TileKey>;          // None if an index is out of range
    pub fn render(&self) -> Result<IndexedImage>;  // offsets in block x/y space
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dt1 { pub version: (i32, i32), pub tiles: Vec<Tile> }
impl Dt1 {
    pub fn parse(bytes: &[u8]) -> Result<Dt1>;
    /// Canonical layout (P11); an unchanged parse writes back byte for byte,
    /// any model parses back to itself. UnsupportedVersion unless version.0 == 7;
    /// NotRepresentable for a block over 65535 bytes, format != raw_format, > 2 GiB.
    pub fn to_bytes(&self) -> Result<Vec<u8>>;
}
// remap::remap_indices still patches pixel literals in place (P7).
// Full corpora (CAPY_FULL_CORPUS=1): 619/619 readable DT1 byte-identical
// (the 12 4.1 files do not parse); all 895,820 blocks re-encode
// (with_pixels(render())) to the same pixels and the same bytes.

// ───────────── dc6.rs / dcc.rs / cof.rs / animdata.rs (wave 2) ─────────────
pub struct Dc6 { pub version: i32, pub flags: u32, pub encoding: u32, pub directions: u32, pub frames_per_direction: u32, pub frames: Vec<Dc6Frame> }
pub struct Dc6Frame { pub flipped: bool, pub width: u32, pub height: u32, pub offset_x: i32, pub offset_y: i32, pub data: Vec<u8> }
impl Dc6 { pub fn parse(bytes: &[u8]) -> Result<Dc6>; pub fn frame(&self, dir: u32, frame: u32) -> Option<&Dc6Frame>; }
impl Dc6Frame { pub fn render(&self) -> Result<IndexedImage>; }

pub struct Dcc { pub version: u8, pub frames_per_direction: u32, pub directions: Vec<DccDirection> }
pub struct DccDirection { pub frames: Vec<IndexedImage> }
impl Dcc { pub fn parse(bytes: &[u8]) -> Result<Dcc>; }
/// The decoder's cell partition along one axis: (start relative to the direction box, length).
/// Partial first cell up to the 4-px grid, then 4-px cells; a trailing 1-px sliver merges (cells up to 5 px).
pub fn dcc::cell_spans(frame_min: i64, dir_min: i64, frame_len: i64) -> Vec<(i64, i64)>;

pub struct CofLayer { pub component: u8, pub shadow: u8, pub selectable: u8, pub transparent: u8, pub draw_effect: u8, pub weapon_class: [u8; 4] }
pub struct Cof { pub directions: u8, pub frames_per_direction: u8, pub layers: Vec<CofLayer>, pub keyframes: Vec<u8>, pub layer_order: Vec<u8> }
impl Cof {
    pub fn parse(bytes: &[u8]) -> Result<Cof>;
    pub fn draw_order(&self, dir: u8, frame: u8) -> Option<&[u8]>;
    /// The record that counts for `component`: the LAST one when a COF repeats it (cof.c:306-318).
    pub fn component_layer(&self, component: u8) -> Option<&CofLayer>;
}
impl CofLayer {
    pub fn component_code(&self) -> Option<&'static str>; // "HD".."S8"
    pub fn weapon_class_token(&self) -> String;           // b"HTH\0" -> "hth"
}
pub const cof::COMPONENT_CODES: [&str; 16];                 // HD TR LG RA LA RH LH SH S1..S8
/// NULs/whitespace trimmed, lower case: how a name part appears in a sprite path.
pub fn cof::path_token(s: &str) -> String;
/// `{base}\{token}\cof\{token}{mode}{class}.cof` (cof.c:253); `base` e.g. `data\global\monsters`.
pub fn cof::cof_path(base: &str, token: &str, mode: &str, weapon_class: &str) -> String;
/// `[.dcc, .dc6]` of `{base}\{token}\{cc}\{token}{cc}{armor}{mode}{class}` (cof.c:322-333); the .dc6 only when no .dcc exists.
pub fn cof::component_paths(base: &str, token: &str, mode: &str, weapon_class: &str, component: &str, armor: &str) -> [String; 2];

// blend.rs: shared by capy-render (re-exported as capy_render::Blend / capy_render::compose::Blend) and capy-sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend { Normal, Alpha25, Alpha50, Alpha75, Additive, Multiply, Screen, Darken(u8) }
impl Blend {
    /// COF trans_a/trans_b -> blend (preview.c:2131): blended only when transparent and trans_b is 0..=4 or 6.
    pub fn for_cof_layer(transparent: bool, draw_effect: u8) -> Blend;
    /// One source colour over a straight-RGBA destination (DS1Edit's Allegro blenders, preview.c:182,291-380).
    pub fn apply(self, dst: [u8; 4], src: [u8; 3]) -> [u8; 4];
}

pub struct AnimRecord { pub cof_name: String, pub frames_per_direction: u32, pub speed: u32, pub flags: [u8; 144] }
pub struct AnimData { /* records */ }
impl AnimData { pub fn parse(bytes: &[u8]) -> Result<AnimData>; pub fn get(&self, cof_name: &str) -> Option<&AnimRecord>; pub fn records(&self) -> &[AnimRecord]; }
```

## Palette index remapping (`d2_formats::remap`, P7)

Rewrites the palette indices of a whole sprite or tile file without touching
anything else, for the "Convert to palette" tool.

```rust
pub enum SpriteKind { Dt1, Dc6, Dcc }
impl SpriteKind {
    /// From the file extension of a game or file path (case-insensitive).
    pub fn from_path(path: &str) -> Option<SpriteKind>;
}

/// `table[i]` is the new index for old index `i`. `table[0]` must be 0 and
/// `table[i]` must not be 0 for `i != 0`: transparency is never created or
/// removed. Anything else is `FormatError::Invalid`.
pub type IndexTable = [u8; 256];

/// How many pixels use each index. Transparent skips are not pixels; literal
/// index-0 bytes count under 0.
pub fn index_histogram(kind: SpriteKind, bytes: &[u8]) -> Result<[u64; 256]>;

/// The file with every pixel index `i` replaced by `table[i]`.
/// - DT1 and DC6 are patched in place: every byte that is not a pixel
///   literal (headers, tile/frame tables, block headers, RLE control bytes,
///   padding, trailing bytes) is identical to the input, so an identity
///   table returns the input unchanged.
/// - DCC is decoded, mapped and re-encoded with the existing encoder: the
///   decoded frames equal the mapped input frames; the byte layout may
///   change. A mapping never adds colours to a 4x4 cell, so the cell limit
///   still holds.
/// Files `parse` rejects (including the DT1 4.1 files (six per install)) are errors.
pub fn remap_indices(kind: SpriteKind, bytes: &[u8], table: &IndexTable) -> Result<Vec<u8>>;
```
As implemented:
- `SpriteKind` derives `Clone, Copy, Debug, PartialEq, Eq, Hash`;
  `from_path` takes the extension of the last path segment (`\` or `/`).
- Table errors are `FormatError::Invalid { format: "remap", what: "index table", offset: <index> }`
  and are reported before the file is parsed.
- Histogram: DT1/DC6 count the literal bytes of every block/frame stream once
  per block/frame that draws them (tiles sharing sub-block data count it
  twice; DC6 literals a frame clips away still count). Isometric DT1 blocks
  that are not exactly 256 bytes draw nothing and count nothing. DCC counts
  the opaque pixels of the decoded frames, so its index-0 count is 0.
- DT1/DC6 data shared by several tiles/frames is patched once. Bytes no
  table entry, tile or block points at (padding, DC6 slack, unreferenced
  frame records, trailing data) are untouched.
- A damaged DT1/DC6 whose pixel literals overlap one of its own headers or
  RLE control bytes is `FormatError::Invalid { what: "pixel data" }` when
  the table would change such a byte (the identity table still returns it
  unchanged). No shipped file does this.
- DCC: the re-encoded file is decoded again and must give exactly the mapped
  frames, otherwise `FormatError::Invalid { what: "re-encoded frames" }`.
- Full corpora (vanilla + Median XL, `CAPY_FULL_CORPUS=1`): 643/655 DT1
  (the 12 others are the 4.1 files, six per install) and 4065/4065 DC6 are
  byte-identical under the identity table and change only pixel bytes under
  an act5 → act1 style table and a random table; 47,939/47,939 DCC decode to
  the mapped frames.
