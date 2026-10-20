//! `Dt1::to_bytes` and the sub-block encoders: synthetic cases, proptest
//! round trips and the private-corpus gate (every readable DT1 is written
//! back byte for byte; every block's decoded pixels re-encode exactly).

use super::*;

fn err_is_not_representable<T: std::fmt::Debug>(r: Result<T>) -> bool {
    matches!(r, Err(FormatError::NotRepresentable { .. }))
}

/// `(x, y, index)` of every opaque pixel, in block space, row-major.
pub(super) fn opaque(img: &IndexedImage) -> Vec<(i32, i32, u8)> {
    let w = img.width as usize;
    let mut out = Vec::new();
    for (i, (&p, &a)) in img.pixels.iter().zip(&img.alpha).enumerate() {
        if a != 0 {
            out.push((
                img.offset_x + (i % w) as i32,
                img.offset_y + (i / w) as i32,
                p,
            ));
        }
    }
    out
}

fn sample_tiles() -> Vec<TileSpec> {
    vec![
        TileSpec::new(
            3,
            1,
            2,
            vec![
                BlockSpec {
                    x: 0,
                    y: -15,
                    grid: (1, 2),
                    format: 1,
                    data: iso_data(4),
                },
                BlockSpec {
                    x: 32,
                    y: -32,
                    grid: (0, 0),
                    format: 0x1001,
                    data: vec![2, 3, 9, 9, 9, 0, 0, 1, 2, 7, 7, 0, 0],
                },
            ],
        ),
        TileSpec::new(0, 4, 5, Vec::new()),
        TileSpec::new(13, 0, 0, vec![]),
    ]
}

#[test]
fn unchanged_synthetic_file_writes_back_identically() {
    let bytes = build(&sample_tiles());
    let dt1 = Dt1::parse(&bytes).unwrap();
    assert_eq!(dt1.to_bytes().unwrap(), bytes);
    let empty = Dt1 {
        version: (7, 6),
        tiles: Vec::new(),
    };
    let out = empty.to_bytes().unwrap();
    assert_eq!(out.len(), 276);
    assert_eq!(Dt1::parse(&out).unwrap(), empty);
}

#[test]
fn preserved_header_bytes_and_flags_survive() {
    let mut bytes = build(&sample_tiles());
    let h = 276;
    bytes[h + 16..h + 20].copy_from_slice(&[1, 2, 3, 4]);
    bytes[h + 36..h + 40].copy_from_slice(&[0xff, 0, 0xff, 0]);
    bytes[h + 65..h + 72].copy_from_slice(&[5, 6, 7, 8, 9, 10, 11]);
    bytes[h + 84..h + 96].copy_from_slice(&[0xAA; 12]);
    for t in 0..25 {
        bytes[h + 40 + t] = t as u8;
    }
    let dt1 = Dt1::parse(&bytes).unwrap();
    let p = &dt1.tiles[0].preserved;
    assert_eq!(p.unknown_36, [0xff, 0, 0xff, 0]);
    assert_eq!(p.runtime_84, [0xAA; 12]);
    // file byte 0 is the bottom-left sub-tile
    assert_eq!(dt1.tiles[0].subtile_flags[20], 0);
    assert_eq!(dt1.tiles[0].subtile_flags[0], 20);
    assert_eq!(dt1.to_bytes().unwrap(), bytes);
}

#[test]
fn edits_round_trip_through_the_model() {
    let mut dt1 = Dt1::parse(&build(&sample_tiles())).unwrap();
    // changed flags
    dt1.tiles[0].subtile_flags[7] = 0x0D;
    // duplicated tile under a new sub index, removed tile, added tile
    let mut copy = dt1.tiles[0].clone();
    copy.sub_index = 9;
    dt1.tiles.push(copy);
    dt1.tiles.remove(1);
    dt1.tiles.push(Tile {
        orientation: 0,
        main_index: 30,
        height: -128,
        width: 160,
        subtile_flags: [1; 25],
        ..Tile::default()
    });
    // re-painted pixels in both formats
    let iso = dt1.tiles[0].blocks[0].render().unwrap();
    let mut painted = iso.clone();
    painted.pixels[234] = 0x42;
    painted.alpha[234] = 255;
    dt1.tiles[0].blocks[0] = dt1.tiles[0].blocks[0].with_pixels(&painted).unwrap();
    let mut rle = dt1.tiles[0].blocks[1].render().unwrap();
    rle.pixels[0] = 0;
    rle.alpha[0] = 0;
    dt1.tiles[0].blocks[1] = dt1.tiles[0].blocks[1].with_pixels(&rle).unwrap();

    let out = dt1.to_bytes().unwrap();
    let back = Dt1::parse(&out).unwrap();
    assert_eq!(back, dt1);
    assert_eq!(back.to_bytes().unwrap(), out);
    assert_eq!(
        opaque(&back.tiles[0].blocks[0].render().unwrap()),
        opaque(&painted)
    );
    assert_eq!(
        opaque(&back.tiles[0].blocks[1].render().unwrap()),
        opaque(&rle)
    );
    // the duplicate is still identical to the edited original but for its sub
    assert_eq!(back.tiles[2].sub_index, 9);
    assert_eq!(back.tiles[2].blocks.len(), 2);
}

#[test]
fn writer_rejects_unrepresentable_models() {
    let good = Dt1::parse(&build(&sample_tiles())).unwrap();
    let mut bad = good.clone();
    bad.version.0 = 4;
    assert!(matches!(
        bad.to_bytes(),
        Err(FormatError::UnsupportedVersion { version: 4, .. })
    ));
    let mut bad = good.clone();
    bad.tiles[0].blocks[0].format = BlockFormat::Rle;
    assert!(err_is_not_representable(bad.to_bytes()));
    let mut bad = good.clone();
    bad.tiles[0].blocks[1].data = vec![0; 65536];
    assert!(err_is_not_representable(bad.to_bytes()));
    let mut ok = good;
    ok.tiles[0].blocks[1].data = vec![0; 65535];
    assert_eq!(Dt1::parse(&ok.to_bytes().unwrap()).unwrap(), ok);
}

#[test]
fn isometric_encoder_samples_the_diamond() {
    let data = iso_data(7);
    let block = Block {
        x: 64,
        y: -15,
        grid_x: 2,
        grid_y: 3,
        raw_format: 1,
        format: BlockFormat::Isometric,
        data: data.clone(),
    };
    let img = block.render().unwrap();
    assert_eq!(
        (img.width, img.height, img.offset_x, img.offset_y),
        (32, 15, 64, -15)
    );
    assert_eq!(block.with_pixels(&img).unwrap(), block);
    assert_eq!(Block::isometric(64, -15, 2, 3, &img), block);
    // pixels outside the image or transparent become 0
    let empty = IndexedImage::new(0, 0, 0, 0);
    assert_eq!(Block::isometric(0, 0, 0, 0, &empty).data, vec![0; 256]);
}

#[test]
fn rle_encoder_splits_long_runs_and_skips() {
    // one row: 300 pixels starting 600 columns in; one blank row; one pixel
    let mut img = IndexedImage::new(1000, 3, 10, 20);
    for x in 600..900 {
        img.pixels[x] = (x % 250) as u8 + 1;
        img.alpha[x] = 255;
    }
    img.pixels[2 * 1000 + 5] = 9;
    img.alpha[2 * 1000 + 5] = 255;
    let block = Block::rle(10, 20, 0, 0, 0x1001, &img, 0).unwrap();
    let mut want = vec![255, 0, 255, 0, 90, 255];
    want.extend((600..855).map(|x| (x % 250) as u8 + 1));
    want.extend([0, 45]);
    want.extend((855..900).map(|x| (x % 250) as u8 + 1));
    want.extend([0, 0, 0, 0, 5, 1, 9, 0, 0]);
    assert_eq!(block.data, want);
    assert_eq!(opaque(&block.render().unwrap()), opaque(&img));
    // min_rows pads with terminated empty rows
    let padded = Block::rle(10, 20, 0, 0, 0x1001, &img, 32).unwrap();
    assert!(padded.data.starts_with(&want));
    assert!(padded.data[want.len()..].iter().all(|&b| b == 0));
    assert_eq!(padded.data.len(), want.len() + 2 * 29);
    assert_eq!(opaque(&padded.render().unwrap()), opaque(&img));
    let blank = Block::rle(0, 0, 0, 0, 0x2005, &IndexedImage::new(0, 0, 0, 0), 15).unwrap();
    assert_eq!(blank.data, vec![0; 30]);
}

#[test]
fn rle_encoder_rejects_what_it_cannot_store() {
    let mut img = IndexedImage::new(2, 2, 0, 0);
    img.alpha[0] = 255;
    img.pixels[0] = 3;
    // pixel left of / above the origin
    assert!(err_is_not_representable(Block::rle(1, 0, 0, 0, 0, &img, 0)));
    assert!(err_is_not_representable(Block::rle(0, 1, 0, 0, 0, &img, 0)));
    // format 1 is not RLE
    assert!(err_is_not_representable(Block::rle(0, 0, 0, 0, 1, &img, 0)));
    // far away or too many rows
    assert!(err_is_not_representable(Block::rle(
        i16::MIN,
        i16::MIN,
        0,
        0,
        0,
        &IndexedImage {
            offset_x: i32::MAX - 2,
            offset_y: i32::MAX - 2,
            ..img.clone()
        },
        0
    )));
    assert!(err_is_not_representable(Block::rle(
        0,
        0,
        0,
        0,
        0,
        &img,
        usize::MAX
    )));
    // a full 256x256 opaque picture does not fit 65535 bytes
    let mut big = IndexedImage::new(256, 256, 0, 0);
    big.alpha.fill(255);
    big.pixels.fill(1);
    assert!(err_is_not_representable(Block::rle(0, 0, 0, 0, 0, &big, 0)));
}

// ---------------------------------------------------------------------------

fn image_strategy() -> impl Strategy<Value = IndexedImage> {
    (0u32..70, 0u32..40, -40i32..40, -40i32..40).prop_flat_map(|(w, h, ox, oy)| {
        let n = (w * h) as usize;
        (
            prop::collection::vec(any::<u8>(), n),
            prop::collection::vec(prop::bool::weighted(0.6), n),
        )
            .prop_map(move |(pixels, mask)| {
                // an opaque index 0 would decode as transparent: keep the
                // decoders' invariant "alpha 0 exactly where index 0"
                let alpha = mask
                    .iter()
                    .zip(&pixels)
                    .map(|(&m, &p)| if m && p != 0 { 255 } else { 0 })
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
    })
}

fn tile_model_strategy() -> impl Strategy<Value = Tile> {
    let block = (
        any::<i16>(),
        any::<i16>(),
        any::<(u8, u8)>(),
        any::<i16>(),
        prop::collection::vec(any::<u8>(), 0..400),
    )
        .prop_map(|(x, y, (gx, gy), raw_format, data)| Block {
            x,
            y,
            grid_x: gx,
            grid_y: gy,
            raw_format,
            format: if raw_format == 1 {
                BlockFormat::Isometric
            } else {
                BlockFormat::Rle
            },
            data,
        });
    (
        any::<(i32, i16, u8, u8, i32, i32)>(),
        any::<(i32, i32, i32, i32)>(),
        any::<[u8; 25]>(),
        any::<([u8; 4], [u8; 4], [u8; 7], [u8; 12])>(),
        prop::collection::vec(block, 0..5),
    )
        .prop_map(
            |(
                (direction, roof_height, sound, animated, height, width),
                (orientation, main_index, sub_index, rarity),
                subtile_flags,
                (unknown_16, unknown_36, unknown_65, runtime_84),
                blocks,
            )| Tile {
                direction,
                roof_height,
                sound,
                animated,
                height,
                width,
                orientation,
                main_index,
                sub_index,
                rarity,
                subtile_flags,
                blocks,
                preserved: TilePreserved {
                    unknown_16,
                    unknown_36,
                    unknown_65,
                    runtime_84,
                },
            },
        )
}

proptest! {
    #[test]
    fn any_model_round_trips(
        tiles in prop::collection::vec(tile_model_strategy(), 0..5),
        flags in any::<i32>(),
    ) {
        let dt1 = Dt1 { version: (7, flags), tiles };
        let out = dt1.to_bytes().unwrap();
        let back = Dt1::parse(&out).unwrap();
        prop_assert_eq!(&back, &dt1);
        prop_assert_eq!(back.to_bytes().unwrap(), out);
    }

    #[test]
    fn synthetic_files_write_back_identically(
        tiles in prop::collection::vec(
            (prop::collection::vec(block_strategy(), 0..4), -1i32..300),
            0..4,
        ),
    ) {
        let specs: Vec<TileSpec> = tiles
            .into_iter()
            .map(|(blocks, o)| TileSpec::new(o, 1, 2, blocks))
            .collect();
        let bytes = build(&specs);
        prop_assert_eq!(Dt1::parse(&bytes).unwrap().to_bytes().unwrap(), bytes);
    }

    #[test]
    fn rle_encoding_reproduces_opaque_pixels(
        img in image_strategy(),
        min_rows in 0usize..40,
        raw_format in prop::sample::select(vec![0i16, 0x1001, 0x2005, -3]),
    ) {
        let (bx, by) = (img.offset_x.min(0) as i16 - 3, img.offset_y.min(0) as i16);
        let block = Block::rle(bx, by, 0, 0, raw_format, &img, min_rows).unwrap();
        let back = block.render().unwrap();
        prop_assert_eq!(opaque(&back), opaque(&img));
        // decode → encode is a fixed point
        prop_assert_eq!(block.with_pixels(&back).unwrap(), block);
    }

    #[test]
    fn isometric_encoding_reproduces_the_diamond(img in image_strategy(), x in -40i16..40, y in -40i16..40) {
        let block = Block::isometric(x, y, 0, 0, &img);
        prop_assert_eq!(block.data.len(), 256);
        let back = block.render().unwrap();
        for (bx, by, v) in opaque(&back) {
            let i = ((by - img.offset_y) as u32 * img.width + (bx - img.offset_x) as u32) as usize;
            prop_assert_eq!((img.pixels[i], img.alpha[i]), (v, 255));
        }
        prop_assert_eq!(block.with_pixels(&back).unwrap(), block);
    }
}

// ---------------------------------------------------------------------------
// corpus gate

#[test]
fn corpus_dt1_files_write_back_identically() {
    let (mut files, mut identical, mut old) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::find_files(&root, "dt1") {
            files += 1;
            let bytes = std::fs::read(&path).unwrap();
            let name = path.strip_prefix(&root).unwrap().display().to_string();
            match Dt1::parse(&bytes) {
                Err(FormatError::UnsupportedVersion { version: 4, .. }) => old += 1,
                Err(e) => failures.push(format!("{profile}:{name}: parse: {e}")),
                Ok(dt1) => match dt1.to_bytes() {
                    Ok(out) if out == bytes => identical += 1,
                    Ok(out) => {
                        let at = out.iter().zip(&bytes).position(|(a, b)| a != b);
                        failures.push(format!(
                            "{profile}:{name}: differs at {at:?} ({} vs {} bytes)",
                            out.len(),
                            bytes.len()
                        ));
                    }
                    Err(e) => failures.push(format!("{profile}:{name}: write: {e}")),
                },
            }
        }
    }
    println!(
        "dt1 write-back: {files} files, {identical} byte-identical, \
         {old} version-4.1 files unreadable (not written), {} failures {failures:#?}",
        failures.len()
    );
    assert!(failures.is_empty());
    assert_eq!(identical + old, files);
}

/// Decode → encode of every block (1 file in 4 unless `CAPY_FULL_CORPUS`):
/// the re-encoded block decodes to the same pixels, and the tile rebuilt from
/// re-encoded blocks renders the same.
#[test]
fn corpus_dt1_blocks_reencode_to_the_same_pixels() {
    let (mut blocks, mut same_bytes, mut tiles) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::sample(testutil::find_files(&root, "dt1"), 4) {
            let bytes = std::fs::read(&path).unwrap();
            let name = path.strip_prefix(&root).unwrap().display().to_string();
            let Ok(dt1) = Dt1::parse(&bytes) else {
                continue;
            };
            for (t, tile) in dt1.tiles.iter().enumerate() {
                tiles += 1;
                let mut rebuilt = tile.clone();
                for (b, block) in tile.blocks.iter().enumerate() {
                    blocks += 1;
                    let img = block.render().unwrap();
                    let re = match block.with_pixels(&img) {
                        Ok(re) => re,
                        Err(e) => {
                            failures.push(format!("{profile}:{name} tile {t} block {b}: {e}"));
                            continue;
                        }
                    };
                    if re.data == block.data {
                        same_bytes += 1;
                    }
                    if re.render().unwrap() != img {
                        failures.push(format!(
                            "{profile}:{name} tile {t} block {b}: pixels differ"
                        ));
                    }
                    rebuilt.blocks[b] = re;
                }
                if rebuilt.render().unwrap() != tile.render().unwrap() {
                    failures.push(format!("{profile}:{name} tile {t}: tile render differs"));
                }
            }
        }
    }
    println!(
        "dt1 re-encode: {tiles} tiles, {blocks} blocks, {same_bytes} re-encoded byte-identically, \
         {} failures {:#?}",
        failures.len(),
        failures.iter().take(20).collect::<Vec<_>>()
    );
    assert!(failures.is_empty());
}
