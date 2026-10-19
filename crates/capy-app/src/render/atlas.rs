//! Texture atlas: shelf packing of tile images into square pages, and the
//! premultiplied RGBA bake.
//!
//! Every page is `size x size`; the pages are the layers of one 2D array
//! texture, so a map is a single mesh and a single draw call and the draw
//! order is just the index order. `size` is the smallest power of two (256 up
//! to [`MAX_PAGE`]) that holds everything in one page; if even [`MAX_PAGE`]
//! is not enough the atlas gets several layers.
//!
//! # Texel encoding (premultiplied alpha)
//!
//! * Terrain: opaque texels are `(r, g, b, 255)`, transparent ones `(0, 0, 0, 0)`.
//! * DT1 shadows: DS1Edit draws them in its 50% mode, `out = d * 127/255 +
//!   s * (128/255)^2` (`capy-render` measured this against the oracle). With
//!   the texel baked as `rgb = s * (128/255)^2`, `a = 128/255` and the
//!   premultiplied blend `src + dst * (1 - a)` the GPU computes exactly that.

use std::collections::HashMap;

use capy_render::TileRef;
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

/// Largest page edge. 4096 x 4096 x 4 bytes = 64 MiB per layer.
pub const MAX_PAGE: u32 = 4096;
const MIN_PAGE: u32 = 256;
/// Empty texels kept between neighbours so nearest sampling at a quad edge
/// never picks up the next tile.
pub const PAD: u32 = 1;

/// Shadow blend weight, `128/255` (the reference tints with a byte).
const SHADOW_WEIGHT: f64 = 128.0 / 255.0;

/// Where one image sits in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What an atlas entry is: a tile, baked either as terrain or as a shadow.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AtlasKey {
    pub tile: TileRef,
    pub shadow: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packing {
    pub size: u32,
    pub layers: u32,
    /// One rectangle per input, in input order.
    pub rects: Vec<Rect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    /// Input `index` (with padding) does not fit in a page of [`MAX_PAGE`].
    TooLarge { index: usize, w: u32, h: u32 },
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::TooLarge { index, w, h } => {
                write!(
                    f,
                    "tile image #{index} ({w}x{h}) does not fit in a {MAX_PAGE} page"
                )
            }
        }
    }
}

impl std::error::Error for PackError {}

/// Shelf-packs `sizes` into pages of exactly `size`. Returns `None` for an
/// input that cannot fit a page at all.
fn shelf_pack(sizes: &[(u32, u32)], size: u32) -> Option<(Vec<Rect>, u32)> {
    // Tallest first, then widest, so shelves waste little height.
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| {
        (
            std::cmp::Reverse(sizes[i].1),
            std::cmp::Reverse(sizes[i].0),
            i,
        )
    });
    let mut rects = vec![
        Rect {
            layer: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0
        };
        sizes.len()
    ];
    let (mut layer, mut x, mut y, mut row_h) = (0u32, 0u32, 0u32, 0u32);
    let mut any = false;
    for &i in &order {
        let (w, h) = sizes[i];
        if w > size || h > size {
            return None;
        }
        if w == 0 || h == 0 {
            rects[i] = Rect {
                layer,
                x: 0,
                y: 0,
                w,
                h,
            };
            continue;
        }
        any = true;
        if x + w > size {
            // Next shelf.
            y += row_h + PAD;
            x = 0;
            row_h = 0;
        }
        if y + h > size {
            // Next page.
            layer += 1;
            x = 0;
            y = 0;
            row_h = 0;
        }
        rects[i] = Rect { layer, x, y, w, h };
        x += w + PAD;
        row_h = row_h.max(h);
    }
    Some((rects, if any { layer + 1 } else { 1 }))
}

/// Packs images of the given `(width, height)` sizes.
pub fn pack(sizes: &[(u32, u32)]) -> Result<Packing, PackError> {
    let too_large = || {
        sizes
            .iter()
            .position(|&(w, h)| w > MAX_PAGE || h > MAX_PAGE)
            .map(|index| PackError::TooLarge {
                index,
                w: sizes[index].0,
                h: sizes[index].1,
            })
    };
    if let Some(e) = too_large() {
        return Err(e);
    }
    let mut size = MIN_PAGE;
    while size < MAX_PAGE {
        if let Some((rects, 1)) = shelf_pack(sizes, size) {
            return Ok(Packing {
                size,
                layers: 1,
                rects,
            });
        }
        size *= 2;
    }
    match shelf_pack(sizes, MAX_PAGE) {
        Some((rects, layers)) => Ok(Packing {
            size: MAX_PAGE,
            layers,
            rects,
        }),
        None => Err(too_large().unwrap_or(PackError::TooLarge {
            index: 0,
            w: 0,
            h: 0,
        })),
    }
}

/// Premultiplied shadow colour of one palette channel value:
/// `round(s * (128/255)^2)`.
pub fn shadow_texel(s: u8) -> u8 {
    (f64::from(s) * SHADOW_WEIGHT * SHADOW_WEIGHT + 0.5) as u8
}

/// Alpha byte of a baked shadow texel (`128`).
pub const SHADOW_ALPHA: u8 = 128;

/// Bakes `image` through `palette` (already gamma-corrected) into premultiplied
/// RGBA bytes, `width * height * 4`.
pub fn bake(image: &IndexedImage, palette: &Palette, shadow: bool) -> Vec<u8> {
    let n = (image.width as usize) * (image.height as usize);
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let alpha = image.alpha.get(i).copied().unwrap_or(0);
        if alpha == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let index = image.pixels.get(i).copied().unwrap_or(0);
        let [r, g, b] = palette.colors[usize::from(index)];
        if shadow {
            out.extend_from_slice(&[
                shadow_texel(r),
                shadow_texel(g),
                shadow_texel(b),
                SHADOW_ALPHA,
            ]);
        } else {
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

/// Which texels a palette/gamma combination was baked with; an atlas is only
/// reusable for the same combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteKey {
    pub act: u8,
    pub gamma_bits: u32,
}

/// The placement table of an atlas, shared with the mesh builder.
#[derive(Clone, Debug)]
pub struct AtlasLayout {
    pub size: u32,
    pub layers: u32,
    pub palette: PaletteKey,
    pub rects: HashMap<AtlasKey, Rect>,
}

impl AtlasLayout {
    /// True when every key is already placed.
    pub fn covers<'a>(&self, keys: impl IntoIterator<Item = &'a AtlasKey>) -> bool {
        keys.into_iter().all(|k| self.rects.contains_key(k))
    }
}

/// Pixel data of an atlas: `layers` pages of `size * size * 4` bytes.
pub struct AtlasPixels {
    pub size: u32,
    pub layers: u32,
    pub data: Vec<u8>,
}

/// Packs and bakes `entries` (each one an image and whether to bake it as a
/// shadow). Duplicate keys are packed once.
pub fn build(
    entries: &[(AtlasKey, std::sync::Arc<IndexedImage>)],
    palette: &Palette,
    palette_key: PaletteKey,
) -> Result<(AtlasLayout, AtlasPixels), PackError> {
    let mut seen: HashMap<&AtlasKey, ()> = HashMap::new();
    let unique: Vec<&(AtlasKey, std::sync::Arc<IndexedImage>)> = entries
        .iter()
        .filter(|(k, _)| seen.insert(k, ()).is_none())
        .collect();
    let sizes: Vec<(u32, u32)> = unique
        .iter()
        .map(|(_, img)| (img.width, img.height))
        .collect();
    let packing = pack(&sizes)?;
    let page_bytes = (packing.size as usize) * (packing.size as usize) * 4;
    let mut data = vec![0u8; page_bytes * packing.layers as usize];
    let mut rects = HashMap::with_capacity(unique.len());
    for ((key, img), rect) in unique.iter().zip(&packing.rects) {
        let baked = bake(img, palette, key.shadow);
        let row_bytes = (img.width as usize) * 4;
        for row in 0..img.height as usize {
            let dst = rect.layer as usize * page_bytes
                + ((rect.y as usize + row) * packing.size as usize + rect.x as usize) * 4;
            let src = row * row_bytes;
            if let (Some(d), Some(s)) = (
                data.get_mut(dst..dst + row_bytes),
                baked.get(src..src + row_bytes),
            ) {
                d.copy_from_slice(s);
            }
        }
        rects.insert(key.clone(), *rect);
    }
    Ok((
        AtlasLayout {
            size: packing.size,
            layers: packing.layers,
            palette: palette_key,
            rects,
        },
        AtlasPixels {
            size: packing.size,
            layers: packing.layers,
            data,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.layer == b.layer
            && a.w > 0
            && b.w > 0
            && a.h > 0
            && b.h > 0
            && a.x < b.x + b.w
            && b.x < a.x + a.w
            && a.y < b.y + b.h
            && b.y < a.y + a.h
    }

    /// Cheap deterministic pseudo-random sizes (no rand dependency).
    fn sizes(n: usize, seed: u32) -> Vec<(u32, u32)> {
        let mut s = seed;
        let mut next = move || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            s >> 8
        };
        (0..n)
            .map(|_| (16 + next() % 145, 8 + next() % 233))
            .collect()
    }

    fn check_valid(sizes: &[(u32, u32)], p: &Packing) {
        assert_eq!(p.rects.len(), sizes.len());
        for (r, &(w, h)) in p.rects.iter().zip(sizes) {
            assert_eq!((r.w, r.h), (w, h));
            assert!(r.layer < p.layers);
            assert!(r.x + r.w <= p.size && r.y + r.h <= p.size, "{r:?} outside");
        }
        for i in 0..p.rects.len() {
            for j in (i + 1)..p.rects.len() {
                assert!(
                    !overlaps(&p.rects[i], &p.rects[j]),
                    "{:?} overlaps {:?}",
                    p.rects[i],
                    p.rects[j]
                );
            }
        }
    }

    #[test]
    fn small_sets_use_one_small_page() {
        let s = sizes(20, 1);
        let p = pack(&s).expect("pack");
        assert_eq!(p.layers, 1);
        assert!(p.size <= 1024, "page {}", p.size);
        check_valid(&s, &p);
    }

    #[test]
    fn page_grows_to_fit_one_layer_before_adding_layers() {
        let s = sizes(600, 7);
        let p = pack(&s).expect("pack");
        check_valid(&s, &p);
        if p.layers == 1 {
            assert!(p.size > MIN_PAGE);
        }
    }

    #[test]
    fn overflow_adds_layers_at_the_maximum_page() {
        // Each 2000 x 2000 image takes a quarter of a 4096 page: nine of them
        // need three layers.
        let s = vec![(2000, 2000); 9];
        let p = pack(&s).expect("pack");
        assert_eq!(p.size, MAX_PAGE);
        assert_eq!(p.layers, 3);
        check_valid(&s, &p);
    }

    #[test]
    fn neighbours_keep_a_gap_and_oversize_is_an_error() {
        let p = pack(&[(10, 10), (10, 10)]).expect("pack");
        let (a, b) = (p.rects[0], p.rects[1]);
        assert!(a.x + a.w < b.x || b.x + b.w < a.x || a.y + a.h < b.y || b.y + b.h < a.y);
        assert!(matches!(
            pack(&[(5, 5), (MAX_PAGE + 1, 4)]),
            Err(PackError::TooLarge { index: 1, .. })
        ));
        // Empty input and empty images are fine.
        assert_eq!(pack(&[]).expect("empty").layers, 1);
        let z = pack(&[(0, 0), (4, 4)]).expect("zero");
        assert_eq!(z.rects[0].w, 0);
    }

    #[test]
    fn shadow_bake_blends_to_the_reference_formula() {
        // Premultiplied blend on the GPU: out = src + dst * (1 - a).
        // Reference: out = d * 127/255 + s * (128/255)^2.
        let a = f64::from(SHADOW_ALPHA) / 255.0;
        for s in 0..=255u8 {
            let baked = f64::from(shadow_texel(s));
            for d in [0u8, 1, 17, 64, 127, 128, 200, 254, 255] {
                let gpu = baked + f64::from(d) * (1.0 - a);
                let reference =
                    f64::from(d) * 127.0 / 255.0 + f64::from(s) * (128.0 / 255.0) * (128.0 / 255.0);
                assert!(
                    (gpu - reference).abs() <= 1.0 / 2.0 + 1e-9,
                    "s={s} d={d}: gpu {gpu} reference {reference}"
                );
                // After the 8-bit store the two agree within one level.
                assert!((gpu.round() - reference.round()).abs() <= 1.0);
            }
        }
        assert_eq!(shadow_texel(0), 0);
        assert_eq!(shadow_texel(255), 64); // 255 * 0.2520 = 64.26
    }

    #[test]
    fn bake_encodes_terrain_opaque_and_shadow_premultiplied() {
        let mut colors = [[0u8; 3]; 256];
        colors[1] = [200, 100, 50];
        let pal = Palette { colors };
        let mut img = IndexedImage::new(2, 1, 0, 0);
        img.pixels = vec![1, 1];
        img.alpha = vec![255, 0];
        assert_eq!(bake(&img, &pal, false), vec![200, 100, 50, 255, 0, 0, 0, 0]);
        let sh = bake(&img, &pal, true);
        assert_eq!(&sh[4..], &[0, 0, 0, 0], "transparent stays (0,0,0,0)");
        assert_eq!(sh[3], SHADOW_ALPHA);
        assert_eq!(sh[0], shadow_texel(200));
    }

    #[test]
    fn build_places_pixels_at_the_rectangles() {
        let mut colors = [[0u8; 3]; 256];
        colors[3] = [9, 8, 7];
        colors[4] = [90, 80, 70];
        let pal = Palette { colors };
        let mk = |idx: u8, w: u32, h: u32| {
            let mut img = IndexedImage::new(w, h, 0, 0);
            img.pixels = vec![idx; (w * h) as usize];
            img.alpha = vec![255; (w * h) as usize];
            std::sync::Arc::new(img)
        };
        let key = |dt1, tile, shadow| AtlasKey {
            tile: TileRef { dt1, tile },
            shadow,
        };
        let entries = vec![
            (key(0, 0, false), mk(3, 5, 4)),
            (key(0, 1, false), mk(4, 7, 3)),
            (key(0, 0, false), mk(3, 5, 4)), // duplicate
        ];
        let pk = PaletteKey {
            act: 0,
            gamma_bits: 1.3f32.to_bits(),
        };
        let (layout, pixels) = build(&entries, &pal, pk).expect("build");
        assert_eq!(layout.rects.len(), 2);
        assert!(layout.covers([&key(0, 0, false), &key(0, 1, false)]));
        assert!(!layout.covers([&key(0, 0, true)]));
        for (k, expect) in [
            (key(0, 0, false), [9, 8, 7, 255]),
            (key(0, 1, false), [90, 80, 70, 255]),
        ] {
            let r = layout.rects[&k];
            for (px, py) in [(0, 0), (r.w - 1, r.h - 1)] {
                let i = (((r.y + py) * layout.size + r.x + px) * 4) as usize;
                assert_eq!(&pixels.data[i..i + 4], &expect, "{k:?} at {px},{py}");
            }
        }
        // The gap texels stay transparent.
        let r = layout.rects[&key(0, 0, false)];
        let i = (((r.y) * layout.size + r.x + r.w) * 4) as usize;
        assert_eq!(&pixels.data[i..i + 4], &[0, 0, 0, 0]);
    }
}
