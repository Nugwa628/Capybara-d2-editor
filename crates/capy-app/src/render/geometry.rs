//! Mesh data for a textured map and for the missing-tile outlines. Pure
//! functions of the draw list, so the order guarantees are unit-testable.
//!
//! # Draw order
//!
//! `capy_render::draw_list` returns the painter's order. Item `k` becomes
//! quad `k`: vertices `4k..4k+4` and indices `6k..6k+6`. One mesh, one
//! material, one draw call, so the GPU rasterises triangles in index order and
//! blends them in that order. No depth or z sorting is involved.

use capy_render::{DrawItem, MissingTile, TileGeometry};
#[cfg(test)]
use capy_render::TileRef;

#[cfg(test)]
use super::atlas::AtlasLayout;
use super::atlas::{AtlasKey, Rect};
use crate::iso;

/// Cell size at 1:1.
const CELL_W: i32 = 160;
const CELL_H: i32 = 80;

/// CPU-side mesh of the textured quads. World space: y up, so iso y is negated.
#[derive(Default, Debug, PartialEq)]
pub struct TexGeometry {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// `[atlas layer, 0, 0, 1]` per vertex. It travels in the vertex colour
    /// slot because that is a free `Float32x4` attribute of the stock mesh.
    pub layers: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

#[cfg(test)]
impl TexGeometry {
    pub fn quad_count(&self) -> usize {
        self.indices.len() / 6
    }
}

/// Whether an item is drawn as a shadow (50% reference blend).
pub fn is_shadow(item: &DrawItem) -> bool {
    item.pass == capy_render::DrawPass::Shadows
}

pub fn atlas_key(item: &DrawItem) -> AtlasKey {
    AtlasKey::tile(item.tile.clone(), is_shadow(item))
}

/// World-space (1:1 pixels, y down) position of an item's top-left corner when
/// the map is laid out for zoom `1:div`.
///
/// DS1Edit does not shrink a 1:1 picture: at `--zoom=1:N` it places every
/// tile on an integer grid of zoomed pixels, with truncating integer maths
/// (`capy_render::compose`), and draws the bitmap scaled by `1/N`. A camera
/// scaled by `N` over quads placed at `N` times those integers reproduces
/// that exactly, including where the truncation moves a tile by a pixel.
/// At `div == 1` this is just `item.screen_x / screen_y`.
pub fn quad_origin(item: &DrawItem, geom: &TileGeometry, div: u32) -> (i32, i32) {
    let div = i32::try_from(div.clamp(1, 16)).unwrap_or(1);
    let (x, y) = (item.x as i32, item.y as i32);
    let off_x = item.screen_x - (x - y) * CELL_W / 2;
    let off_y = item.screen_y - (x + y) * CELL_H / 2 - geom.bitmap_top(1);
    let (tile_w, tile_h) = (CELL_W / div, CELL_H / div);
    let mx = (y * -tile_w) / 2 + (x * tile_w) / 2;
    let my = (y * tile_h) / 2 + (x * tile_h) / 2;
    (div * mx + off_x, div * (my + geom.bitmap_top(div)) + off_y)
}

/// One quad per item, in item order, laid out for zoom `1:div`. `geometry`
/// looks up a tile's bitmap geometry. Items missing from the layout (which
/// cannot happen for a layout built from the same list) are skipped.
#[cfg(test)]
pub fn build_quads(
    items: &[DrawItem],
    layout: &AtlasLayout,
    div: u32,
    geometry: &dyn Fn(&TileRef) -> Option<TileGeometry>,
) -> TexGeometry {
    let mut geo = TexGeometry {
        positions: Vec::with_capacity(items.len() * 4),
        uvs: Vec::with_capacity(items.len() * 4),
        layers: Vec::with_capacity(items.len() * 4),
        indices: Vec::with_capacity(items.len() * 6),
    };
    let inv = 1.0 / layout.size as f32;
    for item in items {
        let Some(rect) = layout.rects.get(&atlas_key(item)) else {
            continue;
        };
        let Some(geom) = geometry(&item.tile) else {
            continue;
        };
        push_quad(&mut geo, quad_origin(item, &geom, div), rect, inv);
    }
    geo
}

/// Appends one axis-aligned quad showing `rect` of the atlas with its
/// top-left corner at `origin` (1:1 world pixels, y down). Empty rectangles
/// add nothing.
pub fn push_quad(geo: &mut TexGeometry, origin: (i32, i32), rect: &Rect, inv: f32) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let (x0, y0) = (origin.0 as f32, -(origin.1 as f32));
    let (x1, y1) = (x0 + rect.w as f32, y0 - rect.h as f32);
    let (u0, v0) = (rect.x as f32 * inv, rect.y as f32 * inv);
    let (u1, v1) = (
        (rect.x + rect.w) as f32 * inv,
        (rect.y + rect.h) as f32 * inv,
    );
    let base = geo.positions.len() as u32;
    // Top-left, top-right, bottom-right, bottom-left.
    geo.positions
        .extend([[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]]);
    geo.uvs.extend([[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
    geo.layers.extend([[rect.layer as f32, 0.0, 0.0, 1.0]; 4]);
    geo.indices
        .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
}

/// Triangles for the outlines (a thin diamond ring per missing tile), as
/// positions and linear-RGBA vertex colours.
#[derive(Default, Debug, PartialEq)]
pub struct OutlineGeometry {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// Linear RGBA of the outline magenta (sRGB 255, 0, 255).
pub const OUTLINE_COLOR: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

/// A diamond ring of thickness `width` (iso pixels) at each distinct missing
/// cell. Each cell is drawn once however many layers lack a tile there.
pub fn build_outlines(missing: &[MissingTile], width: f32, z: f32) -> OutlineGeometry {
    let mut geo = OutlineGeometry::default();
    let mut seen = std::collections::HashSet::new();
    let (hw, hh) = (iso::TILE_W as f32 / 2.0, iso::TILE_H as f32 / 2.0);
    // Shrink both half-axes by the same fraction so the ring has about `width`
    // across its short side.
    let t = (width / hh).clamp(0.02, 0.9);
    for m in missing {
        if !seen.insert((m.x, m.y)) {
            continue;
        }
        let [cx, cy] = iso::cell_center(m.x as i32, m.y as i32);
        let (cx, cy) = (cx as f32, -(cy as f32));
        let ring = |k: f32| -> [[f32; 3]; 4] {
            let (w, h) = (hw * k, hh * k);
            [
                [cx, cy + h, z],
                [cx + w, cy, z],
                [cx, cy - h, z],
                [cx - w, cy, z],
            ]
        };
        let outer = ring(1.0);
        let inner = ring(1.0 - t);
        let base = geo.positions.len() as u32;
        geo.positions.extend(outer);
        geo.positions.extend(inner);
        geo.colors.extend([OUTLINE_COLOR; 8]);
        for i in 0..4u32 {
            let j = (i + 1) % 4;
            let (o0, o1, i0, i1) = (base + i, base + j, base + 4 + i, base + 4 + j);
            geo.indices.extend([o0, o1, i1, o0, i1, i0]);
        }
    }
    geo
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use capy_render::{DrawPass, TileClass};

    use super::*;
    use crate::render::atlas::PaletteKey;

    fn item(order: u32, pass: DrawPass, dt1: usize, tile: usize, sx: i32, sy: i32) -> DrawItem {
        DrawItem {
            pass,
            order,
            layer: ('f', 0),
            x: 0,
            y: 0,
            tile: TileRef { dt1, tile },
            screen_x: sx,
            screen_y: sy,
        }
    }

    fn layout(entries: &[(AtlasKey, Rect)], size: u32, layers: u32) -> AtlasLayout {
        AtlasLayout {
            size,
            layers,
            palette: PaletteKey {
                act: 0,
                gamma_bits: 0,
            },
            rects: entries.iter().cloned().collect::<HashMap<_, _>>(),
            sprites: Vec::new(),
        }
    }

    fn key(dt1: usize, tile: usize, shadow: bool) -> AtlasKey {
        AtlasKey::tile(TileRef { dt1, tile }, shadow)
    }

    fn geom(class: TileClass, zero_line: i32) -> TileGeometry {
        TileGeometry {
            class,
            width: 160,
            height: 80,
            y_add: 0,
            zero_line,
            roof_y: 0,
        }
    }

    /// Floor geometry: the bitmap sits at the cell's top corner.
    fn flat(_: &TileRef) -> Option<TileGeometry> {
        Some(geom(TileClass::Floor, 0))
    }

    #[test]
    fn quad_origin_is_the_screen_position_at_1_to_1() {
        for (class, zero) in [
            (TileClass::Floor, 0),
            (TileClass::UpperWall, 30),
            (TileClass::LowerWall, 96),
            (TileClass::Shadow, 17),
        ] {
            let g = geom(class, zero);
            let it = DrawItem {
                x: 7,
                y: 3,
                screen_x: (7 - 3) * 80 + 11,
                screen_y: (7 + 3) * 40 + g.bitmap_top(1) + 5,
                ..item(0, DrawPass::Floors, 0, 0, 0, 0)
            };
            assert_eq!(
                quad_origin(&it, &g, 1),
                (it.screen_x, it.screen_y),
                "{class:?}"
            );
        }
    }

    #[test]
    fn quad_origin_follows_the_references_integer_grid_at_1_to_4() {
        // Cell (3, 1) of an upper wall with zero_line 30 and crop offset (5, 7):
        // tile_w = 40, tile_h = 20, mx = (1 * -40) / 2 + (3 * 40) / 2 = 40,
        // my = (1 * 20) / 2 + (3 * 20) / 2 = 40, bitmap_top(4) = -(30 / 4) + 20 = 13.
        let g = geom(TileClass::UpperWall, 30);
        assert_eq!(g.bitmap_top(1), 50);
        let it = DrawItem {
            x: 3,
            y: 1,
            screen_x: 2 * 80 + 5,
            screen_y: 4 * 40 + 50 + 7,
            ..item(0, DrawPass::UpperWalls, 0, 0, 0, 0)
        };
        assert_eq!(quad_origin(&it, &g, 4), (4 * 40 + 5, 4 * (40 + 13) + 7));
        // The 1:1 position would have been (165, 217): the grid moves it by 2.
        assert_eq!((it.screen_x, it.screen_y), (165, 217));
    }

    #[test]
    fn zoomed_quads_keep_item_order() {
        let lay = layout(
            &[
                (
                    key(0, 1, false),
                    Rect {
                        layer: 0,
                        x: 0,
                        y: 0,
                        w: 8,
                        h: 8,
                    },
                ),
                (
                    key(0, 2, false),
                    Rect {
                        layer: 0,
                        x: 9,
                        y: 0,
                        w: 8,
                        h: 8,
                    },
                ),
            ],
            64,
            1,
        );
        let items = vec![
            item(0, DrawPass::Floors, 0, 2, 80, 40),
            item(1, DrawPass::Floors, 0, 1, -80, 40),
        ];
        let geo = build_quads(&items, &lay, 4, &flat);
        assert_eq!(geo.quad_count(), 2);
        assert_eq!(geo.positions[0][0], 80.0);
        assert_eq!(geo.positions[4][0], -80.0);
        assert_eq!(geo.uvs[0][0], 9.0 / 64.0);
        assert_eq!(geo.uvs[4][0], 0.0);
    }

    #[test]
    fn item_order_is_vertex_and_index_order() {
        // Three items on two atlas layers, deliberately not sorted by layer,
        // position or tile: the mesh must follow the list, not any key.
        let lay = layout(
            &[
                (
                    key(0, 5, false),
                    Rect {
                        layer: 1,
                        x: 0,
                        y: 0,
                        w: 160,
                        h: 80,
                    },
                ),
                (
                    key(0, 2, false),
                    Rect {
                        layer: 0,
                        x: 161,
                        y: 0,
                        w: 32,
                        h: 16,
                    },
                ),
                (
                    key(0, 2, true),
                    Rect {
                        layer: 0,
                        x: 200,
                        y: 8,
                        w: 32,
                        h: 16,
                    },
                ),
            ],
            512,
            2,
        );
        let items = vec![
            item(0, DrawPass::Floors, 0, 5, 100, 50),
            item(1, DrawPass::Shadows, 0, 2, -40, 10),
            item(2, DrawPass::UpperWalls, 0, 2, 7, 3),
        ];
        let geo = build_quads(&items, &lay, 1, &flat);
        assert_eq!(geo.quad_count(), 3);
        assert_eq!(geo.positions.len(), 12);
        for (k, it) in items.iter().enumerate() {
            // Quad k starts at vertex 4k and its top-left is the item's anchor.
            assert_eq!(
                geo.positions[4 * k],
                [it.screen_x as f32, -(it.screen_y as f32), 0.0],
                "item {k}"
            );
            let idx = &geo.indices[6 * k..6 * k + 6];
            assert!(
                idx.iter()
                    .all(|&i| (4 * k as u32..4 * k as u32 + 4).contains(&i))
            );
        }
        assert_eq!(geo.layers[0][0], 1.0);
        assert_eq!(
            geo.layers[4][0], 0.0,
            "shadow variant lives in the atlas too"
        );
        // The shadow item used the shadow entry (x = 200), the wall the plain one (161).
        assert_eq!(geo.uvs[4][0], 200.0 / 512.0);
        assert_eq!(geo.uvs[8][0], 161.0 / 512.0);
        // Indices are strictly the concatenation of the quads.
        let firsts: Vec<u32> = geo.indices.chunks(6).map(|c| c[0]).collect();
        assert_eq!(firsts, vec![0, 4, 8]);
    }

    #[test]
    fn quads_are_axis_aligned_with_the_rect_size_and_uv() {
        let lay = layout(
            &[(
                key(1, 1, false),
                Rect {
                    layer: 0,
                    x: 64,
                    y: 32,
                    w: 20,
                    h: 10,
                },
            )],
            256,
            1,
        );
        let geo = build_quads(&[item(0, DrawPass::Floors, 1, 1, 10, 100)], &lay, 1, &flat);
        // y is flipped: the bottom edge has the smaller world y.
        assert_eq!(geo.positions[0], [10.0, -100.0, 0.0]);
        assert_eq!(geo.positions[2], [30.0, -110.0, 0.0]);
        assert_eq!(geo.uvs[0], [64.0 / 256.0, 32.0 / 256.0]);
        assert_eq!(geo.uvs[2], [84.0 / 256.0, 42.0 / 256.0]);
    }

    #[test]
    fn unknown_tiles_are_skipped_without_shifting_the_rest() {
        let lay = layout(
            &[(
                key(0, 1, false),
                Rect {
                    layer: 0,
                    x: 0,
                    y: 0,
                    w: 4,
                    h: 4,
                },
            )],
            64,
            1,
        );
        let items = vec![
            item(0, DrawPass::Floors, 0, 9, 0, 0), // not in the layout
            item(1, DrawPass::Floors, 0, 1, 5, 5),
        ];
        let geo = build_quads(&items, &lay, 1, &flat);
        assert_eq!(geo.quad_count(), 1);
        assert_eq!(geo.positions[0][0], 5.0);
    }

    #[test]
    fn outlines_once_per_cell_inside_the_diamond() {
        let miss = |layer, x, y| MissingTile {
            layer,
            x,
            y,
            key: d2_formats::dt1::TileKey {
                orientation: 0,
                main: 1,
                sub: 1,
            },
        };
        let missing = vec![
            miss(('f', 0), 2, 1),
            miss(('w', 0), 2, 1),
            miss(('f', 0), 0, 0),
        ];
        let geo = build_outlines(&missing, 3.0, 0.5);
        assert_eq!(
            geo.positions.len(),
            16,
            "two distinct cells, 8 vertices each"
        );
        assert_eq!(geo.indices.len(), 2 * 4 * 6);
        assert!(
            geo.indices
                .iter()
                .all(|&i| (i as usize) < geo.positions.len())
        );
        let [cx, cy] = iso::cell_center(2, 1);
        let xs: Vec<f32> = geo.positions[..8].iter().map(|p| p[0]).collect();
        let max_x = xs.iter().cloned().fold(f32::MIN, f32::max);
        let min_x = xs.iter().cloned().fold(f32::MAX, f32::min);
        assert_eq!(max_x, cx as f32 + 80.0);
        assert_eq!(min_x, cx as f32 - 80.0);
        assert_eq!(geo.positions[0][1], -(cy as f32) + 40.0);
        assert!(build_outlines(&[], 3.0, 0.0).positions.is_empty());
    }
}
