//! "Add to this map": right-click a tile marked "!" in Tiles (Ctrl+click
//! picks several) and copy it into the map's level type's own DT1. A preview
//! lists every change; Add saves the DT1, LvlTypes and LvlPrest to the
//! profile's saves, lists the DT1 in the DS1 (one undoable patch) and
//! switches the brush to the new numbers.
//!
//! Undo: the DS1 file list is a `capy-edit` patch (Ctrl+Z). The DT1 and
//! tables are files: each save records a history snapshot, so they are
//! restored from the History panel. The DT1 only ever grows, so undoing the
//! DS1 never leaves a painted cell on a tile that is gone.

use std::collections::BTreeSet;

use bevy_egui::egui::{self, Color32, Id, RichText};
use capy_mod::tiles_register::{
    AddTilesInput, AddTilesOutcome, add_tiles_to_map, own_path_for, type_files,
};
use capy_mod::tiles_review::TilePick;
use d2_assets::GamePath;
use d2_formats::dt1::Dt1;

use super::io;
use crate::browser::brush::{TileId as KeyId, parse_tile_key, tile_key};
use crate::state::AppState;

const WARN: Color32 = Color32::from_rgb(235, 150, 40);
const ERROR: Color32 = Color32::from_rgb(235, 90, 90);

/// A computed, not yet saved, addition.
pub struct Plan {
    out: AddTilesOutcome,
    own_game: GamePath,
    /// The brush after the addition: the first picked tile's new key.
    brush_key: Option<String>,
    bin_note: bool,
}

pub struct Dialog {
    keys: Vec<String>,
    plan: Result<Plan, String>,
}

#[derive(Default)]
pub struct AddMapUi {
    /// Tiles Ctrl+clicked in Tiles, waiting for "Add to this map".
    pub picked: BTreeSet<String>,
    pub dialog: Option<Dialog>,
}

/// Opens the preview for `keys` (tile keys of the Tiles browser).
pub fn open(state: &mut AppState, keys: Vec<String>) {
    let plan = compute(state, &keys);
    state.tools.addmap.dialog = Some(Dialog { keys, plan });
}

fn compute(state: &AppState, keys: &[String]) -> Result<Plan, String> {
    let doc = state.active_doc().ok_or("Open a map first")?;
    let info = doc
        .info
        .as_ref()
        .ok_or("The map's level type is not known yet (the catalog is still loading)")?;
    let type_id = info
        .lvl_type
        .ok_or("This map has no level type, so there is nowhere to register the tiles")?;
    let map_path = doc.path.as_str().to_owned();
    let tables = io::level_tables(state)?;

    // the picked tiles' source libraries
    let mut src_paths: Vec<String> = Vec::new();
    let mut src_dt1: Vec<Dt1> = Vec::new();
    let mut picks: Vec<TilePick> = Vec::new();
    let mut first: Option<(usize, usize)> = None;
    for key in keys {
        let (path, id) = parse_tile_key(key).ok_or_else(|| format!("{key} is not a tile"))?;
        let rel = io::tiles_rel(path);
        let source = match src_paths.iter().position(|p| p.eq_ignore_ascii_case(&rel)) {
            Some(i) => i,
            None => {
                let bytes = io::read_game(state, &GamePath::new(path))?
                    .ok_or_else(|| format!("{path} was not found"))?;
                src_dt1.push(Dt1::parse(&bytes).map_err(|e| format!("{path}: {e}"))?);
                src_paths.push(rel);
                src_paths.len() - 1
            }
        };
        let index = src_dt1[source]
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.orientation == i32::from(id.orientation)
                    && t.main_index == i32::from(id.main)
                    && t.sub_index == i32::from(id.sub)
            })
            .nth(id.n as usize)
            .map(|(i, _)| i)
            .ok_or_else(|| format!("{key}: the tile is not in the library"))?;
        first.get_or_insert((source, index));
        picks.push(TilePick { source, index });
    }
    if picks.is_empty() {
        return Err("No tile is picked".into());
    }
    let sources: Vec<(String, &Dt1)> = src_paths.iter().cloned().zip(src_dt1.iter()).collect();

    // every DT1 of the level type, and the type's own-tiles DT1 if it exists
    let mut type_libs: Vec<(String, Dt1)> = Vec::new();
    for rel in type_files(&tables.types, type_id) {
        if let Some(b) = io::read_game(state, &io::tiles_game(&rel))?
            && let Ok(d) = Dt1::parse(&b)
        {
            type_libs.push((rel, d));
        }
    }
    let type_dt1s: Vec<(String, &Dt1)> = type_libs.iter().map(|(p, d)| (p.clone(), d)).collect();
    let own_rel = own_path_for(&tables.types, type_id, &map_path);
    let own_game = io::tiles_game(&own_rel);
    let existing = match io::read_game(state, &own_game)? {
        Some(b) => Some(Dt1::parse(&b).map_err(|e| format!("{own_game}: {e}"))?),
        None => None,
    };
    let out = add_tiles_to_map(&AddTilesInput {
        map_path: &map_path,
        ds1: doc.edit.ds1(),
        types: &tables.types,
        prest: &tables.prest,
        type_id,
        prest_def: info.lvlprest_def,
        picks: &picks,
        sources: &sources,
        type_dt1s: &type_dt1s,
        existing_own: existing.as_ref(),
        existing_own_path: Some(&own_rel),
    })
    .map_err(|e| e.to_string())?;

    // the brush: the first picked tile at its new number
    let brush_key = first.and_then(|(s, i)| {
        let at = out
            .plan
            .tiles
            .iter()
            .position(|p| p.from.source == s && p.from.index == i)?;
        let me = &out.plan.tiles[at];
        let to = capy_mod::tiles_review::TileId::new(me.id.orientation, me.new_main, me.new_sub);
        // index among the tiles with this number in the library: the earlier
        // frames of the same number (the library had none before)
        let n = out.plan.tiles[..at]
            .iter()
            .filter(|p| {
                p.new_main == to.main && p.new_sub == to.sub && p.id.orientation == to.orientation
            })
            .count();
        Some(tile_key(
            own_game.as_str(),
            KeyId {
                orientation: u8::try_from(to.orientation).ok()?,
                main: u8::try_from(to.main).ok()?,
                sub: u8::try_from(to.sub).ok()?,
                n: n as u32,
            },
        ))
    });
    Ok(Plan {
        out,
        own_game,
        brush_key,
        bin_note: io::reads_bin_tables(state),
    })
}

/// Saves what the preview showed and applies it to the open map.
fn apply(state: &mut AppState, plan: Plan) -> Result<String, String> {
    let Plan {
        out,
        own_game,
        brush_key,
        bin_note,
    } = plan;
    let tables = io::level_tables(state)?;
    let bytes = out
        .dt1
        .to_bytes()
        .map_err(|e| format!("cannot write the DT1: {e}"))?;
    io::save_game(state, &own_game, &bytes)?;
    if out.types != tables.types {
        io::save_txt(state, "lvltypes.txt", &out.types)?;
    }
    if out.prest != tables.prest {
        io::save_txt(state, "lvlprest.txt", &out.prest)?;
    }
    let doc = state.active_doc().ok_or("Open a map first")?;
    if out.ds1.files != doc.edit.ds1().files {
        super::game::replace_map(state, "Add tiles to this map", out.ds1.clone())?;
    }
    if let Some(i) = state.active {
        crate::ui::studio::libs::attach_to_info(state, i, &own_game);
    }
    if !bin_note {
        // The game (and so the catalog) now loads the DT1 for the map.
        state.rebuild_catalog();
    }
    if let Some(key) = brush_key {
        state.set_brush_from_key(&key)?;
    }
    state.tools.addmap.picked.clear();
    let mut msg = format!(
        "Added {} tile(s) to {} ({} renumbered)",
        out.added,
        io::tiles_rel(own_game.as_str()),
        out.remap.len()
    );
    for w in &out.warnings {
        state.warn(w.clone());
    }
    if bin_note {
        state.warn(io::BIN_NOTE);
        msg.push_str(". ");
        msg.push_str(io::BIN_NOTE);
    }
    Ok(msg)
}

/// The scripted form: opens the preview for `keys`.
pub fn script_open(state: &mut AppState, keys: Vec<String>) -> Result<(), String> {
    open(state, keys);
    match state.tools.addmap.dialog.as_ref().map(|d| &d.plan) {
        Some(Err(e)) => Err(e.clone()),
        _ => Ok(()),
    }
}

/// The scripted form of pressing Add.
pub fn script_confirm(state: &mut AppState) -> Result<String, String> {
    let d = state
        .tools
        .addmap
        .dialog
        .take()
        .ok_or("The Add to this map preview is not open")?;
    apply(state, d.plan?)
}

pub fn window(ctx: &egui::Context, state: &mut AppState) {
    let Some(d) = state.tools.addmap.dialog.take() else {
        return;
    };
    let mut open = true;
    let mut add = false;
    let mut cancel = false;
    egui::Window::new("Add to this map")
        .id(Id::new("addmap_dialog"))
        .open(&mut open)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.label(format!("{} tile(s) picked.", d.keys.len()));
            match &d.plan {
                Err(e) => {
                    ui.colored_label(ERROR, e);
                }
                Ok(p) => {
                    ui.label("These changes will be saved to the profile's saves:");
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for c in &p.out.changes {
                            ui.add(egui::Label::new(format!("- {c}")).wrap());
                        }
                    });
                    for w in &p.out.warnings {
                        ui.add(egui::Label::new(RichText::new(w).color(WARN)).wrap());
                    }
                    if p.bin_note {
                        ui.add(egui::Label::new(RichText::new(io::BIN_NOTE).color(WARN)).wrap());
                    }
                    ui.weak(
                        "Ctrl+Z undoes the map's file list; the DT1 and tables can be restored from History.",
                    );
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(d.plan.is_ok(), egui::Button::new("Add")).clicked() {
                    add = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if add {
        match d.plan.and_then(|p| apply(state, p)) {
            Ok(msg) => state.info(msg),
            Err(e) => state.error(format!("Add to this map: {e}")),
        }
    } else if open && !cancel {
        state.tools.addmap.dialog = Some(d);
    }
}
