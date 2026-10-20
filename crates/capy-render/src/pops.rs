//! "Pops": how the game hides roofs (any wall-layer tiles) while a player is
//! inside a building. Port of ds1-studio's `game/pops.ts` (analysis only).
//!
//! * A marker is a special tile (orientation 10/11) on any wall layer with
//!   main index 8-29. Markers pair by main index: the first in the file (wall
//!   layer, then row by row) and the last are the corners.
//! * The first marker's sub index is the main index of the tiles that hide:
//!   every wall-layer tile with it inside the rectangle grown by one cell fades
//!   while a player stands in the rectangle.
//! * The trigger area is the rectangle grown by LvlPrest `PopPad` (sub-tiles,
//!   5 per cell) to the south and east.
//! * The game brings tiles back only in rooms (8x8 cells) next to the
//!   player's, see [`pop_reach`].

use d2_formats::ds1::{Cell, Ds1};

use crate::rooms::ROOM_CELLS;
use crate::walk::is_special;

pub const POP_MIN: u8 = 8;
pub const POP_MAX: u8 = 29;

/// The group a marker's main index belongs to (areas of one group hide together).
pub fn pop_group(main: u8) -> u8 {
    (main / 4).saturating_sub(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopMarker {
    pub x: u32,
    pub y: u32,
    pub layer: usize,
    pub sub: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopArea {
    pub main: u8,
    /// Main index of the tiles that hide (the first marker's sub index).
    pub target: u8,
    pub group: u8,
    pub markers: Vec<PopMarker>,
    /// The trigger rectangle in cells (inclusive).
    pub rect: CellRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

pub fn find_pops(ds1: &Ds1) -> Vec<PopArea> {
    let mut by_main: std::collections::BTreeMap<u8, PopArea> = std::collections::BTreeMap::new();
    let mut order: Vec<u8> = Vec::new();
    for (li, layer) in ds1.walls.iter().enumerate() {
        for y in 0..ds1.height {
            for x in 0..ds1.width {
                let c = layer.tiles.get(x, y).unwrap_or(Cell::EMPTY);
                let m = c.main_index();
                if c.is_empty()
                    || !is_special(ds1.orientation(li, x, y).unwrap_or(0))
                    || !(POP_MIN..=POP_MAX).contains(&m)
                {
                    continue;
                }
                let marker = PopMarker {
                    x,
                    y,
                    layer: li,
                    sub: c.sub_index(),
                };
                by_main
                    .entry(m)
                    .and_modify(|a| a.markers.push(marker))
                    .or_insert_with(|| {
                        order.push(m);
                        PopArea {
                            main: m,
                            target: c.sub_index(),
                            group: pop_group(m),
                            markers: vec![marker],
                            rect: CellRect {
                                x0: 0,
                                y0: 0,
                                x1: 0,
                                y1: 0,
                            },
                        }
                    });
            }
        }
    }
    // First-seen order, as ds1-studio's Map iteration.
    let mut out: Vec<PopArea> = order.iter().filter_map(|m| by_main.remove(m)).collect();
    for a in &mut out {
        let (p, q) = (a.markers[0], a.markers[a.markers.len() - 1]);
        a.rect = CellRect {
            x0: p.x.min(q.x) as i32,
            x1: p.x.max(q.x) as i32,
            y0: p.y.min(q.y) as i32,
            y1: p.y.max(q.y) as i32,
        };
    }
    out
}

/// The cells whose tiles can hide: the rectangle grown by one cell.
pub fn hide_rect(r: CellRect) -> CellRect {
    CellRect {
        x0: r.x0 - 1,
        y0: r.y0 - 1,
        x1: r.x1 + 1,
        y1: r.y1 + 1,
    }
}

/// The trigger area in cells `(x, y, w, h)` (fractional: PopPad grows south/east).
pub fn trigger_rect(r: CellRect, pop_pad: i32) -> (i32, i32, f64, f64) {
    let pad = f64::from(pop_pad) / 5.0;
    (
        r.x0,
        r.y0,
        (f64::from(r.x1 - r.x0 + 1) + pad).max(0.0),
        (f64::from(r.y1 - r.y0 + 1) + pad).max(0.0),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HiddenTile {
    pub layer: usize,
    pub x: u32,
    pub y: u32,
}

/// Wall-layer tiles with main index `target` inside a rectangle (markers excluded).
pub fn tiles_to_hide(ds1: &Ds1, r: CellRect, target: u8) -> Vec<HiddenTile> {
    let mut out = Vec::new();
    for (li, layer) in ds1.walls.iter().enumerate() {
        for y in r.y0.max(0)..=r.y1.min(ds1.height as i32 - 1) {
            for x in r.x0.max(0)..=r.x1.min(ds1.width as i32 - 1) {
                let c = layer.tiles.get(x as u32, y as u32).unwrap_or(Cell::EMPTY);
                if c.is_empty() || is_special(ds1.orientation(li, x as u32, y as u32).unwrap_or(0))
                {
                    continue;
                }
                if c.main_index() == target {
                    out.push(HiddenTile {
                        layer: li,
                        x: x as u32,
                        y: y as u32,
                    });
                }
            }
        }
    }
    out
}

pub fn pop_targets(ds1: &Ds1, a: &PopArea) -> Vec<HiddenTile> {
    tiles_to_hide(ds1, hide_rect(a.rect), a.target)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopReach {
    /// Cells just outside the trigger area from which some hidden tiles are out of reach.
    pub exits: Vec<(i32, i32)>,
    /// A trigger rectangle that keeps every tile in reach, if one exists.
    pub fix: Option<CellRect>,
}

/// Whether leaving a hide area anywhere brings its whole roof back; `None` when it does.
pub fn pop_reach(ds1: &Ds1, a: &PopArea, pop_pad: i32) -> Option<PopReach> {
    let tiles = pop_targets(ds1, a);
    if tiles.is_empty() {
        return None;
    }
    let rc = ROOM_CELLS as i32;
    let room = |v: i32| v.div_euclid(rc);
    let grow = (pop_pad + 4).div_euclid(5).max(0); // ceil(pop_pad / 5)
    let ring = |r: CellRect| {
        let mut out = Vec::new();
        let (ex, ey) = (r.x1 + grow + 1, r.y1 + grow + 1);
        for x in r.x0 - 1..=ex {
            for y in r.y0 - 1..=ey {
                if x < r.x0 || x >= ex || y < r.y0 || y >= ey {
                    out.push((x, y));
                }
            }
        }
        out
    };
    let far = |c: &(i32, i32)| {
        tiles.iter().any(|t| {
            (room(t.x as i32) - room(c.0)).abs() > 1 || (room(t.y as i32) - room(c.1)).abs() > 1
        })
    };
    let exits: Vec<(i32, i32)> = ring(a.rect).into_iter().filter(far).collect();
    if exits.is_empty() {
        return None;
    }
    let tx0 = tiles.iter().map(|t| t.x as i32).min()?;
    let tx1 = tiles.iter().map(|t| t.x as i32).max()?;
    let ty0 = tiles.iter().map(|t| t.y as i32).min()?;
    let ty1 = tiles.iter().map(|t| t.y as i32).max()?;
    let x0 = a.rect.x0.max((room(tx1) - 1) * rc + 1);
    let y0 = a.rect.y0.max((room(ty1) - 1) * rc + 1);
    let x1 = a.rect.x1.min((room(tx0) + 2) * rc - 2 - grow);
    let y1 = a.rect.y1.min((room(ty0) + 2) * rc - 2 - grow);
    let cand = CellRect { x0, y0, x1, y1 };
    let ok = x0 <= x1
        && y0 <= y1
        && x0 - 1 <= tx0
        && y0 - 1 <= ty0
        && x1 + 1 >= tx1
        && y1 + 1 >= ty1
        && !ring(cand).iter().any(far);
    Some(PopReach {
        exits,
        fix: ok.then_some(cand),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopProblem {
    pub severity: PopSeverity,
    pub text: String,
    /// Main index of the area it concerns.
    pub area: Option<u8>,
}

/// What would stop a map's pops working. `pops` = LvlPrest `Pops` for the map
/// (`None` = no row), `pop_pad` its `PopPad`.
pub fn pop_problems(
    ds1: &Ds1,
    areas: &[PopArea],
    pops: Option<i32>,
    pop_pad: i32,
) -> Vec<PopProblem> {
    let mut out = Vec::new();
    if areas.is_empty() {
        return out;
    }
    let n = areas.len() as i32;
    let plural = |k: i32| if k == 1 { "" } else { "s" };
    match pops {
        Some(0) => out.push(PopProblem {
            severity: PopSeverity::Error,
            text: format!(
                "LvlPrest.txt's Pops is 0 for this map, so the game ignores its {n} hide area{}: roofs never disappear.",
                plural(n)
            ),
            area: None,
        }),
        Some(p) if p < n => out.push(PopProblem {
            severity: PopSeverity::Error,
            text: format!("The map has {n} hide areas but LvlPrest.txt's Pops is {p}: the extra markers overrun the game's memory."),
            area: None,
        }),
        _ => {}
    }
    for a in areas {
        let name = format!("Hide area {} (hides tiles #{})", a.main, a.target);
        let w = |text: String| PopProblem {
            severity: PopSeverity::Warning,
            text,
            area: Some(a.main),
        };
        if a.markers.len() == 1 {
            out.push(w(format!(
                "{name} has only one corner marker, so its trigger area is a single cell."
            )));
        }
        if a.markers.len() > 2 {
            out.push(w(format!(
                "{name} has {} markers; the game uses only the first and last as corners ({}x{} cells).",
                a.markers.len(),
                a.rect.x1 - a.rect.x0 + 1,
                a.rect.y1 - a.rect.y0 + 1
            )));
        }
        if a.markers.iter().any(|m| m.sub != a.target) {
            out.push(w(format!(
                "{name}: its markers have different sub indices; the game uses the first one's."
            )));
        }
        if pop_targets(ds1, a).is_empty() {
            out.push(w(format!(
                "{name}: no wall-layer tile with main index {} is in its area.",
                a.target
            )));
        }
        if a.markers.len() == 2 && pop_reach(ds1, a, pop_pad).is_some() {
            out.push(w(format!(
                "{name} is too big for the game's 8x8-cell rooms: part of the roof can stay hidden."
            )));
        }
    }
    out
}
