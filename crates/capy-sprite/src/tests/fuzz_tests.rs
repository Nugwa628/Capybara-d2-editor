//! Fuzz-style robustness tests: random bytes, bit flips and truncations of
//! small valid images for every [`ImageKind`], and frames whose fields
//! disagree with their buffers for the frame operations. Nothing may panic
//! or allocate absurdly. Case counts follow `PROPTEST_CASES`.

use std::io::Cursor;

use d2_formats::IndexedImage;
use d2_formats::cof::Cof;
use image::{ExtendedColorType, ImageEncoder};
use proptest::prelude::*;

use crate::cof::{CofLayerSprite, compose_cof_frame};
use crate::image_io::{ImageKind, export_png, import_image};
use crate::model::{RgbaFrame, Sprite, SpriteFormat};
use crate::ops::{center_offset, join_frames, offset_all, split_frame, trim_borders};
use crate::quantize::{Dither, QuantizeOptions, quantize, to_rgba};
use crate::testutil::{extracted, find_files, synthetic_palette};
use crate::transfer::{export_sheet, import_sheet};

const KINDS: [ImageKind; 4] = [
    ImageKind::Png,
    ImageKind::Bmp,
    ImageKind::Gif,
    ImageKind::Pcx,
];

fn noise_frame(w: u32, h: u32, seed: u32) -> RgbaFrame {
    let mut f = RgbaFrame::new(w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let v = (x * 31 + y * 17 + seed).wrapping_mul(2_654_435_761);
            let a = if v.is_multiple_of(5) { 0 } else { 255 };
            f.put(x, y, [(v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8, a]);
        }
    }
    f
}

fn encode_with(frame: &RgbaFrame, kind: ImageKind) -> Vec<u8> {
    let mut out = Vec::new();
    let (w, h, px) = (frame.width, frame.height, &frame.pixels);
    match kind {
        ImageKind::Png => return export_png(frame).unwrap(),
        ImageKind::Bmp => image::codecs::bmp::BmpEncoder::new(&mut Cursor::new(&mut out))
            .write_image(px, w, h, ExtendedColorType::Rgba8)
            .unwrap(),
        ImageKind::Gif => image::codecs::gif::GifEncoder::new(&mut out)
            .encode(px, w, h, ExtendedColorType::Rgba8)
            .unwrap(),
        ImageKind::Pcx => return pcx_8bit(frame),
    }
    out
}

/// An 8-bit, one-plane, RLE PCX with a 256-colour palette after the data.
fn pcx_8bit(frame: &RgbaFrame) -> Vec<u8> {
    let (w, h) = (frame.width as u16, frame.height as u16);
    let per_line = w + (w & 1);
    let mut out = vec![0u8; 128];
    out[0] = 0x0A;
    out[1] = 5;
    out[2] = 1;
    out[3] = 8;
    out[8..10].copy_from_slice(&(w - 1).to_le_bytes());
    out[10..12].copy_from_slice(&(h - 1).to_le_bytes());
    out[65] = 1;
    out[66..68].copy_from_slice(&per_line.to_le_bytes());
    for y in 0..u32::from(h) {
        let row: Vec<u8> = (0..u32::from(per_line))
            .map(|x| frame.get(x, y).map_or(0, |p| p[0]))
            .collect();
        let mut i = 0;
        while i < row.len() {
            let v = row[i];
            let mut n = 1;
            while i + n < row.len() && row[i + n] == v && n < 63 {
                n += 1;
            }
            if n > 1 || v >= 0xC0 {
                out.push(0xC0 | n as u8);
            }
            out.push(v);
            i += n;
        }
    }
    out.push(0x0C);
    out.extend((0..=255u8).flat_map(|i| [i, 255 - i, i / 2]));
    out
}

/// Small valid files of every kind (1x1 up to 9x7).
fn seeds() -> Vec<(ImageKind, Vec<u8>)> {
    let mut out = Vec::new();
    for (w, h) in [(1, 1), (3, 2), (9, 7)] {
        let f = noise_frame(w, h, w * 7 + h);
        for kind in KINDS {
            out.push((kind, encode_with(&f, kind)));
        }
    }
    out
}

fn check_import(bytes: &[u8], kind: ImageKind) {
    if let Ok(f) = import_image(bytes, kind) {
        assert_eq!(
            f.pixels.len(),
            f.width as usize * f.height as usize * 4,
            "{kind:?}: {}x{}",
            f.width,
            f.height
        );
        assert!(f.width as usize <= 1 << 14 && f.height as usize <= 1 << 14);
    }
}

#[test]
fn seeds_are_valid() {
    for (kind, bytes) in seeds() {
        let f = import_image(&bytes, kind).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        assert!(f.width > 0 && f.height > 0);
    }
}

fn kind() -> impl Strategy<Value = ImageKind> {
    prop::sample::select(KINDS.to_vec())
}

proptest! {
    #[test]
    fn import_random_bytes_never_panic(
        kind in kind(),
        bytes in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        check_import(&bytes, kind);
    }

    /// Random bytes behind a correct magic, so the decoders get past the
    /// signature check.
    #[test]
    fn import_random_bodies_never_panic(
        kind in kind(),
        body in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let magic: &[u8] = match kind {
            ImageKind::Png => b"\x89PNG\r\n\x1a\n",
            ImageKind::Bmp => b"BM",
            ImageKind::Gif => b"GIF89a",
            ImageKind::Pcx => &[0x0A, 5, 1, 8],
        };
        let mut bytes = magic.to_vec();
        bytes.extend(body);
        check_import(&bytes, kind);
    }

    /// Bit flips, byte overwrites and truncations of small valid images.
    #[test]
    fn import_mutated_images_never_panic(
        seed in any::<prop::sample::Index>(),
        flips in prop::collection::vec((any::<prop::sample::Index>(), 0u8..8), 0..8),
        sets in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..4),
        cut in prop::option::of(any::<prop::sample::Index>()),
    ) {
        let all = seeds();
        let (kind, mut bytes) = all[seed.index(all.len())].clone();
        for (at, bit) in flips {
            let i = at.index(bytes.len());
            bytes[i] ^= 1 << bit;
        }
        for (at, v) in sets {
            let i = at.index(bytes.len());
            bytes[i] = v;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut.index(bytes.len() + 1));
        }
        check_import(&bytes, kind);
    }
}

// ---------------------------------------------------------------------------
// Huge declared sizes in tiny files

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_claiming(w: u32, h: u32, colour: u8) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut crc_in = kind.to_vec();
        crc_in.extend_from_slice(data);
        out.extend_from_slice(&crc_in);
        out.extend(crc32(&crc_in).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, colour, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    // A tiny zlib stream of zeros: far too short for the claimed size.
    chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01],
    );
    chunk(b"IEND", &[]);
    out
}

fn bmp_claiming(w: i32, h: i32) -> Vec<u8> {
    let mut out = b"BM".to_vec();
    out.extend(70u32.to_le_bytes()); // file size
    out.extend([0; 4]);
    out.extend(54u32.to_le_bytes()); // pixel data offset
    out.extend(40u32.to_le_bytes()); // BITMAPINFOHEADER
    out.extend(w.to_le_bytes());
    out.extend(h.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend([0; 24]);
    out.extend([0xFF; 16]);
    out
}

fn gif_claiming(w: u16, h: u16) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend(w.to_le_bytes());
    out.extend(h.to_le_bytes());
    out.extend([0x80, 0, 0]); // global table of 2 colours
    out.extend([0, 0, 0, 255, 255, 255]);
    out.push(0x2C); // image descriptor covering the screen
    out.extend([0, 0, 0, 0]);
    out.extend(w.to_le_bytes());
    out.extend(h.to_le_bytes());
    out.push(0);
    out.extend([2, 2, 0x4C, 0x01, 0]); // a few LZW codes
    out.push(0x3B);
    out
}

fn pcx_claiming(w: u16, h: u16, rle: bool) -> Vec<u8> {
    let mut out = vec![0u8; 128];
    out[0] = 0x0A;
    out[2] = u8::from(rle);
    out[3] = 8;
    out[8..10].copy_from_slice(&w.wrapping_sub(1).to_le_bytes());
    out[10..12].copy_from_slice(&h.wrapping_sub(1).to_le_bytes());
    out[65] = 4;
    out[66..68].copy_from_slice(&u16::MAX.to_le_bytes());
    out.extend([0xFF, 7, 0xFF, 9]);
    out
}

#[test]
fn huge_declared_sizes_are_errors() {
    let started = std::time::Instant::now();
    let mut files = Vec::new();
    for (w, h) in [
        (1 << 14, 1 << 14),
        (1 << 14, 4097),
        (16_385, 1),
        (100_000, 100_000),
        (u32::MAX >> 1, u32::MAX >> 1),
    ] {
        for colour in [0u8, 2, 3, 4, 6] {
            files.push((ImageKind::Png, png_claiming(w, h, colour)));
        }
    }
    for (w, h) in [
        (1 << 14, 1 << 14),
        (16_385, 1),
        (i32::MAX, i32::MAX),
        (i32::MAX, i32::MIN),
        (100_000, -100_000),
    ] {
        files.push((ImageKind::Bmp, bmp_claiming(w, h)));
    }
    for (w, h) in [(u16::MAX, u16::MAX), (16_385, 1), (1 << 14, 4097)] {
        files.push((ImageKind::Gif, gif_claiming(w, h)));
    }
    for (w, h) in [(u16::MAX, u16::MAX), (1 << 14, 1 << 12), (1 << 14, 1 << 14)] {
        files.push((ImageKind::Pcx, pcx_claiming(w, h, true)));
        files.push((ImageKind::Pcx, pcx_claiming(w, h, false)));
    }
    for (kind, bytes) in &files {
        assert!(
            import_image(bytes, *kind).is_err(),
            "{kind:?} {} bytes",
            bytes.len()
        );
    }
    // No decoder may have filled a gigabyte buffer on the way.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
}

/// PNG, BMP and GIF exports of the private corpus's first DC6 frames, bit
/// flipped. Skips when the corpus is absent; `CAPY_FUZZ_ITERS` sets the
/// mutations per file (default 40).
#[test]
fn import_mutated_corpus_frames_never_panic() {
    let Some(vanilla) = extracted("vanilla") else {
        eprintln!("private corpus absent, skipping");
        return;
    };
    let iters: usize = std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let pal = synthetic_palette();
    let files: Vec<_> = find_files(&vanilla.join("data/global/ui"), "dc6")
        .into_iter()
        .take(6)
        .collect();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut runs = 0usize;
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(sprite) = Sprite::decode(&bytes, SpriteFormat::Dc6) else {
            continue;
        };
        let Some(img) = sprite.directions.iter().flatten().find(|i| {
            i.width > 0 && i.height > 0 && u64::from(i.width) * u64::from(i.height) < 40_000
        }) else {
            continue;
        };
        let rgba = to_rgba(img, &pal);
        for kind in KINDS {
            let valid = encode_with(&rgba, kind);
            for _ in 0..iters {
                let mut m = valid.clone();
                for _ in 0..1 + next() % 6 {
                    let i = (next() % m.len() as u64) as usize;
                    m[i] ^= 1 << (next() % 8);
                }
                if next() % 4 == 0 {
                    m.truncate((next() % (m.len() as u64 + 1)) as usize);
                }
                check_import(&m, kind);
                runs += 1;
            }
        }
    }
    eprintln!("{runs} mutated corpus images ({iters} per file and kind)");
}

// ---------------------------------------------------------------------------
// Frames whose fields disagree with their buffers

/// Small sides, or sides so large that no buffer could back them.
fn side() -> impl Strategy<Value = u32> {
    prop_oneof![0u32..24, (1u32 << 28)..=u32::MAX]
}

fn offset() -> impl Strategy<Value = i32> {
    prop_oneof![-40i32..40, Just(i32::MIN), Just(i32::MAX), any::<i32>()]
}

fn indexed() -> impl Strategy<Value = IndexedImage> {
    (
        side(),
        side(),
        offset(),
        offset(),
        prop::collection::vec(any::<u8>(), 0..600),
        prop::collection::vec(any::<u8>(), 0..600),
    )
        .prop_map(
            |(width, height, offset_x, offset_y, pixels, alpha)| IndexedImage {
                width,
                height,
                offset_x,
                offset_y,
                pixels,
                alpha,
            },
        )
}

/// A consistent image with any offsets.
fn consistent() -> impl Strategy<Value = IndexedImage> {
    (1u32..10, 1u32..10, offset(), offset(), any::<u64>()).prop_map(|(w, h, ox, oy, seed)| {
        let n = (w * h) as usize;
        let pixels: Vec<u8> = (0..n)
            .map(|i| (seed.rotate_left(i as u32 % 64) >> 3) as u8 % 7)
            .collect();
        let alpha = pixels
            .iter()
            .map(|&p| if p == 0 { 0 } else { 255 })
            .collect();
        IndexedImage {
            width: w,
            height: h,
            offset_x: ox,
            offset_y: oy,
            pixels,
            alpha,
        }
    })
}

fn rgba_frame() -> impl Strategy<Value = RgbaFrame> {
    (
        side(),
        side(),
        offset(),
        offset(),
        prop::collection::vec(any::<u8>(), 0..2400),
    )
        .prop_map(|(width, height, offset_x, offset_y, pixels)| RgbaFrame {
            width,
            height,
            offset_x,
            offset_y,
            pixels,
        })
}

fn cof(layers: u8, frames: u8, dirs: u8) -> Option<Cof> {
    let mut bytes = vec![0u8; 28];
    bytes[0] = layers;
    bytes[1] = frames;
    bytes[2] = dirs;
    for comp in 0..layers {
        bytes.extend_from_slice(&[comp, 0, 0, 0, 0, b'H', b'T', b'H', 0]);
    }
    bytes.extend(std::iter::repeat_n(0, usize::from(frames)));
    for _ in 0..usize::from(dirs) * usize::from(frames) {
        bytes.extend(0..layers);
    }
    Cof::parse(&bytes).ok()
}

proptest! {
    #[test]
    fn quantize_and_to_rgba_survive_bad_frames(
        frame in rgba_frame(),
        img in indexed(),
        dither in any::<bool>(),
    ) {
        let pal = synthetic_palette();
        let opts = QuantizeOptions {
            dither: if dither { Dither::FloydSteinberg } else { Dither::None },
            ..QuantizeOptions::default()
        };
        let q = quantize(&frame, &pal, &opts);
        prop_assert_eq!(q.pixels.len(), q.width as usize * q.height as usize);
        let back = to_rgba(&img, &pal);
        prop_assert_eq!(back.pixels.len(), back.width as usize * back.height as usize * 4);
        let _ = export_png(&frame);
        let _ = frame.get(frame.width.saturating_sub(1), frame.height.saturating_sub(1));
        let mut f = frame.clone();
        f.put(f.width.saturating_sub(1), f.height.saturating_sub(1), [1, 2, 3, 4]);
    }

    #[test]
    fn frame_ops_survive_bad_frames(
        img in indexed(),
        others in prop::collection::vec(indexed(), 0..4),
        tile in (side(), side()),
        columns in prop_oneof![0u32..6, any::<u32>()],
        shift in (offset(), offset()),
    ) {
        let tiles = split_frame(&img, tile.0, tile.1);
        prop_assert!(!tiles.is_empty());
        let _ = trim_borders(&img);
        let mut c = img.clone();
        center_offset(&mut c);
        let mut all = vec![img];
        all.extend(others);
        let _ = join_frames(&all, columns);
        let mut sprite = Sprite { directions: vec![all.clone()] };
        let _ = sprite.bounds();
        offset_all(&mut sprite, shift.0, shift.1);
        let _ = sprite.encode(SpriteFormat::Dc6);
        let _ = sprite.encode(SpriteFormat::Dcc);
    }

    #[test]
    fn split_and_join_survive_extreme_tiles(
        img in consistent(),
        tw in prop_oneof![1u32..12, any::<u32>()],
        th in prop_oneof![1u32..12, any::<u32>()],
    ) {
        let tiles = split_frame(&img, tw, th);
        let covered: usize = tiles.iter().map(|t| t.pixels.len()).sum();
        prop_assert_eq!(covered, img.pixels.len());
        let columns = img.width.div_ceil(tw.max(1));
        let _ = join_frames(&tiles, columns);
    }

    /// Sheets from sprites with extreme offsets, and sheet imports with any
    /// grid.
    #[test]
    fn sheets_survive_extreme_offsets(
        frames in prop::collection::vec(prop::collection::vec(consistent(), 1..3), 1..3),
        empty_at in prop::option::of((offset(), offset())),
        rows in prop_oneof![0u32..5, any::<u32>()],
        cols in prop_oneof![0u32..5, any::<u32>()],
    ) {
        let pal = synthetic_palette();
        let per = frames[0].len();
        let mut dirs: Vec<Vec<IndexedImage>> =
            frames.into_iter().map(|mut d| { d.resize(per, d[0].clone()); d }).collect();
        if let Some((x, y)) = empty_at {
            dirs[0][0] = IndexedImage::new(0, 0, x, y);
        }
        let sprite = Sprite { directions: dirs };
        if let Ok((png, meta)) = export_sheet(&sprite, &pal) {
            let opts = QuantizeOptions::default();
            let _ = import_sheet(&png, meta.directions, meta.frames_per_direction, &pal, &opts, Some(&meta));
            let _ = import_sheet(&png, rows, cols, &pal, &opts, None);
        }
    }

    #[test]
    fn compose_cof_survives_bad_layers(
        layer_imgs in prop::collection::vec(indexed(), 1..4),
        shape in (1u8..4, 1u8..4, 1u8..3),
        dir in any::<u8>(),
        frame in any::<u8>(),
        effects in (any::<u8>(), any::<bool>()),
    ) {
        let pal = synthetic_palette();
        let Some(cof) = cof(shape.0, shape.1, shape.2) else {
            return Ok(());
        };
        let layers: Vec<CofLayerSprite> = layer_imgs
            .into_iter()
            .enumerate()
            .map(|(i, img)| CofLayerSprite {
                component: i as u8,
                sprite: Sprite { directions: vec![vec![img]; usize::from(shape.2)] },
                draw_effect: effects.0,
                transparent: effects.1,
            })
            .collect();
        if let Ok(out) = compose_cof_frame(&cof, &layers, dir, frame, &pal) {
            prop_assert_eq!(out.pixels.len(), out.width as usize * out.height as usize * 4);
        }
    }
}
