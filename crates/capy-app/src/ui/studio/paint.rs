//! Map gestures added for ds1-studio parity (P9): Rectangle and Fill area
//! paint/erase, random-mix painting, resizing by dragging a map edge,
//! Shift+wheel through the tiles stacked in a cell and double-click to
//! select every copy of a tile. [`map_input`] runs before the regular tool
//! gestures and says whether it took the pointer.

use bevy_egui::egui::{self, Color32, PointerButton, Pos2, Response, Shape, Stroke, Ui};
use capy_edit::{Brush, CellRect, CellValue, LayerRef, tile_key};

use super::super::map_tools::{ViewFrame, cancel_gesture};
use super::{PaintMode, ViewMode, current_view};
use crate::doc::layer_label;
use crate::edit::ops::EditOp;
use crate::edit::tools::{BoxDrag, Tool};
use crate::state::AppState;

/// How close (in cells) the pointer must be to a map edge to grab it.
pub const EDGE_GRAB: f64 = 0.3;

/// A map edge in tile space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// `u = 0` (the north-west edge on screen).
    Left,
    /// `v = 0`.
    Top,
    /// `u = width`.
    Right,
    /// `v = height`.
    Bottom,
}

/// A drag this module owns.
#[derive(Clone, Debug)]
pub enum Drag {
    Rect {
        doc: u64,
        layer: LayerRef,
        erase: bool,
        drag: BoxDrag,
    },
    Edge {
        doc: u64,
        edge: Edge,
        start: (f64, f64),
        current: (f64, f64),
    },
}

impl Drag {
    fn doc(&self) -> u64 {
        match self {
            Drag::Rect { doc, .. } | Drag::Edge { doc, .. } => *doc,
        }
    }
}

/// The edge near tile-space point `(u, v)` of a `w` x `h` map.
pub fn edge_at(u: f64, v: f64, w: u32, h: u32) -> Option<Edge> {
    let (w, h) = (f64::from(w), f64::from(h));
    let along_u = (-EDGE_GRAB..=w + EDGE_GRAB).contains(&u);
    let along_v = (-EDGE_GRAB..=h + EDGE_GRAB).contains(&v);
    let near = |a: f64, b: f64| (a - b).abs() <= EDGE_GRAB;
    if along_v && near(u, 0.0) {
        Some(Edge::Left)
    } else if along_u && near(v, 0.0) {
        Some(Edge::Top)
    } else if along_v && near(u, w) {
        Some(Edge::Right)
    } else if along_u && near(v, h) {
        Some(Edge::Bottom)
    } else {
        None
    }
}

/// `(left, top, right, bottom)` cells to add (negative: remove) for an edge
/// dragged from `start` to `current` (tile space), as `resize_edges` wants.
pub fn edge_delta(edge: Edge, start: (f64, f64), current: (f64, f64)) -> (i32, i32, i32, i32) {
    let du = (current.0 - start.0).round() as i32;
    let dv = (current.1 - start.1).round() as i32;
    match edge {
        Edge::Left => (-du, 0, 0, 0),
        Edge::Top => (0, -dv, 0, 0),
        Edge::Right => (0, 0, du, 0),
        Edge::Bottom => (0, 0, 0, dv),
    }
}

/// The brush of a cell value picked from a layer of the same kind.
pub fn brush_of(layer: LayerRef, value: CellValue) -> Option<Brush> {
    tile_key(layer, value).map(|k| Brush {
        main: k.main,
        sub: k.sub,
        orientation: value.orientation,
    })
}

fn same_kind(a: LayerRef, b: LayerRef) -> bool {
    matches!(
        (a, b),
        (LayerRef::Floor(_), LayerRef::Floor(_))
            | (LayerRef::Wall(_), LayerRef::Wall(_))
            | (LayerRef::Shadow, LayerRef::Shadow)
    )
}

/// The tiles Paint writes on `layer`: the Ctrl+clicked mix when it holds
/// tiles of that layer kind, else the current value. Empty: erase.
/// `None` (after a toast) when there is nothing to paint.
pub fn brushes(state: &mut AppState, layer: LayerRef, erase: bool) -> Option<Vec<Brush>> {
    if erase {
        return Some(Vec::new());
    }
    let mix: Vec<Brush> = state
        .studio
        .mix
        .iter()
        .filter(|(l, _)| same_kind(*l, layer))
        .filter_map(|&(l, v)| brush_of(l, v))
        .collect();
    if !mix.is_empty() {
        return Some(mix);
    }
    let current = state.edit.current;
    match brush_of(layer, current) {
        Some(b) if state.edit.current_layer.is_none_or(|l| same_kind(l, layer)) => Some(vec![b]),
        _ => {
            state.notify("Pick a tile first: use the Eyedropper (I) on the map");
            None
        }
    }
}

/// Freehand painting with a mix of two or more tiles: the stroke's cells as
/// a random-mix paint. `None`: paint the current value as usual.
pub fn freehand_mix(state: &mut AppState, layer: LayerRef, cells: &[(u32, u32)]) -> Option<EditOp> {
    let mix: Vec<Brush> = state
        .studio
        .mix
        .iter()
        .filter(|(l, _)| same_kind(*l, layer))
        .filter_map(|&(l, v)| brush_of(l, v))
        .collect();
    if mix.len() < 2 {
        return None;
    }
    let seed = state.studio.next_seed();
    Some(EditOp::PaintMix {
        layer,
        cells: cells.to_vec(),
        mix,
        seed,
    })
}

/// Ctrl+click on a tile in the Tiles column: add it to the random mix, or
/// take it out again. The first tile of a mix is also the brush.
pub fn toggle_mix(state: &mut AppState, layer: LayerRef, value: CellValue) {
    let mix = &mut state.studio.mix;
    if let Some(i) = mix.iter().position(|e| *e == (layer, value)) {
        mix.remove(i);
    } else {
        mix.retain(|(l, _)| same_kind(*l, layer));
        mix.push((layer, value));
    }
    let n = mix.len();
    if let Some(&(l, v)) = state.studio.mix.first() {
        state.edit.pick(l, v);
        if !state.edit.tool.brushes() {
            state.edit.tool = Tool::Paint;
        }
    }
    state.notify(match n {
        0 => "Random mix cleared".to_owned(),
        1 => "Random mix: 1 tile (Ctrl+click more tiles to mix them)".to_owned(),
        n => format!("Random mix: Paint picks one of {n} tiles per cell"),
    });
}

/// The ribbon's line about the random mix.
pub fn mix_line(ui: &mut Ui, state: &mut AppState) {
    let n = state.studio.mix.len();
    let muted = state.shell.shades.muted;
    let text = match n {
        0 | 1 => "Ctrl+click tiles to mix".to_owned(),
        n => format!("Mix of {n} tiles"),
    };
    ui.label(egui::RichText::new(text).size(11.0).color(muted))
        .on_hover_text(
            "Ctrl+click several tiles in the Tiles column: Paint, Rectangle and Fill              area then pick one of them at random for every cell",
        );
}

fn mods_of(i: &egui::InputState) -> (bool, bool) {
    (i.modifiers.shift, i.modifiers.ctrl || i.modifiers.command)
}

/// Takes the pointer for this module's gestures. True when it handled the
/// press (the regular tool must not see it).
pub fn map_input(
    ui: &mut Ui,
    state: &mut AppState,
    f: &ViewFrame,
    response: &Response,
    panning: bool,
) -> bool {
    let Some(index) = state.active else {
        return false;
    };
    let doc_id = state.docs[index].id;
    if state
        .studio
        .drag
        .as_ref()
        .is_some_and(|d| d.doc() != doc_id)
    {
        state.studio.drag = None;
    }
    let (pressed, down, released, pos, space, scroll, (shift, _ctrl)) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.primary_down(),
            i.pointer.button_released(PointerButton::Primary),
            i.pointer.latest_pos(),
            i.key_down(egui::Key::Space),
            i.smooth_scroll_delta,
            mods_of(i),
        )
    });
    let hover = response.hover_pos();
    if pressed
        && let Some(p) = hover
        && let Some(cell) = in_map(state, index, f.cell_at(p))
    {
        state.studio.pinned_cell = Some(cell);
    }
    if state.edit.paste.is_some()
        || state.edit.modal_open()
        || state.docs[index].snapshot_view.is_some()
        || panning
    {
        state.studio.drag = None;
        return false;
    }

    // Shift+wheel: step through the tiles stacked in the hovered cell.
    if shift && let Some(p) = hover {
        state.studio.stack_accum += scroll.x + scroll.y;
        let mut steps = 0;
        while state.studio.stack_accum.abs() >= 40.0 {
            steps += state.studio.stack_accum.signum() as i32;
            state.studio.stack_accum -= state.studio.stack_accum.signum() * 40.0;
        }
        if steps != 0
            && let Some(cell) = in_map(state, index, f.cell_at(p))
        {
            step_stack(state, index, cell, steps);
        }
    } else {
        state.studio.stack_accum = 0.0;
    }

    // A drag in progress.
    if state.studio.drag.is_some() {
        if let Some(p) = pos {
            update_drag(state, f, p);
        }
        if released || !down {
            finish_drag(state, index);
        }
        return true;
    }

    // The paint tools of the Automap and Walkability modes.
    if super::modes::input(ui, state, f, response, space) {
        return true;
    }

    let tool = state.edit.tool;
    // Double-click with the Select tool: every copy of the tile.
    if tool == Tool::SelectBox
        && response.double_clicked()
        && let Some(p) = hover
        && let Some(cell) = in_map(state, index, f.cell_at(p))
    {
        cancel_gesture(state);
        select_copies(state, index, cell, shift);
        return true;
    }

    if tool == Tool::SelectBox
        && let Some(p) = hover
        && !space
        && current_view(state) != ViewMode::Objects
    {
        let (u, v) = f.screen_to_tile(p);
        let (w, h) = (state.docs[index].ds1.width, state.docs[index].ds1.height);
        if let Some(edge) = edge_at(u, v, w, h) {
            ui.ctx().set_cursor_icon(match edge {
                Edge::Left | Edge::Right => egui::CursorIcon::ResizeNeSw,
                Edge::Top | Edge::Bottom => egui::CursorIcon::ResizeNwSe,
            });
            if pressed && response.hovered() {
                state.studio.drag = Some(Drag::Edge {
                    doc: doc_id,
                    edge,
                    start: (u, v),
                    current: (u, v),
                });
                return true;
            }
        }
    }

    let paint_mode = state.studio.paint_mode;
    if !tool.brushes() || paint_mode == PaintMode::Freehand {
        return false;
    }
    if !(pressed && response.hovered() && !space) {
        return false;
    }
    let Some(p) = pos else { return false };
    let cell = f.cell_at(p);
    let erase = tool == Tool::Erase;
    let layer = state.docs[index].active_layer;
    if layer == LayerRef::Tag {
        state.notify("Rectangle and Fill area paint tile layers, not the tag layer");
        return true;
    }
    match paint_mode {
        PaintMode::Rect => {
            if brushes(state, layer, erase).is_some() {
                state.studio.stack_rect = ui.input(|i| i.modifiers.alt);
                state.studio.drag = Some(Drag::Rect {
                    doc: doc_id,
                    layer,
                    erase,
                    drag: BoxDrag::new(cell),
                });
            }
        }
        PaintMode::Fill => {
            if let Some((x, y)) = in_map(state, index, cell) {
                fill_at(state, index, layer, (x, y), erase);
            }
        }
        PaintMode::Freehand => {}
    }
    true
}

fn in_map(state: &AppState, index: usize, cell: (i32, i32)) -> Option<(u32, u32)> {
    let d = &state.docs[index].ds1;
    let (x, y) = (u32::try_from(cell.0).ok()?, u32::try_from(cell.1).ok()?);
    (x < d.width && y < d.height).then_some((x, y))
}

fn update_drag(state: &mut AppState, f: &ViewFrame, p: Pos2) {
    match &mut state.studio.drag {
        Some(Drag::Rect { drag, .. }) => drag.update(f.cell_at(p)),
        Some(Drag::Edge { current, .. }) => *current = f.screen_to_tile(p),
        None => {}
    }
}

pub fn finish_drag(state: &mut AppState, index: usize) {
    let Some(drag) = state.studio.drag.take() else {
        return;
    };
    match drag {
        Drag::Rect {
            layer, erase, drag, ..
        } => {
            let (w, h) = (state.docs[index].ds1.width, state.docs[index].ds1.height);
            let Some(rect) = drag.rect(w, h) else { return };
            paint_rect(state, index, layer, rect, erase);
        }
        Drag::Edge {
            edge,
            start,
            current,
            ..
        } => {
            let (left, top, right, bottom) = edge_delta(edge, start, current);
            if (left, top, right, bottom) != (0, 0, 0, 0) {
                state.exec(
                    index,
                    EditOp::ResizeEdges {
                        left,
                        top,
                        right,
                        bottom,
                    },
                );
            }
        }
    }
}

/// Paint (or erase) `rect` on `layer` with the current brush or mix.
pub fn paint_rect(
    state: &mut AppState,
    index: usize,
    layer: LayerRef,
    rect: CellRect,
    erase: bool,
) {
    if std::mem::take(&mut state.studio.stack_rect)
        && !erase
        && super::stack::stackable(layer)
        && !layer.is_empty_value(state.edit.current)
    {
        let cells: Vec<(u32, u32)> = (rect.y0..=rect.y1)
            .flat_map(|y| (rect.x0..=rect.x1).map(move |x| (x, y)))
            .collect();
        state.docs[index]
            .edit
            .begin_group("Paint rectangle (stacked)");
        super::stack::paint_cells(state, index, layer, &cells, &mut Default::default());
        state.docs[index].edit.end_group();
        return;
    }
    let Some(mix) = brushes(state, layer, erase) else {
        return;
    };
    let seed = state.studio.next_seed();
    if state.exec(
        index,
        EditOp::PaintRect {
            layer,
            rect,
            mix,
            seed,
        },
    ) && !erase
    {
        let value = state.edit.current;
        state.note_paint(index, layer, value);
    }
}

/// Fill (or erase) the connected area of the same tile around `cell`,
/// limited to the selection when the cell is selected.
pub fn fill_at(state: &mut AppState, index: usize, layer: LayerRef, cell: (u32, u32), erase: bool) {
    let Some(mix) = brushes(state, layer, erase) else {
        return;
    };
    let in_selection = state.docs[index].selection.contains(layer, cell.0, cell.1);
    let seed = state.studio.next_seed();
    if state.exec(
        index,
        EditOp::FloodFill {
            layer,
            x: cell.0,
            y: cell.1,
            mix,
            in_selection,
            seed,
        },
    ) && !erase
    {
        let value = state.edit.current;
        state.note_paint(index, layer, value);
    }
}

/// Double-click: select every cell holding the same tile (orientation, main,
/// sub) on the layers the selection tools look at.
pub fn select_copies(state: &mut AppState, index: usize, cell: (u32, u32), add: bool) {
    let scope = state.edit.scope;
    let doc = &mut state.docs[index];
    let layers = crate::edit::tools::scope_layers(
        scope,
        doc.active_layer,
        &LayerRef::all_in(&doc.ds1),
        |l| doc.is_visible(crate::doc::LayerKey::from_ref(l)),
    );
    // The tile under the pointer: the active layer's, else the topmost in scope.
    let seed = std::iter::once(doc.active_layer)
        .chain(layers.iter().rev().copied())
        .filter(|l| *l != LayerRef::Tag)
        .find_map(|l| {
            l.get(&doc.ds1, cell.0, cell.1)
                .and_then(|v| tile_key(l, v))
                .map(|k| (l, k))
        });
    let Some((layer, key)) = seed else {
        if !add {
            doc.selection.clear();
        }
        return;
    };
    let same: Vec<LayerRef> = layers
        .iter()
        .copied()
        .filter(|l| same_kind(*l, layer))
        .collect();
    let found = doc.edit.find_tiles(&same, key, None);
    if !add {
        doc.selection.clear();
    }
    for (l, x, y) in &found {
        doc.selection.insert(*l, *x, *y);
    }
    let n = found.len();
    state.notify(format!(
        "Selected {n} cell(s) with tile {}/{} (orientation {})",
        key.main, key.sub, key.orientation
    ));
}

/// Shift+wheel: make the next (or previous) tile stacked in `cell` the
/// active layer and select it there.
pub fn step_stack(state: &mut AppState, index: usize, cell: (u32, u32), steps: i32) {
    let doc = &state.docs[index];
    let stacked: Vec<LayerRef> = LayerRef::all_in(&doc.ds1)
        .into_iter()
        .filter(|l| *l != LayerRef::Tag)
        .filter(|l| {
            l.get(&doc.ds1, cell.0, cell.1)
                .is_some_and(|v| !l.is_empty_value(v))
        })
        .collect();
    if stacked.is_empty() {
        return;
    }
    let n = stacked.len() as i32;
    let now = match state.studio.stack {
        Some((c, i)) if c == cell => i as i32,
        _ => stacked
            .iter()
            .position(|l| *l == doc.active_layer)
            .map_or(-1, |i| i as i32),
    };
    let next = (now + steps).rem_euclid(n) as usize;
    let layer = stacked[next];
    state.studio.stack = Some((cell, next));
    let doc = &mut state.docs[index];
    doc.active_layer = layer;
    doc.selection.clear();
    doc.selection.insert(layer, cell.0, cell.1);
    let value = layer.get(&doc.ds1, cell.0, cell.1).unwrap_or_default();
    let c = d2_formats::ds1::Cell(value.cell);
    state.notify(format!(
        "{} of {n}: {} tile {}/{}",
        next + 1,
        layer_label(layer),
        c.main_index(),
        c.sub_index()
    ));
}

fn diamond_rect(f: &ViewFrame, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Pos2> {
    vec![
        f.tile_to_screen(x0, y0),
        f.tile_to_screen(x1, y0),
        f.tile_to_screen(x1, y1),
        f.tile_to_screen(x0, y1),
    ]
}

/// The rectangle being dragged and the edge being moved.
pub fn paint_overlays(ui: &Ui, state: &AppState, f: &ViewFrame) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    let painter = ui.painter_at(f.rect);
    match &state.studio.drag {
        Some(Drag::Rect { erase, drag, .. }) => {
            if let Some(r) = drag.rect(doc.ds1.width, doc.ds1.height) {
                let pts = diamond_rect(
                    f,
                    f64::from(r.x0),
                    f64::from(r.y0),
                    f64::from(r.x1 + 1),
                    f64::from(r.y1 + 1),
                );
                let (fill, line) = if *erase {
                    (
                        Color32::from_rgba_unmultiplied(255, 90, 90, 50),
                        Color32::from_rgb(255, 120, 120),
                    )
                } else {
                    (
                        Color32::from_rgba_unmultiplied(120, 220, 140, 50),
                        Color32::from_rgb(140, 240, 160),
                    )
                };
                painter.add(Shape::convex_polygon(pts, fill, Stroke::new(1.5, line)));
                let n = (r.x1 - r.x0 + 1) * (r.y1 - r.y0 + 1);
                painter.text(
                    f.tile_to_screen(f64::from(r.x1 + 1), f64::from(r.y1 + 1)),
                    egui::Align2::LEFT_TOP,
                    format!("{} x {} ({n})", r.x1 - r.x0 + 1, r.y1 - r.y0 + 1),
                    egui::FontId::proportional(12.0),
                    line,
                );
            }
        }
        Some(Drag::Edge {
            edge,
            start,
            current,
            ..
        }) => {
            let (l, t, r, b) = edge_delta(*edge, *start, *current);
            let (w, h) = (f64::from(doc.ds1.width), f64::from(doc.ds1.height));
            let pts = diamond_rect(
                f,
                -f64::from(l),
                -f64::from(t),
                w + f64::from(r),
                h + f64::from(b),
            );
            painter.add(Shape::closed_line(
                pts,
                Stroke::new(2.0, Color32::from_rgb(255, 200, 80)),
            ));
            let nw = i64::from(doc.ds1.width) + i64::from(l) + i64::from(r);
            let nh = i64::from(doc.ds1.height) + i64::from(t) + i64::from(b);
            if let Some(p) = state.pointer {
                painter.text(
                    p + egui::vec2(14.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    format!("{nw} x {nh}"),
                    egui::FontId::proportional(13.0),
                    Color32::from_rgb(255, 200, 80),
                );
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_are_found_near_the_four_sides_only() {
        assert_eq!(edge_at(0.1, 5.0, 10, 8), Some(Edge::Left));
        assert_eq!(edge_at(5.0, -0.2, 10, 8), Some(Edge::Top));
        assert_eq!(edge_at(9.8, 5.0, 10, 8), Some(Edge::Right));
        assert_eq!(edge_at(5.0, 8.1, 10, 8), Some(Edge::Bottom));
        assert_eq!(edge_at(5.0, 4.0, 10, 8), None);
        assert_eq!(edge_at(20.0, 8.0, 10, 8), None);
    }

    #[test]
    fn dragging_outwards_grows_and_inwards_shrinks() {
        assert_eq!(
            edge_delta(Edge::Right, (10.0, 3.0), (12.4, 3.0)),
            (0, 0, 2, 0)
        );
        assert_eq!(
            edge_delta(Edge::Left, (0.0, 3.0), (-3.0, 9.0)),
            (3, 0, 0, 0)
        );
        assert_eq!(edge_delta(Edge::Top, (4.0, 0.0), (4.0, 2.0)), (0, -2, 0, 0));
        assert_eq!(
            edge_delta(Edge::Bottom, (4.0, 8.0), (1.0, 7.0)),
            (0, 0, 0, -1)
        );
    }

    #[test]
    fn brushes_keep_the_wall_orientation() {
        let v = CellValue {
            cell: d2_formats::ds1::Cell(0x81).with_indices(5, 2).0,
            orientation: 3,
        };
        let b = brush_of(LayerRef::Wall(0), v).expect("brush");
        assert_eq!((b.main, b.sub, b.orientation), (5, 2, 3));
        assert!(brush_of(LayerRef::Floor(0), CellValue::EMPTY).is_none());
    }
}
