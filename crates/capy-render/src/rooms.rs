//! Game rooms, monster-spawn regions and the walkable/spawn overviews.
//!
//! Port of ds1-studio's `game/spawnRegions.ts` (analysis part; its
//! `noSpawnPlan` edit planner is not ported) and `game/mapOverlays.ts`.
//!
//! The game cuts a preset map into rooms of 8x8 cells from its top corner
//! (the game's map size is the DS1 size minus one). With LvlPrest Logicals=1
//! each room's (8+1)x(8+1) grid is split into regions bounded by first-layer
//! walls; a region whose seed cell has a hidden first-layer floor, or main
//! index 30, is a "node" and spawns nothing; a room with a level warp (special
//! tile, main 0-7) spawns nothing.

use d2_formats::ds1::{Cell, Ds1};

use crate::walk::{BLOCK_PLAYER_WALK, BLOCK_WALK, SubTileGrid, is_special};

pub const ROOM_CELLS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresetRoom {
    pub x0: u32,
    pub y0: u32,
    /// Its own cells (up to 8); the grid adds one more row and column.
    pub w: u32,
    pub h: u32,
}

/// The 8x8 game rooms a map is cut into (row-major).
pub fn preset_rooms(width: u32, height: u32) -> Vec<PresetRoom> {
    let (mw, mh) = (width.saturating_sub(1), height.saturating_sub(1));
    let mut out = Vec::new();
    let mut y0 = 0;
    while y0 < mh {
        let mut x0 = 0;
        while x0 < mw {
            out.push(PresetRoom {
                x0,
                y0,
                w: ROOM_CELLS.min(mw - x0),
                h: ROOM_CELLS.min(mh - y0),
            });
            x0 += ROOM_CELLS;
        }
        y0 += ROOM_CELLS;
    }
    out
}

/// The room a cell belongs to (its own cells, not the shared border).
pub fn room_at(rooms: &[PresetRoom], x: u32, y: u32) -> Option<&PresetRoom> {
    rooms
        .iter()
        .find(|r| x >= r.x0 && x < r.x0 + r.w && y >= r.y0 && y < r.y0 + r.h)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicRegion {
    /// The first cell scanned; its first-layer floor decides node-ness.
    pub seed: (u32, u32),
    pub node: bool,
    /// The room's own cells in the region.
    pub cells: Vec<(u32, u32)>,
    /// How many of those have a floor tile on any floor layer.
    pub floored: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomRegions {
    pub room: PresetRoom,
    pub regions: Vec<LogicRegion>,
    /// A level warp is in the room: the game spawns nothing in it.
    pub warp: bool,
}

const WALL_CLASS: [i32; 20] = [-1, 0, 1, 2, 2, 0, 1, 3, 0, 1, 0, 1, 4, -1, 4, 0, 0, 0, 0, 0];
const WALL_FLOW: [u8; 25] = [
    23, 0, 5, 21, 17, 15, 3, 0, 9, 7, 39, 0, 0, 5, 3, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31,
];
const STEPS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// The game's node test: a hidden floor, or main index 30 with no layer bits
/// (bits 18-19) in the word.
pub fn is_node_floor(c: Cell) -> bool {
    c.hidden() || (c.main_index() == 30 && (c.0 >> 16) & 0x0c == 0 && !c.is_empty())
}

fn first_wall(ds1: &Ds1, x: u32, y: u32) -> Cell {
    ds1.walls
        .first()
        .and_then(|l| l.tiles.get(x, y))
        .unwrap_or(Cell::EMPTY)
}

fn is_barrier(ds1: &Ds1, x: u32, y: u32) -> bool {
    let c = first_wall(ds1, x, y);
    if c.is_empty() {
        return false;
    }
    let o = ds1.orientation(0, x, y).unwrap_or(0);
    o != 0 && o != 13 && o != 15
}

struct Flood<'a> {
    ds1: &'a Ds1,
    room: PresetRoom,
    gw: i32,
    gh: i32,
    barrier: Vec<bool>,
    owner: Vec<i32>,
}

impl Flood<'_> {
    fn orientation(&self, i: i32, j: i32) -> usize {
        let (x, y) = (self.room.x0 as i32 + i, self.room.y0 as i32 + j);
        if x < 0 || y < 0 {
            return 0;
        }
        usize::from(self.ds1.orientation(0, x as u32, y as u32).unwrap_or(0))
    }

    /// DRLGLOGIC_SetTileGridFlags: open cells spread to all four sides; a wall
    /// passes on by its kind and the side it was entered from, and may
    /// continue diagonally.
    fn fill(&mut self, id: i32, mut x: i32, mut y: i32, mut from: i32) {
        while x >= 0 && y >= 0 && x < self.gw && y < self.gh {
            let k = (y * self.gw + x) as usize;
            if self.owner[k] >= 0 {
                return;
            }
            if !self.barrier[k] {
                self.owner[k] = id;
                for (dir, (dx, dy)) in STEPS.iter().enumerate() {
                    self.fill(id, x + dx, y + dy, dir as i32);
                }
                return;
            }
            let cls = WALL_CLASS.get(self.orientation(x, y)).copied().unwrap_or(0);
            let flow = if cls < 0 {
                0
            } else {
                usize::try_from(from + 5 * cls + 1)
                    .ok()
                    .and_then(|i| WALL_FLOW.get(i))
                    .copied()
                    .unwrap_or(0)
            };
            if flow & 1 != 0 {
                self.owner[k] = id;
            }
            if flow & 2 != 0 && from != 2 {
                self.fill(id, x + 1, y, 0);
            }
            if flow & 4 != 0 && from != 3 {
                self.fill(id, x, y + 1, 1);
            }
            if flow & 8 != 0 && from != 0 {
                self.fill(id, x - 1, y, 2);
            }
            if flow & 16 != 0 && from != 1 {
                self.fill(id, x, y - 1, 3);
            }
            if flow & 32 == 0 {
                return;
            }
            x += 1;
            y += 1;
            from = -1;
        }
    }
}

/// The regions of one room, as the game floods them.
pub fn room_regions(ds1: &Ds1, room: PresetRoom) -> RoomRegions {
    let (gw, gh) = ((room.w + 1) as i32, (room.h + 1) as i32);
    let mut barrier = vec![false; (gw * gh) as usize];
    for j in 0..gh {
        for i in 0..gw {
            let (x, y) = (room.x0 + i as u32, room.y0 + j as u32);
            if x < ds1.width && y < ds1.height && is_barrier(ds1, x, y) {
                barrier[(j * gw + i) as usize] = true;
            }
        }
    }
    let mut fl = Flood {
        ds1,
        room,
        gw,
        gh,
        barrier,
        owner: vec![-1; (gw * gh) as usize],
    };
    let mut all: Vec<LogicRegion> = Vec::new();
    for j in 0..gh {
        for i in 0..gw {
            if fl.owner[(j * gw + i) as usize] >= 0 {
                continue;
            }
            let (x, y) = (room.x0 + i as u32, room.y0 + j as u32);
            let node = x < ds1.width
                && y < ds1.height
                && ds1
                    .floors
                    .first()
                    .and_then(|g| g.get(x, y))
                    .is_some_and(is_node_floor);
            all.push(LogicRegion {
                seed: (x, y),
                node,
                cells: Vec::new(),
                floored: 0,
            });
            fl.fill(all.len() as i32 - 1, i, j, -1);
        }
    }
    for j in 0..room.h {
        for i in 0..room.w {
            let id = fl.owner[(j as i32 * gw + i as i32) as usize];
            if id < 0 {
                continue;
            }
            let (x, y) = (room.x0 + i, room.y0 + j);
            let r = &mut all[id as usize];
            r.cells.push((x, y));
            if ds1
                .floors
                .iter()
                .any(|g| g.get(x, y).is_some_and(|c| !c.is_empty()))
            {
                r.floored += 1;
            }
        }
    }
    let mut warp = false;
    'outer: for j in 0..room.h {
        for i in 0..room.w {
            for (li, l) in ds1.walls.iter().enumerate() {
                let (x, y) = (room.x0 + i, room.y0 + j);
                let c = l.tiles.get(x, y).unwrap_or(Cell::EMPTY);
                if !c.is_empty()
                    && is_special(ds1.orientation(li, x, y).unwrap_or(0))
                    && c.main_index() < 8
                {
                    warp = true;
                    break 'outer;
                }
            }
        }
    }
    all.retain(|r| !r.cells.is_empty());
    RoomRegions {
        room,
        regions: all,
        warp,
    }
}

/// Whether monsters can be placed in a region: not a node, in a room without a
/// warp, with floor to stand on.
pub fn spawns(rr: &RoomRegions, r: &LogicRegion) -> bool {
    !rr.warp && !r.node && r.floored > 0
}

/// Rooms that look meant to spawn nothing (some region seed has a hidden
/// first-layer floor) but still have regions monsters can be placed in.
/// Returns `(room regions, indices of the open regions)`.
pub fn partly_no_spawn_rooms(ds1: &Ds1) -> Vec<(RoomRegions, Vec<usize>)> {
    let mut out = Vec::new();
    let Some(f1) = ds1.floors.first() else {
        return out;
    };
    for room in preset_rooms(ds1.width, ds1.height) {
        let rr = room_regions(ds1, room);
        if rr.warp {
            continue;
        }
        let marked = rr
            .regions
            .iter()
            .any(|r| f1.get(r.seed.0, r.seed.1).is_some_and(Cell::hidden));
        let open: Vec<usize> = rr
            .regions
            .iter()
            .enumerate()
            .filter(|(_, r)| spawns(&rr, r))
            .map(|(i, _)| i)
            .collect();
        if marked && !open.is_empty() {
            out.push((rr, open));
        }
    }
    out
}

// ---- overviews ------------------------------------------------------------

fn no_floor(ds1: &Ds1, x: u32, y: u32) -> bool {
    ds1.floors
        .iter()
        .all(|g| g.get(x, y).is_none_or(Cell::is_empty))
}

/// Void players can walk into: sub-tiles of floorless cells that nothing blocks
/// (for players), reached from walkable floor through side-by-side sub-tiles,
/// and the floor sub-tiles where that happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenVoid {
    /// Per sub-tile (grid index): 1 = reachable void.
    pub reach: Vec<u8>,
    /// Per sub-tile: 1 = walkable floor next to such void.
    pub edge: Vec<u8>,
    pub edge_cells: Vec<(u32, u32)>,
}

pub fn open_void(ds1: &Ds1, grid: &SubTileGrid) -> OpenVoid {
    let (width, height) = (ds1.width, ds1.height);
    let (ww, hh) = ((width * 5) as i32, (height * 5) as i32);
    let is_void: Vec<bool> = (0..width * height)
        .map(|i| no_floor(ds1, i % width, i / width))
        .collect();
    let mut reach = vec![0u8; grid.flags.len()];
    let mut edge = vec![0u8; grid.flags.len()];
    let idx = |sx: i32, sy: i32| {
        ((sy / 5 * width as i32 + sx / 5) * 25 + (sy % 5) * 5 + (sx % 5)) as usize
    };
    let void_at = |sx: i32, sy: i32| is_void[(sy / 5 * width as i32 + sx / 5) as usize];
    let free = |sx: i32, sy: i32| grid.flags[idx(sx, sy)] & (BLOCK_WALK | BLOCK_PLAYER_WALK) == 0;
    let inside = |sx: i32, sy: i32| sx >= 0 && sy >= 0 && sx < ww && sy < hh;
    let steps = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for sy in 0..hh {
        for sx in 0..ww {
            if void_at(sx, sy) || !free(sx, sy) {
                continue;
            }
            for (dx, dy) in steps {
                let (nx, ny) = (sx + dx, sy + dy);
                if !inside(nx, ny) || !void_at(nx, ny) || !free(nx, ny) {
                    continue;
                }
                edge[idx(sx, sy)] = 1;
                let k = idx(nx, ny);
                if reach[k] == 0 {
                    reach[k] = 1;
                    stack.push((nx, ny));
                }
            }
        }
    }
    while let Some((sx, sy)) = stack.pop() {
        for (dx, dy) in steps {
            let (nx, ny) = (sx + dx, sy + dy);
            if !inside(nx, ny) || !void_at(nx, ny) || !free(nx, ny) {
                continue;
            }
            let k = idx(nx, ny);
            if reach[k] != 0 {
                continue;
            }
            reach[k] = 1;
            stack.push((nx, ny));
        }
    }
    let edge_cells = (0..width * height)
        .filter(|i| {
            edge[(*i * 25) as usize..(*i * 25 + 25) as usize]
                .iter()
                .any(|v| *v != 0)
        })
        .map(|i| (i % width, i / width))
        .collect();
    OpenVoid {
        reach,
        edge,
        edge_cells,
    }
}

/// Class of a sub-tile in the walkable overview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkClass {
    None,
    Walk,
    /// Blocks players only (monsters may walk).
    MonstersOnly,
    Blocked,
    /// Floorless sub-tile players can walk into.
    Void,
    /// Floor sub-tile next to such void.
    OpenEdge,
}

/// Per-sub-tile classes of the walkable overview (`grid` = game collision,
/// see [`crate::walk::Collision::Game`]).
pub fn walkable_overlay(ds1: &Ds1, grid: &SubTileGrid) -> Vec<WalkClass> {
    let open = open_void(ds1, grid);
    let mut out = vec![WalkClass::None; grid.flags.len()];
    for i in 0..ds1.width * ds1.height {
        let empty = no_floor(ds1, i % ds1.width, i / ds1.width);
        for k in 0..25usize {
            let j = i as usize * 25 + k;
            let f = grid.flags[j];
            out[j] = if empty {
                if open.reach[j] != 0 {
                    WalkClass::Void
                } else {
                    WalkClass::None
                }
            } else if open.edge[j] != 0 {
                WalkClass::OpenEdge
            } else if f & BLOCK_WALK != 0 {
                WalkClass::Blocked
            } else if f & BLOCK_PLAYER_WALK != 0 {
                WalkClass::MonstersOnly
            } else {
                WalkClass::Walk
            };
        }
    }
    out
}

/// The level settings that decide random monsters; `None` = unknown (allows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpawnLevel {
    /// LvlPrest Populate.
    pub populate: Option<bool>,
    /// LvlPrest Logicals.
    pub logicals: Option<bool>,
    /// Levels.txt MonDen per difficulty.
    pub mon_den: Option<[i32; 3]>,
    /// Levels.txt `mon1..` count and `nmon1..` count (non-zero when listed).
    pub monsters: Option<[i32; 2]>,
}

/// Why the level gets no random monsters at all.
pub fn level_no_spawn(l: &SpawnLevel) -> Option<&'static str> {
    if l.populate == Some(false) {
        return Some("LvlPrest Populate is 0: no random monsters in this preset");
    }
    if l.mon_den.is_some_and(|d| d.iter().all(|v| *v <= 0)) {
        return Some("Levels.txt MonDen is 0: no random monsters");
    }
    if l.monsters.is_some_and(|m| m[0] + m[1] == 0) {
        return Some("Levels.txt lists no monsters: no random monsters");
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnClass {
    None,
    Spawn,
    Blocked,
    Node,
    Warp,
    /// Open, but the level spawns none.
    Off,
    /// Last row/column (in no game room).
    Edge,
    /// Floorless sub-tile of a region with floor that nothing blocks.
    Void,
}

/// Per-sub-tile classes of the monster-spawn overview.
pub fn spawn_overlay(ds1: &Ds1, grid: &SubTileGrid, level: &SpawnLevel) -> Vec<SpawnClass> {
    let (width, height) = (ds1.width, ds1.height);
    let mut why = vec![SpawnClass::Edge; (width * height) as usize];
    let mut floored = vec![false; (width * height) as usize];
    let off = level_no_spawn(level).is_some();
    for room in preset_rooms(width, height) {
        let rr = room_regions(ds1, room);
        for r in &rr.regions {
            let reason = if rr.warp {
                SpawnClass::Warp
            } else if r.node && level.logicals != Some(false) {
                SpawnClass::Node
            } else if off {
                SpawnClass::Off
            } else {
                SpawnClass::Spawn
            };
            for (x, y) in &r.cells {
                let i = (y * width + x) as usize;
                why[i] = reason;
                floored[i] = r.floored > 0;
            }
        }
    }
    let open = open_void(ds1, grid);
    let mut sub = vec![SpawnClass::None; grid.flags.len()];
    for i in 0..width * height {
        let empty = no_floor(ds1, i % width, i / width);
        for k in 0..25usize {
            let j = i as usize * 25 + k;
            if !empty {
                sub[j] = if grid.flags[j] & BLOCK_WALK != 0 {
                    SpawnClass::Blocked
                } else {
                    why[i as usize]
                };
            } else if open.reach[j] != 0 && floored[i as usize] && grid.flags[j] & BLOCK_WALK == 0 {
                sub[j] = if why[i as usize] == SpawnClass::Spawn {
                    SpawnClass::Void
                } else {
                    why[i as usize]
                };
            }
        }
    }
    sub
}
