//! Ways out of a level: warp tiles, their target levels, exits and the
//! portal-arrival check.
//!
//! Port of ds1-studio's `game/warps.ts` (the read side), `exits.ts`,
//! `arrival.ts`. A warp special tile (orientation 10/11) with main index N
//! (0-7) is the level's link N: Levels.txt column `VisN` gives the target
//! level and `WarpN` the LvlWarp.txt row. The table data is passed in by the
//! caller ([`LevelRow`], [`LevelRef`]); nothing here reads game files.
//!
//! Level light (where ds1-studio keeps it): Levels.txt `Intensity` (0 = the
//! act's own daylight, else 1-255) and `Red`/`Green`/`Blue`. It edits them as
//! one `cellFix` on the map's Levels.txt row ("Level light" mode), not in the
//! DS1. See [`LevelLight`].

use std::collections::BTreeMap;

use d2_formats::ds1::{Cell, Ds1};

use crate::walk::is_special;

/// The town of each act (level ids), where an exit usually leads.
pub const ACT_TOWNS: [i32; 5] = [1, 40, 75, 103, 109];

/// A Levels.txt row, reduced to what links and light need. `vis`/`warp` use
/// `0` / `-1` for "no link" (an empty cell).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelRow {
    pub id: i32,
    pub name: String,
    pub vis: [i32; 8],
    pub warp: [i32; 8],
    pub intensity: i32,
    pub rgb: [i32; 3],
}

/// A level as a link target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelRef {
    pub id: i32,
    pub name: String,
    /// Map presets (DS1 game paths) the level is built from; empty for random levels.
    pub maps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarpLink {
    pub vis: u8,
    pub target: LevelRef,
    /// LvlWarp row id and its name, when `WarpN` is set.
    pub warp: Option<(i32, String)>,
}

/// Links 0-7 of a level (`None` = slot unused).
pub fn level_links(
    row: &LevelRow,
    target: &dyn Fn(i32) -> LevelRef,
    warp_name: &dyn Fn(i32) -> Option<String>,
) -> [Option<WarpLink>; 8] {
    std::array::from_fn(|v| {
        let t = row.vis[v];
        if t <= 0 {
            return None;
        }
        let w = row.warp[v];
        let warp = (w >= 0).then(|| (w, warp_name(w).unwrap_or_else(|| format!("LvlWarp {w}"))));
        Some(WarpLink {
            vis: v as u8,
            target: target(t),
            warp,
        })
    })
}

fn is_warp(ds1: &Ds1, li: usize, x: u32, y: u32, c: Cell) -> bool {
    !c.is_empty() && is_special(ds1.orientation(li, x, y).unwrap_or(0)) && c.main_index() <= 7
}

/// The warp tiles on a map, by link number (cells in layer then row order).
pub fn warp_tiles(ds1: &Ds1) -> BTreeMap<u8, Vec<(u32, u32)>> {
    let mut out: BTreeMap<u8, Vec<(u32, u32)>> = BTreeMap::new();
    for (li, layer) in ds1.walls.iter().enumerate() {
        for y in 0..ds1.height {
            for x in 0..ds1.width {
                let c = layer.tiles.get(x, y).unwrap_or(Cell::EMPTY);
                if is_warp(ds1, li, x, y, c) {
                    out.entry(c.main_index()).or_default().push((x, y));
                }
            }
        }
    }
    out
}

/// A warp tile on the map with where it leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapWarp {
    pub x: u32,
    pub y: u32,
    pub link: u8,
    /// `None`: Levels.txt has no `VisN` for this link (the warp leads nowhere).
    pub to: Option<WarpLink>,
}

/// Every warp tile with its resolved target.
pub fn map_warps(ds1: &Ds1, links: &[Option<WarpLink>; 8]) -> Vec<MapWarp> {
    warp_tiles(ds1)
        .into_iter()
        .flat_map(|(n, cells)| {
            let to = links.get(usize::from(n)).cloned().flatten();
            cells.into_iter().map(move |(x, y)| MapWarp {
                x,
                y,
                link: n,
                to: to.clone(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Info,
}

/// The link to set up and whether a warp tile for it still has to be placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitFix {
    pub vis: u8,
    pub edit: bool,
    pub place: bool,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitProblem {
    pub severity: Severity,
    pub title: String,
    pub cells: Vec<(u32, u32)>,
    pub fix: Option<ExitFix>,
}

pub struct ExitOptions<'a> {
    pub has_waypoint: bool,
    /// The map is the level's only preset (else another map may hold the exit).
    pub only_preset: bool,
    pub is_town: bool,
    pub level_name: &'a dyn Fn(i32) -> String,
}

/// Warp tiles that lead nowhere, and a level with no way out.
pub fn exit_problems(ds1: &Ds1, level: &LevelRow, o: &ExitOptions) -> Vec<ExitProblem> {
    let mut out = Vec::new();
    let vis = |n: u8| level.vis[usize::from(n)].max(0);
    let tiles = warp_tiles(ds1);
    for (n, cells) in &tiles {
        if vis(*n) != 0 {
            continue;
        }
        let one = cells.len() == 1;
        out.push(ExitProblem {
            severity: Severity::Warning,
            title: format!(
                "{} for link {n} {} nowhere",
                if one {
                    "A warp tile".to_string()
                } else {
                    format!("{} warp tiles", cells.len())
                },
                if one { "leads" } else { "lead" }
            ),
            cells: cells.clone(),
            fix: Some(ExitFix {
                vis: *n,
                edit: true,
                place: false,
                label: format!("Choose where link {n} leads..."),
            }),
        });
    }
    let linked = tiles.keys().any(|n| vis(*n) != 0);
    if o.is_town || !o.only_preset || linked || o.has_waypoint {
        return out;
    }
    let table_link = (0u8..8).find(|n| vis(*n) != 0);
    let unlinked = tiles.keys().next().copied();
    let free = (0u8..8).find(|n| vis(*n) == 0).unwrap_or(0);
    let fix = match (table_link, unlinked) {
        (Some(n), _) => Some(ExitFix {
            vis: n,
            edit: false,
            place: true,
            label: format!(
                "Place the warp tile for link {n} (to {})",
                (o.level_name)(vis(n))
            ),
        }),
        (None, Some(_)) => None,
        (None, None) => Some(ExitFix {
            vis: free,
            edit: true,
            place: true,
            label: "Add an exit warp...".into(),
        }),
    };
    out.push(ExitProblem {
        severity: Severity::Warning,
        title: "No way out of this level except a town portal".into(),
        cells: Vec::new(),
        fix,
    });
    out
}

/// Where the game puts a player who arrives without a warp (a map item's
/// portal): a waypoint object, a room with a warp tile, the room at the
/// level's centre, any room. A big map with its tiles off to one side has
/// nothing at its centre.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrivalProblem {
    /// The centre room's first cell (rooms are 8x8).
    pub room: (u32, u32),
    /// Cropping to the painted cells (2 around): deltas (<= 0) per edge.
    pub crop: Option<[i32; 4]>,
    pub cropped: Option<(u32, u32)>,
}

/// `None` when portal arrivals are fine. `is_waypoint(type, id)` identifies
/// waypoint objects.
pub fn arrival_problem(
    ds1: &Ds1,
    is_waypoint: &dyn Fn(i32, i32) -> bool,
) -> Option<ArrivalProblem> {
    if ds1
        .objects
        .iter()
        .any(|o| o.kind == 2 && is_waypoint(o.kind, o.id))
    {
        return None;
    }
    let has_warp = ds1.walls.iter().enumerate().any(|(li, l)| {
        (0..ds1.height).any(|y| {
            (0..ds1.width).any(|x| {
                let c = l.tiles.get(x, y).unwrap_or(Cell::EMPTY);
                // ds1-studio tests only orientation + main index (no prop1).
                is_special(ds1.orientation(li, x, y).unwrap_or(0)) && c.main_index() <= 7
            })
        })
    });
    if has_warp {
        return None;
    }
    let (w, h) = (ds1.width as i32, ds1.height as i32);
    let rx = ((((w - 1) / 2) - 2).div_euclid(8)) * 8;
    let ry = ((((h - 1) / 2) - 2).div_euclid(8)) * 8;
    let floor_at = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && ds1
                .floors
                .iter()
                .any(|g| g.get(x as u32, y as u32).is_some_and(|c| c.prop1() != 0))
    };
    for y in ry..(ry + 8).min(h) {
        for x in rx..(rx + 8).min(w) {
            if floor_at(x, y) {
                return None;
            }
        }
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, -1, -1);
    for y in 0..h {
        for x in 0..w {
            let any = floor_at(x, y)
                || ds1.walls.iter().any(|l| {
                    l.tiles
                        .get(x as u32, y as u32)
                        .is_some_and(|c| c.prop1() != 0)
                });
            if any {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    let m = 2;
    let crop = (max_x >= 0).then(|| {
        [
            -(min_x - m).max(0),
            -(min_y - m).max(0),
            -(w - 1 - max_x - m).max(0),
            -(h - 1 - max_y - m).max(0),
        ]
    });
    let useful = crop.filter(|c| c.iter().any(|v| *v != 0));
    Some(ArrivalProblem {
        room: (rx.max(0) as u32, ry.max(0) as u32),
        crop: useful,
        cropped: useful.map(|c| ((w + c[0] + c[2]) as u32, (h + c[1] + c[3]) as u32)),
    })
}

/// Levels.txt light of a level: `intensity` 0 = the act's daylight (drawn as
/// stored), else a fixed light 1-255 with colour `rgb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelLight {
    pub intensity: i32,
    pub rgb: [i32; 3],
}

impl LevelLight {
    pub fn of(row: &LevelRow) -> LevelLight {
        LevelLight {
            intensity: row.intensity,
            rgb: row.rgb,
        }
    }
    /// The colour multiplier `(intensity / 255) * (channel / 255)`; `None` for daylight.
    pub fn multiplier(&self) -> Option<[f32; 3]> {
        (self.intensity != 0).then(|| {
            self.rgb
                .map(|c| (self.intensity as f32 / 255.0) * (c as f32 / 255.0))
        })
    }
}
