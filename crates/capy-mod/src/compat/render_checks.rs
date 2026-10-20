//! The compatibility checks that need collision data: the sub-tile grid the
//! game builds from the map's tiles ([`capy_render::walk::sub_tile_flags`]
//! with `Collision::Game`). They are the walkability of objects, roof hiding
//! ("pops"), open map edges, spawn rooms, warp links and arrival by portal,
//! each judged by the `capy_render` modules (`walk`, `rooms`, `links`,
//! `pops`) and reported as [`Finding`]s like the other checks.

use std::collections::{BTreeMap, HashMap};

use capy_render::links::{
    ExitOptions, LevelRow, Severity as ExitSeverity, arrival_problem, exit_problems,
};
use capy_render::pops::{PopSeverity, find_pops, pop_problems};
use capy_render::rooms::{open_void, partly_no_spawn_rooms, preset_rooms, room_at};
use capy_render::walk::{BLOCK_PLAYER_WALK, BLOCK_WALK, SubTileGrid};
use d2_formats::ds1::Ds1;
use d2_formats::txt::TxtDoc;

use super::tables::TOWNS;
use super::{Area, Edit, Finding, Fix, Severity, TableName, Tables};
use crate::doc_util::{data_rows, get, num};

/// What the render checks need from the caller.
pub struct RenderEnv<'a> {
    /// Sub-tile flags of the map as the game builds them
    /// (`Collision::Game { blank }` with the preset's FillBlanks).
    pub grid: &'a SubTileGrid,
    /// Whether an object of this (type, id) is a waypoint.
    pub is_waypoint: &'a dyn Fn(i32, i32) -> bool,
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// The Levels.txt row of `id` reduced to what links need.
pub fn level_row(levels: &TxtDoc, id: i64) -> Option<(usize, LevelRow)> {
    let row = data_rows(levels)
        .into_iter()
        .find(|&r| num(levels, r, "Id") == id)?;
    let name = {
        let n = get(levels, row, "LevelName");
        if n.is_empty() {
            get(levels, row, "Name")
        } else {
            n
        }
    };
    let n8 = |p: &str| -> [i32; 8] {
        std::array::from_fn(|i| num(levels, row, &format!("{p}{i}")) as i32)
    };
    Some((
        row,
        LevelRow {
            id: id as i32,
            name,
            vis: n8("Vis"),
            warp: n8("Warp"),
            intensity: num(levels, row, "Intensity") as i32,
            rgb: [
                num(levels, row, "Red") as i32,
                num(levels, row, "Green") as i32,
                num(levels, row, "Blue") as i32,
            ],
        },
    ))
}

fn walkable(env: &RenderEnv, ds1: &Ds1, sx: i32, sy: i32) -> bool {
    sx >= 0
        && sy >= 0
        && sx < ds1.width as i32 * 5
        && sy < ds1.height as i32 * 5
        && env.grid.at(sx as u32, sy as u32) & (BLOCK_WALK | BLOCK_PLAYER_WALK) == 0
}

/// Nearest free walkable sub-tile in growing rings, not in `taken`.
fn nearest_free(
    env: &RenderEnv,
    ds1: &Ds1,
    from: (i32, i32),
    taken: &std::collections::HashSet<(i32, i32)>,
) -> Option<(i32, i32)> {
    for r in 1..=25i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue;
                }
                let p = (from.0 + dx, from.1 + dy);
                if walkable(env, ds1, p.0, p.1) && !taken.contains(&p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Appends the render-data findings for `ds1`. `prest_row` is the map's
/// LvlPrest row (document row), when it has one.
pub(super) fn render_checks(
    out: &mut Vec<Finding>,
    ds1: &Ds1,
    tables: &Tables,
    prest_row: Option<usize>,
    env: &RenderEnv,
) {
    let prest = tables.prest.as_ref();
    let prest_name = prest
        .zip(prest_row)
        .map(|(p, r)| get(p, r, "Name"))
        .filter(|n| !n.is_empty());
    let level_id = prest
        .zip(prest_row)
        .map_or(0, |(p, r)| num(p, r, "LevelId"));
    let level = tables
        .levels
        .as_ref()
        .and_then(|l| level_row(l, level_id).map(|(r, row)| (l, r, row)));
    let has_waypoint = ds1.objects.iter().any(|o| (env.is_waypoint)(o.kind, o.id));

    // --- Arrival by portal -------------------------------------------------------
    if let Some(p) = arrival_problem(ds1, env.is_waypoint) {
        let crop = p.cropped.map_or(String::new(), |(w, h)| {
            format!(" Cropping the map to what is painted would make it {w}x{h}.")
        });
        let mut f = Finding::new(
            Severity::Error,
            Area::Level,
            "Players arriving by portal (a map item) land in the empty middle of the map, and the game stops",
        )
        .detail(format!(
            "The room at the level's centre ({}, {}) has no floor. A waypoint fixes it without resizing: the game puts arrivals at the waypoint first, wherever it is.{crop}",
            p.room.0, p.room.1
        ));
        f.cells.push((p.room.0 + 4, p.room.1 + 4));
        out.push(f);
    } else if !has_waypoint
        && !ds1.walls.iter().enumerate().any(|(li, l)| {
            (0..ds1.height).any(|y| {
                (0..ds1.width).any(|x| {
                    matches!(ds1.orientation(li, x, y), Some(10 | 11))
                        && l.tiles.get(x, y).is_some_and(|c| c.main_index() <= 7)
                })
            })
        })
    {
        out.push(
            Finding::new(
                Severity::Warning,
                Area::Level,
                "No waypoint: players arriving by portal (a map item) are put at the centre of the level",
            )
            .detail("With no waypoint and no warp tile, the game puts players who arrive through a map item's portal in the room at the level's centre (it has floor, so this works). Place a waypoint where they should arrive: the game uses it first, so the map's tiles can be anywhere."),
        );
    }

    // --- Warp links --------------------------------------------------------------
    if let Some((levels, lrow, row)) = &level {
        let presets = prest.map_or(0, |p| {
            data_rows(p)
                .into_iter()
                .filter(|&r| num(p, r, "LevelId") == level_id)
                .count()
        });
        let names: HashMap<i32, String> = data_rows(levels)
            .into_iter()
            .map(|r| {
                let n = get(levels, r, "LevelName");
                (
                    num(levels, r, "Id") as i32,
                    if n.is_empty() {
                        get(levels, r, "Name")
                    } else {
                        n
                    },
                )
            })
            .collect();
        let level_name = |id: i32| {
            names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("level {id}"))
        };
        let opts = ExitOptions {
            has_waypoint,
            only_preset: presets <= 1,
            is_town: TOWNS.contains(&(level_id as u32)),
            level_name: &level_name,
        };
        let key = Some(get(levels, *lrow, "Name"));
        for p in exit_problems(ds1, row, &opts) {
            let mut f = Finding::new(
                match p.severity {
                    ExitSeverity::Warning => Severity::Warning,
                    ExitSeverity::Info => Severity::Info,
                },
                Area::Map,
                p.title,
            )
            .cols(&[(TableName::Levels, "Vis0")])
            .fix(Fix::open(TableName::Levels, key.clone()));
            f.cells = p.cells;
            out.push(f);
        }
    }

    // --- Roof hiding -------------------------------------------------------------
    let pops = prest.zip(prest_row).map(|(p, r)| num(p, r, "Pops") as i32);
    let pop_pad = prest
        .zip(prest_row)
        .map_or(0, |(p, r)| num(p, r, "PopPad") as i32);
    let areas = find_pops(ds1);
    for p in pop_problems(ds1, &areas, pops, pop_pad) {
        let mut f = Finding::new(
            match p.severity {
                PopSeverity::Error => Severity::Error,
                PopSeverity::Warning => Severity::Warning,
            },
            Area::Map,
            p.text,
        );
        match p.area.and_then(|m| areas.iter().find(|a| a.main == m)) {
            Some(a) => f.cells = a.markers.iter().map(|m| (m.x, m.y)).collect(),
            None => {
                f = f
                    .cols(&[(TableName::LvlPrest, "Pops")])
                    .fix(Fix::open(TableName::LvlPrest, prest_name.clone()));
            }
        }
        out.push(f);
    }

    // --- Open edges --------------------------------------------------------------
    let open = open_void(ds1, env.grid);
    if !open.edge_cells.is_empty() {
        let rooms = preset_rooms(ds1.width, ds1.height);
        let mut by_room: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
        for &(x, y) in &open.edge_cells {
            let key = room_at(&rooms, x, y).map_or_else(
                || "last row/column".to_owned(),
                |r| {
                    format!(
                        "room {},{}-{},{}",
                        r.x0,
                        r.y0,
                        r.x0 + r.w - 1,
                        r.y0 + r.h - 1
                    )
                },
            );
            by_room.entry(key).or_default().push((x, y));
        }
        let list = |c: &[(u32, u32)]| {
            let mut s = c
                .iter()
                .take(12)
                .map(|(x, y)| format!("{x},{y}"))
                .collect::<Vec<_>>()
                .join(" ");
            if c.len() > 12 {
                s.push_str(&format!(" ... (+{})", c.len() - 12));
            }
            s
        };
        let mut f = Finding::new(
            Severity::Warning,
            Area::Map,
            format!(
                "Walkable edge borders void (players can walk off): {} cells in {} room{}",
                open.edge_cells.len(),
                by_room.len(),
                plural(by_room.len(), "", "s")
            ),
        )
        .detail(format!(
            "In game a cell without a floor blocks nothing unless a tile there does (with LvlPrest FillBlanks, the hidden blank floor 30 the game puts there), so players walk off the art into the void. Put blocking tiles or walls along these edges, or a blocking blank tile in the level's DT1s. {}",
            by_room
                .iter()
                .map(|(r, c)| format!("{r}: {} ({})", c.len(), list(c)))
                .collect::<Vec<_>>()
                .join("; ")
        ));
        f.cells = open.edge_cells.clone();
        out.push(f);
    }

    // --- Objects on unwalkable ground, stacked objects ---------------------------
    let mut taken: std::collections::HashSet<(i32, i32)> =
        ds1.objects.iter().map(|o| (o.x, o.y)).collect();
    let (w, h) = (ds1.width as i32, ds1.height as i32);
    let mut blocked: Vec<(u32, u32)> = Vec::new();
    let mut blocked_moves: Vec<(usize, i32, i32)> = Vec::new();
    let mut spots: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    let mut any_off = false;
    for (i, o) in ds1.objects.iter().enumerate() {
        let (cx, cy) = (o.x.div_euclid(5), o.y.div_euclid(5));
        if cx < 0 || cy < 0 || cx >= w || cy >= h {
            any_off = true;
            continue;
        }
        if o.kind == 1 && !walkable(env, ds1, o.x, o.y) {
            blocked.push((cx as u32, cy as u32));
            if let Some(to) = nearest_free(env, ds1, (o.x, o.y), &taken) {
                taken.insert(to);
                blocked_moves.push((i, to.0, to.1));
            }
        }
        spots.entry((o.x, o.y)).or_default().push(i);
    }
    if !blocked.is_empty() {
        let mut f = Finding::new(
            Severity::Warning,
            Area::Objects,
            format!("{} NPCs/monsters stand on unwalkable ground", blocked.len()),
        )
        .detail("They may be stuck or fail to spawn.");
        f.cells = blocked.clone();
        if !blocked_moves.is_empty() {
            f = f.fix(Fix::edits(
                format!(
                    "Move {} to the nearest walkable spot",
                    if blocked_moves.len() == blocked.len() {
                        "them".to_owned()
                    } else {
                        format!("{} of them", blocked_moves.len())
                    }
                ),
                vec![Edit::MoveObjects {
                    moves: blocked_moves,
                }],
            ));
        }
        out.push(f);
    }
    let stacked: Vec<&Vec<usize>> = spots.values().filter(|l| l.len() > 1).collect();
    if !stacked.is_empty() {
        let mut moves = Vec::new();
        for list in &stacked {
            for &i in list.iter().skip(1) {
                let o = &ds1.objects[i];
                if let Some(to) = nearest_free(env, ds1, (o.x, o.y), &taken) {
                    taken.insert(to);
                    moves.push((i, to.0, to.1));
                }
            }
        }
        let mut f = Finding::new(
            Severity::Warning,
            Area::Objects,
            format!(
                "{} spots with several objects on the same sub-tile",
                stacked.len()
            ),
        )
        .detail("NPC paths attached to such spots are dropped by the game and by WinDS1.");
        f.cells = stacked
            .iter()
            .map(|l| {
                let o = &ds1.objects[l[0]];
                (o.x.div_euclid(5) as u32, o.y.div_euclid(5) as u32)
            })
            .collect();
        if !moves.is_empty() {
            f = f.fix(Fix::edits(
                "Spread them onto free neighbouring spots",
                vec![Edit::MoveObjects { moves }],
            ));
        }
        out.push(f);
    }
    if blocked.is_empty() && stacked.is_empty() && !any_off {
        out.push(Finding::new(
            Severity::Ok,
            Area::Objects,
            format!("{} objects placed on valid ground", ds1.objects.len()),
        ));
    }

    // --- Spawn rooms -------------------------------------------------------------
    let partly = partly_no_spawn_rooms(ds1);
    if !partly.is_empty() {
        let regions: usize = partly.iter().map(|(_, o)| o.len()).sum();
        let logicals_off = prest
            .zip(prest_row)
            .is_some_and(|(p, r)| num(p, r, "Logicals") != 1);
        let mut f = Finding::new(
            Severity::Warning,
            Area::Map,
            format!(
                "{} room{} only partly no-spawn: monsters can still spawn in {regions} region{}",
                partly.len(),
                plural(partly.len(), " is", "s are"),
                plural(regions, "", "s")
            ),
        )
        .detail(format!(
            "{}{}. Each 8x8 room is split into regions by the first wall layer; a region gets no random monsters only when its first cell (seed) has a hidden floor1 tile. Some regions of these rooms have one, these don't: give every region seed of the room a hidden floor1 tile to make the whole room no-spawn.{}",
            partly
                .iter()
                .take(12)
                .map(|(rr, open)| format!(
                    "room {},{}: seeds {}",
                    rr.room.x0,
                    rr.room.y0,
                    open.iter()
                        .map(|&i| format!("{},{}", rr.regions[i].seed.0, rr.regions[i].seed.1))
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
                .collect::<Vec<_>>()
                .join("; "),
            if partly.len() > 12 { "; ..." } else { "" },
            if logicals_off {
                " Also, the map's LvlPrest row has Logicals 0, so the game doesn't use regions at all: set Logicals to 1."
            } else {
                ""
            }
        ));
        f.cells = partly
            .iter()
            .flat_map(|(rr, open)| open.iter().map(|&i| rr.regions[i].seed))
            .collect();
        if logicals_off {
            f = f
                .cols(&[(TableName::LvlPrest, "Logicals")])
                .fix(Fix::open(TableName::LvlPrest, prest_name.clone()));
        }
        out.push(f);
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
    use capy_render::walk::SubTileGrid;
    use d2_formats::ds1::Object;

    fn ds1_with(objects: Vec<Object>) -> Ds1 {
        let mut d = Ds1::new(4, 4, 0);
        d.objects = objects;
        d
    }

    fn obj(kind: i32, x: i32, y: i32) -> Object {
        Object {
            kind,
            id: 1,
            x,
            y,
            flags: 0,
        }
    }

    #[test]
    fn blocked_and_stacked_objects_get_idempotent_move_fixes() {
        let mut ds1 = ds1_with(vec![obj(1, 2, 2), obj(2, 8, 8), obj(2, 8, 8)]);
        let mut grid = SubTileGrid {
            width: 4,
            height: 4,
            flags: vec![0; 4 * 4 * 25],
        };
        grid.flags[(2 * 5 + 2) as usize] = BLOCK_WALK;
        let no_wp = |_: i32, _: i32| false;
        let env = RenderEnv {
            grid: &grid,
            is_waypoint: &no_wp,
        };
        let mut out = Vec::new();
        render_checks(&mut out, &ds1, &Tables::default(), None, &env);
        let blocked = out
            .iter()
            .find(|f| f.title.contains("unwalkable ground"))
            .expect("a blocked NPC");
        let stacked = out
            .iter()
            .find(|f| f.title.contains("same sub-tile"))
            .expect("a stacked spot");
        for f in [blocked, stacked] {
            let super::super::FixAction::Edits(edits) = &f.fixes[0].action else {
                panic!("edits");
            };
            assert!(edits[0].apply_ds1(&mut ds1));
            assert!(!edits[0].apply_ds1(&mut ds1), "second time changes nothing");
        }
        let mut again = Vec::new();
        render_checks(&mut again, &ds1, &Tables::default(), None, &env);
        assert!(
            again
                .iter()
                .all(|f| !f.title.contains("unwalkable") && !f.title.contains("same sub-tile")),
            "{again:?}"
        );
    }
}
