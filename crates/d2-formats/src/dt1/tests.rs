use super::*;
use crate::testutil;
use proptest::prelude::*;
use std::collections::BTreeMap;

mod fuzz;

// ---------------------------------------------------------------------------
// An independent DT1 builder written from the layout in the module docs.

#[derive(Clone, Debug)]
struct BlockSpec {
    x: i16,
    y: i16,
    grid: (u8, u8),
    format: i16,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct TileSpec {
    direction: i32,
    roof: i16,
    height: i32,
    width: i32,
    orientation: i32,
    main: i32,
    sub: i32,
    rarity: i32,
    flags: [u8; 25],
    blocks: Vec<BlockSpec>,
}

impl TileSpec {
    fn new(orientation: i32, main: i32, sub: i32, blocks: Vec<BlockSpec>) -> Self {
        Self {
            direction: 0,
            roof: 0,
            height: -80,
            width: 160,
            orientation,
            main,
            sub,
            rarity: 1,
            flags: [0; 25],
            blocks,
        }
    }
}

fn iso_data(seed: u8) -> Vec<u8> {
    (0..256u32)
        .map(|i| (i as u8).wrapping_add(seed) | 1)
        .collect()
}

fn build(tiles: &[TileSpec]) -> Vec<u8> {
    let mut out = vec![0u8; 276];
    out[0..4].copy_from_slice(&7i32.to_le_bytes());
    out[4..8].copy_from_slice(&6i32.to_le_bytes());
    out[268..272].copy_from_slice(&(tiles.len() as i32).to_le_bytes());
    out[272..276].copy_from_slice(&276i32.to_le_bytes());
    let headers_end = 276 + 96 * tiles.len();
    out.resize(headers_end, 0);
    for (i, t) in tiles.iter().enumerate() {
        let tiles_ptr = out.len();
        let n = t.blocks.len();
        let mut hdrs = vec![0u8; 20 * n];
        let mut data = Vec::new();
        let data_base = 20 * n;
        for (s, b) in t.blocks.iter().enumerate() {
            let h = &mut hdrs[s * 20..s * 20 + 20];
            h[0..2].copy_from_slice(&b.x.to_le_bytes());
            h[2..4].copy_from_slice(&b.y.to_le_bytes());
            h[6] = b.grid.0;
            h[7] = b.grid.1;
            h[8..10].copy_from_slice(&b.format.to_le_bytes());
            h[10..12].copy_from_slice(&(b.data.len() as u16).to_le_bytes());
            h[16..20].copy_from_slice(&((data_base + data.len()) as i32).to_le_bytes());
            data.extend_from_slice(&b.data);
        }
        out.extend_from_slice(&hdrs);
        out.extend_from_slice(&data);
        let h = &mut out[276 + 96 * i..276 + 96 * (i + 1)];
        h[0..4].copy_from_slice(&t.direction.to_le_bytes());
        h[4..6].copy_from_slice(&t.roof.to_le_bytes());
        h[6] = 3;
        h[7] = 1;
        h[8..12].copy_from_slice(&t.height.to_le_bytes());
        h[12..16].copy_from_slice(&t.width.to_le_bytes());
        h[20..24].copy_from_slice(&t.orientation.to_le_bytes());
        h[24..28].copy_from_slice(&t.main.to_le_bytes());
        h[28..32].copy_from_slice(&t.sub.to_le_bytes());
        h[32..36].copy_from_slice(&t.rarity.to_le_bytes());
        h[40..65].copy_from_slice(&t.flags);
        h[72..76].copy_from_slice(&(tiles_ptr as i32).to_le_bytes());
        h[76..80].copy_from_slice(&((20 * n + data.len()) as i32).to_le_bytes());
        h[80..84].copy_from_slice(&(n as i32).to_le_bytes());
    }
    out
}

fn px(img: &IndexedImage, x: i32, y: i32) -> Option<(u8, u8)> {
    let (cx, cy) = (x - img.offset_x, y - img.offset_y);
    if cx < 0 || cy < 0 || cx >= img.width as i32 || cy >= img.height as i32 {
        return None;
    }
    let i = cy as usize * img.width as usize + cx as usize;
    Some((img.pixels[i], img.alpha[i]))
}

// ---------------------------------------------------------------------------

#[test]
fn zero_tiles_parse() {
    let bytes = build(&[]);
    let dt1 = Dt1::parse(&bytes).unwrap();
    assert_eq!(dt1.version, (7, 6));
    assert!(dt1.tiles.is_empty());
}

#[test]
fn tile_without_blocks_renders_empty() {
    let bytes = build(&[TileSpec::new(0, 0, 0, vec![])]);
    let dt1 = Dt1::parse(&bytes).unwrap();
    let img = dt1.tiles[0].render().unwrap();
    assert_eq!((img.width, img.height), (0, 0));
}

#[test]
fn isometric_block_renders_known_pixels() {
    let data = iso_data(0);
    let bytes = build(&[TileSpec::new(
        0,
        4,
        9,
        vec![BlockSpec {
            x: 32,
            y: 16,
            grid: (1, 0),
            format: 1,
            data: data.clone(),
        }],
    )]);
    let dt1 = Dt1::parse(&bytes).unwrap();
    let tile = &dt1.tiles[0];
    assert_eq!(tile.blocks[0].format, BlockFormat::Isometric);
    assert_eq!((tile.blocks[0].grid_x, tile.blocks[0].grid_y), (1, 0));
    assert_eq!(
        tile.key(),
        Some(TileKey {
            orientation: 0,
            main: 4,
            sub: 9
        })
    );
    let img = tile.render().unwrap();
    assert_eq!((img.width, img.height), (32, 15));
    assert_eq!((img.offset_x, img.offset_y), (32, 16));
    // row 0: 4 pixels starting at x = 14; row 7: the full 32
    assert_eq!(px(&img, 32 + 14, 16), Some((data[0], 255)));
    assert_eq!(px(&img, 32 + 17, 16), Some((data[3], 255)));
    assert_eq!(px(&img, 32 + 13, 16), Some((0, 0)), "outside the diamond");
    assert_eq!(px(&img, 32 + 18, 16), Some((0, 0)));
    let row7_start: usize = [4, 8, 12, 16, 20, 24, 28].iter().sum();
    assert_eq!(px(&img, 32, 16 + 7), Some((data[row7_start], 255)));
    assert_eq!(
        px(&img, 32 + 31, 16 + 7),
        Some((data[row7_start + 31], 255))
    );
    // all 256 pixels are present exactly once
    assert_eq!(img.alpha.iter().filter(|&&a| a == 255).count(), 256);
}

#[test]
fn isometric_block_with_wrong_length_draws_nothing() {
    let mut data = iso_data(0);
    data.pop();
    let bytes = build(&[TileSpec::new(
        0,
        0,
        0,
        vec![BlockSpec {
            x: 0,
            y: 0,
            grid: (0, 0),
            format: 1,
            data,
        }],
    )]);
    let img = Dt1::parse(&bytes).unwrap().tiles[0].render().unwrap();
    assert_eq!((img.width, img.height), (0, 0));
}

#[test]
fn rle_block_renders_with_skips_and_rows() {
    // row 0: skip 2, 3 literals; row 1: skip 0, 1 literal; row 2 (after an
    // empty row 1 marker): skip 5, 2 literals
    let data = vec![
        2, 3, 11, 12, 13, // row 0
        0, 0, // end of row
        0, 1, 21, // row 1
        0, 0, // end of row
        5, 2, 31, 32, // row 2
    ];
    let bytes = build(&[TileSpec::new(
        3,
        1,
        2,
        vec![BlockSpec {
            x: -10,
            y: -40,
            grid: (0, 0),
            format: 0,
            data,
        }],
    )]);
    let dt1 = Dt1::parse(&bytes).unwrap();
    assert_eq!(dt1.tiles[0].blocks[0].format, BlockFormat::Rle);
    let img = dt1.tiles[0].render().unwrap();
    assert_eq!((img.offset_x, img.offset_y), (-10, -40));
    assert_eq!(px(&img, -8, -40), Some((11, 255)));
    assert_eq!(px(&img, -6, -40), Some((13, 255)));
    assert_eq!(px(&img, -10, -39), Some((21, 255)));
    assert_eq!(px(&img, -5, -38), Some((31, 255)));
    assert_eq!(px(&img, -4, -38), Some((32, 255)));
    assert_eq!(px(&img, -9, -40), Some((0, 0)));
    assert_eq!((img.width, img.height), (7, 3));
}

#[test]
fn rle_literal_zero_is_transparent_and_overwrites() {
    let a = vec![0, 4, 5, 6, 7, 8]; // x 0..4 = 5,6,7,8
    let b = vec![1, 2, 0, 9]; // x 1 = 0 (transparent), x 2 = 9
    let bytes = build(&[TileSpec::new(
        1,
        0,
        0,
        vec![
            BlockSpec {
                x: 0,
                y: 0,
                grid: (0, 0),
                format: 2,
                data: a,
            },
            BlockSpec {
                x: 0,
                y: 0,
                grid: (0, 0),
                format: 2,
                data: b,
            },
        ],
    )]);
    let img = Dt1::parse(&bytes).unwrap().tiles[0].render().unwrap();
    assert_eq!(px(&img, 0, 0), Some((5, 255)));
    assert_eq!(
        px(&img, 1, 0),
        Some((0, 0)),
        "zero overwrote the earlier pixel"
    );
    assert_eq!(px(&img, 2, 0), Some((9, 255)), "later block wins");
    assert_eq!(px(&img, 3, 0), Some((8, 255)));
}

#[test]
fn rle_truncated_literal_run_is_clamped() {
    // claims 5 literals, only 3 present
    let bytes = build(&[TileSpec::new(
        1,
        0,
        0,
        vec![BlockSpec {
            x: 0,
            y: 0,
            grid: (0, 0),
            format: 0,
            data: vec![0, 5, 1, 2, 7],
        }],
    )]);
    let img = Dt1::parse(&bytes).unwrap().tiles[0].render().unwrap();
    assert_eq!((img.width, img.height), (3, 1));
}

#[test]
fn floor_wall_roof_tiles_and_flag_reorder() {
    let mut floor = TileSpec::new(0, 1, 2, vec![]);
    floor.rarity = 7;
    floor.direction = 3;
    floor.roof = -5;
    // file byte t belongs to grid cell (x = t % 5, y = 4 - t / 5)
    for (t, f) in floor.flags.iter_mut().enumerate() {
        *f = t as u8 + 1;
    }
    let mut wall = TileSpec::new(1, 63, 255, vec![]);
    wall.height = -192;
    wall.width = 160;
    let mut roof = TileSpec::new(15, 0, 0, vec![]);
    roof.roof = 120;
    let bad_index = TileSpec::new(256, 0, 0, vec![]);
    let neg_index = TileSpec::new(1, -1, 0, vec![]);
    let dt1 = Dt1::parse(&build(&[floor, wall, roof, bad_index, neg_index])).unwrap();
    assert_eq!(dt1.tiles.len(), 5);
    let t = &dt1.tiles[0];
    assert_eq!((t.rarity, t.direction, t.roof_height), (7, 3, -5));
    assert_eq!((t.sound, t.animated), (3, 1));
    assert_eq!((t.height, t.width), (-80, 160));
    for tt in 0..25usize {
        let (x, y) = (tt % 5, 4 - tt / 5);
        assert_eq!(t.subtile_flags[y * 5 + x], tt as u8 + 1, "t={tt}");
    }
    assert_eq!(dt1.tiles[1].height, -192);
    assert_eq!(
        dt1.tiles[1].key(),
        Some(TileKey {
            orientation: 1,
            main: 63,
            sub: 255
        })
    );
    assert_eq!(dt1.tiles[2].roof_height, 120);
    assert_eq!(dt1.tiles[3].key(), None);
    assert_eq!(dt1.tiles[4].key(), None);
}

#[test]
fn many_tiles_keep_their_own_blocks() {
    let tiles: Vec<TileSpec> = (0..6)
        .map(|i| {
            TileSpec::new(
                0,
                i,
                i + 1,
                (0..i as usize)
                    .map(|b| BlockSpec {
                        x: b as i16 * 32,
                        y: 0,
                        grid: (b as u8, 0),
                        format: 1,
                        data: iso_data(i as u8 * 10 + b as u8),
                    })
                    .collect(),
            )
        })
        .collect();
    let dt1 = Dt1::parse(&build(&tiles)).unwrap();
    for (i, t) in dt1.tiles.iter().enumerate() {
        assert_eq!(t.blocks.len(), i);
        for (b, block) in t.blocks.iter().enumerate() {
            assert_eq!(block.data, iso_data(i as u8 * 10 + b as u8));
        }
    }
}

#[test]
fn version_checks() {
    let mut bytes = build(&[]);
    bytes[0..4].copy_from_slice(&4i32.to_le_bytes());
    bytes[4..8].copy_from_slice(&1i32.to_le_bytes());
    assert!(matches!(
        Dt1::parse(&bytes),
        Err(FormatError::UnsupportedVersion { version: 4, .. })
    ));
    // the flags dword is stored but never compared
    let mut odd = build(&[]);
    odd[4..8].copy_from_slice(&0x22i32.to_le_bytes());
    assert_eq!(Dt1::parse(&odd).unwrap().version, (7, 0x22));
}

#[test]
fn corrupt_inputs_error_not_panic() {
    assert!(Dt1::parse(&[]).is_err());
    assert!(Dt1::parse(&[7, 0, 0, 0]).is_err());
    assert!(Dt1::parse(&[0; 275]).is_err());

    let good = build(&[TileSpec::new(
        0,
        0,
        0,
        vec![BlockSpec {
            x: 0,
            y: 0,
            grid: (0, 0),
            format: 1,
            data: iso_data(1),
        }],
    )]);
    // huge tile count
    let mut huge = good.clone();
    huge[268..272].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(Dt1::parse(&huge).is_err());
    let mut neg = good.clone();
    neg[268..272].copy_from_slice(&(-3i32).to_le_bytes());
    assert!(Dt1::parse(&neg).is_err());
    // tile table offset past EOF / negative
    let mut far = good.clone();
    far[272..276].copy_from_slice(&0x7fff_0000i32.to_le_bytes());
    assert!(Dt1::parse(&far).is_err());
    let mut negt = good.clone();
    negt[272..276].copy_from_slice(&(-276i32).to_le_bytes());
    assert!(Dt1::parse(&negt).is_err());
    // sub-block header offset past EOF
    let mut ptr = good.clone();
    ptr[276 + 72..276 + 76].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(Dt1::parse(&ptr).is_err());
    // huge sub-block count
    let mut cnt = good.clone();
    cnt[276 + 80..276 + 84].copy_from_slice(&0x0fff_ffffi32.to_le_bytes());
    assert!(Dt1::parse(&cnt).is_err());
    // block data offset past EOF
    let ptr_abs = i32::from_le_bytes(good[276 + 72..276 + 76].try_into().unwrap()) as usize;
    let mut off = good.clone();
    off[ptr_abs + 16..ptr_abs + 20].copy_from_slice(&0x00ff_ffffi32.to_le_bytes());
    assert!(Dt1::parse(&off).is_err());
    let mut negoff = good.clone();
    negoff[ptr_abs + 16..ptr_abs + 20].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(Dt1::parse(&negoff).is_err());
    // block length running past EOF
    let mut len = good.clone();
    len[ptr_abs + 10..ptr_abs + 12].copy_from_slice(&0xffffu16.to_le_bytes());
    assert!(Dt1::parse(&len).is_err());
    // every truncation is an error or a parse, never a panic
    for cut in 0..good.len() {
        let _ = Dt1::parse(&good[..cut])
            .and_then(|d| d.tiles.iter().try_for_each(|t| t.render().map(drop)));
    }
    // the intact file still works
    assert!(Dt1::parse(&good).is_ok());
}

#[test]
fn shared_data_cannot_amplify() {
    // 600 sub-blocks that all point at the same 60 KB: the copy budget
    // (file length + 1 MiB) must refuse it instead of allocating 36 MB.
    let n = 600usize;
    let mut out = vec![0u8; 276 + 96];
    out[0..4].copy_from_slice(&7i32.to_le_bytes());
    out[4..8].copy_from_slice(&6i32.to_le_bytes());
    out[268..272].copy_from_slice(&1i32.to_le_bytes());
    out[272..276].copy_from_slice(&276i32.to_le_bytes());
    let ptr = out.len();
    out[276 + 72..276 + 76].copy_from_slice(&(ptr as i32).to_le_bytes());
    out[276 + 80..276 + 84].copy_from_slice(&(n as i32).to_le_bytes());
    for _ in 0..n {
        let mut h = [0u8; 20];
        h[8..10].copy_from_slice(&0i16.to_le_bytes());
        h[10..12].copy_from_slice(&60_000u16.to_le_bytes());
        h[16..20].copy_from_slice(&((20 * n) as i32).to_le_bytes());
        out.extend_from_slice(&h);
    }
    out.extend_from_slice(&vec![1u8; 60_000]);
    assert!(Dt1::parse(&out).is_err());
}

#[test]
fn oversized_extent_is_a_render_error() {
    let bytes = build(&[TileSpec::new(
        1,
        0,
        0,
        vec![
            BlockSpec {
                x: -30000,
                y: 0,
                grid: (0, 0),
                format: 0,
                data: vec![0, 1, 5],
            },
            BlockSpec {
                x: 30000,
                y: 0,
                grid: (0, 0),
                format: 0,
                data: vec![0, 1, 5],
            },
        ],
    )]);
    let dt1 = Dt1::parse(&bytes).unwrap();
    assert!(dt1.tiles[0].render().is_err());
}

// ---------------------------------------------------------------------------

fn block_strategy() -> impl Strategy<Value = BlockSpec> {
    (
        any::<i16>(),
        any::<i16>(),
        any::<i16>(),
        prop::collection::vec(any::<u8>(), 0..300),
    )
        .prop_map(|(x, y, format, data)| BlockSpec {
            x,
            y,
            grid: (0, 0),
            format: if format % 3 == 0 { 1 } else { format },
            data,
        })
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
        if let Ok(dt1) = Dt1::parse(&bytes) {
            for t in &dt1.tiles {
                let _ = t.render();
                let _ = t.key();
            }
        }
    }

    #[test]
    fn plausible_header_random_body_never_panics(
        count in 0i32..5,
        body in prop::collection::vec(any::<u8>(), 0..800),
    ) {
        let mut bytes = vec![0u8; 276];
        bytes[0..4].copy_from_slice(&7i32.to_le_bytes());
        bytes[4..8].copy_from_slice(&6i32.to_le_bytes());
        bytes[268..272].copy_from_slice(&count.to_le_bytes());
        bytes[272..276].copy_from_slice(&276i32.to_le_bytes());
        bytes.extend_from_slice(&body);
        if let Ok(dt1) = Dt1::parse(&bytes) {
            for t in &dt1.tiles {
                let _ = t.render();
            }
        }
    }

    #[test]
    fn structured_random_tiles_parse_and_render_without_panic(
        blocks in prop::collection::vec(block_strategy(), 0..6),
        orientation in -2i32..300,
    ) {
        let spec = TileSpec::new(orientation, 1, 2, blocks.clone());
        let dt1 = Dt1::parse(&build(&[spec])).unwrap();
        prop_assert_eq!(dt1.tiles.len(), 1);
        prop_assert_eq!(dt1.tiles[0].blocks.len(), blocks.len());
        for (got, want) in dt1.tiles[0].blocks.iter().zip(&blocks) {
            prop_assert_eq!(&got.data, &want.data);
            prop_assert_eq!(got.x, want.x);
        }
        if let Ok(img) = dt1.tiles[0].render() {
            prop_assert_eq!(img.pixels.len(), img.width as usize * img.height as usize);
            prop_assert_eq!(img.alpha.len(), img.pixels.len());
            // alpha is exactly "pixel is not the transparent key"
            for (p, a) in img.pixels.iter().zip(&img.alpha) {
                prop_assert_eq!(*a == 255, *p != 0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// corpus

#[test]
fn corpus_dt1_files_parse_and_render() {
    let mut files = 0usize;
    let mut tiles = 0usize;
    let mut blocks = 0usize;
    let mut histogram: BTreeMap<i32, usize> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut expected_old: Vec<String> = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::find_files(&root, "dt1") {
            files += 1;
            let bytes = std::fs::read(&path).unwrap();
            let name = path.strip_prefix(&root).unwrap().display().to_string();
            let dt1 = match Dt1::parse(&bytes) {
                Ok(d) => d,
                Err(FormatError::UnsupportedVersion { version: 4, .. }) => {
                    expected_old.push(format!("{profile}:{name}"));
                    continue;
                }
                Err(e) => {
                    failures.push(format!("{profile}:{name}: parse: {e}"));
                    continue;
                }
            };
            for (i, t) in dt1.tiles.iter().enumerate() {
                tiles += 1;
                blocks += t.blocks.len();
                *histogram.entry(t.orientation).or_default() += 1;
                if t.key().is_none() {
                    failures.push(format!("{profile}:{name}: tile {i} has no key"));
                }
                if let Err(e) = t.render() {
                    failures.push(format!("{profile}:{name}: tile {i}: render: {e}"));
                }
            }
        }
    }
    println!(
        "dt1 corpus: {files} files, {tiles} tiles, {blocks} blocks; \
         {} version-4.1 files skipped (older unreadable layout): {expected_old:?}",
        expected_old.len()
    );
    println!("orientation histogram: {histogram:?}");
    println!("failures: {} {failures:#?}", failures.len());
    assert!(failures.is_empty(), "{failures:#?}");
    if files > 0 {
        assert!(expected_old.len() == 12 || expected_old.len() == 6 || expected_old.is_empty());
    }
}
