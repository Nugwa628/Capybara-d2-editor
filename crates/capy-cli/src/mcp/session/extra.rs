//! The spawn, tile-list, placeable-list, layer, clear and history tools
//! (ds1-studio's `regions`, `no_spawn_area`, `list_tiles`, `list_placeable`,
//! `add_layer`, `clear` and `history`).

use std::collections::BTreeSet;

use capy_edit::no_spawn::{NoSpawnOptions, no_spawn_plan};
use capy_edit::{CellRect, LayerRef, SelectOp, Selection};
use capy_render::rooms::{
    LogicRegion, PresetRoom, RoomRegions, preset_rooms, room_at, room_regions, spawns,
};
use d2_formats::ds1::Ds1;
use serde_json::Value;

use super::{Session, ToolResult, err_text, opt_int, parse_cells, parse_rect, text};

/// DT1 orientation names (DS1Edit's list).
fn orientation_name(o: u32) -> &'static str {
    match o {
        0 => "floor",
        1 => "left wall",
        2 => "right wall",
        3 => "right part of north corner wall",
        4 => "left part of north corner wall",
        5 => "left end wall",
        6 => "right end wall",
        7 => "south corner wall",
        8 => "left wall with door",
        9 => "right wall with door",
        10 => "special tile 1",
        11 => "special tile 2",
        12 => "pillar / column / standalone",
        13 => "shadow",
        14 => "tree",
        15 => "roof",
        16 => "lower left wall",
        17 => "lower right wall",
        18 => "lower right north corner wall",
        19 => "lower left north corner wall",
        _ => "unknown",
    }
}

/// What a special tile (orientation 10 or 11) means.
fn special_tile_label(main: u8, sub: u8) -> String {
    match (main, sub) {
        (30, 11) => "Map entry".to_owned(),
        (30, 0) => "Arrival spot (player start)".to_owned(),
        (31..=33, 0) => format!("Arrival spot {}", main - 29),
        (0..=7, _) => format!("Warp, Vis {main}"),
        (8..=29, _) => format!("Hide area {main}, hides #{sub}"),
        _ => format!("Special {main}/{sub}"),
    }
}

fn room_text(r: &PresetRoom) -> String {
    format!("{},{}-{},{}", r.x0, r.y0, r.x0 + r.w - 1, r.y0 + r.h - 1)
}

fn cells_text(cells: &[(u32, u32)]) -> String {
    let mut s = cells
        .iter()
        .take(12)
        .map(|(x, y)| format!("{x},{y}"))
        .collect::<Vec<_>>()
        .join(" ");
    if cells.len() > 12 {
        s.push_str(&format!(" ... (+{})", cells.len() - 12));
    }
    s
}

fn region_state(rr: &RoomRegions, g: &LogicRegion) -> &'static str {
    if g.node {
        "node (no spawns)"
    } else if rr.warp {
        "warp room"
    } else if g.floored == 0 {
        "no floor"
    } else {
        "SPAWNS"
    }
}

/// Cells from `cells`, `rect` and/or `regions` (each cell standing for the
/// whole spawn region it is in), without repeats.
fn pick_cells(args: &Value, ds1: &Ds1) -> Result<Vec<(u32, u32)>, String> {
    let mut out: BTreeSet<(u32, u32)> = BTreeSet::new();
    let inside = |(x, y): (u32, u32)| {
        if x < ds1.width && y < ds1.height {
            Ok((x, y))
        } else {
            Err(format!(
                "({x}, {y}) is outside the {}x{} map.",
                ds1.width, ds1.height
            ))
        }
    };
    for c in parse_cells(args.get("cells"))? {
        out.insert(inside(c)?);
    }
    if let Some(r) = args.get("rect").filter(|r| !r.is_null()) {
        let r = parse_rect(Some(r), ds1)?;
        for y in r.y0..=r.y1 {
            for x in r.x0..=r.x1 {
                out.insert((x, y));
            }
        }
    }
    let regions = parse_cells(args.get("regions"))?;
    if !regions.is_empty() {
        let rooms = preset_rooms(ds1.width, ds1.height);
        for c in regions {
            let (x, y) = inside(c)?;
            let room = room_at(&rooms, x, y).ok_or_else(|| {
                format!("{x},{y} is in no game room (the last row and column of a map only border rooms).")
            })?;
            if let Some(reg) = room_regions(ds1, *room)
                .regions
                .iter()
                .find(|r| r.cells.contains(&(x, y)))
            {
                out.extend(reg.cells.iter().copied());
            }
        }
    }
    if out.is_empty() {
        return Err("Give cells, rect or regions.".into());
    }
    Ok(out.into_iter().collect())
}

impl Session {
    pub(super) fn regions(&mut self, args: &Value) -> ToolResult {
        let spawning_only = args
            .get("spawning_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let ds1 = self.open_mut()?.doc.ds1().clone();
        let rect = match args.get("rect").filter(|r| !r.is_null()) {
            Some(r) => Some(parse_rect(Some(r), &ds1)?),
            None => None,
        };
        let rooms: Vec<PresetRoom> = preset_rooms(ds1.width, ds1.height)
            .into_iter()
            .filter(|m| {
                rect.is_none_or(|r| {
                    m.x0 <= r.x1 && m.x0 + m.w > r.x0 && m.y0 <= r.y1 && m.y0 + m.h > r.y0
                })
            })
            .collect();
        let mut lines = Vec::new();
        let mut open = 0;
        for room in &rooms {
            let rr = room_regions(&ds1, *room);
            open += rr.regions.iter().filter(|g| spawns(&rr, g)).count();
            let shown: Vec<&LogicRegion> = rr
                .regions
                .iter()
                .filter(|g| !spawning_only || spawns(&rr, g))
                .collect();
            if shown.is_empty() {
                continue;
            }
            lines.push(format!(
                "Room {}{}:",
                room_text(room),
                if rr.warp {
                    " (level warp: nothing spawns)"
                } else {
                    ""
                }
            ));
            for g in shown {
                lines.push(format!(
                    "  seed {},{} - {} - {} cells, {} floored - {}",
                    g.seed.0,
                    g.seed.1,
                    region_state(&rr, g),
                    g.cells.len(),
                    g.floored,
                    cells_text(&g.cells)
                ));
            }
        }
        const MAX: usize = 600;
        let mut out = vec![format!(
            "{} room{}, {open} region{} monsters can spawn in.",
            rooms.len(),
            if rooms.len() == 1 { "" } else { "s" },
            if open == 1 { "" } else { "s" }
        )];
        out.extend(lines.iter().take(MAX).cloned());
        if lines.len() > MAX {
            out.push(format!(
                "... {} more lines: give a rect.",
                lines.len() - MAX
            ));
        }
        text(out.join("\n"))
    }

    pub(super) fn no_spawn_area(&mut self, args: &Value) -> ToolResult {
        let whole_rooms = args
            .get("whole_rooms")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let o = self.open_mut()?;
        let cells = pick_cells(args, o.doc.ds1())?;
        let plan = no_spawn_plan(
            o.doc.ds1(),
            &cells,
            NoSpawnOptions {
                whole_rooms,
                ..NoSpawnOptions::default()
            },
        );
        if plan.rooms.is_empty() {
            return Err("Those cells are in no game room (the last row and column of a map only border rooms).".into());
        }
        o.doc.apply_no_spawn(&plan).map_err(err_text)?;
        let ds1 = o.doc.ds1();
        let still: Vec<String> = plan
            .rooms
            .iter()
            .flat_map(|room| {
                let rr = room_regions(ds1, *room);
                rr.regions
                    .iter()
                    .filter(|g| spawns(&rr, g))
                    .map(|g| format!("{},{}", g.seed.0, g.seed.1))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut out = vec![
            format!(
                "{} room{} covered (cells {}).",
                plan.rooms.len(),
                if plan.rooms.len() == 1 { "" } else { "s" },
                plan.rooms
                    .iter()
                    .map(room_text)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            format!(
                "{} seed floor{} hidden under a floor2 copy, {} void seed{} given a hidden unwalkable floor.",
                plan.copied,
                if plan.copied == 1 { "" } else { "s" },
                plan.voids,
                if plan.voids == 1 { "" } else { "s" }
            ),
        ];
        if !plan.skipped.is_empty() {
            out.push(format!("Not changed: {}.", plan.skipped.join("; ")));
        }
        out.push(if still.is_empty() {
            "Every region of those rooms is now a node: no random monsters there.".to_owned()
        } else {
            format!(
                "Regions that can still spawn monsters (seeds): {}.",
                still.join(" ")
            )
        });
        out.push(
            "Nodes only work when the map's LvlPrest row has Logicals 1. (undo with the undo tool)"
                .to_owned(),
        );
        text(out.join("\n"))
    }

    pub(super) fn list_tiles(&mut self, args: &Value) -> ToolResult {
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "floor" | "wall" | "shadow") {
            return Err("kind must be \"floor\", \"wall\" or \"shadow\".".into());
        }
        let dt1 = args
            .get("dt1")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let orientation = opt_int(args.get("orientation"), "orientation", 0, 31)?;
        let limit = opt_int(args.get("limit"), "limit", 1, 100_000)?.unwrap_or(400) as usize;
        self.ensure_tiles()?;
        let tiles = self
            .open
            .as_ref()
            .and_then(|o| o.tiles.as_ref())
            .ok_or("no tile set")?;
        let fits = |o: u8| match kind {
            "floor" => o == 0,
            "shadow" => o == 13,
            _ => o != 0 && o != 13 && o != 4,
        };
        let mut rows: Vec<(u32, u8, u8, usize, Vec<String>)> = Vec::new();
        for (key, refs) in tiles.keys() {
            if !fits(key.orientation) || orientation.is_some_and(|o| o as u8 != key.orientation) {
                continue;
            }
            let mut srcs: Vec<String> = refs
                .iter()
                .filter_map(|r| tiles.dt1_paths().get(r.dt1))
                .map(|p| {
                    p.as_str()
                        .replace('\\', "/")
                        .trim_start_matches("data/global/tiles/")
                        .to_owned()
                })
                .collect();
            srcs.sort();
            srcs.dedup();
            if !dt1.is_empty() && !srcs.iter().any(|s| s.to_ascii_lowercase().contains(&dt1)) {
                continue;
            }
            rows.push((
                u32::from(key.orientation),
                key.main,
                key.sub,
                refs.len(),
                srcs,
            ));
        }
        rows.sort();
        let lines: Vec<String> = rows
            .iter()
            .take(limit)
            .map(|(o, main, sub, n, srcs)| {
                format!(
                    "{main}/{sub}{}{} - {}{}",
                    if kind == "wall" {
                        format!(" orientation {o} ({})", orientation_name(*o))
                    } else {
                        String::new()
                    },
                    if *n > 1 {
                        format!(" - {n} variants")
                    } else {
                        String::new()
                    },
                    srcs.join(", "),
                    if matches!(o, 10 | 11) {
                        format!(" - {}", special_tile_label(*main, *sub))
                    } else {
                        String::new()
                    }
                )
            })
            .collect();
        text(format!(
            "{} tile{}{}:\n{}",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" },
            if rows.len() > limit {
                format!(" (first {limit})")
            } else {
                String::new()
            },
            lines.join("\n")
        ))
    }

    pub(super) fn list_placeable(&mut self, args: &Value) -> ToolResult {
        let filter = args
            .get("filter")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = opt_int(args.get("type"), "type", 1, 2)?;
        let own = self.open_mut()?.doc.ds1().effective_act();
        let from = match opt_int(args.get("act"), "act", 1, 5)? {
            Some(a) => (a - 1) as u8,
            None => own,
        };
        self.load_object_catalog();
        let catalog = self
            .objects
            .as_ref()
            .and_then(Option::as_ref)
            .ok_or("The object tables could not be read for this profile.")?;
        // The id that reaches another act's object from this map: below 150 the
        // map's act table runs back into earlier acts'; 150 and up is an
        // objects.txt row.
        let mut lines = Vec::new();
        for info in catalog.placeable(from) {
            if (from != own && info.ds1_kind != 2)
                || kind.is_some_and(|k| k != i64::from(info.ds1_kind))
                || !(filter.is_empty() || info.name.to_ascii_lowercase().contains(&filter))
            {
                continue;
            }
            let id = if from == own {
                info.ds1_id
            } else if from < own {
                info.ds1_id + (i32::from(from) - i32::from(own)) * 150
            } else if let Some(row) = info.row {
                150 + row as i32
            } else {
                continue;
            };
            lines.push(format!("type {} id {id}: {}", info.ds1_kind, info.name));
        }
        let head = if from == own {
            String::new()
        } else {
            format!(
                "Act {} objects, with the ids that place them in this act {} map:\n",
                from + 1,
                own + 1
            )
        };
        text(format!(
            "{head}{}",
            if lines.is_empty() {
                "None found.".to_owned()
            } else {
                lines.join("\n")
            }
        ))
    }

    pub(super) fn add_layer(&mut self, args: &Value) -> ToolResult {
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "floor" | "wall") {
            return Err("kind must be \"floor\" or \"wall\".".into());
        }
        let n = opt_int(args.get("count"), "count", 1, 4)?.unwrap_or(1) as usize;
        self.edit("add_layer", move |doc| {
            let have = if kind == "floor" {
                doc.ds1().floors.len()
            } else {
                doc.ds1().walls.len()
            };
            let max = if kind == "floor" { 2 } else { 4 };
            if have + n > max {
                return Err(format!(
                    "The map has {have} {kind} layer{}; a DS1 holds at most {max}.",
                    if have == 1 { "" } else { "s" }
                ));
            }
            let layer = if kind == "floor" {
                LayerRef::Floor((have + n - 1) as u8)
            } else {
                LayerRef::Wall((have + n - 1) as u8)
            };
            doc.add_layer(layer).map_err(err_text)?;
            Ok(format!(
                "added {n} {kind} layer{}; layers: {}",
                if n == 1 { "" } else { "s" },
                LayerRef::all_in(doc.ds1())
                    .into_iter()
                    .map(super::layer_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
    }

    pub(super) fn clear(&mut self, args: &Value) -> ToolResult {
        let objects = args
            .get("objects")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let names: Option<Vec<LayerRef>> = match args.get("layers").and_then(Value::as_array) {
            Some(a) => Some(
                a.iter()
                    .map(|v| super::parse_layer(Some(v)))
                    .collect::<Result<_, _>>()?,
            ),
            None => None,
        };
        let rect_arg = args.get("rect").cloned();
        self.edit("clear", move |doc| {
            let rect: CellRect = parse_rect(rect_arg.as_ref(), doc.ds1())?;
            let layers = names.clone().unwrap_or_else(|| LayerRef::all_in(doc.ds1()));
            let mut sel = Selection::new();
            sel.box_select(doc.ds1(), rect, &layers, false, SelectOp::Replace);
            let cells = sel.len();
            let gone: Vec<usize> = if objects {
                doc.ds1()
                    .objects
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| {
                        let (cx, cy) = (o.x.div_euclid(5), o.y.div_euclid(5));
                        cx >= rect.x0 as i32
                            && cx <= rect.x1 as i32
                            && cy >= rect.y0 as i32
                            && cy <= rect.y1 as i32
                    })
                    .map(|(i, _)| i)
                    .collect()
            } else {
                Vec::new()
            };
            doc.begin_group("Clear area");
            let run = (|| {
                doc.erase(&sel)?;
                if !gone.is_empty() {
                    doc.delete_objects(&gone)?;
                }
                Ok::<(), capy_edit::EditError>(())
            })();
            doc.end_group();
            run.map_err(err_text)?;
            Ok(format!(
                "{cells} tile cell(s) cleared in {},{}-{},{}{}",
                rect.x0,
                rect.y0,
                rect.x1,
                rect.y1,
                if objects {
                    format!(", {} object(s) removed", gone.len())
                } else {
                    String::new()
                }
            ))
        })
    }

    pub(super) fn history(&mut self) -> ToolResult {
        let o = self.open_mut()?;
        let (done, undone) = o.doc.history();
        let mut lines: Vec<String> = done
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {s}", i + 1))
            .collect();
        lines.extend(undone.iter().map(|s| format!("(undone) {s}")));
        text(if lines.is_empty() {
            "No edits yet.".to_owned()
        } else {
            lines.join("\n")
        })
    }
}
