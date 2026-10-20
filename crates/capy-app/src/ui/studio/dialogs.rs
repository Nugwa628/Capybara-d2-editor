//! P9 dialogs (ds1-studio `EditDialogs.tsx`, `FloorRerollDialog.tsx`,
//! `ReassignPanel.tsx`): Find & replace tiles (Ctrl+H), Re-roll floors,
//! Reassign tiles and New map. Each applies one `capy_edit` patch, so one
//! Ctrl+Z takes it back.

use std::collections::{BTreeMap, BTreeSet};

use bevy_egui::egui::{self, Context, Id, RichText, Ui};
use capy_edit::{FloorChoice, LayerRef, NewMapSpec, RerollOptions, TileKey, tile_key};
use d2_assets::GamePath;

use crate::doc::layer_label;
use crate::edit::ops::EditOp;
use crate::state::AppState;

#[derive(Default)]
pub struct Dialogs {
    pub replace: Option<ReplaceDialog>,
    pub reroll: Option<RerollDialog>,
    pub reassign: Option<ReassignDialog>,
    pub new_map: Option<NewMapDialog>,
}

/// Three numbers naming a tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyEdit {
    pub orientation: u32,
    pub main: u8,
    pub sub: u8,
}

impl KeyEdit {
    pub fn key(self) -> TileKey {
        TileKey {
            orientation: self.orientation,
            main: self.main,
            sub: self.sub,
        }
    }

    pub fn of(k: TileKey) -> KeyEdit {
        KeyEdit {
            orientation: k.orientation,
            main: k.main,
            sub: k.sub,
        }
    }

    /// `o/m/s` or `o:m/s` as typed in scripts.
    pub fn parse(text: &str) -> Option<KeyEdit> {
        let parts: Vec<&str> = text.split(['/', ':']).collect();
        let [o, m, s] = parts.as_slice() else {
            return None;
        };
        Some(KeyEdit {
            orientation: o.parse().ok()?,
            main: m.parse().ok()?,
            sub: s.parse().ok()?,
        })
    }
}

fn key_row(ui: &mut Ui, label: &str, k: &mut KeyEdit) {
    ui.label(label);
    ui.add(
        egui::DragValue::new(&mut k.orientation)
            .range(0..=19)
            .prefix("o "),
    )
    .on_hover_text("Orientation: 0 floors, 13 shadows, 1-9 and 16+ walls, 15 roofs");
    ui.add(
        egui::DragValue::new(&mut k.main)
            .range(0..=63)
            .prefix("main "),
    );
    ui.add(egui::DragValue::new(&mut k.sub).prefix("sub "));
}

/// The layers a tile key lives on: floors for orientation 0, the shadow
/// layer for 13, the wall layers otherwise.
pub fn layers_for(ds1: &d2_formats::ds1::Ds1, orientation: u32) -> Vec<LayerRef> {
    LayerRef::all_in(ds1)
        .into_iter()
        .filter(|l| match (l, orientation) {
            (LayerRef::Floor(_), 0) => true,
            (LayerRef::Shadow, 13) => true,
            (LayerRef::Wall(_), o) => o != 0 && o != 13,
            _ => false,
        })
        .collect()
}

// ───────────────────────── replace ─────────────────────────

pub struct ReplaceDialog {
    pub from: KeyEdit,
    pub to: KeyEdit,
    /// Every tile layer of the map with whether it is searched.
    pub layers: Vec<(LayerRef, bool)>,
    pub in_selection: bool,
}

/// The key of the brush, and of the cell under (or last clicked by) the
/// pointer on the active layer.
fn brush_key(state: &AppState) -> Option<TileKey> {
    let layer = state
        .edit
        .current_layer
        .or_else(|| state.active_doc().map(|d| d.active_layer))?;
    tile_key(layer, state.edit.current)
}

fn cell_key(state: &AppState) -> Option<TileKey> {
    let doc = state.active_doc()?;
    let (x, y) = super::sections::shown_cell(state)?;
    let l = doc.active_layer;
    tile_key(l, l.get(&doc.ds1, x, y)?)
}

pub fn open_replace(state: &mut AppState) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    let from = cell_key(state)
        .or_else(|| brush_key(state))
        .map(KeyEdit::of)
        .unwrap_or_default();
    let to = brush_key(state).map_or(from, KeyEdit::of);
    let wanted = layers_for(&doc.ds1, from.orientation);
    let layers = LayerRef::all_in(&doc.ds1)
        .into_iter()
        .filter(|l| *l != LayerRef::Tag)
        .map(|l| (l, wanted.contains(&l)))
        .collect();
    let in_selection = !doc.selection.is_empty();
    state.studio.dialogs.replace = Some(ReplaceDialog {
        from: from,
        to: to,
        layers,
        in_selection,
    });
}

/// Runs a replace on map `index`: one undo step.
pub fn apply_replace(
    state: &mut AppState,
    index: usize,
    layers: Vec<LayerRef>,
    from: TileKey,
    to: TileKey,
    in_selection: bool,
) -> bool {
    state.exec(
        index,
        EditOp::ReplaceTiles {
            layers,
            from,
            to,
            in_selection,
        },
    )
}

fn replace_window(ctx: &Context, state: &mut AppState) {
    let Some(mut d) = state.studio.dialogs.replace.take() else {
        return;
    };
    let Some(index) = state.active else { return };
    let mut open = true;
    let mut close = false;
    let mut apply = false;
    let mut select = false;
    let has_selection = !state.docs[index].selection.is_empty();
    let layers: Vec<LayerRef> = d
        .layers
        .iter()
        .filter(|(_, on)| *on)
        .map(|(l, _)| *l)
        .collect();
    let within = (d.in_selection && has_selection).then_some(&state.docs[index].selection);
    let found = state.docs[index]
        .edit
        .find_tiles(&layers, d.from.key(), within);
    egui::Window::new("Find & replace tiles")
        .id(Id::new("studio_replace"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos([520.0, 200.0])
        .show(ctx, |ui| {
            egui::Grid::new("replace_keys").num_columns(4).show(ui, |ui| {
                key_row(ui, "Find", &mut d.from);
                ui.end_row();
                key_row(ui, "Replace with", &mut d.to);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.small_button("Find = cell").on_hover_text("The tile of the cell under the pointer (or last clicked) on the active layer").clicked()
                    && let Some(k) = cell_key(state)
                {
                    d.from = KeyEdit::of(k);
                }
                if ui.small_button("Find = brush").clicked()
                    && let Some(k) = brush_key(state)
                {
                    d.from = KeyEdit::of(k);
                }
                if ui.small_button("Replace = brush").clicked()
                    && let Some(k) = brush_key(state)
                {
                    d.to = KeyEdit::of(k);
                }
            });
            ui.separator();
            ui.label("Layers");
            ui.horizontal_wrapped(|ui| {
                for (l, on) in &mut d.layers {
                    ui.checkbox(on, layer_label(*l));
                }
            });
            ui.add_enabled(
                has_selection,
                egui::Checkbox::new(&mut d.in_selection, "Only inside the selection"),
            );
            ui.separator();
            ui.label(format!("{} cell(s) match", found.len()));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!found.is_empty(), egui::Button::new("Select them"))
                    .clicked()
                {
                    select = true;
                }
                if ui
                    .add_enabled(
                        !found.is_empty() && d.from != d.to,
                        egui::Button::new(RichText::new("Replace all").strong()),
                    )
                    .clicked()
                {
                    apply = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
            ui.weak("Other cell bits (hidden, unwalkable) are kept. One Ctrl+Z undoes it.");
        });
    if select {
        let doc = &mut state.docs[index];
        doc.selection.clear();
        for (l, x, y) in &found {
            doc.selection.insert(*l, *x, *y);
        }
    }
    if apply {
        let in_sel = d.in_selection && has_selection;
        apply_replace(state, index, layers, d.from.key(), d.to.key(), in_sel);
    }
    if open && !close {
        state.studio.dialogs.replace = Some(d);
    }
}

// ───────────────────────── re-roll ─────────────────────────

pub struct RerollDialog {
    pub layer: u8,
    pub in_selection: bool,
    pub opts: RerollOptions,
    /// Checked floor tiles of the map's tile set: (DT1 index, tile index).
    pub picks: BTreeSet<(usize, usize)>,
    /// The DT1 listed.
    pub dt1: usize,
}

pub fn open_reroll(state: &mut AppState) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    let layer = match doc.active_layer {
        LayerRef::Floor(i) => i,
        _ => 0,
    };
    let in_selection = !doc.selection.is_empty();
    state.studio.dialogs.reroll = Some(RerollDialog {
        layer,
        in_selection,
        opts: RerollOptions {
            preserve_walkability: true,
            avoid_repeats: true,
            seed: 1,
        },
        picks: BTreeSet::new(),
        dt1: 0,
    });
}

/// A floor tile's walkability as 25 bits (bit k: sub-tile k blocks walking).
pub fn walk_signature(flags: &[u8; 25]) -> u32 {
    flags
        .iter()
        .enumerate()
        .filter(|(_, f)| **f & capy_render::walk::BLOCK_WALK != 0)
        .fold(0, |acc, (k, _)| acc | (1 << k))
}

/// Floor tiles of DT1 `d`: (tile index, main, sub, rarity, animated).
fn floors_of(tiles: &capy_render::TileSet, d: usize) -> Vec<(usize, u8, u8, i32, bool)> {
    let mut out = Vec::new();
    let mut t = 0;
    while let Some(tile) = tiles.tile(&capy_render::TileRef { dt1: d, tile: t }) {
        if tile.orientation == 0
            && let (Ok(m), Ok(s)) = (u8::try_from(tile.main_index), u8::try_from(tile.sub_index))
        {
            out.push((t, m, s, tile.rarity, tile.animated != 0));
        }
        t += 1;
    }
    out
}

/// The walkability signature of every floor key the tile set resolves.
pub fn walk_table(tiles: &capy_render::TileSet) -> BTreeMap<(u8, u8), u32> {
    let mut out = BTreeMap::new();
    for d in 0..tiles.dt1_paths().len() {
        for (_, m, s, _, _) in floors_of(tiles, d) {
            let key = d2_formats::dt1::TileKey {
                orientation: 0,
                main: m,
                sub: s,
            };
            if let Some(r) = tiles.resolve(key)
                && let Some(t) = tiles.tile(&r)
            {
                out.insert((m, s), walk_signature(&t.subtile_flags));
            }
        }
    }
    out
}

/// Builds the choices of the checked tiles and runs the re-roll.
pub fn apply_reroll(state: &mut AppState, index: usize, d: &RerollDialog) -> bool {
    let Some(tiles) = state.docs[index].tiles.clone() else {
        return false;
    };
    let mut choices: BTreeMap<(u8, u8), FloorChoice> = BTreeMap::new();
    for &(dt1, t) in &d.picks {
        let Some(tile) = tiles.tile(&capy_render::TileRef { dt1, tile: t }) else {
            continue;
        };
        let (Ok(main), Ok(sub)) = (u8::try_from(tile.main_index), u8::try_from(tile.sub_index))
        else {
            continue;
        };
        let rarity = u32::try_from(tile.rarity.max(0)).unwrap_or(0);
        let c = choices.entry((main, sub)).or_insert(FloorChoice {
            main,
            sub,
            rarity,
            walk: walk_signature(&tile.subtile_flags),
        });
        c.rarity = c.rarity.max(rarity);
    }
    if choices.is_empty() {
        state.notify("Check at least one floor tile to re-roll with");
        return false;
    }
    let in_selection = d.in_selection && !state.docs[index].selection.is_empty();
    state.exec(
        index,
        EditOp::RerollFloors {
            layer: d.layer,
            in_selection,
            choices: choices.into_values().collect(),
            opts: d.opts,
            walk: walk_table(&tiles),
        },
    )
}

fn reroll_window(ctx: &Context, state: &mut AppState) {
    let Some(mut d) = state.studio.dialogs.reroll.take() else {
        return;
    };
    let Some(index) = state.active else { return };
    let Some(tiles) = state.docs[index].tiles.clone() else {
        state.notify("Re-roll floors needs the map's tiles: wait until it is drawn");
        return;
    };
    let floors_n = state.docs[index].ds1.floors.len();
    let has_selection = !state.docs[index].selection.is_empty();
    let mut open = true;
    let mut close = false;
    let mut apply = false;
    let paths: Vec<String> = tiles
        .dt1_paths()
        .iter()
        .map(|p| {
            p.as_str()
                .trim_start_matches("data\\global\\tiles\\")
                .replace('\\', "/")
        })
        .collect();
    d.dt1 = d.dt1.min(paths.len().saturating_sub(1));
    let list = floors_of(&tiles, d.dt1);
    egui::Window::new("Re-roll floors")
        .id(Id::new("studio_reroll"))
        .open(&mut open)
        .collapsible(false)
        .default_size([460.0, 480.0])
        .default_pos([480.0, 140.0])
        .show(ctx, |ui| {
            ui.weak(
                "Check floor tiles of this map's tile set. The re-roll spreads them evenly, \
                 avoids repeating a neighbour, leaves empty cells alone and can keep the \
                 walkability of every cell.",
            );
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("reroll_layer")
                    .selected_text(format!("Floor {}", d.layer + 1))
                    .show_ui(ui, |ui| {
                        for i in 0..floors_n {
                            ui.selectable_value(&mut d.layer, i as u8, format!("Floor {}", i + 1));
                        }
                    });
                ui.add_enabled(
                    has_selection,
                    egui::Checkbox::new(&mut d.in_selection, "Selection only"),
                );
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.opts.preserve_walkability, "Preserve walkability");
                ui.checkbox(&mut d.opts.avoid_repeats, "Avoid adjacent repeats");
                ui.add(egui::DragValue::new(&mut d.opts.seed).prefix("variation "));
            });
            egui::ComboBox::from_id_salt("reroll_dt1")
                .width(400.0)
                .selected_text(paths.get(d.dt1).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (i, p) in paths.iter().enumerate() {
                        ui.selectable_value(&mut d.dt1, i, p);
                    }
                });
            ui.horizontal(|ui| {
                if ui.small_button("Check every floor of this DT1").clicked() {
                    for (t, ..) in &list {
                        d.picks.insert((d.dt1, *t));
                    }
                }
                if ui.small_button("Clear").clicked() {
                    d.picks.clear();
                }
            });
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .id_salt("reroll_list")
                .show(ui, |ui| {
                    if list.is_empty() {
                        ui.weak("This DT1 has no floor tiles.");
                    }
                    for (t, m, s, rarity, anim) in &list {
                        let mut on = d.picks.contains(&(d.dt1, *t));
                        let text = format!(
                            "#{t}  {m}/{s}  rarity {rarity}{}",
                            if *anim { "  animated" } else { "" }
                        );
                        if ui.checkbox(&mut on, text).changed() {
                            if on {
                                d.picks.insert((d.dt1, *t));
                            } else {
                                d.picks.remove(&(d.dt1, *t));
                            }
                        }
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("{} tile(s) checked", d.picks.len()));
                if ui
                    .add_enabled(
                        !d.picks.is_empty(),
                        egui::Button::new(RichText::new("Re-roll").strong()),
                    )
                    .clicked()
                {
                    apply = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
            ui.weak("Tiles from other DT1s need the DT1 manager (later). One Ctrl+Z undoes it.");
        });
    if apply && apply_reroll(state, index, &d) {
        close = true;
    }
    if open && !close {
        state.studio.dialogs.reroll = Some(d);
    }
}

// ───────────────────────── reassign ─────────────────────────

pub struct ReassignDialog {
    pub rows: Vec<(KeyEdit, KeyEdit)>,
}

pub fn open_reassign(state: &mut AppState) {
    let from = cell_key(state)
        .or_else(|| brush_key(state))
        .map(KeyEdit::of)
        .unwrap_or_default();
    state.studio.dialogs.reassign = Some(ReassignDialog {
        rows: vec![(from, from)],
    });
}

pub fn apply_reassign(state: &mut AppState, index: usize, rows: &[(KeyEdit, KeyEdit)]) -> bool {
    let moves: BTreeMap<TileKey, TileKey> = rows
        .iter()
        .filter(|(a, b)| a != b)
        .map(|(a, b)| (a.key(), b.key()))
        .collect();
    if moves.is_empty() {
        state.notify("Nothing to reassign: every row maps a tile to itself");
        return false;
    }
    state.exec(index, EditOp::ReassignTiles(moves))
}

fn reassign_window(ctx: &Context, state: &mut AppState) {
    let Some(mut d) = state.studio.dialogs.reassign.take() else {
        return;
    };
    let Some(index) = state.active else { return };
    let mut open = true;
    let mut close = false;
    let mut apply = false;
    let mut remove = None;
    let all: Vec<LayerRef> = LayerRef::all_in(&state.docs[index].ds1)
        .into_iter()
        .filter(|l| *l != LayerRef::Tag)
        .collect();
    egui::Window::new("Reassign tiles")
        .id(Id::new("studio_reassign"))
        .open(&mut open)
        .collapsible(false)
        .default_pos([480.0, 180.0])
        .show(ctx, |ui| {
            ui.weak(
                "Give placed tiles new numbers (after renumbering them in a DT1): every \
                 cell with the old orientation/main/sub gets the new one, on every layer.",
            );
            egui::Grid::new("reassign_rows")
                .num_columns(9)
                .show(ui, |ui| {
                    for (i, (from, to)) in d.rows.iter_mut().enumerate() {
                        key_row(ui, "", from);
                        key_row(ui, "\u{2192}", to);
                        let n = state.docs[index]
                            .edit
                            .find_tiles(&all, from.key(), None)
                            .len();
                        ui.weak(format!("{n} cell(s)"));
                        if ui.small_button("x").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("Add row").clicked() {
                    d.rows.push(Default::default());
                }
                if ui.button(RichText::new("Reassign").strong()).clicked() {
                    apply = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    if let Some(i) = remove {
        d.rows.remove(i);
    }
    if apply && apply_reassign(state, index, &d.rows) {
        close = true;
    }
    if open && !close {
        state.studio.dialogs.reassign = Some(d);
    }
}

// ───────────────────────── new map ─────────────────────────

pub struct NewMapDialog {
    pub path: String,
    pub width: u32,
    pub height: u32,
    /// 1-based act.
    pub act: u8,
    pub walls: usize,
    pub floors: usize,
    pub tag: bool,
    /// LvlType to draw it with (`None`: guess from the folder).
    pub lvl_type: Option<u32>,
    pub error: Option<String>,
}

pub fn open_new_map(state: &mut AppState) {
    let folder = state
        .active_doc()
        .map(|d| {
            let p = d.path.as_str();
            p.rsplit_once('\\').map_or(p, |(dir, _)| dir).to_owned()
        })
        .unwrap_or_else(|| "data\\global\\tiles\\act1\\town".to_owned());
    let act = state
        .active_doc()
        .map_or(1, |d| d.ds1.effective_act().clamp(0, 4) as u8 + 1);
    state.studio.dialogs.new_map = Some(NewMapDialog {
        path: format!("{folder}\\newmap.ds1"),
        width: 24,
        height: 24,
        act,
        walls: 1,
        floors: 1,
        tag: false,
        lvl_type: None,
        error: None,
    });
}

/// Checks the game path of a new map.
pub fn check_new_path(state: &AppState, path: &str) -> Result<GamePath, String> {
    let lower = path.trim().to_ascii_lowercase().replace('/', "\\");
    if !lower.starts_with("data\\global\\tiles\\") || !lower.ends_with(".ds1") {
        return Err("The path must be data\\global\\tiles\\...\\name.ds1".to_owned());
    }
    let gp = GamePath::new(&lower);
    if let Some(session) = &state.session
        && session.maps.contains(&gp)
    {
        return Err(format!("{gp} already exists: pick another name"));
    }
    if state.docs.iter().any(|d| d.path == gp) {
        return Err(format!("{gp} is open already"));
    }
    Ok(gp)
}

/// Creates the map, opens it and saves it to the profile's saves folder.
pub fn create_new_map(state: &mut AppState, d: &NewMapDialog) -> Result<(), String> {
    let path = check_new_path(state, &d.path)?;
    let spec = NewMapSpec {
        width: d.width,
        height: d.height,
        act: i32::from(d.act.clamp(1, 5)) - 1,
        walls: d.walls,
        floors: d.floors,
        tag: d.tag,
        version: None,
    };
    let ds1 = capy_edit::new_map(&spec).map_err(|e| e.to_string())?;
    let index = state
        .open_new_map(path.clone(), ds1)
        .ok_or_else(|| "No profile is open".to_owned())?;
    if let Some(t) = d.lvl_type {
        state.set_map_override(index, t, None);
    }
    state.save_doc(index);
    state.notify(format!(
        "New map {path}: {} x {}, saved to the profile's saves folder",
        d.width, d.height
    ));
    Ok(())
}

fn new_map_window(ctx: &Context, state: &mut AppState) {
    let Some(mut d) = state.studio.dialogs.new_map.take() else {
        return;
    };
    let mut open = true;
    let mut close = false;
    let mut create = false;
    let lvl_types: Vec<u32> = state
        .catalog
        .ready()
        .map(|c| c.lvl_type_ids())
        .unwrap_or_default();
    egui::Window::new("New map")
        .id(Id::new("studio_new_map"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos([520.0, 200.0])
        .show(ctx, |ui| {
            egui::Grid::new("new_map_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Game path");
                    ui.add(egui::TextEdit::singleline(&mut d.path).desired_width(340.0));
                    ui.end_row();
                    ui.label("Size (cells)");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut d.width).range(1..=1024));
                        ui.label("x");
                        ui.add(egui::DragValue::new(&mut d.height).range(1..=1024));
                    });
                    ui.end_row();
                    ui.label("Act");
                    ui.add(egui::DragValue::new(&mut d.act).range(1..=5));
                    ui.end_row();
                    ui.label("Layers");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut d.floors)
                                .range(1..=2)
                                .suffix(" floor(s)"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut d.walls)
                                .range(1..=4)
                                .suffix(" wall(s)"),
                        );
                        ui.checkbox(&mut d.tag, "tag layer");
                    });
                    ui.end_row();
                    ui.label("LvlType");
                    egui::ComboBox::from_id_salt("new_map_lvltype")
                        .selected_text(d.lvl_type.map_or("Guess from the folder".to_owned(), |t| {
                            format!("LvlType {t}")
                        }))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut d.lvl_type, None, "Guess from the folder");
                            for t in &lvl_types {
                                ui.selectable_value(
                                    &mut d.lvl_type,
                                    Some(*t),
                                    format!("LvlType {t}"),
                                );
                            }
                        });
                    ui.end_row();
                });
            ui.weak("Version 18. Saved at once to capybara\\saves (listed in CHANGED.txt).");
            if let Some(e) = &d.error {
                ui.colored_label(egui::Color32::from_rgb(235, 90, 90), e);
            }
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Create").strong()).clicked() {
                    create = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    if create {
        match create_new_map(state, &d) {
            Ok(()) => close = true,
            Err(e) => d.error = Some(e),
        }
    }
    if open && !close {
        state.studio.dialogs.new_map = Some(d);
    }
}

pub fn show(ctx: &Context, state: &mut AppState) {
    replace_window(ctx, state);
    reroll_window(ctx, state);
    reassign_window(ctx, state);
    new_map_window(ctx, state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_from_scripts() {
        assert_eq!(
            KeyEdit::parse("0/5/2"),
            Some(KeyEdit {
                orientation: 0,
                main: 5,
                sub: 2
            })
        );
        assert_eq!(KeyEdit::parse("13:1/0").map(|k| k.orientation), Some(13));
        assert_eq!(KeyEdit::parse("5/2"), None);
    }

    #[test]
    fn walk_signatures_have_a_bit_per_blocked_subtile() {
        let mut f = [0u8; 25];
        assert_eq!(walk_signature(&f), 0);
        f[0] = 1;
        f[24] = 9;
        f[3] = 2; // light only
        assert_eq!(walk_signature(&f), 1 | (1 << 24));
    }

    #[test]
    fn layers_follow_the_orientation() {
        let mut ds = d2_formats::ds1::Ds1::new(2, 2, 0);
        ds.add_wall_layer().expect("wall");
        assert_eq!(layers_for(&ds, 0), vec![LayerRef::Floor(0)]);
        assert_eq!(layers_for(&ds, 13), vec![LayerRef::Shadow]);
        assert!(
            layers_for(&ds, 3)
                .iter()
                .all(|l| matches!(l, LayerRef::Wall(_)))
        );
    }
}
