//! The minimap in the map's bottom-left corner (View > Minimap, K): the
//! whole map drawn small by `capy_render` on a worker thread (the layers
//! that are shown, without objects), with the visible part outlined. Until
//! the picture is ready it is a schematic diamond, one pixel per cell (walls
//! brown, floors green-grey). Click or drag to move there.

use bevy_egui::egui::{
    self, Color32, ColorImage, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Ui, Vec2,
};

use crate::iso::{self, HALF_H, HALF_W};
use crate::state::AppState;

/// Largest size of the minimap box (points).
const BOX: Vec2 = Vec2::new(220.0, 110.0);
const MARGIN: f32 = 10.0;

const FLOOR: [u8; 4] = [86, 98, 72, 255];
const WALL: [u8; 4] = [150, 128, 98, 255];

/// The texture of one map revision.
pub struct Cache {
    doc: u64,
    rev: u64,
    texture: TextureHandle,
}

/// Where the minimap sits inside the map rectangle.
pub fn rect_in(map: Rect) -> Rect {
    let size = BOX.min(map.size() * 0.4);
    Rect::from_min_size(
        Pos2::new(map.left() + MARGIN, map.bottom() - MARGIN - size.y),
        size,
    )
}

/// One RGBA pixel per cell.
fn pixels(ds1: &d2_formats::ds1::Ds1) -> Vec<u8> {
    let (w, h) = (ds1.width, ds1.height);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let wall = ds1
                .walls
                .iter()
                .any(|l| l.tiles.get(x, y).is_some_and(|c| !c.is_empty()));
            let floor = ds1
                .floors
                .iter()
                .any(|l| l.get(x, y).is_some_and(|c| !c.is_empty()));
            let px = if wall {
                WALL
            } else if floor {
                FLOOR
            } else {
                continue;
            };
            let i = ((y * w + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}

pub fn show(ui: &mut Ui, state: &mut AppState, map: Rect) {
    let ppp = ui.ctx().pixels_per_point();
    let s = state.shell.shades;
    let Some(index) = state.active else {
        return;
    };
    let (doc_id, rev, w, h) = {
        let d = &state.docs[index];
        (d.id, d.rev, d.ds1.width.max(1), d.ds1.height.max(1))
    };
    let stale = state
        .shell
        .minimap
        .as_ref()
        .is_none_or(|c| c.doc != doc_id || c.rev != rev);
    if stale {
        let image = ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            &pixels(&state.docs[index].ds1),
        );
        let texture = ui
            .ctx()
            .load_texture("minimap", image, TextureOptions::NEAREST);
        state.shell.minimap = Some(Cache {
            doc: doc_id,
            rev,
            texture,
        });
    }
    let Some(cache) = &state.shell.minimap else {
        return;
    };
    let rect = rect_in(map);
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        4.0,
        s.bg.gamma_multiply(0.85),
        Stroke::new(1.0, s.border2),
        StrokeKind::Inside,
    );
    // The floor diamond's iso bounds, fitted into the box.
    let (fw, fh) = (f64::from(w), f64::from(h));
    let x0 = -fh * HALF_W;
    let x1 = fw * HALF_W;
    let y1 = (fw + fh) * HALF_H;
    let inner = rect.shrink(6.0);
    let scale = (f64::from(inner.width()) / (x1 - x0)).min(f64::from(inner.height()) / y1);
    let origin = Pos2::new(
        inner.center().x - ((x0 + x1) / 2.0 * scale) as f32,
        inner.center().y - (y1 / 2.0 * scale) as f32,
    );
    let to_screen = |p: [f64; 2]| origin + Vec2::new((p[0] * scale) as f32, (p[1] * scale) as f32);
    let to_iso = |p: Pos2| -> [f64; 2] {
        [
            f64::from(p.x - origin.x) / scale,
            f64::from(p.y - origin.y) / scale,
        ]
    };

    let schematic = cache.texture.id();
    match super::studio::views::minimap_texture(ui.ctx(), state, index) {
        Some((texture, [ix0, iy0, ix1, iy1])) => {
            let r = Rect::from_min_max(to_screen([ix0, iy0]), to_screen([ix1, iy1]));
            painter.image(
                texture.id(),
                r,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let mut mesh = Mesh::with_texture(schematic);
            for (u, v, uv) in [
                (0.0, 0.0, Pos2::new(0.0, 0.0)),
                (fw, 0.0, Pos2::new(1.0, 0.0)),
                (fw, fh, Pos2::new(1.0, 1.0)),
                (0.0, fh, Pos2::new(0.0, 1.0)),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: to_screen(iso::tile_to_iso(u, v)),
                    uv,
                    color: Color32::WHITE,
                });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(Shape::mesh(mesh));
        }
    }

    // The part of the map on screen.
    let view = state.docs[index].view;
    let half = [
        f64::from(map.width() * ppp) / 2.0,
        f64::from(map.height() * ppp) / 2.0,
    ];
    let a = to_screen(view.offset_to_iso([-half[0], -half[1]]));
    let b = to_screen(view.offset_to_iso([half[0], half[1]]));
    painter.rect_stroke(
        Rect::from_two_pos(a, b),
        0.0,
        Stroke::new(1.5, s.accent),
        StrokeKind::Middle,
    );

    let resp = ui.interact(rect, ui.id().with("minimap"), Sense::click_and_drag());
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged())
    {
        state.docs[index].view.center = to_iso(pos);
    }
}
