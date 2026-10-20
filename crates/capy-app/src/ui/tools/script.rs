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

use super::{diag, dt1, game, package, presets, tables};
use crate::state::{AppState, CatalogState};

/// Outcome of one script line: done, or call again next frame.
pub enum Outcome {
    Done,
    Retry,
}

fn bad(usage: &str) -> String {
    format!("expected {usage}")
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
        "dt1-open" => {
            dt1::open_lib(state, rest)?;
            Ok(Outcome::Done)
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
