//! Pure helpers for the map-editing operations (`Document` methods in
//! `document/map_edit.rs`): tile keys and brushes, a small seedable RNG, new
//! maps and object checks. All game data (candidate tile lists, object
//! catalogs) is supplied by the caller.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use d2_formats::Grid;
use d2_formats::ds1::{Cell, Ds1, Object, WallLayer};

use crate::error::{EditError, Result};
use crate::layer::{CellValue, LayerRef};

/// Orientation ds1-studio gives shadow tiles.
pub const SHADOW_ORIENTATION: u32 = 13;
/// Sub-tiles per tile edge (object and path coordinates).
pub const SUBTILES: i64 = 5;
/// Largest map `new_map` and `resize` accept (1024 x 1024 cells).
pub const MAX_CELLS: u64 = 1 << 20;

/// A small seedable PRNG (SplitMix64). Same seed, same sequence.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (0 when `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// A tile to paint. `orientation` is the raw orientation dword and only used
/// on wall layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Brush {
    pub main: u8,
    pub sub: u8,
    pub orientation: u32,
}

impl Brush {
    pub fn floor(main: u8, sub: u8) -> Brush {
        Brush {
            main,
            sub,
            orientation: 0,
        }
    }
}

/// A tile as identified in a map: orientation (0 floors, 13 shadows, the raw
/// orientation dword on walls), main and sub index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub orientation: u32,
    pub main: u8,
    pub sub: u8,
}

impl TileKey {
    pub fn brush(self) -> Brush {
        Brush {
            main: self.main,
            sub: self.sub,
            orientation: self.orientation,
        }
    }
}

/// The key of a cell value on `layer`; `None` for an empty cell and for the tag layer.
pub fn tile_key(layer: LayerRef, value: CellValue) -> Option<TileKey> {
    if matches!(layer, LayerRef::Tag) || layer.is_empty_value(value) {
        return None;
    }
    let cell = Cell(value.cell);
    let orientation = match layer {
        LayerRef::Wall(_) => value.orientation,
        LayerRef::Shadow => SHADOW_ORIENTATION,
        _ => 0,
    };
    Some(TileKey {
        orientation,
        main: cell.main_index(),
        sub: cell.sub_index(),
    })
}

/// The `prop1` byte a freshly painted cell gets (as ds1-studio).
fn default_prop1(layer: LayerRef) -> u32 {
    match layer {
        LayerRef::Floor(_) => 0xC2,
        LayerRef::Wall(_) => 0x81,
        _ => 0x80,
    }
}

/// What `current` becomes when painted with `brush` (`None` = erase). Painting
/// over a tile keeps its other bits (hidden, unwalkable, ...).
pub fn painted(layer: LayerRef, current: CellValue, brush: Option<Brush>) -> CellValue {
    let Some(b) = brush else {
        return CellValue::EMPTY;
    };
    let base = if layer.is_empty_value(current) {
        Cell(default_prop1(layer))
    } else {
        Cell(current.cell)
    };
    layer.normalize(CellValue {
        cell: base.with_indices(b.main, b.sub).0,
        orientation: b.orientation,
    })
}

/// A floor tile the re-roll may choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorChoice {
    pub main: u8,
    pub sub: u8,
    /// DT1 rarity: 0 means "never chosen" next to tiles with a rarity; higher
    /// values are chosen more often.
    pub rarity: u32,
    /// Compares walkability across tiles (e.g. a bit per sub-tile flag).
    pub walk: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RerollOptions {
    /// Only use choices whose `walk` equals that of the tile being replaced
    /// (needs the `walk_of` lookup of `reroll_floors`; unknown tiles are skipped).
    pub preserve_walkability: bool,
    /// Prefer a tile different from the left and upper neighbours.
    pub avoid_repeats: bool,
    pub seed: u64,
}

impl Default for RerollOptions {
    fn default() -> Self {
        RerollOptions {
            preserve_walkability: false,
            avoid_repeats: true,
            seed: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RerollReport {
    /// Cells whose tile changed.
    pub changed: usize,
    /// Cells with no eligible choice.
    pub skipped: usize,
}

fn hash32(n: u32) -> u32 {
    let n = (n ^ (n >> 16)).wrapping_mul(0x045D_9F3B);
    n ^ (n >> 16)
}

/// Balanced deterministic pick for the cell at `xy`: the eligible choice with
/// the lowest score (neighbour repeats, then use count scaled by rarity, then
/// a seeded hash). `counts` tracks what was assigned so far.
pub(crate) fn pick_floor<'a>(
    eligible: &[&'a FloorChoice],
    neighbors: [Option<(u8, u8)>; 2],
    counts: &HashMap<(u8, u8), u32>,
    opts: &RerollOptions,
    xy: (u32, u32),
    cells: u64,
) -> Option<&'a FloorChoice> {
    let any_rare = eligible.iter().any(|c| c.rarity > 0);
    let seed = opts.seed as u32 ^ (opts.seed >> 32) as u32;
    eligible
        .iter()
        .filter(|c| !any_rare || c.rarity > 0)
        .min_by_key(|c| {
            let key = (c.main, c.sub);
            let repeats = if opts.avoid_repeats {
                neighbors.iter().filter(|n| **n == Some(key)).count() as u64
            } else {
                0
            };
            let used = u64::from(counts.get(&key).copied().unwrap_or(0)) * 1000
                / u64::from(c.rarity.max(1));
            let h = hash32(
                seed ^ xy.0.wrapping_add(1).wrapping_mul(73_856_093)
                    ^ xy.1.wrapping_add(1).wrapping_mul(19_349_663)
                    ^ (u32::from(c.main) * 256 + u32::from(c.sub) + 1).wrapping_mul(83_492_791),
            );
            repeats * (cells + 1) * 1000 + used + u64::from(h % 997)
        })
        .copied()
}

// ----- new maps ---------------------------------------------------------------

/// Parameters of [`new_map`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMapSpec {
    pub width: u32,
    pub height: u32,
    pub act: i32,
    /// 1..=4.
    pub walls: usize,
    /// 1..=2.
    pub floors: usize,
    /// Adds a tag layer (`tag_type` 1 from v10; v4..=9 cannot store one: `NeedsVersion`).
    pub tag: bool,
    /// DS1 version; `None` is 18.
    pub version: Option<i32>,
}

impl NewMapSpec {
    /// One wall layer, one floor layer, no tag layer, version 18.
    pub fn new(width: u32, height: u32, act: i32) -> NewMapSpec {
        NewMapSpec {
            width,
            height,
            act,
            walls: 1,
            floors: 1,
            tag: false,
            version: None,
        }
    }
}

/// A blank map. Fails with `LayerLimit` for a bad size or layer count and with
/// `NeedsVersion` when the requested version cannot hold the layers.
pub fn new_map(spec: &NewMapSpec) -> Result<Ds1> {
    let cells = u64::from(spec.width) * u64::from(spec.height);
    if spec.width == 0 || spec.height == 0 || cells > MAX_CELLS {
        return Err(EditError::LayerLimit(format!(
            "map size {}x{} is outside 1x1..{} cells",
            spec.width, spec.height, MAX_CELLS
        )));
    }
    if !(1..=4).contains(&spec.walls) || !(1..=2).contains(&spec.floors) {
        return Err(EditError::LayerLimit(
            "a map has 1..4 wall layers and 1..2 floor layers".into(),
        ));
    }
    let mut ds = Ds1::new(spec.width, spec.height, spec.act);
    let (w, h) = (spec.width, spec.height);
    ds.version = spec.version.unwrap_or(18);
    ds.walls = (0..spec.walls)
        .map(|_| WallLayer {
            tiles: Grid::new(w, h),
            orientations: Grid::new(w, h),
        })
        .collect();
    ds.floors = (0..spec.floors).map(|_| Grid::new(w, h)).collect();
    if spec.tag {
        ds.tag = Some(Grid::new(w, h));
        // Below v10 the tag layer is stored without a type (v<4) or not at all.
        ds.tag_type = i32::from(ds.version >= 10);
    }
    let required = ds.min_version_for_model();
    if required > ds.version {
        return Err(EditError::NeedsVersion {
            current: ds.version,
            required,
        });
    }
    ds.to_bytes()?;
    Ok(ds)
}

// ----- object checks ----------------------------------------------------------

/// Maps an object to the objects.txt row of a teleportation pad.
pub type PadRowFn = dyn Fn(&Object) -> Option<u32>;

/// Caller-supplied game data for [`check_objects`]; both are optional.
#[derive(Default)]
pub struct ObjectCheckOptions<'a> {
    /// Whether the object's id exists in the catalog (objects.txt / monster
    /// lists). `None` skips the unknown-id check.
    pub is_known: Option<&'a dyn Fn(&Object) -> bool>,
    /// The objects.txt row of a teleportation pad (OperateFn 27) object, `None`
    /// for any other object. `None` skips the pad-pairing check.
    pub teleport_pad_row: Option<&'a PadRowFn>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectProblem {
    /// Sub-tile position outside the map.
    OutsideMap,
    /// Same kind, id and position as the earlier object `of`.
    Duplicate {
        of: usize,
    },
    UnknownId,
    /// A teleportation pad with no pad of the same row in its own or a touching
    /// 40-sub-tile block.
    UnpairedPad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectFinding {
    /// Index into `Ds1::objects`.
    pub index: usize,
    pub problem: ObjectProblem,
}

/// Problems ds1-studio flags, in object order (several per object possible).
pub fn check_objects(ds1: &Ds1, opts: &ObjectCheckOptions<'_>) -> Vec<ObjectFinding> {
    let mut out = Vec::new();
    let (maxx, maxy) = (
        i64::from(ds1.width) * SUBTILES,
        i64::from(ds1.height) * SUBTILES,
    );
    let mut seen: HashMap<(i32, i32, i32, i32), usize> = HashMap::new();
    let pads: Vec<(usize, u32, i64, i64)> = match opts.teleport_pad_row {
        Some(row_of) => ds1
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.kind == 2)
            .filter_map(|(i, o)| {
                row_of(o).map(|r| {
                    (
                        i,
                        r,
                        i64::from(o.x).div_euclid(40),
                        i64::from(o.y).div_euclid(40),
                    )
                })
            })
            .collect(),
        None => Vec::new(),
    };
    for (index, o) in ds1.objects.iter().enumerate() {
        let (x, y) = (i64::from(o.x), i64::from(o.y));
        if x < 0 || y < 0 || x >= maxx || y >= maxy {
            out.push(ObjectFinding {
                index,
                problem: ObjectProblem::OutsideMap,
            });
        }
        match seen.entry((o.kind, o.id, o.x, o.y)) {
            Entry::Occupied(e) => out.push(ObjectFinding {
                index,
                problem: ObjectProblem::Duplicate { of: *e.get() },
            }),
            Entry::Vacant(e) => {
                e.insert(index);
            }
        }
        if opts.is_known.is_some_and(|known| !known(o)) {
            out.push(ObjectFinding {
                index,
                problem: ObjectProblem::UnknownId,
            });
        }
        if let Some(&(_, row, bx, by)) = pads.iter().find(|p| p.0 == index)
            && !pads.iter().any(|q| {
                q.0 != index && q.1 == row && (q.2 - bx).abs() <= 1 && (q.3 - by).abs() <= 1
            })
        {
            out.push(ObjectFinding {
                index,
                problem: ObjectProblem::UnpairedPad,
            });
        }
    }
    out
}
