//! Thumbnails of tiles for the "Recent tiles" strip and the toolbar's
//! current value: the tile is looked up in the open map's TileSet, painted
//! through the map's act palette and shrunk to fit.

use std::sync::Arc;

use bevy_egui::egui::{self, ColorImage, TextureHandle, TextureOptions};
use capy_edit::{CellValue, LayerRef};
use capy_render::TileSet;
use d2_formats::ds1::{Cell, DIR_LOOKUP};
use d2_formats::dt1::TileKey;
use d2_formats::palette::Palette;

use crate::render::{atlas, job};
use crate::state::AppState;

/// Edge of a thumbnail cell in points.
pub const THUMB: f32 = 44.0;
/// Longest edge of the stored texture in pixels.
const TEXTURE_EDGE: usize = 64;

/// The DT1 key a cell value looks up, for a DS1 of `version` (the orientation
/// of walls goes through the old-version remap below v7). `None` for the tag
/// layer, which has no tile.
pub fn tile_key(layer: LayerRef, value: CellValue, version: i32) -> Option<TileKey> {
    let cell = Cell(value.cell);
    let orientation = match layer {
        LayerRef::Floor(_) => 0,
        LayerRef::Shadow => 13,
        LayerRef::Wall(_) => {
            let low = (value.orientation & 0xFF) as u8;
            if version < 7 {
                *DIR_LOOKUP.get(usize::from(low))?
            } else {
                low
            }
        }
        LayerRef::Tag => return None,
    };
    Some(TileKey {
        orientation,
        main: cell.main_index(),
        sub: cell.sub_index(),
    })
}

/// Shrinks RGBA pixels so the longer edge is at most `max`, keeping the
/// aspect ratio. A box filter over opaque texels keeps thin walls visible.
pub fn shrink_rgba(
    width: usize,
    height: usize,
    rgba: &[u8],
    max: usize,
) -> (usize, usize, Vec<u8>) {
    let longest = width.max(height);
    if longest <= max || width == 0 || height == 0 {
        return (width, height, rgba.to_vec());
    }
    let k = longest.div_ceil(max);
    let (w, h) = (width.div_ceil(k), height.div_ceil(k));
    let mut out = vec![0u8; w * h * 4];
    for oy in 0..h {
        for ox in 0..w {
            let (mut r, mut g, mut b, mut n, mut area) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in oy * k..((oy + 1) * k).min(height) {
                for sx in ox * k..((ox + 1) * k).min(width) {
                    area += 1;
                    let i = (sy * width + sx) * 4;
                    if rgba.get(i + 3).copied().unwrap_or(0) > 0 {
                        r += u32::from(rgba[i]);
                        g += u32::from(rgba[i + 1]);
                        b += u32::from(rgba[i + 2]);
                        n += 1;
                    }
                }
            }
            if n > 0 && n * 4 >= area {
                // At least a quarter of the box is opaque: show it.
                let o = (oy * w + ox) * 4;
                out[o] = (r / n) as u8;
                out[o + 1] = (g / n) as u8;
                out[o + 2] = (b / n) as u8;
                out[o + 3] = 255;
            }
        }
    }
    (w, h, out)
}

/// Renders one value to a texture. `None` when the tile is not in the set.
fn render(
    ctx: &egui::Context,
    tiles: &TileSet,
    palette: &Palette,
    layer: LayerRef,
    value: CellValue,
    version: i32,
) -> Option<TextureHandle> {
    let key = tile_key(layer, value, version)?;
    let tile = tiles.resolve(key)?;
    let tile = tiles.animation_frame0(&tile);
    let image = tiles.image(&tile).ok()?;
    let rgba = atlas::bake(&image, palette, false);
    let (w, h, small) = shrink_rgba(
        image.width as usize,
        image.height as usize,
        &rgba,
        TEXTURE_EDGE,
    );
    if w == 0 || h == 0 {
        return None;
    }
    let color = ColorImage::from_rgba_unmultiplied([w, h], &small);
    Some(ctx.load_texture(
        format!("thumb-{layer:?}-{:x}-{:x}", value.cell, value.orientation),
        color,
        TextureOptions::NEAREST,
    ))
}

/// The thumbnail of `value` on `layer` for the active map, cached. `None`
/// until the map's tiles are loaded or when the tile does not exist.
pub fn thumbnail(
    ctx: &egui::Context,
    state: &mut AppState,
    layer: LayerRef,
    value: CellValue,
) -> Option<TextureHandle> {
    let doc = state.active_doc()?;
    let tiles: Arc<TileSet> = doc.tiles.clone()?;
    let owner = (doc.id, Arc::as_ptr(&tiles) as usize);
    let (act, version) = (doc.info.as_ref()?.palette_act, doc.ds1.version);
    let gamma = state.config.gamma;
    let overlay = state.session.as_ref()?.overlay.clone();
    if state.edit.thumbs.owner != Some(owner) {
        state.edit.thumbs.owner = Some(owner);
        state.edit.thumbs.textures.clear();
        state.edit.thumbs.palette = job::load_palette(&overlay, act)
            .ok()
            .map(|p| p.with_gamma(gamma));
    }
    if let Some(cached) = state.edit.thumbs.textures.get(&(layer, value)) {
        return cached.clone();
    }
    let palette = state.edit.thumbs.palette.as_ref()?;
    let made = render(ctx, &tiles, palette, layer, value, version);
    state
        .edit
        .thumbs
        .textures
        .insert((layer, value), made.clone());
    made
}

/// Draws `value` as a thumbnail button-sized image (or a grey tile with its
/// numbers when no image exists) and returns the response.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    layer: LayerRef,
    value: CellValue,
    selected: bool,
) -> egui::Response {
    let texture = thumbnail(ui.ctx(), state, layer, value);
    let size = egui::vec2(THUMB, THUMB);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, egui::Color32::from_gray(36));
    match texture {
        Some(t) => {
            let ts = t.size_vec2();
            let scale = (THUMB / ts.x).min(THUMB / ts.y).min(4.0);
            let target = egui::Rect::from_center_size(rect.center(), ts * scale);
            painter.image(
                t.id(),
                target,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            let c = Cell(value.cell);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{}/{}", c.main_index(), c.sub_index()),
                egui::FontId::monospace(11.0),
                egui::Color32::from_gray(170),
            );
        }
    }
    let stroke = if selected {
        egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 220, 90))
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_gray(80))
    };
    painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_layer() {
        let value = CellValue {
            cell: Cell(1).with_indices(7, 3).0,
            orientation: 4,
        };
        let key = |layer, version| tile_key(layer, value, version);
        assert_eq!(
            key(LayerRef::Floor(1), 18),
            Some(TileKey {
                orientation: 0,
                main: 7,
                sub: 3
            })
        );
        assert_eq!(key(LayerRef::Shadow, 18).map(|k| k.orientation), Some(13));
        assert_eq!(key(LayerRef::Wall(2), 18).map(|k| k.orientation), Some(4));
        // Old files remap wall orientations.
        assert_eq!(
            key(LayerRef::Wall(0), 5).map(|k| k.orientation),
            Some(DIR_LOOKUP[4])
        );
        assert_eq!(key(LayerRef::Tag, 18), None);
        // An orientation outside the table is unknown in old files.
        let wild = CellValue {
            cell: value.cell,
            orientation: 200,
        };
        assert_eq!(tile_key(LayerRef::Wall(0), wild, 5), None);
    }

    #[test]
    fn small_images_pass_through_and_big_ones_shrink() {
        let small = vec![9u8; 4 * 4 * 4];
        assert_eq!(shrink_rgba(4, 4, &small, 64), (4, 4, small.clone()));
        // A 160 x 80 opaque tile shrinks by 3 to 54 x 27.
        let big = vec![200u8; 160 * 80 * 4];
        let (w, h, px) = shrink_rgba(160, 80, &big, 64);
        assert_eq!((w, h), (54, 27));
        assert_eq!(px.len(), w * h * 4);
        assert!(px.chunks(4).all(|p| p[3] == 255 && p[0] == 200));
    }

    #[test]
    fn transparent_areas_stay_transparent() {
        let mut rgba = vec![0u8; 128 * 128 * 4];
        // Only the top-left 64 x 64 quadrant is opaque.
        for y in 0..64 {
            for x in 0..64 {
                let i = (y * 128 + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[10, 20, 30, 255]);
            }
        }
        let (w, h, px) = shrink_rgba(128, 128, &rgba, 32);
        assert_eq!((w, h), (32, 32));
        assert_eq!(px[3], 255);
        let last = (31 * 32 + 31) * 4;
        assert_eq!(px[last + 3], 0);
    }
}
