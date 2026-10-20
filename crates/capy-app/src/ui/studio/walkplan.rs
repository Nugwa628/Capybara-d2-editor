//! Walkability edit planners (ds1-studio `cellFlags.ts`, `walkEdit.ts`,
//! `spawnRegions.ts noSpawnPlan`). Pure: they read a `Ds1` (and the tile
//! set) and return the cell edits, plus the new tiles of the map's
//! `<map>_walk.dt1`. The UI applies them as one undo step.
//!
//! How the game builds a room's collision: every floor, wall and roof tile
//! placed adds its 25 sub-tile flags (hidden tiles too), and a cell with DS1
//! bit 17 blocks walking on all of its sub-tiles. Flags only add up, so
//! blocking puts a hidden floor tile with no graphics and just those flags
//! into a free floor layer, and clearing a sub-tile another tile blocks
//! swaps that tile, in this cell only, for a copy without the flags.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use capy_edit::{CellValue, LayerRef};
use capy_render::TileSet;
use capy_render::rooms::{is_node_floor, preset_rooms, room_regions};
use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::{Dt1, Tile, TileKey, TilePreserved};

/// DS1 cell bit 17: blocks walking on the whole cell.
pub const UNWALKABLE: u32 = 1 << 17;
const HIDDEN: u32 = 1 << 31;
/// `prop1` of a floor cell painted fresh.
const FLOOR_PROP1: u32 = 0xC2;

pub type Edit = (LayerRef, u32, u32, CellValue);

fn floor_value(ds1: &Ds1, l: usize, x: u32, y: u32) -> CellValue {
    CellValue {
        cell: ds1.floors[l].get(x, y).unwrap_or(Cell::EMPTY).0,
        orientation: 0,
    }
}

fn wall_value(ds1: &Ds1, l: usize, x: u32, y: u32) -> CellValue {
    CellValue {
        cell: ds1.walls[l].tiles.get(x, y).unwrap_or(Cell::EMPTY).0,
        orientation: ds1.walls[l].orientations.get(x, y).unwrap_or(0),
    }
}

/// The floor-1 tile the map shows most (visible cells): hidden copies of it
/// hold flags in empty cells. Returns a full cell word to copy.
fn common_floor(ds1: &Ds1, within: Option<&dyn Fn(u32, u32) -> bool>) -> Option<Cell> {
    let mut count: BTreeMap<(u8, u8), (usize, Cell)> = BTreeMap::new();
    for y in 0..ds1.height {
        for x in 0..ds1.width {
            if within.is_some_and(|f| !f(x, y)) {
                continue;
            }
            let Some(c) = ds1.floors.first().and_then(|f| f.get(x, y)) else {
                continue;
            };
            if c.is_empty() || c.hidden() {
                continue;
            }
            count
                .entry((c.main_index(), c.sub_index()))
                .or_insert((0, c))
                .0 += 1;
        }
    }
    count.into_values().max_by_key(|(n, _)| *n).map(|(_, c)| c)
}

// ───────────────────────── whole-cell flag ─────────────────────────

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CellPlan {
    pub edits: Vec<Edit>,
    pub cells: usize,
    /// Empty cells that got a hidden floor copy to hold the flag.
    pub filled: usize,
    pub skipped: usize,
}

/// Cells made unwalkable (`on`) or walkable again: the flag goes on Floor 1,
/// else the first layer with a tile; clearing removes it from every layer.
pub fn cell_unwalkable_plan(ds1: &Ds1, cells: &[(u32, u32)], on: bool) -> CellPlan {
    let mut plan = CellPlan::default();
    let common = common_floor(ds1, None);
    let mut layers: Vec<(LayerRef, bool)> = Vec::new();
    for l in 0..ds1.floors.len() {
        layers.push((LayerRef::Floor(l as u8), true));
    }
    for l in 0..ds1.walls.len() {
        layers.push((LayerRef::Wall(l as u8), false));
    }
    let value = |layer: LayerRef, x, y| match layer {
        LayerRef::Floor(l) => floor_value(ds1, usize::from(l), x, y),
        LayerRef::Wall(l) => wall_value(ds1, usize::from(l), x, y),
        _ => CellValue::EMPTY,
    };
    for &(x, y) in cells {
        if x >= ds1.width || y >= ds1.height {
            continue;
        }
        let present: Vec<(LayerRef, CellValue)> = layers
            .iter()
            .map(|&(l, _)| (l, value(l, x, y)))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        let blocked = present.iter().any(|(_, v)| v.cell & UNWALKABLE != 0);
        if on {
            if blocked {
                continue;
            }
            if let Some((layer, v)) = present.first() {
                plan.edits.push((
                    *layer,
                    x,
                    y,
                    CellValue {
                        cell: v.cell | UNWALKABLE,
                        ..*v
                    },
                ));
                plan.cells += 1;
            } else if let (Some(c), true) = (common, !ds1.floors.is_empty()) {
                plan.edits.push((
                    LayerRef::Floor(0),
                    x,
                    y,
                    CellValue {
                        cell: c.0 | UNWALKABLE | HIDDEN,
                        orientation: 0,
                    },
                ));
                plan.cells += 1;
                plan.filled += 1;
            } else {
                plan.skipped += 1;
            }
        } else if blocked {
            let only_filler = present.len() == 1
                && present[0].1.cell & HIDDEN != 0
                && matches!(present[0].0, LayerRef::Floor(_));
            for (layer, v) in &present {
                if v.cell & UNWALKABLE == 0 {
                    continue;
                }
                let after = if only_filler {
                    CellValue::EMPTY
                } else {
                    CellValue {
                        cell: v.cell & !UNWALKABLE,
                        ..*v
                    }
                };
                plan.edits.push((*layer, x, y, after));
            }
            plan.cells += 1;
        }
    }
    plan
}

// ───────────────────────── no-spawn area ─────────────────────────

#[derive(Debug, Default)]
pub struct NoSpawnPlan {
    /// Floor layers the map needs (a second one holds the floor copies).
    pub floors: usize,
    pub edits: Vec<Edit>,
    pub rooms: usize,
    /// Seeds whose floor moved up to floor 2 under a hidden copy.
    pub copied: usize,
    /// Seeds without a floor that got a hidden floor plus the unwalkable mark.
    pub voids: usize,
    pub skipped: Vec<String>,
}

/// Makes the rooms touching `cells` spawn no random monsters while staying
/// walkable and looking the same: every region's seed gets a hidden
/// first-layer floor. A floored seed's tile is copied to floor 2 first (drawn
/// the same, same collision); a seed without a floor gets a hidden floor tile
/// plus the unwalkable mark. With `whole_rooms` every floored cell of the
/// rooms changes the same way, not only the seeds.
pub fn no_spawn_plan(ds1: &Ds1, cells: &[(u32, u32)], whole_rooms: bool) -> NoSpawnPlan {
    let (w, h) = (ds1.width, ds1.height);
    let rooms: Vec<_> = preset_rooms(w, h)
        .into_iter()
        .filter(|r| {
            cells
                .iter()
                .any(|&(x, y)| x >= r.x0 && x <= r.x0 + r.w && y >= r.y0 && y <= r.y0 + r.h)
        })
        .collect();
    let in_rooms = |x: u32, y: u32| {
        rooms
            .iter()
            .any(|r| x >= r.x0 && x < r.x0 + r.w && y >= r.y0 && y < r.y0 + r.h)
    };
    let void_cell = common_floor(ds1, Some(&in_rooms)).or_else(|| common_floor(ds1, None));
    let f1 = ds1.floors.first();
    let f2 = ds1.floors.get(1);
    let mut plan = NoSpawnPlan {
        rooms: rooms.len(),
        ..NoSpawnPlan::default()
    };
    let mut done: HashSet<(u32, u32)> = HashSet::new();
    let mut hide_at = |plan: &mut NoSpawnPlan, x: u32, y: u32, why: &str| {
        if !done.insert((x, y)) {
            return;
        }
        if x >= w || y >= h {
            plan.skipped
                .push(format!("{x},{y} ({why}) is past the map's edge"));
            return;
        }
        let c1 = f1.and_then(|f| f.get(x, y)).unwrap_or(Cell::EMPTY);
        let value = |c: Cell| CellValue {
            cell: c.0,
            orientation: 0,
        };
        if is_node_floor(c1) {
            return;
        }
        let c2 = f2.and_then(|f| f.get(x, y)).unwrap_or(Cell::EMPTY);
        if !c1.is_empty() {
            if !c2.is_empty() {
                plan.skipped.push(format!(
                    "{x},{y} ({why}) has a floor 2 tile already, so its floor 1 can't be hidden without changing the look"
                ));
                return;
            }
            plan.edits.push((LayerRef::Floor(1), x, y, value(c1)));
            plan.edits
                .push((LayerRef::Floor(0), x, y, value(c1.with_hidden(true))));
            plan.copied += 1;
        } else if !c2.is_empty() {
            plan.edits
                .push((LayerRef::Floor(0), x, y, value(c2.with_hidden(true))));
            plan.copied += 1;
        } else if let Some(v) = void_cell {
            plan.edits.push((
                LayerRef::Floor(0),
                x,
                y,
                CellValue {
                    cell: v.0 | HIDDEN | UNWALKABLE,
                    orientation: 0,
                },
            ));
            plan.voids += 1;
        } else {
            plan.skipped.push(format!(
                "{x},{y} ({why}) has no floor and the map has no floor tile to hide there"
            ));
        }
    };
    for r in &rooms {
        for reg in room_regions(ds1, *r).regions {
            hide_at(&mut plan, reg.seed.0, reg.seed.1, "a region seed");
        }
        if whole_rooms {
            for y in r.y0..r.y0 + r.h {
                for x in r.x0..r.x0 + r.w {
                    if f1.and_then(|f| f.get(x, y)).is_some_and(|c| !c.is_empty()) {
                        hide_at(&mut plan, x, y, "a floored cell");
                    }
                }
            }
        }
    }
    plan.floors = ds1
        .floors
        .len()
        .max(if plan.edits.iter().any(|e| e.0 == LayerRef::Floor(1)) {
            2
        } else {
            1
        });
    plan
}

// ───────────────────────── sub-tile walking ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkMode {
    Block,
    Clear,
}

/// Collision flag bits (`capy_render::walk`): 1 walk, 2 light, 4 jump, 8 player walk.
pub struct WalkPaint {
    pub mode: WalkMode,
    pub bits: u8,
    /// Cell -> sub-tiles painted, a 25-bit mask (bit `sy * 5 + sx`).
    pub cells: BTreeMap<(u32, u32), u32>,
}

#[derive(Debug, Default)]
pub struct WalkPlan {
    /// The map's edits (blockers, swapped tiles, flag clears).
    pub edits: Vec<Edit>,
    pub floors: usize,
    /// The walkability library with any new tiles (`None`: unchanged).
    pub dt1: Option<Dt1>,
    pub changed: usize,
    pub skipped: Vec<String>,
}

type Flags = [u8; 25];

#[derive(Clone)]
struct Contributor {
    layer: LayerRef,
    cell: CellValue,
    keys: Vec<TileKey>,
}

fn is_blocker_tile(t: &Tile) -> bool {
    t.blocks.is_empty() && t.orientation == 0
}

fn key_of(orientation: i32, main: i32, sub: i32) -> Option<TileKey> {
    Some(TileKey {
        orientation: u8::try_from(orientation).ok()?,
        main: u8::try_from(main).ok()?,
        sub: u8::try_from(sub).ok()?,
    })
}

/// Every key the tile set holds, as (orientation, main, sub).
pub fn tileset_keys(tiles: &TileSet) -> BTreeSet<(i32, i32, i32)> {
    let mut out = BTreeSet::new();
    for d in 0..tiles.dt1_paths().len() {
        let mut t = 0;
        while let Some(tile) = tiles.tile(&capy_render::TileRef { dt1: d, tile: t }) {
            out.insert((tile.orientation, tile.main_index, tile.sub_index));
            t += 1;
        }
    }
    out
}

pub fn blocker_tile(main: u8, sub: u8, flags: Flags) -> Tile {
    Tile {
        direction: 3,
        sound: 1,
        height: -128,
        width: 160,
        orientation: 0,
        main_index: i32::from(main),
        sub_index: i32::from(sub),
        subtile_flags: flags,
        preserved: TilePreserved {
            unknown_36: [0xff, 0x00, 0xff, 0x00],
            ..TilePreserved::default()
        },
        ..Tile::default()
    }
}

/// Plans a sub-tile paint stroke. `walk` is the map's walkability library as
/// it is now (`None`: none yet). `void` is what a cell without a floor is in
/// game (LvlPrest FillBlanks), `None` when it is open ground.
pub fn plan_walk(
    ds1: &Ds1,
    tiles: &TileSet,
    walk: Option<&Dt1>,
    paint: &WalkPaint,
    void: Option<Flags>,
) -> WalkPlan {
    let mut records: Vec<Tile> = walk.map(|d| d.tiles.clone()).unwrap_or_default();
    let original = records.len();
    let mut plan = WalkPlan {
        floors: ds1.floors.len(),
        ..WalkPlan::default()
    };
    let mut taken: BTreeSet<(i32, i32, i32)> = tileset_keys(tiles);
    for r in &records {
        taken.insert((r.orientation, r.main_index, r.sub_index));
    }
    let clear_bits = paint.bits;
    let floor_mains: BTreeSet<i32> = taken
        .iter()
        .filter(|k| k.0 == 0)
        .map(|k| k.1)
        .filter(|m| {
            !records
                .iter()
                .any(|r| is_blocker_tile(r) && r.main_index == *m)
        })
        .collect();
    let mut block_main: i32 = records
        .iter()
        .find(|r| is_blocker_tile(r))
        .map_or(-1, |r| r.main_index);
    if block_main < 0 {
        block_main = (0..=63)
            .rev()
            .find(|m| !floor_mains.contains(m))
            .unwrap_or(-1);
    }
    let is_blocker = |records: &[Tile], c: CellValue| {
        let cell = Cell(c.cell);
        !cell.is_empty()
            && i32::from(cell.main_index()) == block_main
            && records.iter().any(|r| {
                is_blocker_tile(r)
                    && r.main_index == block_main
                    && r.sub_index == i32::from(cell.sub_index())
            })
    };
    let blocker_flags = |records: &[Tile], c: CellValue| -> Flags {
        let cell = Cell(c.cell);
        records
            .iter()
            .find(|r| {
                is_blocker_tile(r)
                    && r.main_index == block_main
                    && r.sub_index == i32::from(cell.sub_index())
            })
            .map_or([0; 25], |r| r.subtile_flags)
    };
    let flags_of = |c: &Contributor| -> Flags {
        let mut out = [0u8; 25];
        for k in &c.keys {
            for r in tiles.candidates(*k) {
                if let Some(t) = tiles.tile(r) {
                    for (o, f) in out.iter_mut().zip(&t.subtile_flags) {
                        *o |= *f;
                    }
                }
            }
        }
        out
    };

    for (&(x, y), &mask) in &paint.cells {
        if x >= ds1.width || y >= ds1.height || mask & 0x1ff_ffff == 0 {
            continue;
        }
        let (start_records, start_edits, start_floors) =
            (records.len(), plan.edits.len(), plan.floors);
        let start_taken = taken.clone();
        let r = plan_cell(
            ds1,
            &mut plan,
            &mut records,
            &mut taken,
            (x, y, mask),
            paint,
            clear_bits,
            block_main,
            void,
            &is_blocker,
            &blocker_flags,
            &flags_of,
            tiles,
        );
        if let Err(e) = r {
            records.truncate(start_records);
            plan.edits.truncate(start_edits);
            plan.floors = start_floors;
            taken = start_taken;
            plan.skipped.push(format!("({x}, {y}): {e}"));
        }
    }
    if records.len() > original {
        plan.dt1 = Some(Dt1 {
            version: walk.map_or((7, 6), |d| d.version),
            tiles: records,
        });
    }
    plan
}

fn free_sub(taken: &BTreeSet<(i32, i32, i32)>, pairs: &[(i32, i32)]) -> Option<i32> {
    (0..256).find(|s| pairs.iter().all(|&(o, m)| !taken.contains(&(o, m, *s))))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn plan_cell(
    ds1: &Ds1,
    plan: &mut WalkPlan,
    records: &mut Vec<Tile>,
    taken: &mut BTreeSet<(i32, i32, i32)>,
    (x, y, mask): (u32, u32, u32),
    paint: &WalkPaint,
    clear_bits: u8,
    block_main: i32,
    void: Option<Flags>,
    is_blocker: &dyn Fn(&[Tile], CellValue) -> bool,
    blocker_flags: &dyn Fn(&[Tile], CellValue) -> Flags,
    flags_of: &dyn Fn(&Contributor) -> Flags,
    tiles: &TileSet,
) -> Result<(), String> {
    let mut contributors: Vec<Contributor> = Vec::new();
    for l in 0..ds1.floors.len() {
        let v = floor_value(ds1, l, x, y);
        if !v.is_empty() {
            let c = Cell(v.cell);
            contributors.push(Contributor {
                layer: LayerRef::Floor(l as u8),
                cell: v,
                keys: key_of(0, i32::from(c.main_index()), i32::from(c.sub_index()))
                    .into_iter()
                    .collect(),
            });
        }
    }
    for l in 0..ds1.walls.len() {
        let v = wall_value(ds1, l, x, y);
        let o = ds1.orientation(l, x, y).unwrap_or(0);
        if v.is_empty() || o == 10 || o == 11 {
            continue;
        }
        let c = Cell(v.cell);
        let mut keys: Vec<TileKey> = key_of(
            i32::from(o),
            i32::from(c.main_index()),
            i32::from(c.sub_index()),
        )
        .into_iter()
        .collect();
        if o == 3 {
            keys.extend(key_of(
                4,
                i32::from(c.main_index()),
                i32::from(c.sub_index()),
            ));
        }
        contributors.push(Contributor {
            layer: LayerRef::Wall(l as u8),
            cell: v,
            keys,
        });
    }
    let blocker = contributors
        .iter()
        .position(|c| matches!(c.layer, LayerRef::Floor(_)) && is_blocker(records, c.cell));
    let others: Vec<usize> = (0..contributors.len())
        .filter(|i| Some(*i) != blocker)
        .collect();
    let whole: Vec<usize> = contributors
        .iter()
        .enumerate()
        .filter(|(_, c)| (c.cell.cell >> 16) & 3 != 0)
        .map(|(i, _)| i)
        .collect();
    let no_floor = !contributors
        .iter()
        .any(|c| matches!(c.layer, LayerRef::Floor(_)));

    let mut base = [0u8; 25];
    for &i in &others {
        for (b, f) in base.iter_mut().zip(flags_of(&contributors[i])) {
            *b |= f;
        }
    }
    let whole_flags = whole.iter().fold(0u8, |f, &i| {
        let p3 = (contributors[i].cell.cell >> 16) & 3;
        f | if p3 & 2 != 0 { 1 } else { 0 } | if p3 & 1 != 0 { 4 } else { 0 }
    });
    if whole_flags != 0 {
        base.iter_mut().for_each(|b| *b |= whole_flags);
    }
    if let (true, Some(v)) = (no_floor, void) {
        base.iter_mut().zip(v).for_each(|(b, f)| *b |= f);
    }
    let mut want: Flags = blocker.map_or([0; 25], |i| blocker_flags(records, contributors[i].cell));
    let mut before = [0u8; 25];
    for j in 0..25 {
        before[j] = base[j] | want[j];
    }
    let painted: Vec<bool> = (0..25).map(|k| mask & (1 << k) != 0).collect();
    let mut desired = before;
    for j in 0..25 {
        if painted[j] {
            desired[j] = match paint.mode {
                WalkMode::Block => before[j] | paint.bits,
                WalkMode::Clear => before[j] & !clear_bits,
            };
        }
    }
    if desired == before {
        return Ok(());
    }
    if let (true, Some(v)) = (no_floor, void) {
        for j in 0..25 {
            want[j] |= if paint.mode != WalkMode::Block && painted[j] {
                v[j] & !clear_bits
            } else {
                v[j]
            };
        }
    }
    let mut cell_edits: Vec<Edit> = Vec::new();
    match paint.mode {
        WalkMode::Block => {
            for j in 0..25 {
                if painted[j] {
                    want[j] |= paint.bits & !base[j];
                }
            }
        }
        WalkMode::Clear => {
            for j in 0..25 {
                if painted[j] {
                    want[j] &= !clear_bits;
                }
            }
            let restored = whole_flags & clear_bits;
            for &i in &whole {
                let remove = (if clear_bits & 1 != 0 { 2 } else { 0 })
                    | (if clear_bits & 4 != 0 { 1 } else { 0 });
                let p3 = (contributors[i].cell.cell >> 16) & 3;
                if p3 & remove == 0 {
                    continue;
                }
                contributors[i].cell.cell &= !(remove << 16);
                cell_edits.push((contributors[i].layer, x, y, contributors[i].cell));
            }
            if restored != 0 {
                for j in 0..25 {
                    if !painted[j] {
                        want[j] |= restored;
                    }
                }
            }
            let mut clear = [0u8; 25];
            for j in 0..25 {
                if painted[j] {
                    clear[j] = clear_bits;
                }
            }
            for &i in &others {
                let c = contributors[i].clone();
                if !flags_of(&c).iter().zip(&clear).any(|(f, k)| f & k != 0) {
                    continue;
                }
                let s = fork_for(tiles, records, taken, &c.keys, &clear)
                    .ok_or("its tile couldn't be copied (no free number)")?;
                let mut cell = Cell(c.cell.cell);
                cell = cell.with_indices(cell.main_index(), s);
                let layer = c.layer;
                // keep an earlier whole-cell edit of the same layer
                cell_edits.retain(|e| e.0 != layer);
                let prev = contributors[i].cell;
                cell_edits.push((
                    layer,
                    x,
                    y,
                    CellValue {
                        cell: (prev.cell & !(0xff << 8)) | (u32::from(s) << 8),
                        orientation: prev.orientation,
                    },
                ));
                contributors[i].cell.cell = cell.0;
            }
        }
    }
    let has_flags = want.iter().any(|f| *f != 0);
    let need_floor = no_floor
        && paint.mode != WalkMode::Block
        && void.is_some_and(|v| v.iter().any(|f| f & clear_bits != 0));
    let hidden_blocker = |sub: u8| CellValue {
        cell: Cell(FLOOR_PROP1)
            .with_indices(block_main as u8, sub)
            .with_hidden(true)
            .0,
        orientation: 0,
    };
    let mut blocker_for = |records: &mut Vec<Tile>, flags: Flags| -> Result<u8, String> {
        if let Some(r) = records
            .iter()
            .find(|r| is_blocker_tile(r) && r.main_index == block_main && r.subtile_flags == flags)
        {
            return Ok(r.sub_index as u8);
        }
        if block_main < 0 {
            return Err("No free floor number for a collision blocker.".into());
        }
        let s = free_sub(taken, &[(0, block_main)])
            .ok_or("The walkability library is full (256 blocker patterns).")?;
        records.push(blocker_tile(block_main as u8, s as u8, flags));
        taken.insert((0, block_main, s));
        Ok(s as u8)
    };
    let mut new_floors = plan.floors;
    if let Some(bi) = blocker {
        let b = contributors[bi].clone();
        let floor_besides = contributors
            .iter()
            .enumerate()
            .any(|(i, c)| i != bi && matches!(c.layer, LayerRef::Floor(_)));
        if !has_flags && !need_floor && floor_besides {
            cell_edits.push((b.layer, x, y, CellValue::EMPTY));
        } else if want != blocker_flags(records, b.cell) {
            let s = blocker_for(records, want)?;
            cell_edits.push((b.layer, x, y, hidden_blocker(s)));
        }
    } else if has_flags || need_floor {
        let free = (0..ds1.floors.len()).find(|&l| floor_value(ds1, l, x, y).is_empty());
        let index = free.or_else(|| (ds1.floors.len() < 2).then_some(ds1.floors.len()));
        let Some(index) = index else {
            return Err("both floor layers are used, so there's no room for a blocker (paint the whole cell with the Cell brush instead, or change the tiles themselves)".into());
        };
        new_floors = new_floors.max(index + 1);
        let s = blocker_for(records, want)?;
        cell_edits.push((LayerRef::Floor(index as u8), x, y, hidden_blocker(s)));
    }
    plan.floors = new_floors;
    plan.edits.extend(cell_edits);
    for j in 0..25 {
        if painted[j] {
            let after = match paint.mode {
                WalkMode::Block => base[j] | want[j],
                WalkMode::Clear => (base[j] & !clear_bits) | want[j],
            };
            if after != before[j] {
                plan.changed += 1;
            }
        }
    }
    Ok(())
}

/// A copy of the tiles of `keys` (every variant) with the `clear` bits taken
/// off, under a free sub index; reuses an identical earlier copy.
fn fork_for(
    tiles: &TileSet,
    records: &mut Vec<Tile>,
    taken: &mut BTreeSet<(i32, i32, i32)>,
    keys: &[TileKey],
    clear: &Flags,
) -> Option<u8> {
    let mut copies: Vec<(i32, i32, Tile)> = Vec::new();
    for k in keys {
        for r in tiles.candidates(*k) {
            let mut t = tiles.tile(r)?.clone();
            for (f, c) in t.subtile_flags.iter_mut().zip(clear) {
                *f &= !c;
            }
            copies.push((t.orientation, t.main_index, t));
        }
    }
    if copies.is_empty() {
        return None;
    }
    // The same copy made before: reuse it.
    let subs: BTreeSet<i32> = records
        .iter()
        .filter(|r| {
            keys.iter().any(|k| {
                i32::from(k.orientation) == r.orientation && i32::from(k.main) == r.main_index
            })
        })
        .map(|r| r.sub_index)
        .collect();
    for s in subs {
        let existing: Vec<&Tile> = records
            .iter()
            .filter(|r| {
                r.sub_index == s
                    && keys.iter().any(|k| {
                        i32::from(k.orientation) == r.orientation
                            && i32::from(k.main) == r.main_index
                    })
            })
            .collect();
        let same = existing.len() == copies.len()
            && existing.iter().zip(&copies).all(|(e, (_, _, c))| {
                let mut c = c.clone();
                c.sub_index = s;
                **e == c
            });
        if same {
            return u8::try_from(s).ok();
        }
    }
    let pairs: Vec<(i32, i32)> = keys
        .iter()
        .map(|k| (i32::from(k.orientation), i32::from(k.main)))
        .collect();
    let s = free_sub(taken, &pairs)?;
    for (o, m, mut t) in copies {
        t.sub_index = s;
        records.push(t);
        taken.insert((o, m, s));
    }
    u8::try_from(s).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use d2_formats::dt1::Dt1;

    fn floor_tile(main: i32, sub: i32, flags: Flags) -> Tile {
        Tile {
            orientation: 0,
            main_index: main,
            sub_index: sub,
            subtile_flags: flags,
            height: -80,
            width: 160,
            ..Tile::default()
        }
    }

    fn setup() -> (Ds1, TileSet) {
        let mut ds1 = Ds1::new(4, 4, 0);
        for y in 0..4 {
            for x in 0..4 {
                ds1.floors[0].set(x, y, Cell(FLOOR_PROP1).with_indices(1, 0));
            }
        }
        let dt1 = Dt1 {
            version: (7, 6),
            tiles: vec![floor_tile(1, 0, [0; 25])],
        };
        let tiles = TileSet::from_dt1s(
            vec![d2_assets::GamePath::new("data\\global\\tiles\\a.dt1")],
            vec![dt1],
        );
        (ds1, tiles)
    }

    fn one_subtile(x: u32, y: u32, sub: u32) -> WalkPaint {
        WalkPaint {
            mode: WalkMode::Block,
            bits: 1,
            cells: BTreeMap::from([((x, y), 1 << sub)]),
        }
    }

    #[test]
    fn blocking_a_subtile_adds_a_hidden_blocker_floor() {
        let (ds1, tiles) = setup();
        let plan = plan_walk(&ds1, &tiles, None, &one_subtile(1, 1, 6), None);
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
        assert_eq!(plan.floors, 2);
        assert_eq!(plan.changed, 1);
        let dt1 = plan.dt1.expect("a walk library");
        assert_eq!(dt1.tiles.len(), 1);
        assert_eq!(dt1.tiles[0].subtile_flags[6], 1);
        assert_eq!(
            dt1.tiles[0]
                .subtile_flags
                .iter()
                .filter(|f| **f != 0)
                .count(),
            1
        );
        let (layer, x, y, v) = plan.edits[0];
        assert_eq!((layer, x, y), (LayerRef::Floor(1), 1, 1));
        assert!(Cell(v.cell).hidden());
    }

    #[test]
    fn clearing_what_a_blocker_blocks_removes_the_blocker() {
        let (mut ds1, tiles) = setup();
        let plan = plan_walk(&ds1, &tiles, None, &one_subtile(1, 1, 6), None);
        let dt1 = plan.dt1.clone().unwrap();
        ds1.add_floor_layer().unwrap();
        for (l, x, y, v) in &plan.edits {
            if let LayerRef::Floor(i) = l {
                ds1.floors[usize::from(*i)].set(*x, *y, Cell(v.cell));
            }
        }
        let clear = WalkPaint {
            mode: WalkMode::Clear,
            bits: 1,
            cells: BTreeMap::from([((1, 1), 1 << 6)]),
        };
        let again = plan_walk(&ds1, &tiles, Some(&dt1), &clear, None);
        assert!(again.skipped.is_empty(), "{:?}", again.skipped);
        assert!(again.edits.iter().any(|e| e.3.is_empty()));
        assert!(again.dt1.is_none());
    }

    #[test]
    fn whole_cell_flag_goes_on_the_floor_and_comes_off_again() {
        let (ds1, _) = setup();
        let on = cell_unwalkable_plan(&ds1, &[(2, 2)], true);
        assert_eq!(on.cells, 1);
        assert_ne!(on.edits[0].3.cell & UNWALKABLE, 0);
        let mut ds2 = ds1.clone();
        ds2.floors[0].set(2, 2, Cell(on.edits[0].3.cell));
        let off = cell_unwalkable_plan(&ds2, &[(2, 2)], false);
        assert_eq!(off.edits[0].3.cell & UNWALKABLE, 0);
    }

    #[test]
    fn no_spawn_hides_region_seeds_over_a_copy_on_floor_two() {
        let (ds1, _) = setup();
        let plan = no_spawn_plan(&ds1, &[(1, 1)], false);
        assert!(plan.copied >= 1, "{plan:?}");
        assert_eq!(plan.floors, 2);
        assert!(
            plan.edits
                .iter()
                .any(|e| e.0 == LayerRef::Floor(0) && Cell(e.3.cell).hidden())
        );
    }
}
