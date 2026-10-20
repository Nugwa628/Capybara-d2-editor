//! The view modes and overlays of P10 (ds1-studio `MapView.tsx`,
//! `WalkPanel.tsx`, `ModePanels.tsx`): walkability and spawn regions as one
//! textured quad over the map (one texel per sub-tile), the automap preview,
//! level light, roof-hiding areas, warps with a jump to their target, the
//! 8x8 game rooms, game-view framing and the walkable-area badge. All
//! numbers come from `capy_render::{walk, rooms, links, pops}`.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use bevy_egui::egui::{
    self, Align2, Color32, ColorImage, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, Vec2,
};
use capy_render::pops::{PopArea, find_pops, pop_targets};
use capy_render::rooms::{
    SpawnClass, SpawnLevel, WalkClass, preset_rooms, spawn_overlay, walkable_overlay,
};
use capy_render::walk::{Collision, area_colour, sub_tile_flags, walkable_area};

use super::super::map_tools::ViewFrame;
use super::{ViewMode, current_view, levels};
use crate::iso::ZOOM_STEPS;
use crate::state::AppState;

/// Per map revision: what the overlays need. Recomputed lazily, never in
/// the middle of a gesture (painting would recompute it every frame).
#[derive(Default)]
pub struct Analysis {
    key: Option<(u64, u64, usize)>,
    /// The walkable-area badge value (tiles squared).
    pub area: Option<f64>,
    walk: Option<TextureHandle>,
    spawn: Option<TextureHandle>,
    automap: Option<TextureHandle>,
    pops: Option<Arc<Vec<PopArea>>>,
    hidden: Option<Arc<Vec<(usize, u32, u32)>>>,
    /// The area last written to the log, per map.
    logged: Option<(u64, u64)>,
}

fn busy(state: &AppState) -> bool {
    state.edit.gesture.is_some() || state.studio.drag.is_some()
}

/// Drops the cache when the map, its revision or its tile set changed.
/// False when it cannot be computed now (no tiles yet, or a gesture).
fn refresh(state: &mut AppState, index: usize) -> bool {
    let Some(doc) = state.docs.get(index) else {
        return false;
    };
    let Some(tiles) = &doc.tiles else {
        return false;
    };
    let key = (doc.id, doc.rev, Arc::as_ptr(tiles) as usize);
    let busy = busy(state);
    if state.studio.analysis.key != Some(key) {
        if busy && state.studio.analysis.key.is_some_and(|k| k.0 == key.0) {
            return true; // keep the last results until the gesture ends
        }
        let a = &mut state.studio.analysis;
        let logged = a.logged;
        *a = Analysis {
            key: Some(key),
            logged,
            ..Analysis::default()
        };
    }
    true
}

/// The walkable area of map `index` (computed once per revision).
pub fn walkable(state: &mut AppState, index: usize) -> Option<f64> {
    if !refresh(state, index) {
        return None;
    }
    if state.studio.analysis.area.is_none() {
        let doc = &state.docs[index];
        let tiles = doc.tiles.clone()?;
        let area = walkable_area(&doc.ds1, &tiles);
        state.studio.analysis.area = Some(area);
        let key = (doc.id, doc.rev);
        if state.studio.analysis.logged != Some(key) {
            state.studio.analysis.logged = Some(key);
            let path = doc.path.clone();
            state.info(format!("Walkable area of {path}: {area:.2} tiles2"));
        }
    }
    state.studio.analysis.area
}

fn pops(state: &mut AppState, index: usize) -> Arc<Vec<PopArea>> {
    let cached = refresh(state, index);
    if cached && let Some(p) = &state.studio.analysis.pops {
        return p.clone();
    }
    let p = Arc::new(find_pops(&state.docs[index].ds1));
    if cached {
        state.studio.analysis.pops = Some(p.clone());
    }
    p
}

/// The wall cells roof-hiding areas hide "as if inside", sorted.
pub fn pop_hidden_cells(state: &mut AppState, index: usize) -> Arc<Vec<(usize, u32, u32)>> {
    let cached = refresh(state, index);
    if cached && let Some(h) = &state.studio.analysis.hidden {
        return h.clone();
    }
    let areas = pops(state, index);
    let ds1 = state.docs[index].ds1.clone();
    let mut cells: Vec<(usize, u32, u32)> = areas
        .iter()
        .flat_map(|a| pop_targets(&ds1, a))
        .map(|t| (t.layer, t.x, t.y))
        .collect();
    cells.sort_unstable();
    cells.dedup();
    let cells = Arc::new(cells);
    if cached {
        state.studio.analysis.hidden = Some(cells.clone());
    }
    cells
}

/// Walkability colours (ds1-studio's walk overview).
pub fn walk_colour(c: WalkClass) -> [u8; 4] {
    match c {
        WalkClass::None => [0, 0, 0, 0],
        WalkClass::Walk => [70, 210, 100, 80],
        WalkClass::MonstersOnly => [250, 200, 50, 130],
        WalkClass::Blocked => [230, 60, 60, 140],
        WalkClass::Void => [170, 70, 255, 150],
        WalkClass::OpenEdge => [255, 130, 0, 190],
    }
}

pub fn spawn_colour(c: SpawnClass) -> [u8; 4] {
    match c {
        SpawnClass::None | SpawnClass::Blocked => [0, 0, 0, 0],
        SpawnClass::Spawn => [60, 170, 255, 90],
        SpawnClass::Node => [255, 220, 60, 120],
        SpawnClass::Warp => [200, 80, 255, 120],
        SpawnClass::Off => [140, 140, 140, 100],
        SpawnClass::Edge => [90, 90, 90, 70],
        SpawnClass::Void => [255, 80, 200, 120],
    }
}

/// A `w*5` x `h*5` texture, one texel per sub-tile.
fn subtile_texture(
    ctx: &egui::Context,
    name: &str,
    w: u32,
    h: u32,
    colour: impl Fn(usize) -> [u8; 4],
) -> TextureHandle {
    let (tw, th) = (w as usize * 5, h as usize * 5);
    let mut px = vec![0u8; tw * th * 4];
    for cy in 0..h as usize {
        for cx in 0..w as usize {
            for sy in 0..5 {
                for sx in 0..5 {
                    let i = (cy * w as usize + cx) * 25 + sy * 5 + sx;
                    let (x, y) = (cx * 5 + sx, cy * 5 + sy);
                    let o = (y * tw + x) * 4;
                    px[o..o + 4].copy_from_slice(&colour(i));
                }
            }
        }
    }
    let image = ColorImage::from_rgba_unmultiplied([tw.max(1), th.max(1)], &px);
    ctx.load_texture(name, image, TextureOptions::NEAREST)
}

fn walk_texture(ctx: &egui::Context, state: &mut AppState, index: usize) -> Option<TextureHandle> {
    if !refresh(state, index) {
        return None;
    }
    if state.studio.analysis.walk.is_none() {
        let doc = &state.docs[index];
        let tiles = doc.tiles.clone()?;
        let grid = sub_tile_flags(&doc.ds1, &tiles, Collision::Game { blank: None });
        let classes = walkable_overlay(&doc.ds1, &grid);
        let t = subtile_texture(ctx, "walk_overlay", doc.ds1.width, doc.ds1.height, |i| {
            classes.get(i).map_or([0; 4], |c| walk_colour(*c))
        });
        state.studio.analysis.walk = Some(t);
    }
    state.studio.analysis.walk.clone()
}

fn spawn_texture(ctx: &egui::Context, state: &mut AppState, index: usize) -> Option<TextureHandle> {
    if !refresh(state, index) {
        return None;
    }
    if state.studio.analysis.spawn.is_none() {
        let doc = &state.docs[index];
        let tiles = doc.tiles.clone()?;
        let (w, h) = (doc.ds1.width, doc.ds1.height);
        let grid = sub_tile_flags(&doc.ds1, &tiles, Collision::Game { blank: None });
        let classes = spawn_overlay(&doc.ds1, &grid, &SpawnLevel::default());
        let per_cell = classes.len() == (w * h) as usize;
        let t = subtile_texture(ctx, "spawn_overlay", w, h, |i| {
            let k = if per_cell { i / 25 } else { i };
            classes.get(k).map_or([0; 4], |c| spawn_colour(*c))
        });
        state.studio.analysis.spawn = Some(t);
    }
    state.studio.analysis.spawn.clone()
}

/// The automap preview: walls light, floors dark, one texel per cell.
fn automap_texture(
    ctx: &egui::Context,
    state: &mut AppState,
    index: usize,
) -> Option<TextureHandle> {
    if !refresh(state, index) {
        return None;
    }
    if state.studio.analysis.automap.is_none() {
        let ds1 = state.docs[index].ds1.clone();
        let (w, h) = (ds1.width as usize, ds1.height as usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (ux, uy) = (x as u32, y as u32);
                let wall = ds1.walls.iter().enumerate().any(|(li, l)| {
                    l.tiles
                        .get(ux, uy)
                        .is_some_and(|c| !c.is_empty() && !c.hidden())
                        && crate::render::job::wall_group(i32::from(
                            ds1.orientation(li, ux, uy).unwrap_or(0),
                        ))
                        .is_some()
                });
                let floor = ds1
                    .floors
                    .iter()
                    .any(|l| l.get(ux, uy).is_some_and(|c| !c.is_empty()));
                let c = if wall {
                    [215, 205, 170, 255]
                } else if floor {
                    [52, 56, 62, 255]
                } else {
                    [0, 0, 0, 0]
                };
                let o = (y * w + x) * 4;
                px[o..o + 4].copy_from_slice(&c);
            }
        }
        let image = ColorImage::from_rgba_unmultiplied([w.max(1), h.max(1)], &px);
        state.studio.analysis.automap =
            Some(ctx.load_texture("automap_preview", image, TextureOptions::NEAREST));
    }
    state.studio.analysis.automap.clone()
}

/// Draws `texture` stretched over the whole map diamond.
fn map_quad(painter: &Painter, f: &ViewFrame, w: u32, h: u32, texture: &TextureHandle) {
    let (fw, fh) = (f64::from(w), f64::from(h));
    let mut mesh = Mesh::with_texture(texture.id());
    for (u, v, uv) in [
        (0.0, 0.0, Pos2::new(0.0, 0.0)),
        (fw, 0.0, Pos2::new(1.0, 0.0)),
        (fw, fh, Pos2::new(1.0, 1.0)),
        (0.0, fh, Pos2::new(0.0, 1.0)),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: f.tile_to_screen(u, v),
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

fn cell_rect_outline(f: &ViewFrame, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Pos2> {
    vec![
        f.tile_to_screen(x0, y0),
        f.tile_to_screen(x1, y0),
        f.tile_to_screen(x1, y1),
        f.tile_to_screen(x0, y1),
    ]
}

// ───────────────────────── game view ─────────────────────────

/// Z: frame what the character sees at the chosen resolution, zoomed so the
/// frame fits.
pub fn toggle_game_view(state: &mut AppState) {
    state.studio.game_view = !state.studio.game_view;
    if !state.studio.game_view {
        return;
    }
    let (gw, gh) = (
        f64::from(state.config.ui.studio.game_width),
        f64::from(state.config.ui.studio.game_height),
    );
    let Some(rect) = state.map_rect else { return };
    let ppp = f64::from(state.pixels_per_point);
    let (vw, vh) = (
        f64::from(rect.width()) * ppp,
        f64::from(rect.height()) * ppp,
    );
    let need = ((gw + 40.0) / vw).max((gh + 40.0) / vh);
    let zoom = ZOOM_STEPS
        .iter()
        .position(|&s| f64::from(s) >= need)
        .unwrap_or(ZOOM_STEPS.len() - 1);
    if let Some(doc) = state.active_doc_mut() {
        doc.view.zoom = zoom;
    }
}

/// Screen resolutions offered for Game view (ds1-studio `GameSizePicker`).
pub const GAME_SIZES: [(u32, u32); 6] = [
    (640, 480),
    (800, 600),
    (1024, 768),
    (1280, 720),
    (1600, 900),
    (1920, 1080),
];

fn draw_game_view(painter: &Painter, state: &AppState, f: &ViewFrame) {
    let (gw, gh) = (
        state.config.ui.studio.game_width as f32,
        state.config.ui.studio.game_height as f32,
    );
    let div = f.view.divisor() as f32;
    let size = Vec2::new(gw / div / f.ppp, gh / div / f.ppp);
    let frame = Rect::from_center_size(f.rect.center(), size);
    let dim = Color32::from_rgba_unmultiplied(0, 0, 0, 150);
    let r = f.rect;
    for band in [
        Rect::from_min_max(r.min, Pos2::new(r.max.x, frame.top())),
        Rect::from_min_max(Pos2::new(r.min.x, frame.bottom()), r.max),
        Rect::from_min_max(
            Pos2::new(r.min.x, frame.top()),
            Pos2::new(frame.left(), frame.bottom()),
        ),
        Rect::from_min_max(
            Pos2::new(frame.right(), frame.top()),
            Pos2::new(r.max.x, frame.bottom()),
        ),
    ] {
        if band.is_positive() {
            painter.rect_filled(band, 0.0, dim);
        }
    }
    painter.rect_stroke(
        frame,
        0.0,
        Stroke::new(1.5, Color32::from_rgb(255, 220, 120)),
        StrokeKind::Outside,
    );
    painter.text(
        frame.left_top() + Vec2::new(6.0, 4.0),
        Align2::LEFT_TOP,
        format!("Game view {gw:.0}\u{d7}{gh:.0}"),
        FontId::proportional(12.0),
        Color32::from_rgb(255, 220, 120),
    );
}

// ───────────────────────── overlays ─────────────────────────

/// Everything P10 draws over the map, under the selection and gestures.
pub fn paint_overlays(ui: &mut Ui, state: &mut AppState, f: &ViewFrame) {
    let Some(index) = state.active else { return };
    let painter = ui.painter_at(f.rect);
    let ctx = ui.ctx().clone();
    let (w, h) = (state.docs[index].ds1.width, state.docs[index].ds1.height);
    let view = current_view(state);
    match view {
        ViewMode::Walk => {
            if let Some(t) = walk_texture(&ctx, state, index) {
                map_quad(&painter, f, w, h, &t);
            }
        }
        ViewMode::Automap => {
            let base = cell_rect_outline(f, 0.0, 0.0, f64::from(w), f64::from(h));
            painter.add(Shape::convex_polygon(
                base,
                Color32::from_rgba_unmultiplied(8, 10, 14, 235),
                Stroke::NONE,
            ));
            if let Some(t) = automap_texture(&ctx, state, index) {
                map_quad(&painter, f, w, h, &t);
            }
        }
        ViewMode::Light => draw_light(&painter, state, index, f),
        ViewMode::Roofs => draw_pops(&painter, state, index, f),
        ViewMode::Tiles | ViewMode::Objects => {}
    }
    if state.config.ui.studio.spawn_regions
        && let Some(t) = spawn_texture(&ctx, state, index)
    {
        map_quad(&painter, f, w, h, &t);
    }
    if state.studio.rooms {
        draw_rooms(&painter, f, w, h);
    }
    if state.config.ui.studio.warps {
        draw_warps(ui, state, index, f);
    }
    if state.studio.game_view {
        draw_game_view(&painter, state, f);
    }
    badges(ui, state, index, f, view);
}

fn draw_rooms(painter: &Painter, f: &ViewFrame, w: u32, h: u32) {
    let stroke = Stroke::new(1.5, Color32::from_rgba_unmultiplied(120, 200, 255, 170));
    for r in preset_rooms(w, h) {
        let pts = cell_rect_outline(
            f,
            f64::from(r.x0),
            f64::from(r.y0),
            f64::from(r.x0 + r.w),
            f64::from(r.y0 + r.h),
        );
        painter.add(Shape::closed_line(pts, stroke));
    }
}

fn draw_light(painter: &Painter, state: &mut AppState, index: usize, f: &ViewFrame) {
    let (w, h) = (state.docs[index].ds1.width, state.docs[index].ds1.height);
    let row = levels::row_of(state, index);
    let m = row
        .as_ref()
        .and_then(|r| capy_render::links::LevelLight::of(r).multiplier());
    if let Some([r, g, b]) = m {
        // egui cannot multiply: darken by the mean, tint with the colour.
        let mean = (r + g + b) / 3.0;
        let dark = ((1.0 - mean).clamp(0.0, 0.9) * 255.0) as u8;
        let pts = cell_rect_outline(f, 0.0, 0.0, f64::from(w), f64::from(h));
        painter.add(Shape::convex_polygon(
            pts.clone(),
            Color32::from_rgba_unmultiplied(0, 0, 0, dark),
            Stroke::NONE,
        ));
        let tint = Color32::from_rgba_unmultiplied(
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            40,
        );
        painter.add(Shape::convex_polygon(pts, tint, Stroke::NONE));
    }
}

fn draw_pops(painter: &Painter, state: &mut AppState, index: usize, f: &ViewFrame) {
    let areas = pops(state, index);
    let ds1 = state.docs[index].ds1.clone();
    let inside = state.studio.pops_inside;
    for a in areas.iter() {
        let r = a.rect;
        let trigger = cell_rect_outline(
            f,
            f64::from(r.x0),
            f64::from(r.y0),
            f64::from(r.x1 + 1),
            f64::from(r.y1 + 1),
        );
        painter.add(Shape::convex_polygon(
            trigger.clone(),
            Color32::from_rgba_unmultiplied(255, 160, 40, 40),
            Stroke::new(2.0, Color32::from_rgb(255, 170, 60)),
        ));
        for t in pop_targets(&ds1, a) {
            let d = f.diamond(t.x as i32, t.y as i32);
            let fill = if inside {
                Color32::from_rgba_unmultiplied(80, 160, 255, 70)
            } else {
                Color32::from_rgba_unmultiplied(80, 160, 255, 35)
            };
            painter.add(Shape::convex_polygon(d.to_vec(), fill, Stroke::NONE));
        }
        for m in &a.markers {
            let c = f.tile_to_screen(f64::from(m.x) + 0.5, f64::from(m.y) + 0.5);
            painter.circle_filled(c, 3.5, Color32::from_rgb(255, 120, 40));
        }
        painter.text(
            trigger[0],
            Align2::CENTER_BOTTOM,
            format!("hide area {} (tiles {})", a.main, a.target),
            FontId::proportional(11.0),
            Color32::from_rgb(255, 190, 90),
        );
    }
}

fn draw_warps(ui: &mut Ui, state: &mut AppState, index: usize, f: &ViewFrame) {
    let warps = levels::warps(state, index);
    if warps.is_empty() {
        return;
    }
    let painter = ui.painter_at(f.rect);
    let colour = Color32::from_rgb(200, 120, 255);
    let mut open: Option<(i32, String)> = None;
    // One label per link number, at its first tile.
    let mut labelled = std::collections::BTreeSet::new();
    for w in warps.iter() {
        let d = f.diamond(w.x as i32, w.y as i32);
        painter.add(Shape::closed_line(d.to_vec(), Stroke::new(2.0, colour)));
        if !labelled.insert(w.link) {
            continue;
        }
        let text = match &w.to {
            Some(l) => format!("\u{2192} {} (link {})", l.target.name, w.link),
            None => format!("\u{2192} nowhere (link {}: no Vis{})", w.link, w.link),
        };
        let at = f.tile_to_screen(f64::from(w.x) + 0.5, f64::from(w.y) + 0.5);
        if !f.rect.contains(at) {
            continue;
        }
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
        let rect = Rect::from_center_size(at, galley.size() + Vec2::new(10.0, 4.0));
        let id = ui.id().with(("warp_label", w.link));
        let resp = ui.interact(rect, id, Sense::click());
        let fill = if resp.hovered() {
            Color32::from_rgba_unmultiplied(110, 50, 160, 240)
        } else {
            Color32::from_rgba_unmultiplied(60, 25, 90, 220)
        };
        painter.rect(
            rect,
            4.0,
            fill,
            Stroke::new(1.0, colour),
            StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
        if let Some(l) = &w.to {
            let resp = resp.on_hover_text(format!(
                "Click: open {} (level {})",
                l.target.name, l.target.id
            ));
            if resp.clicked() {
                open = Some((l.target.id, l.target.name.clone()));
            }
        }
    }
    if let Some((level, name)) = open {
        levels::open_target(state, level, &name);
    }
}

/// The mode name and legend (top left) and the walkable-area badge (top
/// right).
fn badges(ui: &Ui, state: &mut AppState, index: usize, f: &ViewFrame, view: ViewMode) {
    let painter = ui.painter_at(f.rect);
    let s = state.shell.shades;
    let mut y = f.rect.top() + 34.0;
    let mut line = |painter: &Painter, text: String, colour: Color32| {
        let g = painter.layout_no_wrap(text, FontId::proportional(12.0), colour);
        let r = Rect::from_min_size(
            Pos2::new(f.rect.left() + 10.0, y),
            g.size() + Vec2::new(12.0, 6.0),
        );
        painter.rect_filled(r, 4.0, s.panel.gamma_multiply(0.92));
        painter.galley(r.min + Vec2::new(6.0, 3.0), g, colour);
        y += r.height() + 3.0;
    };
    if !matches!(view, ViewMode::Tiles | ViewMode::Objects) {
        line(&painter, format!("{} view", view.label()), s.accent_text);
    }
    match view {
        ViewMode::Walk => {
            for (c, label) in [
                (WalkClass::Walk, "walkable"),
                (WalkClass::MonstersOnly, "monsters only"),
                (WalkClass::Blocked, "blocked"),
                (WalkClass::Void, "open void (players fall into it)"),
                (WalkClass::OpenEdge, "edge next to open void"),
            ] {
                let [r, g, b, _] = walk_colour(c);
                line(
                    &painter,
                    format!("\u{25a0} {label}"),
                    Color32::from_rgb(r, g, b),
                );
            }
        }
        ViewMode::Automap => line(
            &painter,
            "Preview only: walls light, floors dark (editing comes later)".to_owned(),
            s.muted,
        ),
        ViewMode::Light => {
            let text = match levels::row_of(state, index) {
                Some(r) if r.intensity > 0 => format!(
                    "{}: Intensity {}, RGB {} {} {} (Levels.txt; view only)",
                    r.name, r.intensity, r.rgb[0], r.rgb[1], r.rgb[2]
                ),
                Some(r) => format!("{}: act daylight (Intensity 0)", r.name),
                None => "No Levels.txt row for this map".to_owned(),
            };
            line(&painter, text, s.text);
        }
        ViewMode::Roofs => {
            let n = pops(state, index).len();
            line(
                &painter,
                format!(
                    "{n} hide area(s){}",
                    if state.studio.pops_inside {
                        ": shown as from inside (Shift+H)"
                    } else {
                        " (Shift+H: as if inside)"
                    }
                ),
                s.text,
            );
        }
        ViewMode::Tiles | ViewMode::Objects => {}
    }
    if state.config.ui.studio.spawn_regions {
        line(
            &painter,
            "Spawn regions: blue spawn, yellow nodes, violet warps, grey off".to_owned(),
            s.muted,
        );
    }
    // Walkable area, top right.
    if let Some(area) = walkable(state, index) {
        let ([r, g, b], _) = area_colour(area);
        let text = format!("Walkable {}", group_thousands(area));
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.5), Color32::WHITE);
        let rect = Rect::from_min_size(
            Pos2::new(f.rect.right() - galley.size().x - 22.0, f.rect.top() + 10.0),
            galley.size() + Vec2::new(12.0, 6.0),
        );
        painter.rect_filled(rect, 4.0, Color32::from_rgb(r, g, b));
        painter.galley(rect.min + Vec2::new(6.0, 3.0), galley, Color32::WHITE);
    }
}

/// `3512.4` -> `3,512`.
pub fn group_thousands(v: f64) -> String {
    let n = v.round() as i64;
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

// ───────────────────────── minimap ─────────────────────────

/// A rendered picture of a map for the minimap.
pub struct Rendered {
    pub image: ColorImage,
    /// Iso-space rectangle `[x0, y0, x1, y1]` the image covers.
    pub iso: [f64; 4],
}

/// `(doc, rev, result)` of a minimap drawing in flight.
type MinimapJob = (u64, u64, std::sync::Mutex<Receiver<Option<Rendered>>>);

/// The minimap's rendered map: drawn on a worker thread per revision.
#[derive(Default)]
pub struct MinimapRender {
    job: Option<MinimapJob>,
    done: Option<(u64, u64)>,
    texture: Option<(u64, TextureHandle, [f64; 4])>,
}

/// The rendered minimap picture of map `index` (the last one drawn, while a
/// newer revision is being drawn). Starts the drawing when it is due.
pub fn minimap_texture(
    ctx: &egui::Context,
    state: &mut AppState,
    index: usize,
) -> Option<(TextureHandle, [f64; 4])> {
    let doc_id = state.docs[index].id;
    let rev = state.docs[index].rev;
    // A finished job.
    let finished = state.studio.minimap.job.as_ref().and_then(|(d, r, rx)| {
        rx.lock()
            .ok()
            .and_then(|rx| rx.try_recv().ok())
            .map(|res| (*d, *r, res))
    });
    if let Some((d, r, result)) = finished {
        state.studio.minimap.job = None;
        state.studio.minimap.done = Some((d, r));
        if let Some(rendered) = result {
            let t = ctx.load_texture("minimap_render", rendered.image, TextureOptions::LINEAR);
            state.studio.minimap.texture = Some((d, t, rendered.iso));
        }
    }
    let due = state.studio.minimap.done != Some((doc_id, rev))
        && state.studio.minimap.job.is_none()
        && !busy(state);
    if due {
        start_minimap(state, index);
    }
    match &state.studio.minimap.texture {
        Some((d, t, iso)) if *d == doc_id => Some((t.clone(), *iso)),
        _ => None,
    }
}

fn start_minimap(state: &mut AppState, index: usize) {
    let doc = &state.docs[index];
    let (Some(tiles), Some(info), Some(session)) =
        (doc.tiles.clone(), doc.info.as_ref(), state.session.as_ref())
    else {
        return;
    };
    let palette = match crate::render::job::load_palette(&session.overlay, info.palette_act) {
        Ok(p) => p,
        Err(_) => return,
    };
    let ds1 = doc.ds1.clone();
    let mut vis = doc.visibility();
    vis.objects = false;
    let gamma = state.config.gamma;
    let (tx, rx) = channel();
    state.studio.minimap.job = Some((doc.id, doc.rev, std::sync::Mutex::new(rx)));
    std::thread::spawn(move || {
        let out = render_small(&ds1, &tiles, &palette, vis, gamma);
        let _ = tx.send(out);
    });
}

/// The map at a zoom that keeps the picture near 512 pixels wide.
pub fn render_small(
    ds1: &d2_formats::ds1::Ds1,
    tiles: &capy_render::TileSet,
    palette: &d2_formats::palette::Palette,
    visibility: capy_render::Visibility,
    gamma: f32,
) -> Option<Rendered> {
    let (x0, _, x1, _) = capy_render::reference_bounds(ds1, tiles, 1)?;
    let div = (((x1 - x0).max(1) as u32).div_ceil(512)).clamp(1, 16);
    let opts = capy_render::RenderOptions {
        zoom_div: div,
        gamma,
        visibility,
        ..capy_render::RenderOptions::default()
    };
    let out = capy_render::render(ds1, tiles, palette, &opts).ok()?;
    let img = out.image;
    let image =
        ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    let d = f64::from(div);
    let shift = f64::from(capy_render::CELL_DIAMOND_TOP_X);
    let (ox, oy) = (f64::from(out.origin.0), f64::from(out.origin.1));
    Some(Rendered {
        image,
        iso: [
            ox * d - shift,
            oy * d,
            (ox + f64::from(img.width)) * d - shift,
            (oy + f64::from(img.height)) * d,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(3512.4), "3,512");
        assert_eq!(group_thousands(999.6), "1,000");
        assert_eq!(group_thousands(12.0), "12");
        assert_eq!(group_thousands(1_234_567.0), "1,234,567");
    }
}
