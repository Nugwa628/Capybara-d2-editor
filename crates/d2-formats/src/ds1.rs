//! DS1 map presets.
//!
//! The format is a chain of conditionals on the first dword (the version); see
//! `docs/guides/ds1-map-format.md` in the reference tree for the full ladder.
//! This module reads and writes every version the game's loader accepts
//! (there is no maximum, negative versions take every "below" branch) and
//! keeps everything it does not model in [`Preserved`] and in raw fields so an
//! unchanged file is written back byte for byte.
//!
//! Version gates (a value `v >= N` means "at least version N"):
//!
//! | gate | adds |
//! |---|---|
//! | `v >= 2` | object block |
//! | `v >= 3` | file list |
//! | `v >= 4` | wall-layer count (below: one wall, five-block layer order) |
//! | `v >= 6` | per-object flags |
//! | `v < 7`  | orientation remap through [`DIR_LOOKUP`] (applied on read access only) |
//! | `v >= 8` | `act` |
//! | `9..=13` | two unknown dwords |
//! | `v >= 10` | `tag_type` |
//! | `v >= 12` | group block (when `tag_type` is 1 or 2) |
//! | `v >= 13` | fifth dword per group |
//! | `v >= 14` | NPC path block |
//! | `v >= 15` | `action` per path point |
//! | `v >= 16` | floor-layer count (below: one floor) |
//! | `v >= 18` | dword before the group count |

use crate::{FormatError, Grid, Result};

pub const MAX_WALL_LAYERS: usize = 4;
pub const MAX_FLOOR_LAYERS: usize = 2;
pub const WORKSPACE_PREFIX: &[u8] = b"DS1EDIT_WRKSPC_";

/// Orientation remap applied to raw values when `version < 7`
/// (`dir_lookup` in the reference `ds1.c`, byte for byte the table at
/// `0x6fddbcc0` in D2Common 1.13c).
pub const DIR_LOOKUP: [u8; 25] = [
    0, 1, 2, 1, 2, 3, 3, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20,
];

/// How many bytes past the end of the input the reader will treat as zero.
///
/// Two shipped files (`trees.ds1`, `xtransition.ds1`) declare 12 and 4 bytes
/// more than they contain and the game reads zeros there. Anything much larger
/// than that is a corrupt file, and bounding it also bounds every allocation.
pub const MAX_MISSING_BYTES: usize = 256;

const FORMAT: &str = "ds1";

/// One cell dword of a wall/floor/shadow layer.
///
/// Bit map (guide appendix): 0-7 layer tag (`prop1`), 8-15 `sub_index`,
/// 16 unknown, 17 unwalkable, 20-25 `main_index`, 26 unknown, 27 is-shadow,
/// 31 hidden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Cell(pub u32);

impl Cell {
    pub const EMPTY: Cell = Cell(0);

    /// Bits 0-7: the layer-type tag the authoring tool wrote.
    pub fn prop1(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// Bits 8-15.
    pub fn sub_index(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// Bits 20-25 (six bits, as D2Common's `AND ESI,0x3f`).
    pub fn main_index(self) -> u8 {
        ((self.0 >> 20) & 0x3F) as u8
    }

    /// Bit 17.
    pub fn unwalkable(self) -> bool {
        self.0 & (1 << 17) != 0
    }

    /// Bit 31.
    pub fn hidden(self) -> bool {
        self.0 & (1 << 31) != 0
    }

    /// A cell is empty exactly when its `prop1` byte is zero.
    ///
    /// This is how the reference editor decides occupancy
    /// (`ds1.c:129`, `:206`, `:240`; `editor/tiles.c:520,546,572`), and the
    /// guide measured it as a property of the shipped data: of the 2,761,752
    /// vanilla cells with a zero `prop1`, none has any other bit set. A fully
    /// zero dword is therefore the canonical empty cell ([`Cell::EMPTY`]).
    pub fn is_empty(self) -> bool {
        self.prop1() == 0
    }

    /// Replaces `main_index` (low 6 bits of `main` are used) and `sub_index`,
    /// keeping every other bit.
    pub fn with_indices(self, main: u8, sub: u8) -> Cell {
        let kept = self.0 & !((0x3F << 20) | (0xFF << 8));
        Cell(kept | (u32::from(main & 0x3F) << 20) | (u32::from(sub) << 8))
    }

    pub fn with_hidden(self, hidden: bool) -> Cell {
        if hidden {
            Cell(self.0 | (1 << 31))
        } else {
            Cell(self.0 & !(1 << 31))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallLayer {
    pub tiles: Grid<Cell>,
    /// Raw orientation dwords exactly as stored.
    pub orientations: Grid<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Object {
    pub kind: i32,
    pub id: i32,
    pub x: i32,
    pub y: i32,
    /// Stored from version 6; zero below.
    pub flags: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Group {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Stored from version 13; zero below.
    pub unknown: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathPoint {
    pub x: i32,
    pub y: i32,
    /// Stored from version 15; 1 below.
    pub action: i32,
}

/// A walking route, keyed by the owning object's sub-tile coordinate (not by
/// object index; two objects on one sub-tile are indistinguishable).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcPath {
    pub x: i32,
    pub y: i32,
    pub points: Vec<PathPoint>,
}

/// Bytes that are not part of the editable model but must survive a round trip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preserved {
    /// Versions 9..=13 only (vanilla always stores `(5000, 5000)`).
    pub unknown_v9_13: [i32; 2],
    /// Version 18+ with a group block.
    pub pre_group_dword: i32,
    /// Bytes after the last block the version defines (e.g. the NPC count the
    /// v12/13 authoring tools wrote, `swamp2.ds1`'s extra group records).
    pub trailing: Vec<u8>,
    /// Bytes the declared structure needed past EOF; they were read as zero.
    /// `to_bytes` drops them again if they are still all zero.
    pub missing: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ds1 {
    pub version: i32,
    /// Real width (stored value + 1).
    pub width: u32,
    /// Real height (stored value + 1).
    pub height: u32,
    /// Raw stored value, 0-based; 0 when `version < 8`.
    pub act: i32,
    /// 0 when `version < 10`.
    pub tag_type: i32,
    /// Raw file-list strings without their NUL, in order, including
    /// `DS1EDIT_WRKSPC_*` entries. Empty when `version < 3`.
    pub files: Vec<Vec<u8>>,
    pub walls: Vec<WallLayer>,
    pub floors: Vec<Grid<Cell>>,
    pub shadow: Grid<Cell>,
    /// `Some` when `version < 4`, or when `version >= 10` and `1 <= tag_type <= 2`.
    pub tag: Option<Grid<u32>>,
    pub objects: Vec<Object>,
    pub groups: Vec<Group>,
    pub npc_paths: Vec<NpcPath>,
    pub preserved: Preserved,
}

/// Facts noticed while parsing that do not stop the parse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Report {
    pub missing_bytes: usize,
    pub trailing_bytes: usize,
    pub warnings: Vec<String>,
}

// --------------------------------------------------------------------------
// version gates, shared by the reader, the writer and `min_version_for_model`

fn has_objects(v: i32) -> bool {
    v >= 2
}
fn has_files(v: i32) -> bool {
    v >= 3
}
fn has_layer_counts(v: i32) -> bool {
    v >= 4
}
fn has_object_flags(v: i32) -> bool {
    v >= 6
}
fn has_act(v: i32) -> bool {
    v >= 8
}
fn has_unknown_pair(v: i32) -> bool {
    (9..=13).contains(&v)
}
fn has_tag_type(v: i32) -> bool {
    v >= 10
}
fn has_floor_count(v: i32) -> bool {
    v >= 16
}
fn has_group_unknown(v: i32) -> bool {
    v >= 13
}
fn has_npc_block(v: i32) -> bool {
    v >= 14
}
fn has_path_action(v: i32) -> bool {
    v >= 15
}
fn has_pre_group_dword(v: i32) -> bool {
    v >= 18
}
/// Whether the group block exists for this version and `tag_type`.
fn has_groups(v: i32, tag_type: i32) -> bool {
    v >= 12 && (1..=2).contains(&tag_type)
}
/// Whether a tag layer exists for this version and `tag_type`.
fn has_tag_layer(v: i32, tag_type: i32) -> bool {
    v < 4 || (v >= 10 && (1..=2).contains(&tag_type))
}

// --------------------------------------------------------------------------
// reader

/// Cursor over a zero-padded copy of the input. Reads past the real end return
/// zeros (up to [`MAX_MISSING_BYTES`]); `pos - real_len` is the missing count.
struct Reader {
    buf: Vec<u8>,
    real_len: usize,
    pos: usize,
}

impl Reader {
    fn new(bytes: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(bytes.len() + MAX_MISSING_BYTES);
        buf.extend_from_slice(bytes);
        buf.resize(bytes.len() + MAX_MISSING_BYTES, 0);
        Reader {
            buf,
            real_len: bytes.len(),
            pos: 0,
        }
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// Fails unless `n` more bytes are available (padding included).
    fn need(&self, n: u64) -> Result<()> {
        if n > self.remaining() as u64 {
            return Err(FormatError::Truncated {
                format: FORMAT,
                offset: self.pos,
                needed: usize::try_from(n - self.remaining() as u64).unwrap_or(usize::MAX),
            });
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&[u8]> {
        self.need(n as u64)?;
        let start = self.pos;
        self.pos += n;
        self.buf
            .get(start..start + n)
            .ok_or(FormatError::Truncated {
                format: FORMAT,
                offset: start,
                needed: n,
            })
    }

    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([
            b.first().copied().unwrap_or(0),
            b.get(1).copied().unwrap_or(0),
            b.get(2).copied().unwrap_or(0),
            b.get(3).copied().unwrap_or(0),
        ]))
    }

    /// A NUL-terminated string (terminator consumed, not returned).
    fn cstring(&mut self) -> Result<Vec<u8>> {
        let start = self.pos;
        let rest = self.buf.get(start..).unwrap_or(&[]);
        match rest.iter().position(|&b| b == 0) {
            Some(len) => {
                let s = rest.get(..len).unwrap_or(&[]).to_vec();
                self.pos = start + len + 1;
                Ok(s)
            }
            None => Err(FormatError::Truncated {
                format: FORMAT,
                offset: self.buf.len(),
                needed: 1,
            }),
        }
    }

    /// `cells` little-endian dwords, after checking they fit.
    fn dwords(&mut self, cells: u64) -> Result<Vec<u32>> {
        let bytes = cells.checked_mul(4).ok_or(FormatError::Truncated {
            format: FORMAT,
            offset: self.pos,
            needed: usize::MAX,
        })?;
        self.need(bytes)?;
        let n = usize::try_from(cells).map_err(|_| FormatError::Truncated {
            format: FORMAT,
            offset: self.pos,
            needed: usize::MAX,
        })?;
        let raw = self.take(n * 4)?;
        let (chunks, _) = raw.as_chunks::<4>();
        Ok(chunks.iter().map(|c| u32::from_le_bytes(*c)).collect())
    }

    /// A non-negative element count.
    fn count(&mut self, what: &'static str) -> Result<u64> {
        let at = self.pos;
        let n = self.i32()?;
        u64::try_from(n).map_err(|_| FormatError::Invalid {
            format: FORMAT,
            what,
            offset: at,
            detail: format!("negative count {n}"),
        })
    }
}

fn grid_from(width: u32, height: u32, cells: Vec<u32>) -> Result<Grid<u32>> {
    Grid::from_vec(width, height, cells).ok_or(FormatError::Invalid {
        format: FORMAT,
        what: "layer",
        offset: 0,
        detail: "layer size does not match dimensions".to_string(),
    })
}

fn cell_grid(width: u32, height: u32, cells: Vec<u32>) -> Result<Grid<Cell>> {
    Grid::from_vec(width, height, cells.into_iter().map(Cell).collect()).ok_or(
        FormatError::Invalid {
            format: FORMAT,
            what: "layer",
            offset: 0,
            detail: "layer size does not match dimensions".to_string(),
        },
    )
}

impl Ds1 {
    /// Parses a DS1, ignoring the diagnostic report.
    pub fn parse(bytes: &[u8]) -> Result<Ds1> {
        Self::parse_with_report(bytes).map(|(ds1, _)| ds1)
    }

    /// Parses a DS1 and reports missing/trailing bytes and oddities.
    ///
    /// The header and all cell layers must be fully present. Anything the
    /// object, group and NPC-path blocks need past the end of `bytes` (up to
    /// [`MAX_MISSING_BYTES`]) reads as zero and is recorded in
    /// `preserved.missing`; anything after the last block is kept verbatim in
    /// `preserved.trailing`. Layer counts above [`MAX_WALL_LAYERS`] /
    /// [`MAX_FLOOR_LAYERS`] are rejected as invalid.
    pub fn parse_with_report(bytes: &[u8]) -> Result<(Ds1, Ds1Report)> {
        let mut r = Reader::new(bytes);
        let mut report = Ds1Report::default();

        let version = r.i32()?;
        let width = dimension(&mut r, "width")?;
        let height = dimension(&mut r, "height")?;
        let act = if has_act(version) { r.i32()? } else { 0 };
        let tag_type = if has_tag_type(version) { r.i32()? } else { 0 };

        let mut files = Vec::new();
        if has_files(version) {
            let n = r.count("file count")?;
            // every string is at least its NUL, so the count is bounded by the data
            r.need(n)?;
            files.reserve(usize::try_from(n).unwrap_or(0).min(r.remaining()));
            for _ in 0..n {
                files.push(r.cstring()?);
            }
        }

        let mut preserved = Preserved::default();
        if has_unknown_pair(version) {
            preserved.unknown_v9_13 = [r.i32()?, r.i32()?];
        }

        let wall_count = if has_layer_counts(version) {
            layer_count(&mut r, "wall layer count", MAX_WALL_LAYERS)?
        } else {
            1
        };
        let floor_count = if has_floor_count(version) {
            layer_count(&mut r, "floor layer count", MAX_FLOOR_LAYERS)?
        } else {
            1
        };

        let cells = u64::from(width) * u64::from(height);
        let mut walls = Vec::with_capacity(wall_count);
        let mut floors = Vec::with_capacity(floor_count);
        let shadow;
        let mut tag = None;
        let want_tag = has_tag_layer(version, tag_type);

        if has_layer_counts(version) {
            for _ in 0..wall_count {
                let tiles = cell_grid(width, height, r.dwords(cells)?)?;
                let orientations = grid_from(width, height, r.dwords(cells)?)?;
                walls.push(WallLayer {
                    tiles,
                    orientations,
                });
            }
            for _ in 0..floor_count {
                floors.push(cell_grid(width, height, r.dwords(cells)?)?);
            }
            shadow = cell_grid(width, height, r.dwords(cells)?)?;
            if want_tag {
                tag = Some(grid_from(width, height, r.dwords(cells)?)?);
            }
        } else {
            // wall 1, floor 1, orientation 1, tag, shadow
            let tiles = cell_grid(width, height, r.dwords(cells)?)?;
            floors.push(cell_grid(width, height, r.dwords(cells)?)?);
            let orientations = grid_from(width, height, r.dwords(cells)?)?;
            walls.push(WallLayer {
                tiles,
                orientations,
            });
            tag = Some(grid_from(width, height, r.dwords(cells)?)?);
            shadow = cell_grid(width, height, r.dwords(cells)?)?;
        }

        // Padding is only tolerated in the object/group/path blocks (that is
        // where the two shipped short files run out); the header and every
        // cell layer must really be there.
        if r.pos > r.real_len {
            return Err(FormatError::Truncated {
                format: FORMAT,
                offset: r.real_len,
                needed: r.pos - r.real_len,
            });
        }

        let mut objects = Vec::new();
        if has_objects(version) {
            let n = r.count("object count")?;
            let rec: u64 = if has_object_flags(version) { 20 } else { 16 };
            r.need(n.saturating_mul(rec))?;
            objects.reserve(usize::try_from(n).unwrap_or(0));
            for _ in 0..n {
                let kind = r.i32()?;
                let id = r.i32()?;
                let x = r.i32()?;
                let y = r.i32()?;
                let flags = if has_object_flags(version) {
                    r.i32()?
                } else {
                    0
                };
                objects.push(Object {
                    kind,
                    id,
                    x,
                    y,
                    flags,
                });
            }
        }

        let mut groups = Vec::new();
        if has_groups(version, tag_type) {
            if has_pre_group_dword(version) {
                preserved.pre_group_dword = r.i32()?;
            }
            let n = r.count("group count")?;
            let rec: u64 = if has_group_unknown(version) { 20 } else { 16 };
            r.need(n.saturating_mul(rec))?;
            groups.reserve(usize::try_from(n).unwrap_or(0));
            for _ in 0..n {
                let x = r.i32()?;
                let y = r.i32()?;
                let width = r.i32()?;
                let height = r.i32()?;
                let unknown = if has_group_unknown(version) {
                    r.i32()?
                } else {
                    0
                };
                groups.push(Group {
                    x,
                    y,
                    width,
                    height,
                    unknown,
                });
            }
        }

        let mut npc_paths = Vec::new();
        if has_npc_block(version) {
            let n = r.count("npc path count")?;
            r.need(n.saturating_mul(12))?;
            for _ in 0..n {
                let point_count = r.count("path point count")?;
                let x = r.i32()?;
                let y = r.i32()?;
                let rec: u64 = if has_path_action(version) { 12 } else { 8 };
                r.need(point_count.saturating_mul(rec))?;
                let mut points = Vec::with_capacity(usize::try_from(point_count).unwrap_or(0));
                for _ in 0..point_count {
                    let px = r.i32()?;
                    let py = r.i32()?;
                    let action = if has_path_action(version) {
                        r.i32()?
                    } else {
                        1
                    };
                    points.push(PathPoint {
                        x: px,
                        y: py,
                        action,
                    });
                }
                npc_paths.push(NpcPath { x, y, points });
            }
        }

        if r.pos > r.real_len {
            preserved.missing = r.pos - r.real_len;
        } else {
            preserved.trailing = bytes.get(r.pos..).unwrap_or(&[]).to_vec();
        }
        report.missing_bytes = preserved.missing;
        report.trailing_bytes = preserved.trailing.len();

        let ds1 = Ds1 {
            version,
            width,
            height,
            act,
            tag_type,
            files,
            walls,
            floors,
            shadow,
            tag,
            objects,
            groups,
            npc_paths,
            preserved,
        };
        ds1.collect_warnings(&mut report.warnings);
        Ok((ds1, report))
    }

    fn collect_warnings(&self, out: &mut Vec<String>) {
        if has_act(self.version) && !(0..=4).contains(&self.act) {
            out.push(format!(
                "act {} is outside 0..=4 (the game clamps it to {})",
                self.act,
                self.effective_act()
            ));
        }
        if has_tag_type(self.version) && !(0..=2).contains(&self.tag_type) {
            out.push(format!(
                "tag_type {} is not 0, 1 or 2: no tag layer or groups are read",
                self.tag_type
            ));
        }
        if self.version < 7 {
            let bad = self
                .walls
                .iter()
                .flat_map(|w| w.orientations.as_slice())
                .filter(|&&o| (o & 0xFF) as usize >= DIR_LOOKUP.len())
                .count();
            if bad > 0 {
                out.push(format!(
                    "{bad} orientation value(s) index past the {}-entry remap table",
                    DIR_LOOKUP.len()
                ));
            }
        }
        let max_x = i64::from(self.width) * 5;
        let max_y = i64::from(self.height) * 5;
        let outside = self
            .objects
            .iter()
            .filter(|o| {
                !(0..max_x).contains(&i64::from(o.x)) || !(0..max_y).contains(&i64::from(o.y))
            })
            .count();
        if outside > 0 {
            out.push(format!("{outside} object(s) lie outside the sub-tile grid"));
        }
        for p in &self.npc_paths {
            let owners = self
                .objects
                .iter()
                .filter(|o| o.x == p.x && o.y == p.y)
                .count();
            if owners != 1 {
                out.push(format!(
                    "npc path at ({}, {}) matches {owners} objects (the game needs exactly one)",
                    p.x, p.y
                ));
            }
        }
    }
}

fn dimension(r: &mut Reader, what: &'static str) -> Result<u32> {
    let at = r.pos;
    let stored = r.i32()?;
    u32::try_from(i64::from(stored) + 1).map_err(|_| FormatError::Invalid {
        format: FORMAT,
        what,
        offset: at,
        detail: format!("stored value {stored} is below -1"),
    })
}

fn layer_count(r: &mut Reader, what: &'static str, max: usize) -> Result<usize> {
    let at = r.pos;
    let n = r.i32()?;
    match usize::try_from(n) {
        Ok(n) if n <= max => Ok(n),
        _ => Err(FormatError::Invalid {
            format: FORMAT,
            what,
            offset: at,
            detail: format!("{n} is outside 0..={max}"),
        }),
    }
}

// --------------------------------------------------------------------------
// model operations

impl Ds1 {
    /// A blank version-18 map: one wall layer, one floor layer, no tag layer.
    pub fn new(width: u32, height: u32, act: i32) -> Ds1 {
        Ds1 {
            version: 18,
            width,
            height,
            act,
            tag_type: 0,
            files: Vec::new(),
            walls: vec![WallLayer {
                tiles: Grid::new(width, height),
                orientations: Grid::new(width, height),
            }],
            floors: vec![Grid::new(width, height)],
            shadow: Grid::new(width, height),
            tag: None,
            objects: Vec::new(),
            groups: Vec::new(),
            npc_paths: Vec::new(),
            preserved: Preserved::default(),
        }
    }

    /// The act the game uses: `act` clamped to `0..=4`.
    pub fn effective_act(&self) -> u8 {
        self.act.clamp(0, 4) as u8
    }

    /// The effective orientation of a wall cell: the low byte of the stored
    /// dword, passed through [`DIR_LOOKUP`] when `version < 7`.
    ///
    /// `None` when the wall layer or coordinate does not exist, or when a
    /// `version < 7` value indexes past the 25-entry table (the game reads
    /// out of bounds there; the editor treats it as unknown).
    pub fn orientation(&self, wall: usize, x: u32, y: u32) -> Option<u8> {
        let raw = self.walls.get(wall)?.orientations.get(x, y)?;
        let low = (raw & 0xFF) as u8;
        if self.version < 7 {
            DIR_LOOKUP.get(usize::from(low)).copied()
        } else {
            Some(low)
        }
    }

    /// File-list entries that are editor workspace data (`DS1EDIT_WRKSPC_*`).
    pub fn workspace_entries(&self) -> impl Iterator<Item = &[u8]> {
        self.files
            .iter()
            .map(Vec::as_slice)
            .filter(|f| f.starts_with(WORKSPACE_PREFIX))
    }

    /// File-list entries that name tilesets (everything but workspace data).
    pub fn tileset_refs(&self) -> impl Iterator<Item = &[u8]> {
        self.files
            .iter()
            .map(Vec::as_slice)
            .filter(|f| !f.starts_with(WORKSPACE_PREFIX))
    }

    /// Appends an empty wall layer and returns its index.
    pub fn add_wall_layer(&mut self) -> Result<usize> {
        if !has_layer_counts(self.version) {
            return Err(self.not_representable("versions below 4 have exactly one wall layer"));
        }
        if self.walls.len() >= MAX_WALL_LAYERS {
            return Err(self.not_representable("at most 4 wall layers"));
        }
        self.walls.push(WallLayer {
            tiles: Grid::new(self.width, self.height),
            orientations: Grid::new(self.width, self.height),
        });
        Ok(self.walls.len() - 1)
    }

    /// Appends an empty floor layer and returns its index.
    pub fn add_floor_layer(&mut self) -> Result<usize> {
        if !has_floor_count(self.version) {
            return Err(self.not_representable("versions below 16 have exactly one floor layer"));
        }
        if self.floors.len() >= MAX_FLOOR_LAYERS {
            return Err(self.not_representable("at most 2 floor layers"));
        }
        self.floors.push(Grid::new(self.width, self.height));
        Ok(self.floors.len() - 1)
    }

    /// Smallest version `>= self.version` whose layout can hold the current
    /// model. An all-zero tag layer that a candidate version has no room for
    /// does not count against it (the caller drops it when changing the
    /// version). When no version can hold the model (it is internally
    /// inconsistent, e.g. a non-zero tag layer with `tag_type == 0` at
    /// version 18), returns `max(self.version, 18)`.
    pub fn min_version_for_model(&self) -> i32 {
        const GATES: [i32; 12] = [2, 3, 4, 6, 8, 10, 12, 13, 14, 15, 16, 18];
        let mut candidates = vec![self.version];
        candidates.extend(GATES.iter().copied().filter(|&g| g > self.version));
        candidates
            .into_iter()
            .find(|&v| self.check_fits(v, true).is_ok())
            .unwrap_or(self.version.max(18))
    }

    fn not_representable(&self, detail: &str) -> FormatError {
        FormatError::NotRepresentable {
            format: FORMAT,
            version: i64::from(self.version),
            detail: detail.to_string(),
        }
    }

    /// Whether the model can be written as version `v`, and if not, why.
    ///
    /// With `drop_empty_tag` an all-zero tag layer that `v` has no room for
    /// is not an obstacle (nothing would be lost by leaving it out).
    fn check_fits(&self, v: i32, drop_empty_tag: bool) -> std::result::Result<(), String> {
        let (w, h) = (self.width, self.height);
        let same_dims = |gw: u32, gh: u32| gw == w && gh == h;
        if i64::from(w) > i64::from(i32::MAX) + 1 || i64::from(h) > i64::from(i32::MAX) + 1 {
            return Err("dimensions do not fit the stored i32".to_string());
        }
        for (i, wall) in self.walls.iter().enumerate() {
            if !same_dims(wall.tiles.width(), wall.tiles.height())
                || !same_dims(wall.orientations.width(), wall.orientations.height())
            {
                return Err(format!("wall layer {i} does not match the map size"));
            }
        }
        for (i, floor) in self.floors.iter().enumerate() {
            if !same_dims(floor.width(), floor.height()) {
                return Err(format!("floor layer {i} does not match the map size"));
            }
        }
        if !same_dims(self.shadow.width(), self.shadow.height()) {
            return Err("shadow layer does not match the map size".to_string());
        }

        if has_layer_counts(v) {
            if self.walls.len() > MAX_WALL_LAYERS {
                return Err(format!("{} wall layers (max 4)", self.walls.len()));
            }
        } else if self.walls.len() != 1 {
            return Err(format!(
                "{} wall layers, but versions below 4 have exactly one",
                self.walls.len()
            ));
        }
        if has_floor_count(v) {
            if self.floors.len() > MAX_FLOOR_LAYERS {
                return Err(format!("{} floor layers (max 2)", self.floors.len()));
            }
        } else if self.floors.len() != 1 {
            return Err(format!(
                "{} floor layers, but versions below 16 have exactly one",
                self.floors.len()
            ));
        }

        if !has_act(v) && self.act != 0 {
            return Err("act is only stored from version 8".to_string());
        }
        if !has_tag_type(v) && self.tag_type != 0 {
            return Err("tag_type is only stored from version 10".to_string());
        }

        match (&self.tag, has_tag_layer(v, self.tag_type)) {
            (Some(t), true) => {
                if !same_dims(t.width(), t.height()) {
                    return Err("tag layer does not match the map size".to_string());
                }
            }
            (None, true) => return Err("this version needs a tag layer".to_string()),
            (Some(t), false) => {
                if !(drop_empty_tag && t.as_slice().iter().all(|&c| c == 0)) {
                    return Err("this version and tag_type store no tag layer".to_string());
                }
            }
            (None, false) => {}
        }

        if !has_files(v) && !self.files.is_empty() {
            return Err("the file list is only stored from version 3".to_string());
        }
        if self.files.iter().any(|f| f.contains(&0)) {
            return Err("a file-list string contains a NUL byte".to_string());
        }
        if !has_objects(v) && !self.objects.is_empty() {
            return Err("objects are only stored from version 2".to_string());
        }
        if !has_object_flags(v) && self.objects.iter().any(|o| o.flags != 0) {
            return Err("object flags are only stored from version 6".to_string());
        }

        let groups_stored = has_groups(v, self.tag_type);
        if !groups_stored && !self.groups.is_empty() {
            return Err("groups need version 12+ and tag_type 1 or 2".to_string());
        }
        if !has_group_unknown(v) && self.groups.iter().any(|g| g.unknown != 0) {
            return Err("the group unknown dword is only stored from version 13".to_string());
        }
        if !(groups_stored && has_pre_group_dword(v)) && self.preserved.pre_group_dword != 0 {
            return Err("pre_group_dword needs version 18 with a group block".to_string());
        }
        if !has_unknown_pair(v) && self.preserved.unknown_v9_13 != [0, 0] {
            return Err("unknown_v9_13 is only stored in versions 9..=13".to_string());
        }
        if !has_npc_block(v) && !self.npc_paths.is_empty() {
            return Err("npc paths are only stored from version 14".to_string());
        }
        if !has_path_action(v)
            && self
                .npc_paths
                .iter()
                .flat_map(|p| &p.points)
                .any(|pt| pt.action != 1)
        {
            return Err("path point actions are only stored from version 15".to_string());
        }
        Ok(())
    }

    /// Serializes in `self.version`'s layout.
    ///
    /// For a model produced by [`Ds1::parse`] and left unchanged the output is
    /// byte-identical to the input. Returns `NotRepresentable` when the model
    /// holds something that version cannot store (see the gate table in the
    /// module docs) or is internally inconsistent (layer sizes, tag layer).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let v = self.version;
        self.check_fits(v, false)
            .map_err(|detail| self.not_representable(&detail))?;

        let cells = self.width as usize * self.height as usize;
        let layers = self.walls.len() * 2 + self.floors.len() + 2;
        let mut out = Vec::with_capacity(64 + cells * 4 * layers + self.objects.len() * 20);

        put(&mut out, v);
        put(&mut out, stored_dim(self.width));
        put(&mut out, stored_dim(self.height));
        if has_act(v) {
            put(&mut out, self.act);
        }
        if has_tag_type(v) {
            put(&mut out, self.tag_type);
        }
        if has_files(v) {
            put_len(&mut out, self.files.len(), v)?;
            for f in &self.files {
                out.extend_from_slice(f);
                out.push(0);
            }
        }
        if has_unknown_pair(v) {
            put(&mut out, self.preserved.unknown_v9_13[0]);
            put(&mut out, self.preserved.unknown_v9_13[1]);
        }
        if has_layer_counts(v) {
            put_len(&mut out, self.walls.len(), v)?;
        }
        if has_floor_count(v) {
            put_len(&mut out, self.floors.len(), v)?;
        }

        if has_layer_counts(v) {
            for wall in &self.walls {
                put_cells(&mut out, wall.tiles.as_slice());
                put_dwords(&mut out, wall.orientations.as_slice());
            }
            for floor in &self.floors {
                put_cells(&mut out, floor.as_slice());
            }
            put_cells(&mut out, self.shadow.as_slice());
            if let Some(tag) = &self.tag {
                put_dwords(&mut out, tag.as_slice());
            }
        } else if let (Some(wall), Some(floor), Some(tag)) =
            (self.walls.first(), self.floors.first(), &self.tag)
        {
            put_cells(&mut out, wall.tiles.as_slice());
            put_cells(&mut out, floor.as_slice());
            put_dwords(&mut out, wall.orientations.as_slice());
            put_dwords(&mut out, tag.as_slice());
            put_cells(&mut out, self.shadow.as_slice());
        }

        if has_objects(v) {
            put_len(&mut out, self.objects.len(), v)?;
            for o in &self.objects {
                put(&mut out, o.kind);
                put(&mut out, o.id);
                put(&mut out, o.x);
                put(&mut out, o.y);
                if has_object_flags(v) {
                    put(&mut out, o.flags);
                }
            }
        }

        if has_groups(v, self.tag_type) {
            if has_pre_group_dword(v) {
                put(&mut out, self.preserved.pre_group_dword);
            }
            put_len(&mut out, self.groups.len(), v)?;
            for g in &self.groups {
                put(&mut out, g.x);
                put(&mut out, g.y);
                put(&mut out, g.width);
                put(&mut out, g.height);
                if has_group_unknown(v) {
                    put(&mut out, g.unknown);
                }
            }
        }

        if has_npc_block(v) {
            put_len(&mut out, self.npc_paths.len(), v)?;
            for p in &self.npc_paths {
                put_len(&mut out, p.points.len(), v)?;
                put(&mut out, p.x);
                put(&mut out, p.y);
                for pt in &p.points {
                    put(&mut out, pt.x);
                    put(&mut out, pt.y);
                    if has_path_action(v) {
                        put(&mut out, pt.action);
                    }
                }
            }
        }

        out.extend_from_slice(&self.preserved.trailing);

        if self.preserved.missing > 0 && self.preserved.trailing.is_empty() {
            let n = self.preserved.missing.min(out.len());
            let keep = out.len() - n;
            if out
                .get(keep..)
                .is_some_and(|tail| tail.iter().all(|&b| b == 0))
            {
                out.truncate(keep);
            }
        }
        Ok(out)
    }
}

fn stored_dim(real: u32) -> i32 {
    // check_fits guarantees real <= i32::MAX + 1, so this never wraps.
    (i64::from(real) - 1) as i32
}

fn put(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_len(out: &mut Vec<u8>, len: usize, version: i32) -> Result<()> {
    let v = i32::try_from(len).map_err(|_| FormatError::NotRepresentable {
        format: FORMAT,
        version: i64::from(version),
        detail: format!("{len} entries do not fit a stored count"),
    })?;
    put(out, v);
    Ok(())
}

fn put_cells(out: &mut Vec<u8>, cells: &[Cell]) {
    for c in cells {
        out.extend_from_slice(&c.0.to_le_bytes());
    }
}

fn put_dwords(out: &mut Vec<u8>, cells: &[u32]) {
    for c in cells {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

#[cfg(test)]
mod tests;
