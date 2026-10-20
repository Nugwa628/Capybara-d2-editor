//! Runs the `CAPY_SCRIPT` commands of `edit::script_studio` through the same
//! functions the mouse and the dialogs use. `Ok(true)`: done, `Ok(false)`:
//! try again next frame (the map is not drawn yet, an export is running).

use capy_edit::{CellRect, CellValue};
use d2_formats::ds1::Cell;

use super::dialogs::{self, KeyEdit, NewMapDialog};
use super::paint::{self, Drag, Edge};
use super::{ViewMode, current_view, set_view, views};
use crate::edit::script_studio::StudioCmd;
use crate::state::AppState;

fn view_named(name: &str) -> Result<ViewMode, String> {
    Ok(match name {
        "tiles" => ViewMode::Tiles,
        "objects" => ViewMode::Objects,
        "walk" | "walkability" => ViewMode::Walk,
        "automap" => ViewMode::Automap,
        "light" => ViewMode::Light,
        "roofs" | "pops" => ViewMode::Roofs,
        other => return Err(format!("no view {other:?}")),
    })
}

fn key(text: &str) -> Result<KeyEdit, String> {
    KeyEdit::parse(text).ok_or_else(|| format!("{text:?} is not o/m/s"))
}

pub fn step(state: &mut AppState, cmd: &StudioCmd) -> Result<bool, String> {
    if let StudioCmd::NewMap {
        path,
        width,
        height,
        act,
    } = cmd
    {
        let d = NewMapDialog {
            path: path.clone(),
            width: *width,
            height: *height,
            act: *act,
            walls: 1,
            floors: 1,
            tag: false,
            lvl_type: None,
            error: None,
        };
        return dialogs::create_new_map(state, &d)
            .map(|()| true)
            .map_err(|e| format!("new-map: {e}"));
    }
    let Some(index) = state.active else {
        return Err("no map is open (use `open <game path>` first)".to_owned());
    };
    match cmd {
        StudioCmd::NewMap { .. } => {}
        StudioCmd::SetBrush {
            layer,
            cell,
            orientation,
        } => state.edit.pick(
            *layer,
            CellValue {
                cell: *cell,
                orientation: *orientation,
            },
        ),
        StudioCmd::MixAdd {
            layer,
            cell,
            orientation,
        } => paint::toggle_mix(
            state,
            *layer,
            CellValue {
                cell: *cell,
                orientation: *orientation,
            },
        ),
        StudioCmd::PaintRect {
            layer,
            x0,
            y0,
            x1,
            y1,
            erase,
        } => {
            let before = state.docs[index].edit.revision();
            paint::paint_rect(
                state,
                index,
                *layer,
                CellRect::new((*x0, *y0), (*x1, *y1)),
                *erase,
            );
            if state.docs[index].edit.revision() == before {
                return Err("paint-rect changed nothing".to_owned());
            }
        }
        StudioCmd::Fill { layer, x, y, erase } => {
            let before = state.docs[index].edit.revision();
            paint::fill_at(state, index, *layer, (*x, *y), *erase);
            if state.docs[index].edit.revision() == before {
                return Err("fill changed nothing".to_owned());
            }
        }
        StudioCmd::Replace { from, to } => {
            let (from, to) = (key(from)?, key(to)?);
            let layers = dialogs::layers_for(&state.docs[index].ds1, from.orientation);
            if !dialogs::apply_replace(state, index, layers, from.key(), to.key(), false) {
                return Err("replace failed".to_owned());
            }
        }
        StudioCmd::DragEdge { edge, cells } => {
            let (w, h) = (
                f64::from(state.docs[index].ds1.width),
                f64::from(state.docs[index].ds1.height),
            );
            let c = f64::from(*cells);
            let (edge, start, current) = match edge.as_str() {
                "left" => (Edge::Left, (0.0, h / 2.0), (-c, h / 2.0)),
                "top" => (Edge::Top, (w / 2.0, 0.0), (w / 2.0, -c)),
                "right" => (Edge::Right, (w, h / 2.0), (w + c, h / 2.0)),
                "bottom" => (Edge::Bottom, (w / 2.0, h), (w / 2.0, h + c)),
                other => return Err(format!("drag-edge: no edge {other:?}")),
            };
            state.studio.drag = Some(Drag::Edge {
                doc: state.docs[index].id,
                edge,
                start,
                current,
            });
            paint::finish_drag(state, index);
        }
        StudioCmd::View(name) => set_view(state, view_named(name)?),
        StudioCmd::AssertView(name) => {
            let want = view_named(name)?;
            let now = current_view(state);
            if now != want {
                return Err(format!("assert-view: the view is {}", now.label()));
            }
        }
        StudioCmd::AssertTile { layer, x, y, tile } => {
            let doc = &state.docs[index];
            let v = layer
                .get(&doc.ds1, *x, *y)
                .ok_or_else(|| format!("assert-tile: no cell {x}, {y} on that layer"))?;
            let got = (!layer.is_empty_value(v)).then(|| {
                let c = Cell(v.cell);
                (c.main_index(), c.sub_index())
            });
            if got != *tile {
                return Err(format!(
                    "assert-tile {x} {y}: expected {tile:?}, found {got:?} ({:#010x})",
                    v.cell
                ));
            }
        }
        StudioCmd::AssertSize(w, h) => {
            let d = &state.docs[index].ds1;
            if (d.width, d.height) != (*w, *h) {
                return Err(format!(
                    "assert-size: the map is {} x {}, not {w} x {h}",
                    d.width, d.height
                ));
            }
        }
        StudioCmd::AssertWalkable {
            expected,
            tolerance,
        } => {
            let Some(area) = views::walkable(state, index) else {
                return Ok(false);
            };
            state.info(format!(
                "assert-walkable: badge {area:.2}, expected {expected:.2}"
            ));
            if (area - expected).abs() > *tolerance {
                return Err(format!(
                    "assert-walkable: the badge shows {area:.2}, expected {expected:.2} +- {tolerance}"
                ));
            }
        }
        StudioCmd::ExportPng { file, selection } => {
            let ex = &state.studio.export;
            if ex.script_wait {
                if ex.busy() {
                    return Ok(false);
                }
                state.studio.export.script_wait = false;
                return match state.studio.export.last_export.clone() {
                    Some(Ok(_)) if file.exists() => Ok(true),
                    Some(Err(e)) => Err(format!("export-png: {e}")),
                    _ => Err("export-png: nothing was written".to_owned()),
                };
            }
            if state.docs[index].tiles.is_none() {
                return Ok(false);
            }
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            super::export::export_png(state, *selection, Some(file.clone()));
            state.studio.export.script_wait = true;
            return Ok(false);
        }
        StudioCmd::CopyView => {
            if state.studio.export.script_wait {
                if state.studio.export.busy() {
                    return Ok(false);
                }
                state.studio.export.script_wait = false;
                return match state.studio.export.last_copy {
                    Some((w, h)) => {
                        state.info(format!("copy-view: {w} x {h}"));
                        Ok(true)
                    }
                    None => Err("copy-view: nothing was captured".to_owned()),
                };
            }
            if state.map_rect.is_none() {
                return Ok(false);
            }
            state.studio.export.last_copy = None;
            super::super::commands::run(state, "view.snapshot");
            state.studio.export.script_wait = true;
            return Ok(false);
        }
    }
    Ok(true)
}
