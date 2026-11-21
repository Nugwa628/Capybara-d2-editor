//! `CAPY_SCRIPT` support for the tool windows: `tool <verb> args...`.
//!
//! ```text
//! tool cmd <command id>                         # run a registry command (menu items)
//! tool dt1-open data\global\tiles\x.dt1
//! tool dt1-recolor <hue degrees> [all]
//! tool dt1-save
//! tool dt1-copy <new name>                      # save a copy next to it, open the copy
//! tool dt1-flags add|remove|set|walkable <hex> <cell,cell|all>
//! tool dt1-actsafe [all]
//! tool dt1-assert-dirty true|false
//! tool table-open levels.txt
//! tool table-set <row> <column> <value...>
//! tool table-save
//! tool table-assert <row> <column> <value...>
//! tool add-to-game new|existing <level id> <map path under tiles> <name...>
//! tool change-type <type id> [copy]
//! tool addmap <tile key>...                     # open "Add to this map" for these Tiles keys
//! tool addmap-confirm                           # press Add in that dialog
//! tool addmap-assert-loaded <game path> true|false  # in the map's DT1 list (catalog)
//! tool newtile <kind> [o=<n>] [h=<px>] [roof=<px>] [solid=<i>|image=<file>|blank]
//!              [place=as-is|center|fit] [dither=fs|none] [act=1-5] [into=map|lib]
//!                                               # kind: floor roof shadow wall corner lower
//! tool newtile-flags add|remove|set|walkable <hex> <cell,cell|all>
//! tool newtile-assert-warning <text...>|none    # the preview's warnings
//! tool newtile-add                              # Add to this map (opens the preview) or to the library
//! tool newtile-edit                             # Open in editor
//! tool editor-add-new                           # the editor's Add for new tiles
//! tool editor-shape <kind> [o=] [h=] [roof=]    # Shape... on the editor's current tile
//! tool dt1-new <folder> <name> [register]       # New empty DT1 (opened)
//! tool dt1-newtile                              # New tile into this DT1
//! tool dt1-shape <tile> <kind> [o=] [h=] [roof=]  # Shape of the selected tile
//! tool dt1-undo | dt1-redo
//! tool dt1-assert-tiles <n>
//! tool wait-catalog
//! tool custom-object <row> <frames> <act> <png> <name...>
//! tool preset-export <zip> [preset name]        # all presets without a name
//! tool preset-import <zip or json>
//! tool preset-copy-tiles <preset name>          # into the open map's own DT1
//! tool compat [errors <n>]                      # run the checker; optionally assert the error count
//! tool package-export <zip>
//! tool package-import <zip> [overwrite]
//! tool preset-save <name> [category]            # the selection
//! tool assert-saved <game path>                 # in the saves and in CHANGED.txt
//! tool assert-map-listed <game path>            # in the Maps list
//! ```

use capy_history::SavesDir;
use capy_mod::tiles_flags::{FlagOp, edit_subtile_flags};
use d2_assets::GamePath;

use super::{diag, dt1, game, newtile, package, presets, tables};
use crate::state::{AppState, CatalogState};

/// Outcome of one script line: done, or call again next frame.
pub enum Outcome {
    Done,
    Retry,
}

fn bad(usage: &str) -> String {
    format!("expected {usage}")
}

/// `add|remove|set|walkable <hex>` of a flags command.
fn flag_op_arg(args: &[&str]) -> Result<FlagOp, String> {
    let op = args
        .first()
        .copied()
        .ok_or_else(|| bad("add|remove|set|walkable <hex> <cells>"))?;
    let bits = u8::from_str_radix(args.get(1).copied().unwrap_or("0"), 16)
        .map_err(|_| "flag bits are hexadecimal".to_owned())?;
    Ok(match op {
        "add" => FlagOp::Add(bits),
        "remove" => FlagOp::Remove(bits),
        "set" => FlagOp::Set(bits),
        "walkable" => FlagOp::MakeWalkable,
        other => return Err(format!("unknown flag operation {other:?}")),
    })
}

/// `all` or `3,4,5`.
fn cells_arg(arg: Option<&str>) -> Result<Vec<usize>, String> {
    match arg {
        Some("all") | None => Ok((0..25).collect()),
        Some(list) => list
            .split(',')
            .map(|c| c.parse().map_err(|_| format!("{c:?} is not a cell number")))
            .collect(),
    }
}

pub fn run(state: &mut AppState, line: &str) -> Result<Outcome, String> {
    let (verb, rest) = line
        .trim()
        .split_once(char::is_whitespace)
        .map_or((line.trim(), ""), |(v, r)| (v, r.trim()));
    let args: Vec<&str> = rest.split_whitespace().collect();
    match verb {
        "cmd" => {
            let id = args.first().ok_or_else(|| bad("<command id>"))?;
            if crate::ui::commands_run(state, id) {
                Ok(Outcome::Done)
            } else {
                Err(format!("command {id} is unknown or not enabled"))
            }
        }
        "wait-catalog" => Ok(if matches!(state.catalog, CatalogState::Building) {
            Outcome::Retry
        } else {
            Outcome::Done
        }),
        "addmap" => {
            if args.is_empty() {
                return Err(bad("<tile key> [tile key...]"));
            }
            super::addmap::script_open(state, args.iter().map(|s| (*s).to_owned()).collect())?;
            Ok(Outcome::Done)
        }
        "addmap-assert-loaded" => {
            let want = args.last() == Some(&"true");
            let path = args[..args.len().saturating_sub(1)].join(" ");
            let got = state
                .map_dt1s()
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&path));
            if got == want {
                Ok(Outcome::Done)
            } else {
                Err(format!(
                    "{path} is {}loaded by the map",
                    if got { "" } else { "not " }
                ))
            }
        }
        "addmap-confirm" => {
            let msg = super::addmap::script_confirm(state)?;
            state.info(format!("addmap: {msg}"));
            Ok(Outcome::Done)
        }
        "dt1-open" => {
            dt1::open_lib(state, rest)?;
            Ok(Outcome::Done)
        }
        "newtile" => {
            newtile::script_set(state, &args)?;
            Ok(Outcome::Done)
        }
        "newtile-flags" => {
            let op = flag_op_arg(&args)?;
            let cells = cells_arg(args.get(2).copied())?;
            newtile::script_flags(state, op, &cells);
            Ok(Outcome::Done)
        }
        "newtile-assert-warning" => {
            let got = newtile::script_warnings(state)?;
            let want = rest.trim();
            let ok = if want == "none" {
                got.is_empty()
            } else {
                got.iter().any(|w| w.contains(want))
            };
            if ok {
                Ok(Outcome::Done)
            } else {
                Err(format!("the preview warns {got:?}, expected {want:?}"))
            }
        }
        "newtile-add" => {
            match state.tools.newtile.target {
                crate::sprites::dt1edit::NewTilesTarget::Map => newtile::add_to_map(state)?,
                crate::sprites::dt1edit::NewTilesTarget::Library(_) => {
                    let msg = newtile::add_to_library(state)?;
                    state.info(format!("newtile: {msg}"));
                }
            }
            Ok(Outcome::Done)
        }
        "newtile-edit" => {
            newtile::open_in_editor(state)?;
            Ok(Outcome::Done)
        }
        "editor-add-new" => {
            let msg = newtile::add_from_editor(state)?;
            state.info(format!("editor-add-new: {msg}"));
            Ok(Outcome::Done)
        }
        "editor-shape" => {
            let (c, left) = newtile::parse_shape(&args)?;
            if let Some(t) = left.first() {
                return Err(format!("unknown shape option {t:?}"));
            }
            let shape = c.shape().ok_or("one tile's shape")?;
            let e = state
                .sprites
                .editor
                .as_mut()
                .ok_or("nothing is open in the editor")?;
            let frame = e.cur.frame;
            let n = e.dt1_reshape(frame, shape)?;
            state.info(format!("editor-shape: tile {frame}, {n} pixel(s) left out"));
            Ok(Outcome::Done)
        }
        "dt1-new" => {
            let (folder, name) = match args.as_slice() {
                [f, n, ..] => (*f, *n),
                _ => return Err(bad("<folder> <name> [register]")),
            };
            let folder = if folder == "-" { "" } else { folder };
            let msg = dt1::create_empty(state, folder, name, args.get(2) == Some(&"register"))?;
            state.info(format!("dt1-new: {msg}"));
            Ok(Outcome::Done)
        }
        "dt1-newtile" => {
            newtile::open_for_library(state)?;
            Ok(Outcome::Done)
        }
        "dt1-shape" => {
            let (tile, rest_args) = args
                .split_first()
                .ok_or_else(|| bad("<tile> <kind> [o=] [h=] [roof=]"))?;
            let tile: usize = tile.parse().map_err(|_| "tile is a number".to_owned())?;
            let (c, left) = newtile::parse_shape(rest_args)?;
            if let Some(t) = left.first() {
                return Err(format!("unknown shape option {t:?}"));
            }
            let shape = c.shape().ok_or("one tile's shape")?;
            let lib = state.tools.dt1.lib.as_mut().ok_or("No library is open")?;
            if tile >= lib.dt1.tiles.len() {
                return Err(format!("the library has no tile {tile}"));
            }
            lib.tile = tile;
            let n = dt1::reshape_selected(state, shape)?;
            state.info(format!("dt1-shape: tile {tile}, {n} pixel(s) left out"));
            Ok(Outcome::Done)
        }
        "dt1-undo" | "dt1-redo" => {
            let lib = state.tools.dt1.lib.as_mut().ok_or("No library is open")?;
            let done = if verb == "dt1-undo" {
                lib.undo()
            } else {
                lib.redo()
            };
            if done {
                Ok(Outcome::Done)
            } else {
                Err(format!("{verb}: nothing to do"))
            }
        }
        "dt1-assert-tiles" => {
            let want: usize = args
                .first()
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| bad("<n>"))?;
            let got = state
                .tools
                .dt1
                .lib
                .as_ref()
                .map_or(0, |l| l.dt1.tiles.len());
            if got == want {
                Ok(Outcome::Done)
            } else {
                Err(format!("the library has {got} tile(s), expected {want}"))
            }
        }
        "dt1-recolor" => {
            let hue: f32 = args
                .first()
                .and_then(|h| h.parse().ok())
                .ok_or_else(|| bad("<hue degrees> [all]"))?;
            state.tools.dt1.hue = hue;
            let all = args.get(1) == Some(&"all");
            if !all && let Some(l) = state.tools.dt1.lib.as_mut() {
                l.picked.clear();
            }
            dt1::recolor(state, all)?;
            Ok(Outcome::Done)
        }
        "dt1-save" => {
            dt1::save_lib(state)?;
            Ok(Outcome::Done)
        }
        "dt1-copy" => {
            dt1::save_copy(state, rest)?;
            Ok(Outcome::Done)
        }
        "dt1-flags" => {
            let op = args
                .first()
                .copied()
                .ok_or_else(|| bad("add|remove|set|walkable <hex> <cells>"))?;
            let bits = u8::from_str_radix(args.get(1).copied().unwrap_or("0"), 16)
                .map_err(|_| "flag bits are hexadecimal".to_owned())?;
            let cells: Vec<usize> = match args.get(2).copied() {
                Some("all") | None => (0..25).collect(),
                Some(list) => list
                    .split(',')
                    .map(|c| c.parse().map_err(|_| format!("{c:?} is not a cell number")))
                    .collect::<Result<_, _>>()?,
            };
            let op = match op {
                "add" => FlagOp::Add(bits),
                "remove" => FlagOp::Remove(bits),
                "set" => FlagOp::Set(bits),
                "walkable" => FlagOp::MakeWalkable,
                other => return Err(format!("unknown flag operation {other:?}")),
            };
            let lib = state.tools.dt1.lib.as_mut().ok_or("No library is open")?;
            let tiles = lib.targets();
            lib.edit(|d| edit_subtile_flags(d, &tiles, &cells, op).map_err(|e| e.to_string()))?;
            Ok(Outcome::Done)
        }
        "dt1-actsafe" => {
            let msg = dt1::act_safe_apply(state, args.first() == Some(&"all"))?;
            state.info(format!("dt1-actsafe: {msg}"));
            Ok(Outcome::Done)
        }
        "dt1-assert-dirty" => {
            let want = args.first() == Some(&"true");
            let got = state.tools.dt1.lib.as_ref().is_some_and(|l| l.dirty);
            if got == want {
                Ok(Outcome::Done)
            } else {
                Err(format!(
                    "the library is {}dirty",
                    if got { "" } else { "not " }
                ))
            }
        }
        "table-open" => {
            tables::open_table(state, rest)?;
            Ok(Outcome::Done)
        }
        "table-set" => {
            let (row, rest2) = rest
                .split_once(char::is_whitespace)
                .ok_or_else(|| bad("<row> <column> <value>"))?;
            let rest2 = rest2.trim();
            let (col, value) = rest2.split_once(char::is_whitespace).unwrap_or((rest2, ""));
            let row: usize = row.parse().map_err(|_| "row is a number".to_owned())?;
            state.tools.tables.set_cell(row, col, value.trim())?;
            Ok(Outcome::Done)
        }
        "table-save" => {
            tables::save_table(state)?;
            Ok(Outcome::Done)
        }
        "table-assert" => {
            let (row, rest2) = rest
                .split_once(char::is_whitespace)
                .ok_or_else(|| bad("<row> <column> <value>"))?;
            let rest2 = rest2.trim();
            let (col, value) = rest2.split_once(char::is_whitespace).unwrap_or((rest2, ""));
            let row: usize = row.parse().map_err(|_| "row is a number".to_owned())?;
            let doc = state.tools.tables.doc.as_ref().ok_or("No table is open")?;
            let c = doc.column(col).ok_or_else(|| format!("no column {col}"))?;
            let got = doc.cell(row, c).unwrap_or_default();
            if got == value.trim() {
                Ok(Outcome::Done)
            } else {
                Err(format!("cell is {got:?}, expected {:?}", value.trim()))
            }
        }
        "add-to-game" => {
            if args.len() < 4 {
                return Err(bad("new|existing <level id> <map path> <name...>"));
            }
            let form = game::AddForm {
                new_level: args[0] == "new",
                level_id: args[1]
                    .parse()
                    .map_err(|_| "level id is a number".to_owned())?,
                map_rel: args[2].to_owned(),
                name: args[3..].join(" "),
                new_type: false,
            };
            let lines = game::add_to_game_run(state, &form, true)?;
            for l in lines {
                state.info(format!("add-to-game: {l}"));
            }
            Ok(Outcome::Done)
        }
        "change-type" => {
            let target: u32 = args
                .first()
                .and_then(|t| t.parse().ok())
                .ok_or_else(|| bad("<type id> [copy]"))?;
            let lines =
                game::change_type_run(state, target, args.get(1) == Some(&"copy"), None, true)?;
            for l in lines {
                state.info(format!("change-type: {l}"));
            }
            Ok(Outcome::Done)
        }
        "compat" => {
            let kept = state.tools.diag.kept.clone();
            let findings = diag::run_compat(state, &kept)?;
            let errors = findings
                .iter()
                .filter(|f| f.severity == capy_mod::compat::Severity::Error)
                .count();
            state.info(format!(
                "compat: {} finding(s), {errors} error(s)",
                findings.len()
            ));
            state.tools.diag.findings = findings;
            if args.first() == Some(&"errors") {
                let want: usize = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(0);
                if errors != want {
                    return Err(format!("{errors} error(s), expected {want}"));
                }
            }
            Ok(Outcome::Done)
        }
        "custom-object" => {
            if args.len() < 5 {
                return Err(bad("<row> <frames> <act> <png> <name...>"));
            }
            let num = |s: &str| {
                s.parse::<usize>()
                    .map_err(|_| format!("{s} is not a number"))
            };
            let msg = game::custom_object_run(
                state,
                num(args[0])?,
                &args[4..].join(" "),
                args[3],
                num(args[1])? as u32,
                num(args[2])? as u8,
                false,
            )?;
            state.info(format!("custom-object: {msg}"));
            Ok(Outcome::Done)
        }
        "package-export" => {
            let msg = package::export_run(state, rest, "")?;
            state.info(format!("package-export: {msg}"));
            Ok(Outcome::Done)
        }
        "package-import" => {
            let overwrite = args.last() == Some(&"overwrite");
            let path = if overwrite {
                args[..args.len() - 1].join(" ")
            } else {
                rest.to_owned()
            };
            let msg = package::import_run(state, &path, overwrite, false)?;
            state.info(format!("package-import: {msg}"));
            Ok(Outcome::Done)
        }
        "preset-save" => {
            let name = args.first().ok_or_else(|| bad("<name> [category]"))?;
            let cat = args.get(1).copied().unwrap_or("");
            presets::save_selection(state, name, cat)?;
            Ok(Outcome::Done)
        }
        "preset-export" => {
            let out = args.first().ok_or_else(|| bad("<zip> [preset name]"))?;
            let name = args[1..].join(" ");
            let scope = if name.is_empty() {
                presets::ExportScope::All
            } else {
                presets::ExportScope::Named(&name)
            };
            let msg = presets::export_run(state, scope, out)?;
            state.info(format!("preset-export: {msg}"));
            Ok(Outcome::Done)
        }
        "preset-import" => {
            let msg = presets::import_run(state, rest)?;
            state.info(format!("preset-import: {msg}"));
            Ok(Outcome::Done)
        }
        "preset-copy-tiles" => {
            let preset = presets::load_library(state)
                .into_iter()
                .find(|p| p.name == rest)
                .ok_or_else(|| format!("no preset named {rest}"))?;
            let msg = presets::copy_tiles_run(state, &preset)?;
            state.info(format!("preset-copy-tiles: {msg}"));
            Ok(Outcome::Done)
        }
        "preset-place" => {
            if args.len() < 3 {
                return Err(bad("<x> <y> <name>"));
            }
            let at = (
                args[0].parse().map_err(|_| "x is a number".to_owned())?,
                args[1].parse().map_err(|_| "y is a number".to_owned())?,
            );
            let name = args[2..].join(" ");
            let preset = presets::load_library(state)
                .into_iter()
                .find(|p| p.name == name)
                .ok_or_else(|| format!("no preset named {name}"))?;
            presets::place_now(state, &preset, at)?;
            Ok(Outcome::Done)
        }
        "assert-saved" => {
            let id = state
                .active_profile_id()
                .ok_or("No profile is open")?
                .to_owned();
            let saves = SavesDir::new(&state.paths.saves, &id);
            let gp = GamePath::new(rest);
            let file = saves.path_for(&gp).map_err(|e| e.to_string())?;
            if !file.is_file() {
                return Err(format!("{} is not in the saves", gp));
            }
            let listed = saves.changed().map_err(|e| e.to_string())?.contains(&gp);
            if !listed {
                return Err(format!("{gp} is not listed in CHANGED.txt"));
            }
            Ok(Outcome::Done)
        }
        "assert-map-listed" => {
            let gp = GamePath::new(rest);
            let listed = state.session.as_ref().is_some_and(|s| s.maps.contains(&gp));
            if listed {
                Ok(Outcome::Done)
            } else {
                Err(format!("{gp} is not in the Maps list"))
            }
        }
        "" => Err(bad("a verb")),
        other => Err(format!("unknown tool verb {other:?}")),
    }
}
