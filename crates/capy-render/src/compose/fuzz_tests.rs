//! Fuzz-style robustness tests of the renderer: hostile DT1 tiles (any
//! height, width, orientation, block position), DS1 cells and size fields,
//! object frames with extreme offsets, PNG input and image comparison.
//! Nothing may panic, hang or allocate absurdly. Case counts follow
//! `PROPTEST_CASES`; the corpus test's mutation count follows
//! `CAPY_FUZZ_ITERS`.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use d2_formats::IndexedImage;
use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::{Block, BlockFormat, Dt1, Tile};
use proptest::prelude::*;

use super::*;
use crate::compare::{CompareOptions, Coverage, compare_images_with};
use crate::objects::{FrameLayer, ObjectFrame, file_direction, frame_index, object_anchor};
use crate::order::{ObjectDrawItem, draw_list, map_screen_bounds};
use crate::testutil::{dt1, palette, tileset};

fn interesting_i32() -> impl Strategy<Value = i32> {
    prop_oneof![
        -300i32..300,
        Just(i32::MIN),
        Just(i32::MAX),
        Just(i32::MIN + 1),
        Just(i32::MAX - 1),
        any::<i32>(),
    ]
}

fn interesting_i16() -> impl Strategy<Value = i16> {
    prop_oneof![-200i16..200, Just(i16::MIN), Just(i16::MAX), any::<i16>()]
}

fn orientation() -> impl Strategy<Value = i32> {
    prop_oneof![
        prop::sample::select(vec![0, 1, 3, 4, 5, 10, 11, 13, 14, 15, 16, 18, 19, 20]),
        Just(-1),
        Just(i32::MIN),
        Just(i32::MAX),
        any::<i32>(),
    ]
}

fn block() -> impl Strategy<Value = Block> {
    (
        interesting_i16(),
        interesting_i16(),
        any::<bool>(),
        prop::collection::vec(any::<u8>(), 0..300),
    )
        .prop_map(|(x, y, iso, data)| Block {
            x,
            y,
            grid_x: 0,
            grid_y: 0,
            raw_format: if iso { 1 } else { 0 },
            format: if iso {
                BlockFormat::Isometric
            } else {
                BlockFormat::Rle
            },
            data,
        })
}

fn hostile_tile() -> impl Strategy<Value = Tile> {
    (
        orientation(),
        0i32..3,
        0i32..2,
        prop_oneof![0i32..4, any::<i32>()],
        prop_oneof![
            prop::sample::select(vec![-80, -96, -128, -32, 0]),
            interesting_i32()
        ],
        prop_oneof![Just(160), interesting_i32()],
        interesting_i16(),
        0u8..3,
        prop::collection::vec(block(), 0..4),
    )
        .prop_map(
            |(orientation, main, sub, rarity, height, width, roof_height, animated, blocks)| Tile {
                direction: 0,
                roof_height,
                sound: 0,
                animated,
                height,
                width,
                orientation,
                main_index: main,
                sub_index: sub,
                rarity,
                subtile_flags: [0; 25],
                blocks,
            },
        )
}

fn hostile_cell() -> impl Strategy<Value = Cell> {
    (any::<u8>(), 0u8..3, 0u8..2, any::<bool>()).prop_map(|(prop1, main, sub, hidden)| {
        Cell(
            u32::from(prop1)
                | (u32::from(sub) << 8)
                | (u32::from(main) << 20)
                | (u32::from(hidden) << 31),
        )
    })
}

/// A small map, every cell random, with optionally lying size fields.
fn hostile_map() -> impl Strategy<Value = Ds1> {
    (
        1u32..5,
        1u32..5,
        prop::collection::vec((hostile_cell(), hostile_cell(), hostile_cell()), 16),
        prop::collection::vec(any::<u32>(), 16),
        prop::sample::select(vec![0u32, 1, 1_000_000, u32::MAX]),
        prop::sample::select(vec![0u32, 1, 1_000_000, u32::MAX]),
        any::<i32>(),
    )
        .prop_map(|(w, h, cells, orientations, lie_w, lie_h, version)| {
            let mut ds1 = Ds1::new(w, h, 0);
            for y in 0..h {
                for x in 0..w {
                    let i = (y * 4 + x) as usize;
                    let (f, wall, s) = cells[i];
                    ds1.floors[0].set(x, y, f);
                    ds1.walls[0].tiles.set(x, y, wall);
                    let o = orientations[i];
                    // Mostly real orientations, sometimes any dword.
                    let o = if o % 3 == 0 { o } else { o % 21 };
                    ds1.walls[0].orientations.set(x, y, o);
                    ds1.shadow.set(x, y, s);
                }
            }
            // Lie about the size: the grids stay small.
            if lie_w > 1 {
                ds1.width = lie_w;
            }
            if lie_h > 1 {
                ds1.height = lie_h;
            }
            ds1.version = if version % 2 == 0 { 18 } else { version };
            ds1
        })
}

fn zoom() -> impl Strategy<Value = u32> {
    prop_oneof![0u32..20, Just(u32::MAX), any::<u32>()]
}

fn visibility() -> impl Strategy<Value = Visibility> {
    (any::<[bool; 2]>(), any::<[bool; 4]>(), any::<[bool; 4]>()).prop_map(|(floors, walls, f)| {
        Visibility {
            floors,
            walls,
            shadow: f[0],
            roofs: f[1],
            special: f[2],
            objects: f[3],
        }
    })
}

fn check_render(ds1: &Ds1, ts: &TileSet, zoom_div: u32, vis: Visibility, gamma: f32) {
    let opts = RenderOptions {
        zoom_div,
        gamma,
        visibility: vis.clone(),
        ..RenderOptions::default()
    };
    if let Ok(out) = render(ds1, ts, &palette(), &opts) {
        assert_eq!(
            out.image.pixels.len(),
            out.image.width as usize * out.image.height as usize * 4
        );
    }
    if let Ok((items, _)) = draw_list(ds1, ts, &vis) {
        let _ = map_screen_bounds(&items, ts);
    }
    let _ = reference_bounds(ds1, ts, zoom_div);
}

proptest! {
    #[test]
    fn render_survives_hostile_tiles_and_maps(
        tiles in prop::collection::vec(hostile_tile(), 1..8),
        ds1 in hostile_map(),
        zoom_div in zoom(),
        vis in visibility(),
        gamma in prop_oneof![Just(1.3f32), any::<f32>()],
    ) {
        let ts = tileset(vec![dt1(tiles)]);
        check_render(&ds1, &ts, zoom_div, vis, gamma);
    }

    #[test]
    fn tile_geometry_survives_any_tile(tile in hostile_tile(), div in any::<i32>()) {
        let g = TileGeometry::of(&tile);
        let _ = g.bitmap_top(div);
        let _ = g.bitmap_top(1);
        let ts = tileset(vec![dt1(vec![tile])]);
        let r = crate::tileset::TileRef { dt1: 0, tile: 0 };
        if let Ok(img) = ts.image(&r) {
            prop_assert_eq!(img.pixels.len(), img.width as usize * img.height as usize);
        }
        let _ = ts.animation_frame0(&r);
        let _ = ts.corner_pair(&r);
    }

    #[test]
    fn objects_survive_extreme_frames(
        layers in prop::collection::vec(
            (
                (prop_oneof![0u32..12, any::<u32>()], prop_oneof![0u32..12, any::<u32>()]),
                (interesting_i32(), interesting_i32()),
                (interesting_i32(), interesting_i32()),
                prop::collection::vec(any::<u8>(), 0..150),
                any::<u8>(),
                prop::option::of(any::<u8>()),
            ),
            0..4,
        ),
        offset in (interesting_i32(), interesting_i32()),
        pos in (interesting_i32(), interesting_i32()),
        origin in (interesting_i32(), interesting_i32()),
        div in prop_oneof![1i32..17, any::<i32>()],
        shadow_pass in any::<bool>(),
    ) {
        let blends = [Blend::Normal, Blend::Alpha50, Blend::Darken(10)];
        let frame = ObjectFrame {
            layers: layers
                .into_iter()
                .map(|((w, h), (ox, oy), (bx, by), px, b, shadow)| FrameLayer {
                    component: b % 16,
                    image: Arc::new(IndexedImage {
                        width: w,
                        height: h,
                        offset_x: ox,
                        offset_y: oy,
                        alpha: px.iter().map(|&p| p & 0x80).collect(),
                        pixels: px,
                    }),
                    box_origin: (bx, by),
                    blend: blends[usize::from(b % 3)],
                    shadow,
                })
                .collect(),
            offset,
            order_flag: 0,
            frames_per_direction: 1,
            directions: 1,
        };
        let pal = palette();
        if let Some((img, _)) = frame.composite(&pal) {
            prop_assert_eq!(img.pixels.len(), img.width as usize * img.height as usize * 4);
        }
        let item = ObjectDrawItem {
            pass: if shadow_pass { DrawPass::ObjectShadows } else { DrawPass::UpperWalls },
            order: 0,
            object: 0,
            kind: 1,
            id: 0,
            x: pos.0,
            y: pos.1,
            tile: (0, 0),
            screen_x: 0,
            screen_y: 0,
            frame: Arc::new(frame),
        };
        let mut canvas = RgbaImage::filled(9, 7, [0, 0, 0, 0]);
        draw_object(&mut canvas, &item, &pal, origin, div.max(1));
        let _ = object_anchor(pos.0, pos.1, div);
        let _ = object_anchor(i32::MIN, i32::MAX, div);
    }

    #[test]
    fn object_helpers_take_any_value(
        tick in prop::option::of(any::<u32>()),
        delta in any::<u32>(),
        count in any::<u32>(),
        dirs in any::<u8>(),
        user in any::<u8>(),
    ) {
        let _ = frame_index(tick, delta, count);
        let d = file_direction(dirs, user);
        prop_assert!(dirs == 0 || d < dirs);
    }

    #[test]
    fn from_png_survives_random_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
        signed in any::<bool>(),
    ) {
        let mut data = if signed { b"\x89PNG\r\n\x1a\n".to_vec() } else { Vec::new() };
        data.extend(bytes);
        let _ = RgbaImage::from_png(&data);
    }

    #[test]
    fn from_png_survives_mutations(
        w in 1u32..9,
        h in 1u32..9,
        flips in prop::collection::vec((any::<prop::sample::Index>(), 0u8..8), 0..8),
        cut in prop::option::of(any::<prop::sample::Index>()),
    ) {
        let mut img = RgbaImage::filled(w, h, [1, 2, 3, 4]);
        for (i, p) in img.pixels.iter_mut().enumerate() {
            *p = (i * 37) as u8;
        }
        let mut bytes = img.to_png().unwrap();
        for (at, bit) in flips {
            let i = at.index(bytes.len());
            bytes[i] ^= 1 << bit;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut.index(bytes.len() + 1));
        }
        if let Ok(back) = RgbaImage::from_png(&bytes) {
            prop_assert_eq!(back.pixels.len(), back.width as usize * back.height as usize * 4);
        }
    }

    #[test]
    fn compare_takes_any_shift_and_mask(
        (ow, oh) in (0u32..6, 0u32..6),
        (rw, rh) in (0u32..6, 0u32..6),
        ours_px in prop::collection::vec(any::<u8>(), 0..160),
        oracle_px in prop::collection::vec(any::<u8>(), 0..160),
        mask in prop::option::of(prop::collection::vec(any::<bool>(), 0..40)),
        max_shift in prop_oneof![-3i32..6, Just(i32::MAX), Just(i32::MIN), any::<i32>()],
        tolerance in any::<u8>(),
        both in any::<bool>(),
        div in any::<u32>(),
    ) {
        let ours = RgbaImage { width: ow, height: oh, pixels: ours_px };
        let oracle = RgbaImage { width: rw, height: rh, pixels: oracle_px };
        let report = compare_images_with(&ours, &oracle, &CompareOptions {
            tolerance,
            max_shift,
            coverage: if both { Coverage::Both } else { Coverage::Either },
            mask: mask.as_deref(),
        });
        prop_assert!(report.diff.within_tolerance.is_finite());
        let _ = downscale(&RgbaImage::filled(ow, oh, [9; 4]), div);
    }
}

/// PNG headers claiming huge images in a few bytes are errors, without a
/// matching allocation.
#[test]
fn from_png_rejects_huge_declared_sizes() {
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
    for (w, h, colour) in [
        (100_000u32, 100_000u32, 6u8),
        (1 << 20, 1 << 20, 2),
        (0x7FFF_FFFF, 0x7FFF_FFFF, 0),
        (40_000, 40_000, 6),
    ] {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, colour, 0, 0, 0]);
        png.extend(13u32.to_be_bytes());
        png.extend(&ihdr);
        png.extend(crc32(&ihdr).to_be_bytes());
        let idat = [b'I', b'D', b'A', b'T', 0x78, 0x9C, 0x63, 0, 0, 0, 1, 0, 1];
        png.extend(9u32.to_be_bytes());
        png.extend(idat);
        png.extend(crc32(&idat).to_be_bytes());
        assert!(RgbaImage::from_png(&png).is_err(), "{w}x{h}");
    }
}

/// A map whose size fields claim a million cells a side renders (only its
/// real grids are visited) instead of looping over the claim.
#[test]
fn lying_map_size_does_not_hang() {
    let ts = tileset(vec![dt1(vec![crate::testutil::tile(0, 1, 0, 0, 40)])]);
    let mut ds1 = Ds1::new(2, 2, 0);
    crate::testutil::set_floor(&mut ds1, 0, 1, 1, crate::testutil::cell(1, 1, 0));
    ds1.width = 1_000_000;
    ds1.height = u32::MAX;
    let started = std::time::Instant::now();
    let out = render(&ds1, &ts, &palette(), &RenderOptions::default()).unwrap();
    assert_eq!((out.image.width, out.image.height), (160, 80));
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
}

fn private_tiles_dir() -> Option<PathBuf> {
    let root = std::env::var_os("CAPY_PRIVATE_DIR").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .map(|a| a.join("testdata").join("private"))
                .find(|p| p.is_dir())
        },
        |d| Some(PathBuf::from(d)),
    )?;
    let dir = root.join("extracted/vanilla/data/global/tiles/act1/town");
    dir.is_dir().then_some(dir)
}

/// Real DT1 tiles from the private corpus with their fields and blocks
/// mutated, rendered on a small map. Skips when the corpus is absent.
#[test]
fn mutated_corpus_tiles_render_without_panics() {
    let Some(dir) = private_tiles_dir() else {
        eprintln!("private corpus absent, skipping");
        return;
    };
    let iters: usize = std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dt1")))
        .collect();
    files.sort();
    let Some(tiles) = files.iter().find_map(|p| {
        let dt1 = Dt1::parse(&std::fs::read(p).ok()?).ok()?;
        (dt1.tiles.len() >= 4).then_some(dt1.tiles)
    }) else {
        eprintln!("no usable DT1, skipping");
        return;
    };
    let tiles: Vec<Tile> = tiles.into_iter().take(24).collect();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let extremes = [i32::MIN, i32::MAX, -1, 0, 1 << 20, -(1 << 20)];
    for _ in 0..iters {
        let mut t = tiles.clone();
        for _ in 0..1 + next() % 6 {
            let k = (next() % t.len() as u64) as usize;
            let tile = &mut t[k];
            let e = extremes[(next() % extremes.len() as u64) as usize];
            match next() % 7 {
                0 => tile.height = e,
                1 => tile.width = e,
                2 => tile.roof_height = e as i16,
                3 => tile.rarity = e,
                4 if !tile.blocks.is_empty() => {
                    let b = (next() % tile.blocks.len() as u64) as usize;
                    tile.blocks[b].x = e as i16;
                    tile.blocks[b].y = (e >> 16) as i16;
                }
                5 if !tile.blocks.is_empty() => {
                    let b = (next() % tile.blocks.len() as u64) as usize;
                    let data = &mut tile.blocks[b].data;
                    let cut = (next() % (data.len() as u64 + 1)) as usize;
                    data.truncate(cut);
                }
                _ => tile.animated = (next() % 3) as u8,
            }
        }
        let mut ds1 = Ds1::new(4, 4, 0);
        for y in 0..4 {
            for x in 0..4 {
                let tile = &t[(next() % t.len() as u64) as usize];
                let main = tile.main_index.clamp(0, 63) as u8;
                let sub = tile.sub_index.clamp(0, 255) as u8;
                let c = Cell(1 | (u32::from(sub) << 8) | (u32::from(main) << 20));
                if tile.orientation == 0 {
                    ds1.floors[0].set(x, y, c);
                } else if tile.orientation == 13 {
                    ds1.shadow.set(x, y, c);
                } else {
                    ds1.walls[0].tiles.set(x, y, c);
                    ds1.walls[0]
                        .orientations
                        .set(x, y, tile.orientation.clamp(0, 255) as u32);
                }
            }
        }
        let ts = tileset(vec![dt1(t)]);
        let vis = Visibility {
            special: true,
            ..Visibility::default()
        };
        for zoom_div in [1, 3] {
            check_render(&ds1, &ts, zoom_div, vis.clone(), 1.3);
        }
    }
    eprintln!("{iters} mutated corpus tile sets rendered");
}
