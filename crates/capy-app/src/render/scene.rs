//! The map as the GPU draws it: tiles and object sprites in the exact order
//! of `capy_render::draw_list_with_objects`, split into meshes by blend mode.
//!
//! # Runs, pieces and z
//!
//! One mesh can carry only one blend state, and Additive, Multiply and Screen
//! sprite layers need their own (`render/material.rs`). The entries are
//! therefore cut into **runs**: maximal stretches of consecutive quads with
//! the same [`BlendClass`]. Each run is one mesh entity; inside a run the
//! quads are in list order (index order = rasterisation order), and the runs
//! get strictly increasing z in list order ([`run_z`]), so the transparent
//! 2D phase, which sorts by z, draws them in list order too. The painter's
//! order of the whole list is kept exactly.
//!
//! Runs never span a **piece** boundary. A piece is a maximal stretch of tile
//! entries or of object entries. Tiles never change with the animation, so
//! only the (small) object pieces are rebuilt when a frame changes; the tile
//! meshes stay on the GPU. A tile piece is always a single run (tiles and
//! DT1 shadows are both premultiplied).
//!
//! # Object quads
//!
//! Every sprite layer is its own quad, placed as `capy_render::compose`
//! places it (`preview.c:2091-2185`): anchor [`object_anchor`] at the zoom,
//! plus the Objects.txt offset and the direction-box corner, each divided by
//! the zoom with truncation, then the layer's crop offset inside its box.
//! The same `div * zoomed + offset` world placement as tiles
//! ([`geometry::quad_origin`]) makes the camera's point sampling pick the
//! source pixels the CPU compositor picks. Shadow entries draw the layers
//! that cast a shadow as `Blend::Darken` silhouettes.

use std::ops::Range;
use std::sync::Arc;

use capy_render::{
    Blend, DrawItem, DrawPass, ObjectFrame, TileGeometry, TileRef, frame_index, object_anchor,
};

use super::atlas::{AtlasKey, AtlasLayout, SpriteBake, weight_byte};
use super::geometry::{self, TexGeometry, push_quad};
use super::material::BlendClass;

/// Highest z of a map run; the missing-tile outlines sit above at 0.5.
pub const RUN_Z_MAX: f32 = 0.4;

/// Every frame of one object's animation (one direction).
#[derive(Clone, Debug)]
pub struct ObjectAnim {
    /// `frames_per_direction` frames, frame 0 first.
    pub frames: Vec<Arc<ObjectFrame>>,
    /// Frames advanced per 25 Hz tick, times 256 (Objects.txt FrameDelta, or
    /// the AnimData speed).
    pub delta: u32,
}

impl ObjectAnim {
    /// The frame number shown at `tick` (`None` = frame 0), as
    /// `capy_render::frame_index` computes it.
    pub fn frame_number(&self, tick: Option<u32>) -> usize {
        let count = u32::try_from(self.frames.len()).unwrap_or(1);
        usize::from(frame_index(tick, self.delta, count))
    }

    pub fn frame(&self, tick: Option<u32>) -> Option<&Arc<ObjectFrame>> {
        self.frames
            .get(self.frame_number(tick))
            .or_else(|| self.frames.first())
    }
}

/// One object entry of the draw list.
#[derive(Clone, Debug)]
pub struct ObjectEntry {
    /// `ObjectShadows`, `Objects1` or `UpperWalls`.
    pub pass: DrawPass,
    /// Index into `Ds1::objects`.
    pub object: usize,
    /// DS1 sub-tile coordinates.
    pub x: i32,
    pub y: i32,
    pub anim: Arc<ObjectAnim>,
}

#[derive(Clone, Debug)]
pub enum SceneEntry {
    Tile(DrawItem),
    Object(ObjectEntry),
}

/// A maximal stretch of tile entries or of object entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub objects: bool,
    pub range: Range<usize>,
}

/// Cuts the entries into pieces, in order.
pub fn pieces(entries: &[SceneEntry]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let objects = matches!(e, SceneEntry::Object(_));
        match out.last_mut() {
            Some(p) if p.objects == objects => p.range.end = i + 1,
            _ => out.push(Piece {
                objects,
                range: i..i + 1,
            }),
        }
    }
    out
}

/// The bake and blend class of a sprite layer drawn with `blend`.
pub fn blend_parts(blend: Blend) -> (SpriteBake, BlendClass) {
    match blend {
        Blend::Normal => (SpriteBake::Opaque, BlendClass::Premultiplied),
        Blend::Alpha25 => (
            SpriteBake::Tinted(weight_byte(0.25)),
            BlendClass::Premultiplied,
        ),
        Blend::Alpha50 => (
            SpriteBake::Tinted(weight_byte(0.5)),
            BlendClass::Premultiplied,
        ),
        Blend::Alpha75 => (
            SpriteBake::Tinted(weight_byte(0.75)),
            BlendClass::Premultiplied,
        ),
        // out = dst * level / 32: a black silhouette of coverage 1 - level/32.
        Blend::Darken(level) => (
            SpriteBake::Silhouette(weight_byte(1.0 - f32::from(level.min(32)) / 32.0)),
            BlendClass::Premultiplied,
        ),
        Blend::Additive => (SpriteBake::Opaque, BlendClass::Additive),
        Blend::Multiply => (SpriteBake::Opaque, BlendClass::Multiply),
        Blend::Screen => (SpriteBake::Opaque, BlendClass::Screen),
    }
}

/// One sprite layer to draw: its atlas entry, blend class and the world
/// position (1:1 pixels, y down) of its image's top-left at zoom `1:div`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerDraw {
    pub key: AtlasKey,
    pub class: BlendClass,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}

/// The layers of one object entry for `frame`, in draw order.
pub fn layer_draws(entry: &ObjectEntry, frame: &ObjectFrame, div: u32) -> Vec<LayerDraw> {
    let div = i32::try_from(div.clamp(1, 16)).unwrap_or(1);
    let (dx0, dy0) = object_anchor(entry.x, entry.y, div);
    let shadow_pass = entry.pass == DrawPass::ObjectShadows;
    let mut out = Vec::with_capacity(frame.layers.len());
    for layer in &frame.layers {
        let blend = if shadow_pass {
            match layer.shadow {
                Some(level) => Blend::Darken(level),
                None => continue,
            }
        } else {
            layer.blend
        };
        let (bake, class) = blend_parts(blend);
        let ox = dx0 + frame.offset.0 / div + layer.box_origin.0 / div;
        let oy = dy0 + frame.offset.1 / div + layer.box_origin.1 / div;
        let img = &layer.image;
        let off = (
            img.offset_x - layer.box_origin.0,
            img.offset_y - layer.box_origin.1,
        );
        out.push(LayerDraw {
            key: AtlasKey::sprite(img, bake),
            class,
            origin: (
                div.saturating_mul(ox).saturating_add(off.0),
                div.saturating_mul(oy).saturating_add(off.1),
            ),
            size: (img.width, img.height),
        });
    }
    out
}

/// Every atlas entry the object entries can need, over all their frames.
pub fn sprite_keys(entries: &[SceneEntry]) -> Vec<(AtlasKey, Arc<d2_formats::IndexedImage>)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for e in entries {
        let SceneEntry::Object(o) = e else { continue };
        for frame in &o.anim.frames {
            let shadow_pass = o.pass == DrawPass::ObjectShadows;
            for layer in &frame.layers {
                let blend = match (shadow_pass, layer.shadow) {
                    (true, Some(level)) => Blend::Darken(level),
                    (true, None) => continue,
                    (false, _) => layer.blend,
                };
                let key = AtlasKey::sprite(&layer.image, blend_parts(blend).0);
                if seen.insert(key.clone()) {
                    out.push((key, Arc::clone(&layer.image)));
                }
            }
        }
    }
    out
}

/// One mesh worth of quads.
#[derive(Debug, Default, PartialEq)]
pub struct Run {
    pub class: BlendClass,
    pub geo: TexGeometry,
}

fn push_run(runs: &mut Vec<Run>, class: BlendClass) -> &mut TexGeometry {
    if runs.last().is_none_or(|r| r.class != class) {
        runs.push(Run {
            class,
            geo: TexGeometry::default(),
        });
    }
    let last = runs.len() - 1;
    &mut runs[last].geo
}

/// The runs of one piece at zoom `1:div` and animation `tick`.
pub fn build_piece(
    entries: &[SceneEntry],
    piece: &Piece,
    layout: &AtlasLayout,
    div: u32,
    tile_geometry: &dyn Fn(&TileRef) -> Option<TileGeometry>,
    tick: Option<u32>,
) -> Vec<Run> {
    let mut runs = Vec::new();
    let inv = 1.0 / layout.size.max(1) as f32;
    for entry in entries.get(piece.range.clone()).unwrap_or(&[]) {
        match entry {
            SceneEntry::Tile(item) => {
                let Some(rect) = layout.rects.get(&geometry::atlas_key(item)) else {
                    continue;
                };
                let Some(geom) = tile_geometry(&item.tile) else {
                    continue;
                };
                let geo = push_run(&mut runs, BlendClass::Premultiplied);
                push_quad(geo, geometry::quad_origin(item, &geom, div), rect, inv);
            }
            SceneEntry::Object(o) => {
                let Some(frame) = o.anim.frame(tick) else {
                    continue;
                };
                for d in layer_draws(o, frame, div) {
                    let Some(rect) = layout.rects.get(&d.key) else {
                        continue;
                    };
                    let geo = push_run(&mut runs, d.class);
                    push_quad(geo, d.origin, rect, inv);
                }
            }
        }
    }
    runs
}

/// The frame number of every object entry of a piece at `tick`: when it
/// changes, the piece's meshes must be rebuilt.
pub fn piece_frames(entries: &[SceneEntry], piece: &Piece, tick: Option<u32>) -> Vec<usize> {
    entries
        .get(piece.range.clone())
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| match e {
            SceneEntry::Object(o) => Some(o.anim.frame_number(tick)),
            SceneEntry::Tile(_) => None,
        })
        .collect()
}

/// z of run `index` of `total`: strictly increasing in list order, inside
/// `[0, RUN_Z_MAX)`.
pub fn run_z(index: usize, total: usize) -> f32 {
    RUN_Z_MAX * index as f32 / total.max(1) as f32
}

/// Screen rectangles `(object index, [x0, y0, x1, y1])` at 1:1 (world
/// pixels, y down, max exclusive) of the drawn objects at `tick`, in draw
/// order, for picking. Shadows are not part of an object's box.
pub fn object_boxes(entries: &[SceneEntry], tick: Option<u32>) -> Vec<(usize, [i32; 4])> {
    let mut out = Vec::new();
    for e in entries {
        let SceneEntry::Object(o) = e else { continue };
        if o.pass == DrawPass::ObjectShadows {
            continue;
        }
        let Some(frame) = o.anim.frame(tick) else {
            continue;
        };
        let mut bounds: Option<[i32; 4]> = None;
        for d in layer_draws(o, frame, 1) {
            let (w, h) = (
                i32::try_from(d.size.0).unwrap_or(0),
                i32::try_from(d.size.1).unwrap_or(0),
            );
            if w == 0 || h == 0 {
                continue;
            }
            let r = [d.origin.0, d.origin.1, d.origin.0 + w, d.origin.1 + h];
            bounds = Some(match bounds {
                None => r,
                Some(b) => [
                    b[0].min(r[0]),
                    b[1].min(r[1]),
                    b[2].max(r[2]),
                    b[3].max(r[3]),
                ],
            });
        }
        if let Some(b) = bounds {
            out.push((o.object, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use capy_render::{FrameLayer, TileClass};
    use d2_formats::IndexedImage;

    use super::*;
    use crate::render::atlas::{PaletteKey, Rect};

    fn tile(order: u32) -> SceneEntry {
        SceneEntry::Tile(DrawItem {
            pass: DrawPass::Floors,
            order,
            layer: ('f', 0),
            x: 0,
            y: 0,
            tile: TileRef { dt1: 0, tile: 0 },
            screen_x: 10 * order as i32,
            screen_y: 0,
        })
    }

    fn image(w: u32, h: u32, ox: i32, oy: i32) -> Arc<IndexedImage> {
        let mut img = IndexedImage::new(w, h, ox, oy);
        img.pixels = vec![5; (w * h) as usize];
        img.alpha = vec![255; (w * h) as usize];
        Arc::new(img)
    }

    fn layer(image: Arc<IndexedImage>, blend: Blend, shadow: Option<u8>) -> FrameLayer {
        FrameLayer {
            component: 0,
            image,
            box_origin: (0, 0),
            blend,
            shadow,
        }
    }

    fn frame(layers: Vec<FrameLayer>) -> Arc<ObjectFrame> {
        Arc::new(ObjectFrame {
            layers,
            offset: (0, 0),
            order_flag: 0,
            frames_per_direction: 1,
            directions: 1,
        })
    }

    fn object(pass: DrawPass, frames: Vec<Arc<ObjectFrame>>, delta: u32) -> SceneEntry {
        SceneEntry::Object(ObjectEntry {
            pass,
            object: 0,
            x: 7,
            y: 7,
            anim: Arc::new(ObjectAnim { frames, delta }),
        })
    }

    /// A layout holding every key of `entries` plus the tile.
    fn layout_for(entries: &[SceneEntry]) -> AtlasLayout {
        let mut rects = HashMap::new();
        rects.insert(
            AtlasKey::tile(TileRef { dt1: 0, tile: 0 }, false),
            Rect {
                layer: 0,
                x: 0,
                y: 0,
                w: 4,
                h: 4,
            },
        );
        for (i, (key, img)) in sprite_keys(entries).into_iter().enumerate() {
            rects.insert(
                key,
                Rect {
                    layer: 0,
                    x: 8 + 8 * i as u32,
                    y: 0,
                    w: img.width,
                    h: img.height,
                },
            );
        }
        AtlasLayout {
            size: 256,
            layers: 1,
            palette: PaletteKey {
                act: 0,
                gamma_bits: 0,
            },
            rects,
            sprites: Vec::new(),
        }
    }

    fn flat(_: &TileRef) -> Option<TileGeometry> {
        Some(TileGeometry {
            class: TileClass::Floor,
            width: 160,
            height: 80,
            y_add: 0,
            zero_line: 0,
            roof_y: 0,
        })
    }

    #[test]
    fn pieces_alternate_between_tiles_and_objects() {
        let f = frame(vec![layer(image(2, 2, 0, 0), Blend::Normal, None)]);
        let entries = vec![
            tile(0),
            tile(1),
            object(DrawPass::UpperWalls, vec![f.clone()], 256),
            object(DrawPass::UpperWalls, vec![f], 256),
            tile(4),
        ];
        assert_eq!(
            pieces(&entries),
            vec![
                Piece {
                    objects: false,
                    range: 0..2
                },
                Piece {
                    objects: true,
                    range: 2..4
                },
                Piece {
                    objects: false,
                    range: 4..5
                },
            ]
        );
        assert!(pieces(&[]).is_empty());
    }

    #[test]
    fn runs_split_by_blend_class_and_keep_the_draw_order() {
        // Layers: normal, additive, additive, alpha50, screen, normal.
        let imgs: Vec<Arc<IndexedImage>> = (0..6).map(|i| image(1 + i, 1, 0, 0)).collect();
        let blends = [
            Blend::Normal,
            Blend::Additive,
            Blend::Additive,
            Blend::Alpha50,
            Blend::Screen,
            Blend::Normal,
        ];
        let layers = imgs
            .iter()
            .zip(blends)
            .map(|(img, b)| layer(img.clone(), b, None))
            .collect();
        let entries = vec![
            tile(0),
            object(DrawPass::UpperWalls, vec![frame(layers)], 256),
        ];
        let lay = layout_for(&entries);
        let ps = pieces(&entries);
        let tiles = build_piece(&entries, &ps[0], &lay, 1, &flat, None);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].class, BlendClass::Premultiplied);
        let runs = build_piece(&entries, &ps[1], &lay, 1, &flat, None);
        let classes: Vec<(BlendClass, usize)> =
            runs.iter().map(|r| (r.class, r.geo.quad_count())).collect();
        assert_eq!(
            classes,
            vec![
                (BlendClass::Premultiplied, 1),
                (BlendClass::Additive, 2),
                (BlendClass::Premultiplied, 1),
                (BlendClass::Screen, 1),
                (BlendClass::Premultiplied, 1),
            ]
        );
        // Concatenating the runs gives the layers in COF order (widths 1..=6).
        let widths: Vec<f32> = runs
            .iter()
            .flat_map(|r| {
                r.geo
                    .positions
                    .chunks(4)
                    .map(|q| q[1][0] - q[0][0])
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(widths, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn z_rises_strictly_in_run_order_below_the_outlines() {
        let total = 4000;
        let mut last = -1.0f32;
        for i in 0..total {
            let z = run_z(i, total);
            assert!(z > last, "run {i}");
            assert!((0.0..RUN_Z_MAX).contains(&z));
            last = z;
        }
        assert_eq!(run_z(0, 0), 0.0);
    }

    #[test]
    fn shadow_entries_draw_only_shadow_layers_as_silhouettes() {
        let f = frame(vec![
            layer(image(2, 2, 0, 0), Blend::Normal, Some(10)),
            layer(image(3, 3, 0, 0), Blend::Additive, None),
            layer(image(4, 4, 0, 0), Blend::Alpha50, Some(28)),
        ]);
        let entry = ObjectEntry {
            pass: DrawPass::ObjectShadows,
            object: 0,
            x: 0,
            y: 0,
            anim: Arc::new(ObjectAnim {
                frames: vec![f.clone()],
                delta: 256,
            }),
        };
        let draws = layer_draws(&entry, &f, 1);
        assert_eq!(draws.len(), 2);
        // Level 10: coverage 1 - 10/32 = 0.6875 -> 175; level 28 -> 32.
        assert_eq!(
            draws[0].key,
            AtlasKey::sprite(&f.layers[0].image, SpriteBake::Silhouette(175))
        );
        assert_eq!(
            draws[1].key,
            AtlasKey::sprite(&f.layers[2].image, SpriteBake::Silhouette(32))
        );
        assert!(draws.iter().all(|d| d.class == BlendClass::Premultiplied));
    }

    #[test]
    fn object_quads_follow_the_compositors_integer_placement() {
        // Sub-tile (7, 3), Objects.txt offset (-5, 9), box origin (-30, -61),
        // image crop offset (-27, -50) inside it, at 1:1 and 1:3.
        let img = image(10, 20, -27, -50);
        let f = Arc::new(ObjectFrame {
            layers: vec![FrameLayer {
                component: 0,
                image: img,
                box_origin: (-30, -61),
                blend: Blend::Normal,
                shadow: None,
            }],
            offset: (-5, 9),
            order_flag: 0,
            frames_per_direction: 1,
            directions: 1,
        });
        let entry = ObjectEntry {
            pass: DrawPass::UpperWalls,
            object: 0,
            x: 7,
            y: 3,
            anim: Arc::new(ObjectAnim {
                frames: vec![f.clone()],
                delta: 256,
            }),
        };
        let (ax, ay) = object_anchor(7, 3, 1);
        assert_eq!(
            layer_draws(&entry, &f, 1)[0].origin,
            (ax - 5 - 27, ay + 9 - 50)
        );
        // 1:3: dx = dx0 + (-5)/3 + (-30)/3 = dx0 - 1 - 10; the crop offset
        // inside the box (+3, +11) is added in source pixels.
        let (bx, by) = object_anchor(7, 3, 3);
        assert_eq!(
            layer_draws(&entry, &f, 3)[0].origin,
            (3 * (bx - 1 - 10) + 3, 3 * (by + 3 - 20) + 11)
        );
    }

    #[test]
    fn animation_tick_picks_frames_at_the_object_speed() {
        let frames: Vec<Arc<ObjectFrame>> = (0..4)
            .map(|i| frame(vec![layer(image(1 + i, 1, 0, 0), Blend::Normal, None)]))
            .collect();
        let anim = ObjectAnim {
            frames: frames.clone(),
            delta: 128, // half a frame per tick
        };
        assert_eq!(anim.frame_number(None), 0, "off = frame 0");
        let seen: Vec<usize> = (0..10).map(|t| anim.frame_number(Some(t))).collect();
        assert_eq!(seen, vec![0, 0, 1, 1, 2, 2, 3, 3, 0, 0]);
        let fast = ObjectAnim { frames, delta: 512 };
        assert_eq!(fast.frame_number(Some(3)), 2, "6 frames in, wraps at 4");
        assert!(Arc::ptr_eq(
            anim.frame(Some(5)).expect("frame"),
            &anim.frames[2]
        ));
        // A piece's frame vector changes only when a frame does.
        let entries = vec![SceneEntry::Object(ObjectEntry {
            pass: DrawPass::UpperWalls,
            object: 0,
            x: 0,
            y: 0,
            anim: Arc::new(anim),
        })];
        let p = &pieces(&entries)[0];
        assert_eq!(
            piece_frames(&entries, p, Some(2)),
            piece_frames(&entries, p, Some(3))
        );
        assert_ne!(
            piece_frames(&entries, p, Some(3)),
            piece_frames(&entries, p, Some(4))
        );
    }

    #[test]
    fn object_boxes_skip_shadows_and_cover_the_layers() {
        let f = frame(vec![
            layer(image(4, 6, -2, -6), Blend::Normal, Some(10)),
            layer(image(2, 2, 3, -1), Blend::Normal, None),
        ]);
        let entries = vec![
            object(DrawPass::ObjectShadows, vec![f.clone()], 256),
            object(DrawPass::UpperWalls, vec![f], 256),
        ];
        let boxes = object_boxes(&entries, None);
        assert_eq!(boxes.len(), 1);
        let (ax, ay) = object_anchor(7, 7, 1);
        assert_eq!(boxes[0].1, [ax - 2, ay - 6, ax + 5, ay + 1]);
    }
}
