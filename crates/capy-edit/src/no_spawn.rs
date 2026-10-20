//! The no-spawn planner (ds1-studio's `noSpawnPlan`): makes the 8x8 game rooms
//! touching some cells spawn no random monsters while they stay walkable and
//! look the same. A region of a room (see `capy_render::rooms`) is a "node"
//! when its seed cell has a hidden first-layer floor, so every seed gets one:
//!
//! - a floored seed's tile is copied to floor 2 first (drawn the same, same
//!   collision), then floor 1 is hidden;
//! - a seed with only a floor-2 tile gets a hidden copy of it on floor 1;
//! - a seed with no floor at all gets a hidden floor tile plus the unwalkable
//!   bit, so nothing new can be walked on.
//!
//! The plan is data; [`Document::apply_no_spawn`] makes it one undo step.

use std::collections::{BTreeMap, BTreeSet};

use capy_render::rooms::{PresetRoom, is_node_floor, preset_rooms, room_regions};
use d2_formats::ds1::{Cell, Ds1};

use crate::{CellValue, Document, LayerRef, Result};

/// The cell bit "unwalkable" (bit 17, `prop3 & 0x02`).
const UNWALKABLE: u32 = 1 << 17;
/// The `prop1` byte of a freshly painted floor.
const FLOOR_PROP1: u32 = 0xC2;

/// Options of [`no_spawn_plan`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSpawnOptions {
    /// Change every floored cell of the rooms, not only the region seeds.
    pub whole_rooms: bool,
    /// The floor tile `(main, sub)` for void seeds; default the most common
    /// visible floor-1 tile of the rooms.
    pub void_tile: Option<(u8, u8)>,
}

/// What [`no_spawn_plan`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoSpawnPlan {
    /// Floor layers the map needs (floor 2 is added for the copies).
    pub floors: usize,
    /// `(floor index, x, y, new cell)`.
    pub edits: Vec<(usize, u32, u32, Cell)>,
    /// The rooms covered.
    pub rooms: Vec<PresetRoom>,
    /// Seeds whose floor moved up to floor 2 under a hidden copy.
    pub copied: usize,
    /// Seeds with no floor, given a hidden unwalkable floor.
    pub voids: usize,
    /// Seeds that could not be changed, and why.
    pub skipped: Vec<String>,
}

/// Plans the no-spawn edits for the rooms touching `cells`.
pub fn no_spawn_plan(ds1: &Ds1, cells: &[(u32, u32)], opts: NoSpawnOptions) -> NoSpawnPlan {
    let (width, height) = (ds1.width, ds1.height);
    let rooms: Vec<PresetRoom> = preset_rooms(width, height)
        .into_iter()
        .filter(|r| {
            cells
                .iter()
                .any(|&(x, y)| x >= r.x0 && x <= r.x0 + r.w && y >= r.y0 && y <= r.y0 + r.h)
        })
        .collect();
    let f1 = ds1.floors.first();
    let f2 = ds1.floors.get(1);
    let at = |g: Option<&d2_formats::Grid<Cell>>, x, y| {
        g.and_then(|g| g.get(x, y)).unwrap_or(Cell::EMPTY)
    };
    let void_tile = opts.void_tile.or_else(|| {
        let f1 = f1?;
        let mut count: BTreeMap<(u8, u8), usize> = BTreeMap::new();
        for r in &rooms {
            for y in r.y0..r.y0 + r.h {
                for x in r.x0..r.x0 + r.w {
                    if let Some(c) = f1.get(x, y).filter(|c| !c.is_empty() && !c.hidden()) {
                        *count.entry((c.main_index(), c.sub_index())).or_default() += 1;
                    }
                }
            }
        }
        // Most common; ties go to the smallest tile for a stable answer.
        count
            .iter()
            .max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k)))
            .map(|(k, _)| *k)
            .or_else(|| {
                (0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .find_map(|(x, y)| {
                        f1.get(x, y)
                            .filter(|c| !c.is_empty())
                            .map(|c| (c.main_index(), c.sub_index()))
                    })
            })
    });

    let mut plan = NoSpawnPlan {
        floors: ds1.floors.len(),
        edits: Vec::new(),
        rooms: rooms.clone(),
        copied: 0,
        voids: 0,
        skipped: Vec::new(),
    };
    let mut done: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut hide_at = |plan: &mut NoSpawnPlan, x: u32, y: u32, why: &str| {
        if !done.insert((x, y)) {
            return;
        }
        if x >= width || y >= height {
            plan.skipped
                .push(format!("{x},{y} ({why}) is past the map's edge"));
            return;
        }
        let c1 = at(f1, x, y);
        if is_node_floor(c1) {
            return;
        }
        if !c1.is_empty() {
            if !at(f2, x, y).is_empty() {
                plan.skipped.push(format!(
                    "{x},{y} ({why}) has a floor 2 tile already, so its floor 1 can't be hidden without changing the look"
                ));
                return;
            }
            plan.edits.push((1, x, y, c1));
            plan.edits.push((0, x, y, c1.with_hidden(true)));
            plan.copied += 1;
            return;
        }
        let c2 = at(f2, x, y);
        if !c2.is_empty() {
            plan.edits.push((0, x, y, c2.with_hidden(true)));
            plan.copied += 1;
            return;
        }
        let Some((main, sub)) = void_tile else {
            plan.skipped.push(format!(
                "{x},{y} ({why}) has no floor and the map has no floor tile to hide there"
            ));
            return;
        };
        let hidden = Cell(FLOOR_PROP1).with_indices(main, sub).with_hidden(true);
        plan.edits.push((0, x, y, Cell(hidden.0 | UNWALKABLE)));
        plan.voids += 1;
    };
    for r in &rooms {
        for reg in room_regions(ds1, *r).regions {
            hide_at(&mut plan, reg.seed.0, reg.seed.1, "a region seed");
        }
        if opts.whole_rooms {
            for y in r.y0..r.y0 + r.h {
                for x in r.x0..r.x0 + r.w {
                    if !at(f1, x, y).is_empty() {
                        hide_at(&mut plan, x, y, "a floored cell");
                    }
                }
            }
        }
    }
    if plan.edits.iter().any(|e| e.0 == 1) {
        plan.floors = plan.floors.max(2);
    }
    plan.floors = plan.floors.max(1);
    plan
}

impl Document {
    /// Applies a [`NoSpawnPlan`] as one undo step, adding floor 2 when the plan
    /// needs it. Fails (changing nothing) when the file version cannot hold it.
    pub fn apply_no_spawn(&mut self, plan: &NoSpawnPlan) -> Result<()> {
        if plan.edits.is_empty() {
            return Ok(());
        }
        let before = self.ds1().clone();
        self.begin_group("No-spawn area");
        let r = (|| {
            if plan.floors > self.ds1().floors.len() {
                self.add_layer(LayerRef::Floor(plan.floors as u8 - 1))?;
            }
            for layer in 0..plan.floors.max(1) {
                let cells: Vec<(u32, u32, CellValue)> = plan
                    .edits
                    .iter()
                    .filter(|e| e.0 == layer)
                    .map(|&(_, x, y, c)| {
                        (
                            x,
                            y,
                            CellValue {
                                cell: c.0,
                                orientation: 0,
                            },
                        )
                    })
                    .collect();
                if !cells.is_empty() {
                    self.paint(LayerRef::Floor(layer as u8), &cells)?;
                }
            }
            Ok(())
        })();
        self.end_group();
        if r.is_err() {
            // Roll the half-applied group back so a refused plan changes nothing.
            while self.ds1() != &before && self.undo() {}
        }
        r
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use crate::{NewMapSpec, new_map};
    use capy_render::rooms::spawns;

    /// A 17x9 map (two game rooms) floored with 1/0 everywhere, with a line of
    /// left walls down x = 4.
    fn map() -> Ds1 {
        let mut d = new_map(&NewMapSpec::new(17, 9, 0)).unwrap();
        for y in 0..9 {
            for x in 0..17 {
                d.floors[0].set(x, y, Cell(FLOOR_PROP1).with_indices(1, 0));
            }
            d.walls[0].tiles.set(4, y, Cell(0x81).with_indices(2, 0));
            d.walls[0].orientations.set(4, y, 1);
        }
        d
    }

    #[test]
    fn hides_each_seed_under_a_floor2_copy_and_voids_get_a_hidden_unwalkable_floor() {
        let mut d = map();
        d.floors[0].set(0, 0, Cell::EMPTY); // a void seed
        let plan = no_spawn_plan(&d, &[(2, 2)], NoSpawnOptions::default());
        assert_eq!(
            plan.rooms,
            vec![PresetRoom {
                x0: 0,
                y0: 0,
                w: 8,
                h: 8
            }]
        );
        assert_eq!(
            (plan.copied, plan.voids, plan.floors, plan.skipped.clone()),
            (1, 1, 2, vec![])
        );
        let mut doc = Document::new(d);
        doc.apply_no_spawn(&plan).unwrap();
        let d = doc.ds1();
        let f2 = d.floors[1].get(4, 0).unwrap();
        assert_eq!(
            (f2.main_index(), f2.sub_index(), f2.hidden()),
            (1, 0, false)
        );
        assert!(d.floors[0].get(4, 0).unwrap().hidden());
        let v = d.floors[0].get(0, 0).unwrap();
        assert!(v.hidden() && v.main_index() == 1 && v.unwalkable());
        let room = preset_rooms(17, 9)[0];
        let rr = room_regions(d, room);
        assert!(rr.regions.iter().all(|r| r.node));
        // The other room still spawns, and applying again changes nothing.
        let rr2 = room_regions(d, preset_rooms(17, 9)[1]);
        assert!(rr2.regions.iter().any(|r| spawns(&rr2, r)));
        let again = no_spawn_plan(d, &[(2, 2)], NoSpawnOptions::default());
        assert!(again.edits.is_empty(), "{again:?}");
        // One undo step restores the map.
        assert!(doc.undo());
        assert_eq!(doc.ds1().floors.len(), 1);
        assert!(!doc.ds1().floors[0].get(4, 0).unwrap().hidden());
    }

    #[test]
    fn whole_rooms_changes_every_floored_cell_and_skips_seeds_with_floor2() {
        let mut d = map();
        d.floors.push(d2_formats::Grid::new(17, 9));
        d.floors[1].set(0, 0, Cell(FLOOR_PROP1).with_indices(1, 1));
        let plan = no_spawn_plan(
            &d,
            &[(0, 0)],
            NoSpawnOptions {
                whole_rooms: true,
                ..NoSpawnOptions::default()
            },
        );
        assert_eq!(
            plan.skipped,
            vec!["0,0 (a region seed) has a floor 2 tile already, so its floor 1 can't be hidden without changing the look".to_owned()]
        );
        assert_eq!(plan.copied, 63);
    }
}
