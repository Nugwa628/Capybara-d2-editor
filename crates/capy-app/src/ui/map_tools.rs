//! Mouse editing in the Map panel: the tools' gestures and the overlays that
//! show the selection, the grid, objects and the floating paste.
//!
//! All screen-to-cell work goes through [`ViewFrame`], which wraps the view's
//! iso inverse, so a click lands on the right cell at every zoom.

use bevy_egui::egui::{
    self, Color32, Painter, PointerButton, Pos2, Rect, Response, Shape, Stroke, Ui, Vec2,
};
use capy_edit::{CellValue, LayerRef, SelectOp};

use crate::doc::{LayerKey, MapDoc};
use crate::edit::ops::EditOp;
use crate::edit::paste::{PasteTint, tint_cells};
use crate::edit::state::Gesture;
use crate::edit::tools::{BoxDrag, LassoPath, Mods, Tool, scope_layers, select_op, stroke_cells};
use crate::iso::{self, View};
use crate::objects::ObjectMode;
use crate::state::AppState;

/// Distance (points) within which a click grabs an object marker.
const OBJECT_GRAB: f32 = 12.0;
/// Sub-tiles per tile edge in object coordinates.
const SUB: f64 = 5.0;

/// The Map panel's mapping between screen points and the map.
#[derive(Clone, Copy, Debug)]
pub struct ViewFrame {
    pub rect: Rect,
    pub ppp: f32,
    pub view: View,
}

impl ViewFrame {
    fn offset(&self, pos: Pos2) -> [f64; 2] {
        let d = pos - self.rect.center();
        [f64::from(d.x * self.ppp), f64::from(d.y * self.ppp)]
    }

    /// Iso point under a screen point.
    pub fn iso_at(&self, pos: Pos2) -> [f64; 2] {
        self.view.offset_to_iso(self.offset(pos))
    }

    pub fn screen_of_iso(&self, iso_pt: [f64; 2]) -> Pos2 {
        let o = self.view.iso_to_offset(iso_pt);
        self.rect.center() + Vec2::new(o[0] as f32 / self.ppp, o[1] as f32 / self.ppp)
    }

    /// Screen position of a continuous tile-space point.
    pub fn tile_to_screen(&self, u: f64, v: f64) -> Pos2 {
        self.screen_of_iso(iso::tile_to_iso(u, v))
    }

    /// Continuous tile-space point under a screen point.
    pub fn screen_to_tile(&self, pos: Pos2) -> (f64, f64) {
        iso::iso_to_tile(self.iso_at(pos))
    }

    /// The cell under a screen point (not bounds-checked).
    pub fn cell_at(&self, pos: Pos2) -> (i32, i32) {
        iso::screen_to_cell(self.iso_at(pos))
    }

    /// Corners of cell `(x, y)`: top, right, bottom, left.
    pub fn diamond(&self, x: i32, y: i32) -> [Pos2; 4] {
        let (u, v) = (f64::from(x), f64::from(y));
        [
            self.tile_to_screen(u, v),
            self.tile_to_screen(u + 1.0, v),
            self.tile_to_screen(u + 1.0, v + 1.0),
            self.tile_to_screen(u, v + 1.0),
        ]
    }

    /// Width of one cell on screen, in points.
    pub fn cell_points(&self) -> f32 {
        (iso::TILE_W / self.view.divisor()) as f32 / self.ppp
    }
}

fn layer_color(layer: LayerRef, alpha: u8) -> Color32 {
    let (r, g, b) = match layer {
        LayerRef::Floor(_) => (80, 210, 120),
        LayerRef::Wall(_) => (70, 150, 255),
        LayerRef::Shadow => (175, 120, 255),
        LayerRef::Tag => (255, 90, 220),
    };
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

fn mods_of(i: &egui::InputState) -> Mods {
    Mods {
        shift: i.modifiers.shift,
        ctrl: i.modifiers.ctrl,
        alt: i.modifiers.alt,
    }
}

/// The layers a selection tool works on in `doc`.
fn scope_of(state_scope: crate::edit::tools::SelectScope, doc: &MapDoc) -> Vec<LayerRef> {
    scope_layers(
        state_scope,
        doc.active_layer,
        &LayerRef::all_in(&doc.ds1),
        |l| doc.is_visible(LayerKey::from_ref(l)),
    )
}

// ───────────────────────── input ─────────────────────────

/// Handle the primary button for the active tool (or the paste preview).
pub fn handle_input(
    ui: &mut Ui,
    state: &mut AppState,
    f: &ViewFrame,
    response: &Response,
    panning: bool,
) {
    let Some(index) = state.active else { return };
    let doc_id = state.docs[index].id;
    if let Some(g) = &state.edit.gesture
        && g.doc() != doc_id
    {
        end_gesture(state, false);
    }
    let (pressed, down, released, mods, pos, space) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.primary_down(),
            i.pointer.button_released(PointerButton::Primary),
            mods_of(i),
            i.pointer.latest_pos(),
            i.key_down(egui::Key::Space),
        )
    });
    if state.edit.modal_open() {
        return;
    }
    // A history snapshot shown in place of the map is read-only.
    if state.docs[index].snapshot_view.is_some() {
        if state.edit.gesture.is_some() {
            end_gesture(state, false);
        }
        return;
    }

    // The floating paste owns the pointer: it follows the mouse, a click
    // commits it.
    if state.edit.paste.is_some() {
        if let Some(p) = response.hover_pos()
            && !panning
        {
            let cell = f.cell_at(p);
            if let Some(s) = &mut state.edit.paste {
                s.at = cell;
            }
        }
        if response.clicked_by(PointerButton::Primary) && !space {
            state.commit_paste();
        }
        return;
    }

    let tool = state.edit.tool;
    let start = pressed && response.hovered() && !space && !panning;
    if start && state.edit.gesture.is_none() {
        if let Some(p) = pos {
            begin_gesture(state, index, f, p, mods, tool);
        }
    } else if state.edit.gesture.is_some() {
        if let Some(p) = pos {
            update_gesture(state, index, f, p);
        }
        if released || !down {
            end_gesture(state, true);
        }
    }
}

fn begin_gesture(
    state: &mut AppState,
    index: usize,
    f: &ViewFrame,
    p: Pos2,
    mods: Mods,
    tool: Tool,
) {
    let cell = f.cell_at(p);
    let op = select_op(mods);
    let scope = state.edit.scope;
    let doc_id = state.docs[index].id;
    match tool {
        Tool::SelectBox => {
            state.edit.gesture = Some(Gesture::Box {
                doc: doc_id,
                drag: BoxDrag::new(cell),
                op,
            });
        }
        Tool::Lasso => {
            let (u, v) = f.screen_to_tile(p);
            state.edit.gesture = Some(Gesture::Lasso {
                doc: doc_id,
                path: LassoPath::new((u as f32, v as f32)),
                op,
            });
        }
        Tool::SelectById => {
            let doc = &mut state.docs[index];
            let layers = scope_of(scope, doc);
            let seed = seed_cell(doc, &layers, cell);
            match seed {
                Some((layer, x, y)) => {
                    let value = layer.get(&doc.ds1, x, y).unwrap_or_default();
                    let c = d2_formats::ds1::Cell(value.cell);
                    doc.selection.by_tile_id(
                        &doc.ds1,
                        c.main_index(),
                        c.sub_index(),
                        None,
                        &layers,
                        op,
                    );
                }
                None => clear_if_replace(doc, op),
            }
        }
        Tool::SelectIdentical => {
            let doc = &mut state.docs[index];
            let layers = scope_of(scope, doc);
            match seed_cell(doc, &layers, cell) {
                Some(seed) => doc.selection.identical(&doc.ds1, seed, &layers, op),
                None => clear_if_replace(doc, op),
            }
        }
        Tool::Eyedropper => eyedrop(state, index, cell),
        Tool::Paint | Tool::Erase => {
            let erase = tool == Tool::Erase;
            let layer = state.docs[index].active_layer;
            if !erase && layer.is_empty_value(state.edit.current) {
                state.notify("Pick a tile first: use the Eyedropper (I) on the map");
                return;
            }
            state.docs[index]
                .edit
                .begin_group(if erase { "Erase" } else { "Paint" });
            state.edit.gesture = Some(Gesture::Brush {
                doc: doc_id,
                layer,
                last: cell,
                erase,
            });
            if !stroke(state, index, layer, None, cell, erase) {
                end_gesture(state, false);
            } else if !erase {
                let value = state.edit.current;
                state.note_paint(index, layer, value);
            }
        }
        Tool::Objects => match state.objects.mode {
            ObjectMode::Select => begin_object_gesture(state, index, f, p, mods),
            ObjectMode::Place => place_at(state, index, f, p),
            ObjectMode::Path => begin_path_gesture(state, index, f, p, mods),
        },
    }
}

fn clear_if_replace(doc: &mut MapDoc, op: SelectOp) {
    if op == SelectOp::Replace {
        doc.selection.clear();
    }
}

/// The cell a click-to-select tool reads: the active layer's cell when it
/// holds a tile, else the first layer in scope that does.
fn seed_cell(doc: &MapDoc, layers: &[LayerRef], cell: (i32, i32)) -> Option<(LayerRef, u32, u32)> {
    let (x, y) = (u32::try_from(cell.0).ok()?, u32::try_from(cell.1).ok()?);
    let candidates = std::iter::once(doc.active_layer).chain(layers.iter().rev().copied());
    for layer in candidates {
        if layer == LayerRef::Tag && layer != doc.active_layer {
            continue;
        }
        if let Some(v) = layer.get(&doc.ds1, x, y)
            && !layer.is_empty_value(v)
        {
            return Some((layer, x, y));
        }
    }
    None
}

fn eyedrop(state: &mut AppState, index: usize, cell: (i32, i32)) {
    let doc = &mut state.docs[index];
    let (Ok(x), Ok(y)) = (u32::try_from(cell.0), u32::try_from(cell.1)) else {
        return;
    };
    let all = LayerRef::all_in(&doc.ds1);
    let candidates = std::iter::once(doc.active_layer).chain(all.into_iter().rev());
    let mut found: Option<(LayerRef, CellValue)> = None;
    for layer in candidates {
        if let Some(v) = layer.get(&doc.ds1, x, y)
            && !layer.is_empty_value(v)
        {
            found = Some((layer, v));
            break;
        }
    }
    match found {
        Some((layer, value)) => {
            doc.active_layer = layer;
            state.edit.pick(layer, value);
        }
        None => state.notify("That cell is empty"),
    }
}

fn update_gesture(state: &mut AppState, index: usize, f: &ViewFrame, p: Pos2) {
    let cell = f.cell_at(p);
    let tile = f.screen_to_tile(p);
    let Some(g) = &mut state.edit.gesture else {
        return;
    };
    match g {
        Gesture::Box { drag, .. } => drag.update(cell),
        Gesture::Lasso { path, .. } => {
            path.push((tile.0 as f32, tile.1 as f32));
        }
        Gesture::Objects { current, .. } => *current = tile,
        Gesture::PathNode { .. } => state.objects.path.drag_to(subtile_at(f, p)),
        Gesture::Brush {
            layer, last, erase, ..
        } => {
            let (layer, from, erase) = (*layer, *last, *erase);
            if from == cell {
                return;
            }
            *last = cell;
            if !stroke(state, index, layer, Some(from), cell, erase) {
                end_gesture(state, false);
            }
        }
    }
}

/// One brush segment. False when the edit failed (the stroke stops).
fn stroke(
    state: &mut AppState,
    index: usize,
    layer: LayerRef,
    from: Option<(i32, i32)>,
    to: (i32, i32),
    erase: bool,
) -> bool {
    let doc = &state.docs[index];
    let cells = stroke_cells(from, to, state.edit.brush, doc.ds1.width, doc.ds1.height);
    if cells.is_empty() {
        return true;
    }
    if !erase && let Some(op) = super::studio::paint::freehand_mix(state, layer, &cells) {
        return state.exec(index, op);
    }
    let value = if erase {
        CellValue::EMPTY
    } else {
        state.edit.current
    };
    let cells = cells.into_iter().map(|(x, y)| (x, y, value)).collect();
    state.exec(index, EditOp::Paint { layer, cells })
}

/// Finish (or drop) the gesture in progress.
pub fn end_gesture(state: &mut AppState, apply: bool) {
    let Some(g) = state.edit.gesture.take() else {
        return;
    };
    let Some(index) = state.doc_index(g.doc()) else {
        return;
    };
    let scope = state.edit.scope;
    match g {
        Gesture::Brush { .. } => state.docs[index].edit.end_group(),
        Gesture::Box { drag, op, .. } if apply => {
            let doc = &mut state.docs[index];
            let layers = scope_of(scope, doc);
            match drag.rect(doc.ds1.width, doc.ds1.height) {
                Some(rect) => doc.selection.box_select(&doc.ds1, rect, &layers, false, op),
                None => clear_if_replace(doc, op),
            }
        }
        Gesture::Lasso { path, op, .. } if apply => {
            let doc = &mut state.docs[index];
            let layers = scope_of(scope, doc);
            if path.is_polygon() {
                doc.selection.lasso(&doc.ds1, path.points(), &layers, op);
            } else {
                clear_if_replace(doc, op);
            }
        }
        Gesture::Objects {
            start,
            current,
            indices,
            ..
        } if apply => {
            let dx = ((current.0 - start.0) * SUB).round() as i32;
            let dy = ((current.1 - start.1) * SUB).round() as i32;
            if (dx, dy) != (0, 0) {
                state.exec(index, EditOp::MoveObjects { indices, dx, dy });
            }
        }
        Gesture::PathNode { .. } => {
            if let Some(cmd) = state.objects.path.release()
                && apply
            {
                state.run_path_command(cmd);
            }
        }
        _ => {}
    }
}

/// Escape: drop the gesture in progress. True when there was one.
pub fn cancel_gesture(state: &mut AppState) -> bool {
    if state.edit.gesture.is_none() {
        return false;
    }
    end_gesture(state, false);
    true
}

// ───────────────────────── objects ─────────────────────────

/// Continuous tile-space position of an object's marker (the middle of its
/// sub-tile).
pub fn object_tile_pos(x: i32, y: i32) -> (f64, f64) {
    ((f64::from(x) + 0.5) / SUB, (f64::from(y) + 0.5) / SUB)
}

/// The sub-tile under a screen point (objects and path points live on
/// sub-tiles, 5 per cell edge).
pub fn subtile_at(f: &ViewFrame, p: Pos2) -> (i32, i32) {
    let (u, v) = f.screen_to_tile(p);
    ((u * SUB).floor() as i32, (v * SUB).floor() as i32)
}

/// Screen rectangle of a 1:1 object box from the renderer.
fn box_on_screen(f: &ViewFrame, b: [i32; 4]) -> Rect {
    let shift = f64::from(crate::render::WORLD_SHIFT_X);
    let a = f.screen_of_iso([f64::from(b[0]) + shift, f64::from(b[1])]);
    let c = f.screen_of_iso([f64::from(b[2]) + shift, f64::from(b[3])]);
    Rect::from_two_pos(a, c)
}

/// The object under a screen point: a marker within grab distance first,
/// else the topmost drawn sprite whose box holds the point.
fn object_at(doc: &MapDoc, f: &ViewFrame, p: Pos2) -> Option<usize> {
    let marker = doc
        .ds1
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let (u, v) = object_tile_pos(o.x, o.y);
            (i, f.tile_to_screen(u, v).distance(p))
        })
        .filter(|&(_, d)| d <= OBJECT_GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i);
    marker.or_else(|| {
        doc.render
            .object_boxes
            .iter()
            .rev()
            .find(|(i, b)| *i < doc.ds1.objects.len() && box_on_screen(f, *b).contains(p))
            .map(|(i, _)| *i)
    })
}

fn begin_object_gesture(state: &mut AppState, index: usize, f: &ViewFrame, p: Pos2, mods: Mods) {
    let doc = &mut state.docs[index];
    if !doc.objects_visible {
        return;
    }
    match object_at(doc, f, p) {
        Some(i) => {
            if mods.shift {
                if let Some(pos) = doc.object_sel.iter().position(|&s| s == i) {
                    doc.object_sel.remove(pos);
                    return;
                }
                doc.object_sel.push(i);
            } else if !doc.object_sel.contains(&i) {
                doc.object_sel = vec![i];
            }
            let tile = f.screen_to_tile(p);
            let indices = doc.object_sel.clone();
            let doc_id = doc.id;
            state.edit.gesture = Some(Gesture::Objects {
                doc: doc_id,
                start: tile,
                current: tile,
                indices,
            });
        }
        None => {
            if !mods.shift {
                doc.object_sel.clear();
            }
        }
    }
}

/// "Add": place the picker's choice on the sub-tile under the cursor.
fn place_at(state: &mut AppState, index: usize, f: &ViewFrame, p: Pos2) {
    let Some((kind, id)) = state.objects.picker.chosen else {
        state.notify("Choose an object in the Objects panel first");
        state.objects.focus = true;
        return;
    };
    let (x, y) = subtile_at(f, p);
    let doc = &state.docs[index];
    let (w, h) = (i64::from(doc.ds1.width) * 5, i64::from(doc.ds1.height) * 5);
    if x < 0 || y < 0 || i64::from(x) >= w || i64::from(y) >= h {
        state.notify("That is outside the map");
        return;
    }
    if !doc.objects_visible
        && let Some(doc) = state.docs.get_mut(index)
    {
        doc.objects_visible = true;
    }
    state.place_object(index, kind, id, x, y);
}

/// The path node under a screen point.
fn path_node_at(points: &[d2_formats::ds1::PathPoint], f: &ViewFrame, p: Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let (u, v) = object_tile_pos(q.x, q.y);
            (i, f.tile_to_screen(u, v).distance(p))
        })
        .filter(|&(_, d)| d <= OBJECT_GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// "Path": with one object selected, a click adds a point or grabs a node;
/// without, it selects an object.
fn begin_path_gesture(state: &mut AppState, index: usize, f: &ViewFrame, p: Pos2, mods: Mods) {
    let Some((_, object)) = state.single_object() else {
        begin_object_gesture(state, index, f, p, mods);
        state.edit.gesture = None; // select only, no drag
        return;
    };
    state.objects.path.follow_object(Some(object));
    let points = state.selected_path();
    let hit = path_node_at(&points, f, p);
    let at = subtile_at(f, p);
    match state.objects.path.press(hit, at, points.len()) {
        Some(cmd) => {
            state.run_path_command(cmd);
        }
        None => {
            let doc = state.docs[index].id;
            state.edit.gesture = Some(Gesture::PathNode { doc });
        }
    }
}

// ───────────────────────── overlays ─────────────────────────

/// Draw the grid, selection, objects, gesture feedback and paste preview.
pub fn paint_overlays(ui: &Ui, state: &mut AppState, f: &ViewFrame) {
    let painter = ui.painter_at(f.rect);
    if state.active_doc().is_none() {
        return;
    }
    if state.edit.grid {
        draw_grid(&painter, state, f);
    }
    draw_history_diff(&painter, state, f);
    draw_selection(&painter, state, f);
    draw_objects(&painter, state, f);
    draw_gesture(&painter, state, f);
    draw_brush_cursor(&painter, state, f);
    draw_paste(&painter, state, f);
}

/// Cells whose diamond may touch `rect` (a cheap bounding test on centres).
fn on_screen(f: &ViewFrame, x: u32, y: u32, margin: f32) -> bool {
    let c = f.tile_to_screen(f64::from(x) + 0.5, f64::from(y) + 0.5);
    f.rect.expand(margin).contains(c)
}

fn draw_grid(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    if f.cell_points() < 6.0 {
        return; // lines would be a haze
    }
    // The cells under the viewport corners bound the visible range.
    let corners = [
        f.rect.left_top(),
        f.rect.right_top(),
        f.rect.left_bottom(),
        f.rect.right_bottom(),
    ];
    let (mut u0, mut v0, mut u1, mut v1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in corners {
        let (u, v) = f.screen_to_tile(c);
        u0 = u0.min(u);
        u1 = u1.max(u);
        v0 = v0.min(v);
        v1 = v1.max(v);
    }
    let (w, h) = (f64::from(doc.ds1.width), f64::from(doc.ds1.height));
    let (u0, u1) = (u0.floor().clamp(0.0, w), u1.ceil().clamp(0.0, w));
    let (v0, v1) = (v0.floor().clamp(0.0, h), v1.ceil().clamp(0.0, h));
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 46));
    let mut u = u0;
    while u <= u1 {
        painter.line_segment([f.tile_to_screen(u, v0), f.tile_to_screen(u, v1)], stroke);
        u += 1.0;
    }
    let mut v = v0;
    while v <= v1 {
        painter.line_segment([f.tile_to_screen(u0, v), f.tile_to_screen(u1, v)], stroke);
        v += 1.0;
    }
}

/// Fills `cells` (each with its colour) as one mesh, and outlines them when
/// they are big enough on screen to read.
fn fill_cells(
    painter: &Painter,
    f: &ViewFrame,
    cells: &[((u32, u32), Color32)],
    outline: Option<Color32>,
) {
    let mut mesh = egui::epaint::Mesh::default();
    let outlines = f.cell_points() >= 14.0 && outline.is_some();
    for &((x, y), color) in cells {
        let d = f.diamond(x as i32, y as i32);
        let base = mesh.vertices.len() as u32;
        for p in d {
            mesh.colored_vertex(p, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
        if outlines && let Some(c) = outline {
            painter.add(Shape::closed_line(d.to_vec(), Stroke::new(1.0, c)));
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn draw_selection(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    if doc.selection.is_empty() {
        return;
    }
    let cells: Vec<((u32, u32), Color32)> = doc
        .selection
        .iter()
        .filter(|&(_, x, y)| on_screen(f, x, y, 160.0))
        .map(|(layer, x, y)| ((x, y), layer_color(layer, 70)))
        .collect();
    fill_cells(
        painter,
        f,
        &cells,
        Some(Color32::from_rgba_unmultiplied(255, 255, 255, 190)),
    );
}

/// The diff against the History panel's selected snapshot: green added, red
/// removed, yellow changed cells, and rings for objects added or removed.
fn draw_history_diff(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    if !state.history.overlay_visible {
        return;
    }
    let Some(overlay) = state.history.overlay_for(doc.id) else {
        return;
    };
    let hidden = &state.history.overlay_hidden;
    let cells: Vec<((u32, u32), Color32)> = overlay
        .cells
        .iter()
        .filter(|c| !hidden.contains(&c.layer) && on_screen(f, c.x, c.y, 160.0))
        .map(|c| {
            let [r, g, b, a] = c.change.rgba();
            ((c.x, c.y), Color32::from_rgba_unmultiplied(r, g, b, a))
        })
        .collect();
    fill_cells(
        painter,
        f,
        &cells,
        Some(Color32::from_rgba_unmultiplied(255, 255, 255, 70)),
    );
    let r = marker_radius(f) + 4.0;
    for o in &overlay.objects {
        let (u, v) = object_tile_pos(o.x, o.y);
        let c = f.tile_to_screen(u, v);
        if !f.rect.expand(r).contains(c) {
            continue;
        }
        let [cr, cg, cb, _] = o.change.rgba();
        painter.circle_stroke(c, r, Stroke::new(2.5, Color32::from_rgb(cr, cg, cb)));
    }
}

/// Half-width of an object marker, in points.
fn marker_radius(f: &ViewFrame) -> f32 {
    (16.0 / f.view.divisor() as f32 / f.ppp).clamp(4.0, 12.0)
}

fn diamond_points(c: Pos2, r: f32) -> Vec<Pos2> {
    vec![
        c + Vec2::new(0.0, -r * 0.6),
        c + Vec2::new(r, 0.0),
        c + Vec2::new(0.0, r * 0.6),
        c + Vec2::new(-r, 0.0),
    ]
}

/// Object handles: the diamond markers (with the Objects tool, with View >
/// Object markers, or for objects that draw no sprite), the outline of the
/// selected objects' sprites, the selected object's NPC path and, while
/// placing, the sub-tile under the cursor.
fn draw_objects(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    if !doc.objects_visible {
        return;
    }
    let r = marker_radius(f);
    let objects_tool = state.edit.tool == Tool::Objects;
    let sprites_drawn = doc.render.objects_epoch.is_some();
    let all_markers = objects_tool || state.config.object_markers || !sprites_drawn;
    let missing: std::collections::HashSet<usize> =
        doc.render.missing_objects.iter().map(|(i, _)| *i).collect();
    // While dragging, the selected markers show where they would land.
    let drag = match &state.edit.gesture {
        Some(Gesture::Objects { start, current, .. }) => {
            Some((current.0 - start.0, current.1 - start.1))
        }
        _ => None,
    };
    // Outline the selected objects' sprites.
    let outline = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 235, 120, 200));
    for (i, b) in doc.render.object_boxes.iter() {
        if doc.object_sel.contains(i) && drag.is_none() {
            painter.rect_stroke(
                box_on_screen(f, *b),
                0.0,
                outline,
                egui::StrokeKind::Outside,
            );
        }
    }
    for (i, o) in doc.ds1.objects.iter().enumerate() {
        let selected = doc.object_sel.contains(&i);
        let no_sprite = missing.contains(&i);
        if !(all_markers || selected || no_sprite) {
            continue;
        }
        let (mut u, mut v) = object_tile_pos(o.x, o.y);
        if let (true, Some((du, dv))) = (selected, drag) {
            let (du, dv) = ((du * SUB).round() / SUB, (dv * SUB).round() / SUB);
            u += du;
            v += dv;
        }
        let c = f.tile_to_screen(u, v);
        if !f.rect.expand(r).contains(c) {
            continue;
        }
        let (fill, edge) = if selected {
            (
                Color32::from_rgb(255, 235, 120),
                Stroke::new(1.5, Color32::WHITE),
            )
        } else if no_sprite {
            (
                Color32::from_rgb(200, 90, 230),
                Stroke::new(1.0, Color32::from_rgb(40, 0, 60)),
            )
        } else {
            (
                Color32::from_rgb(235, 140, 40),
                Stroke::new(1.0, Color32::from_rgb(60, 30, 0)),
            )
        };
        painter.add(Shape::convex_polygon(diamond_points(c, r), fill, edge));
    }
    draw_path(painter, state, f);
    if objects_tool
        && state.objects.mode == ObjectMode::Place
        && state.edit.gesture.is_none()
        && let Some(pos) = state.pointer
        && f.rect.contains(pos)
    {
        let (x, y) = subtile_at(f, pos);
        let (u, v) = object_tile_pos(x, y);
        let c = f.tile_to_screen(u, v);
        let ghost = Color32::from_rgba_unmultiplied(120, 255, 160, 150);
        painter.add(Shape::convex_polygon(
            diamond_points(c, r),
            ghost,
            Stroke::new(1.0, Color32::WHITE),
        ));
        painter.text(
            c + Vec2::new(r + 4.0, -r),
            egui::Align2::LEFT_BOTTOM,
            format!("{x}, {y}"),
            egui::FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
}

/// The selected object's NPC path: lines from the object through the
/// numbered nodes.
fn draw_path(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let Some((index, object)) = state.single_object() else {
        return;
    };
    let doc = &state.docs[index];
    let Some(path) = doc.edit.path_of(object) else {
        return;
    };
    let Some(owner) = doc.ds1.objects.get(object) else {
        return;
    };
    let editing = state.edit.tool == Tool::Objects && state.objects.mode == ObjectMode::Path;
    let tool = &state.objects.path;
    let screen = |x: i32, y: i32| {
        let (u, v) = object_tile_pos(x, y);
        f.tile_to_screen(u, v)
    };
    let nodes: Vec<Pos2> = path
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (x, y) = tool.dragged(i).filter(|_| editing).unwrap_or((p.x, p.y));
            screen(x, y)
        })
        .collect();
    let line = Stroke::new(2.0, Color32::from_rgb(90, 200, 255));
    let mut prev = screen(owner.x, owner.y);
    for &n in &nodes {
        painter.line_segment([prev, n], line);
        prev = n;
    }
    let r = (marker_radius(f) * 0.9).max(6.0);
    for (i, (&n, p)) in nodes.iter().zip(&path.points).enumerate() {
        let selected = editing && tool.selected == Some(i);
        let fill = if selected {
            Color32::from_rgb(255, 235, 120)
        } else {
            Color32::from_rgb(30, 110, 170)
        };
        painter.circle(n, r, fill, Stroke::new(1.5, Color32::WHITE));
        painter.text(
            n,
            egui::Align2::CENTER_CENTER,
            format!("{}", i + 1),
            egui::FontId::monospace(r.max(9.0)),
            if selected {
                Color32::BLACK
            } else {
                Color32::WHITE
            },
        );
        if p.action != 1 {
            painter.text(
                n + Vec2::new(r + 2.0, r),
                egui::Align2::LEFT_TOP,
                format!("a{}", p.action),
                egui::FontId::monospace(10.0),
                Color32::from_rgb(180, 230, 255),
            );
        }
    }
}

fn draw_gesture(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let stroke = Stroke::new(1.5, Color32::from_rgb(255, 255, 255));
    let fill = Color32::from_rgba_unmultiplied(255, 255, 255, 28);
    match &state.edit.gesture {
        Some(Gesture::Box { drag, op, .. }) => {
            let (x0, x1) = (
                drag.start.0.min(drag.current.0),
                drag.start.0.max(drag.current.0),
            );
            let (y0, y1) = (
                drag.start.1.min(drag.current.1),
                drag.start.1.max(drag.current.1),
            );
            let (u0, v0) = (f64::from(x0), f64::from(y0));
            let (u1, v1) = (f64::from(x1) + 1.0, f64::from(y1) + 1.0);
            let pts = vec![
                f.tile_to_screen(u0, v0),
                f.tile_to_screen(u1, v0),
                f.tile_to_screen(u1, v1),
                f.tile_to_screen(u0, v1),
            ];
            let tint = match op {
                SelectOp::Subtract => Color32::from_rgba_unmultiplied(255, 80, 80, 40),
                SelectOp::Intersect => Color32::from_rgba_unmultiplied(255, 200, 60, 40),
                _ => fill,
            };
            painter.add(Shape::convex_polygon(pts, tint, stroke));
            let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
            painter.text(
                f.tile_to_screen(f64::from(x1) + 1.0, f64::from(y1) + 1.0) + Vec2::new(8.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!("{w} x {h}"),
                egui::FontId::monospace(12.0),
                Color32::WHITE,
            );
        }
        Some(Gesture::Lasso { path, .. }) => {
            let pts: Vec<Pos2> = path
                .points()
                .iter()
                .map(|&(u, v)| f.tile_to_screen(f64::from(u), f64::from(v)))
                .collect();
            painter.add(Shape::line(pts.clone(), stroke));
            if let (Some(first), Some(last)) = (pts.first(), pts.last()) {
                painter.add(Shape::dashed_line(&[*last, *first], stroke, 6.0, 4.0));
            }
        }
        _ => {}
    }
}

fn draw_brush_cursor(painter: &Painter, state: &AppState, f: &ViewFrame) {
    if !state.edit.tool.brushes() || state.edit.paste.is_some() {
        return;
    }
    let (Some(doc), Some((cx, cy))) = (state.active_doc(), state.hover_cell) else {
        return;
    };
    let cells =
        crate::edit::tools::brush_cells((cx, cy), state.edit.brush, doc.ds1.width, doc.ds1.height);
    let color = if state.edit.tool == Tool::Erase {
        Color32::from_rgba_unmultiplied(255, 90, 90, 60)
    } else {
        Color32::from_rgba_unmultiplied(120, 255, 140, 60)
    };
    let cells: Vec<((u32, u32), Color32)> = cells.into_iter().map(|c| (c, color)).collect();
    fill_cells(painter, f, &cells, None);
}

fn draw_paste(painter: &Painter, state: &mut AppState, f: &ViewFrame) {
    let Some(preview) = state.paste_preview() else {
        return;
    };
    let (Some(session), Some(doc)) = (&state.edit.paste, state.active_doc()) else {
        return;
    };
    let tints = tint_cells(
        &session.clip,
        session.at,
        &preview,
        doc.ds1.width,
        doc.ds1.height,
    );
    let cells: Vec<((u32, u32), Color32)> = tints
        .into_iter()
        .map(|(cell, tint)| {
            let color = match tint {
                PasteTint::Write => Color32::from_rgba_unmultiplied(60, 220, 90, 110),
                PasteTint::Conflict => Color32::from_rgba_unmultiplied(235, 60, 60, 130),
            };
            (cell, color)
        })
        .collect();
    fill_cells(
        painter,
        f,
        &cells,
        Some(Color32::from_rgba_unmultiplied(255, 255, 255, 120)),
    );
    // The clip's bounding box.
    let (u0, v0) = (f64::from(session.at.0), f64::from(session.at.1));
    let (u1, v1) = (
        u0 + f64::from(session.clip.width),
        v0 + f64::from(session.clip.height),
    );
    painter.add(Shape::closed_line(
        vec![
            f.tile_to_screen(u0, v0),
            f.tile_to_screen(u1, v0),
            f.tile_to_screen(u1, v1),
            f.tile_to_screen(u0, v1),
        ],
        Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 200)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::ZOOM_STEPS;

    fn frame(zoom: usize, center: [f64; 2], ppp: f32) -> ViewFrame {
        ViewFrame {
            rect: Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(900.0, 600.0)),
            ppp,
            view: View { center, zoom },
        }
    }

    #[test]
    fn screen_to_cell_round_trips_at_every_zoom() {
        for zoom in 0..ZOOM_STEPS.len() {
            for ppp in [1.0f32, 1.5, 2.0] {
                let f = frame(zoom, [1234.0, 567.0], ppp);
                for (x, y) in [(0, 0), (7, 3), (25, 40), (60, 12), (1, 59)] {
                    let c = f.tile_to_screen(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    assert_eq!(f.cell_at(c), (x, y), "zoom {zoom} ppp {ppp} cell ({x},{y})");
                    // A point a little inside the diamond still hits the cell.
                    let inside = f.tile_to_screen(f64::from(x) + 0.3, f64::from(y) + 0.6);
                    assert_eq!(f.cell_at(inside), (x, y));
                }
            }
        }
    }

    #[test]
    fn diamonds_are_centred_on_their_cell() {
        let f = frame(2, [0.0, 0.0], 1.0);
        let d = f.diamond(4, 6);
        let centre = f.tile_to_screen(4.5, 6.5);
        let mid = Pos2::new((d[0].x + d[2].x) / 2.0, (d[0].y + d[2].y) / 2.0);
        assert!((mid - centre).length() < 1e-3);
        // The diamond is twice as wide as it is tall.
        let w = (d[1].x - d[3].x).abs();
        let h = (d[2].y - d[0].y).abs();
        assert!((w - 2.0 * h).abs() < 1e-3, "{w} vs {h}");
        assert!((f.cell_points() - 160.0 / 3.0).abs() < 1e-3);
    }

    #[test]
    fn objects_sit_in_the_middle_of_their_subtile() {
        assert_eq!(object_tile_pos(0, 0), (0.1, 0.1));
        assert_eq!(object_tile_pos(5, 10), (1.1, 2.1));
        // The marker of sub-tile (7, 13) lies in cell (1, 2).
        let (u, v) = object_tile_pos(7, 13);
        assert_eq!((u.floor() as i32, v.floor() as i32), (1, 2));
    }

    #[test]
    fn a_click_on_a_marker_hits_its_cell_at_any_zoom() {
        for zoom in 0..ZOOM_STEPS.len() {
            let f = frame(zoom, [0.0, 2000.0], 1.0);
            let (u, v) = object_tile_pos(23, 41);
            let screen = f.tile_to_screen(u, v);
            let (bu, bv) = f.screen_to_tile(screen);
            assert!(
                (bu - u).abs() < 1e-3 && (bv - v).abs() < 1e-3,
                "zoom {zoom}"
            );
        }
    }
}
