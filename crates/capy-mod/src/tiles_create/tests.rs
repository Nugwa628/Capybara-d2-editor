use super::*;
use crate::tiles_register::tests::{prest, types};
use crate::tiles_review::tile_ids;
use crate::txt_util::cell;

/// An anchor-space picture over `rect` painted by `f(x, y)` (`None` =
/// transparent).
fn paint(rect: PixelRect, f: impl Fn(i32, i32) -> Option<u8>) -> IndexedImage {
    let mut img = IndexedImage::new(rect.width, rect.height, rect.x, rect.y);
    for row in 0..rect.height as i32 {
        for col in 0..rect.width as i32 {
            if let Some(v) = f(rect.x + col, rect.y + row) {
                let i = (row * rect.width as i32 + col) as usize;
                img.pixels[i] = v;
                img.alpha[i] = 255;
            }
        }
    }
    img
}

/// A colour that never is 0, from the position.
fn colour(x: i32, y: i32) -> u8 {
    (1 + (x * 7 + y * 13).rem_euclid(254)) as u8
}

/// The written file parses back to the model and writes the same bytes again.
fn assert_round_trip(tiles: &[Tile]) {
    let dt1 = Dt1 {
        version: (7, 6),
        tiles: tiles.to_vec(),
    };
    let bytes = dt1.to_bytes().unwrap();
    let back = Dt1::parse(&bytes).unwrap();
    assert_eq!(back, dt1);
    assert_eq!(back.to_bytes().unwrap(), bytes);
}

/// The built tile renders exactly the picture's storable pixels.
fn assert_renders(built: &Tile, picture: &IndexedImage, t: &TileTemplate) {
    let r = built.render().unwrap();
    for y in t.rect.y..t.rect.y + t.rect.height as i32 {
        for x in t.rect.x..t.rect.x + t.rect.width as i32 {
            let want = t.contains(x, y).then(|| stored_at(picture, x, y)).flatten();
            assert_eq!(stored_at(&r, x, y), want, "pixel {x},{y}");
        }
    }
    // nothing outside the template
    for row in 0..r.height as i32 {
        for col in 0..r.width as i32 {
            let (x, y) = (r.offset_x + col, r.offset_y + row);
            if stored_at(&r, x, y).is_some() {
                assert!(t.contains(x, y), "pixel {x},{y} outside the template");
            }
        }
    }
}

#[test]
fn full_floor_is_25_isometric_blocks_in_shipped_order() {
    let t = template(TileShape::floor()).unwrap();
    assert_eq!(
        t.rect,
        PixelRect {
            x: 0,
            y: 0,
            width: 160,
            height: 79
        }
    );
    assert_eq!(t.mask().alpha.iter().filter(|&&a| a != 0).count(), 25 * 256);
    let pic = paint(t.rect, |x, y| Some(colour(x, y)));
    let b = build_tile(&pic, TileShape::floor(), 3, 7, &BuildOptions::default()).unwrap();
    let tile = &b.tile;
    assert_eq!(tile.blocks.len(), 25);
    assert!(tile.blocks.iter().all(|b| b.raw_format == 1));
    // grid (4,4) first, (0,0) last
    assert_eq!((tile.blocks[0].x, tile.blocks[0].y), (64, 64));
    assert_eq!((tile.blocks[0].grid_x, tile.blocks[0].grid_y), (4, 4));
    assert_eq!((tile.blocks[24].x, tile.blocks[24].y), (64, 0));
    assert_eq!(
        (tile.height, tile.width, tile.direction, tile.orientation),
        (-128, 160, 3, 0)
    );
    assert_eq!((tile.main_index, tile.sub_index, tile.rarity), (3, 7, 0));
    assert_eq!(tile.subtile_flags, [0; 25]);
    // the rectangle's corners are outside the diamond
    assert!(b.dropped > 0);
    assert_eq!(b.dropped, 160 * 79 - 25 * 256);
    assert!(b.warning().is_some());
    assert_renders(tile, &pic, &t);
    assert_round_trip(std::slice::from_ref(tile));
}

#[test]
fn partial_floor_cells_are_rle_pieces_and_empty_cells_are_left_out() {
    let t = template(TileShape::floor()).unwrap();
    // left half only: cells cut by x = 80 become 0x2005 pieces
    let pic = paint(t.rect, |x, y| (x < 80).then(|| colour(x, y)));
    let b = build_tile(&pic, TileShape::floor(), 0, 0, &BuildOptions::default()).unwrap();
    let formats: BTreeSet<i16> = b.tile.blocks.iter().map(|b| b.raw_format).collect();
    assert_eq!(formats, [1, FLOOR_RLE_FORMAT].into());
    assert!(b.tile.blocks.len() < 25);
    assert_renders(&b.tile, &pic, &t);
    assert_round_trip(std::slice::from_ref(&b.tile));
    // nothing at all: no blocks
    let empty = IndexedImage::new(0, 0, 0, 0);
    let b = build_tile(&empty, TileShape::roof(80), 0, 0, &BuildOptions::default()).unwrap();
    assert!(b.tile.blocks.is_empty());
    assert_eq!(
        (b.tile.orientation, b.tile.roof_height, b.tile.direction),
        (15, 80, 5)
    );
    assert_eq!(b.dropped, 0);
    assert_round_trip(std::slice::from_ref(&b.tile));
}

#[test]
fn upper_wall_rounds_its_height_and_stands_on_its_base() {
    let shape = TileShape::wall(1, 100);
    let t = template(shape).unwrap();
    assert_eq!(t.shape.height, 128);
    assert_eq!(
        t.rect,
        PixelRect {
            x: 0,
            y: -128,
            width: 160,
            height: 128
        }
    );
    // a 70-pixel wide wall the full height, plus a pixel below the base
    let pic = paint(
        PixelRect {
            x: 0,
            y: -128,
            width: 70,
            height: 129,
        },
        |x, y| Some(colour(x, y)),
    );
    let b = build_tile(&pic, shape, 2, 0, &BuildOptions::default()).unwrap();
    let tile = &b.tile;
    assert_eq!(b.dropped, 70);
    assert_eq!(
        b.dropped_bounds,
        Some(PixelRect {
            x: 0,
            y: 0,
            width: 70,
            height: 1
        })
    );
    assert_eq!((tile.height, tile.width, tile.direction), (-160, 96, 1));
    // 4 rows of 3 cells, top row first, right to left
    let pos: Vec<(i16, i16)> = tile.blocks.iter().map(|b| (b.x, b.y)).collect();
    assert_eq!(&pos[..4], &[(64, -128), (32, -128), (0, -128), (64, -96)]);
    assert_eq!(pos.len(), 12);
    assert!(
        tile.blocks
            .iter()
            .all(|b| b.raw_format == WALL_RLE_FORMAT && (b.grid_x, b.grid_y) == (0, 0))
    );
    assert_eq!(tile.subtile_flags, default_flags(1));
    assert_eq!(tile.subtile_flags[0], 0x07);
    assert_eq!(tile.subtile_flags[5], 0x07);
    assert_eq!(tile.subtile_flags[1], 0);
    assert_renders(tile, &pic, &t);
    assert_round_trip(std::slice::from_ref(tile));
}

#[test]
fn lower_wall_and_shadow_and_special_geometry() {
    let t = template(TileShape::wall(16, 64)).unwrap();
    assert_eq!(
        t.rect,
        PixelRect {
            x: 0,
            y: -96,
            width: 160,
            height: 64
        }
    );
    // bottom row first
    assert_eq!((t.cells[0].x, t.cells[0].y), (128, -64));
    let pic = paint(t.rect, |x, y| Some(colour(x, y)));
    let b = build_tile(
        &pic,
        TileShape::wall(16, 64),
        1,
        1,
        &BuildOptions::default(),
    )
    .unwrap();
    assert_eq!(
        (b.tile.height, b.tile.width, b.tile.direction),
        (-64, 160, 6)
    );
    assert_eq!(b.tile.subtile_flags, [0; 25]);
    assert_eq!(b.dropped, 0);
    assert_renders(&b.tile, &pic, &t);

    let shadow = build_tile(
        &pic,
        TileShape::wall(13, 96),
        0,
        0,
        &BuildOptions::default(),
    )
    .unwrap();
    assert_eq!((shadow.tile.height, shadow.tile.direction), (-128, 3));
    assert_eq!(shadow.tile.subtile_flags, [0; 25]);
    let special = build_tile(
        &pic,
        TileShape::wall(11, 32),
        0,
        0,
        &BuildOptions::default(),
    )
    .unwrap();
    assert_eq!((special.tile.height, special.tile.direction), (-64, 2));
    assert_eq!(&special.tile.subtile_flags[..5], &[7; 5]);
    assert_round_trip(&[b.tile, shadow.tile, special.tile]);
}

#[test]
fn corner_pair_splits_one_picture_at_x_80() {
    let rect = PixelRect {
        x: 0,
        y: -64,
        width: 160,
        height: 64,
    };
    let pic = paint(rect, |x, y| Some(colour(x, y)));
    let (r, l) = build_corner_pair(&pic, 64, 5, 9, &BuildOptions::default()).unwrap();
    assert_eq!((r.tile.orientation, l.tile.orientation), (3, 4));
    assert_eq!((r.tile.main_index, r.tile.sub_index), (5, 9));
    assert_eq!((l.tile.main_index, l.tile.sub_index), (5, 9));
    assert_eq!((r.tile.width, l.tile.width), (160, 96));
    assert_eq!((r.dropped, l.dropped), (0, 0));
    let rr = r.tile.render().unwrap();
    let lr = l.tile.render().unwrap();
    assert_eq!((rr.offset_x, rr.width), (80, 80));
    assert_eq!((lr.offset_x, lr.width), (0, 80));
    assert_renders(&r.tile, &pic, &template(TileShape::wall(3, 64)).unwrap());
    assert_renders(&l.tile, &pic, &template(TileShape::wall(4, 64)).unwrap());
    assert_eq!(r.tile.subtile_flags, l.tile.subtile_flags);
    assert_round_trip(&[r.tile, l.tile]);
    // a lone half drops the other half's pixels
    let lone = build_tile(&pic, TileShape::wall(3, 64), 0, 0, &BuildOptions::default()).unwrap();
    assert_eq!(lone.dropped, 80 * 64);
}

#[test]
fn index_zero_and_options_are_reported_and_kept() {
    let t = template(TileShape::wall(2, 32)).unwrap();
    let pic = paint(t.rect, |x, _| Some(if x < 10 { 0 } else { 5 }));
    let opts = BuildOptions {
        flags: Some([1; 25]),
        rarity: 4,
        sound: 2,
        animated: 0,
    };
    let b = build_tile(&pic, TileShape::wall(2, 32), 0, 0, &opts).unwrap();
    assert_eq!(b.transparent_zero, 10 * 32);
    assert_eq!(b.dropped, 0);
    assert!(b.warning().unwrap().contains("index 0"));
    assert_eq!(
        (b.tile.subtile_flags, b.tile.rarity, b.tile.sound),
        ([1; 25], 4, 2)
    );
    let r = b.tile.render().unwrap();
    assert_eq!(r.offset_x, 10);
}

#[test]
fn shapes_normalize_and_refuse() {
    assert!(TileShape::wall(20, 32).normalized().is_err());
    assert!(TileShape::wall(1, 0).normalized().is_err());
    assert!(
        TileShape::wall(1, MAX_WALL_HEIGHT + 1)
            .normalized()
            .is_err()
    );
    assert_eq!(TileShape::wall(1, 33).normalized().unwrap().height, 64);
    assert_eq!(
        TileShape::wall(1, MAX_WALL_HEIGHT)
            .normalized()
            .unwrap()
            .height,
        MAX_WALL_HEIGHT
    );
    let f = TileShape {
        orientation: 0,
        height: 999,
        roof_height: 7,
    }
    .normalized()
    .unwrap();
    assert_eq!(f, TileShape::floor());
    assert_eq!(TileShape::wall(1, 64).stored_height(), -96);
    assert_eq!(TileShape::wall(17, 64).stored_height(), -64);
    assert_eq!(TileShape::roof(80).stored_height(), -128);
    assert!(
        build_tile(
            &IndexedImage::new(1, 1, 0, 0),
            TileShape::wall(1, 0),
            0,
            0,
            &BuildOptions::default()
        )
        .is_err()
    );
    // the tallest wall renders within the decoder's 2048-pixel limit
    let t = template(TileShape::wall(14, MAX_WALL_HEIGHT)).unwrap();
    let pic = paint(t.rect, |x, y| Some(colour(x, y)));
    let b = build_tile(&pic, t.shape, 0, 0, &BuildOptions::default()).unwrap();
    assert_eq!(b.tile.blocks.len(), 5 * 63);
    assert_renders(&b.tile, &pic, &t);
}

#[test]
fn directions_and_flags_by_orientation() {
    let dirs: Vec<i32> = (0..20).map(direction_of).collect();
    assert_eq!(
        dirs,
        [3, 1, 2, 3, 3, 1, 2, 4, 1, 2, 1, 2, 3, 3, 3, 5, 6, 7, 8, 9]
    );
    for o in [0, 12, 13, 14, 15, 16, 17, 18, 19] {
        assert_eq!(default_flags(o), [0; 25], "orientation {o}");
    }
    assert_eq!(default_flags(8)[20], 7);
    assert_eq!(default_flags(9)[4], 7);
    assert_eq!(default_flags(3).iter().filter(|&&f| f == 7).count(), 9);
    assert_eq!(corner_partner(3), Some(4));
    assert_eq!(number_orientations(4), vec![3, 4]);
    assert_eq!(number_orientations(1), vec![1]);
}

#[test]
fn reshape_keeps_pixels_where_they_still_fit() {
    let t = template(TileShape::wall(2, 96)).unwrap();
    let pic = paint(t.rect, |x, y| Some(colour(x, y)));
    let mut opts = BuildOptions {
        flags: Some([3; 25]),
        rarity: 2,
        ..BuildOptions::default()
    };
    opts.sound = 1;
    let mut wall = build_tile(&pic, t.shape, 4, 4, &opts).unwrap().tile;
    wall.preserved.unknown_36 = [1, 2, 3, 4];
    // taller: everything kept, number/flags/rarity/preserved kept
    let taller = reshape_tile(&wall, TileShape::wall(2, 160)).unwrap();
    assert_eq!(taller.dropped, 0);
    assert_eq!(taller.tile.height, -192);
    assert_eq!(taller.tile.render().unwrap(), wall.render().unwrap());
    assert_eq!(
        (
            taller.tile.subtile_flags,
            taller.tile.rarity,
            taller.tile.sound
        ),
        ([3; 25], 2, 1)
    );
    assert_eq!(taller.tile.preserved.unknown_36, [1, 2, 3, 4]);
    assert_eq!((taller.tile.main_index, taller.tile.sub_index), (4, 4));
    // shorter: the top 64 rows go
    let shorter = reshape_tile(&wall, TileShape::wall(2, 32)).unwrap();
    assert_eq!(shorter.dropped, 160 * 64);
    assert_eq!(shorter.tile.render().unwrap().height, 32);
    // another orientation: default flags of the new one
    let left = reshape_tile(&wall, TileShape::wall(1, 96)).unwrap();
    assert_eq!(left.tile.subtile_flags, default_flags(1));
    assert_eq!(left.tile.direction, 1);
    assert_round_trip(&[taller.tile, shorter.tile, left.tile]);
    // floor <-> roof keeps the diamond
    let ft = template(TileShape::floor()).unwrap();
    let floor = build_tile(
        &paint(ft.rect, |x, y| Some(colour(x, y))),
        TileShape::floor(),
        0,
        0,
        &BuildOptions::default(),
    )
    .unwrap()
    .tile;
    let roof = reshape_tile(&floor, TileShape::roof(DEFAULT_ROOF_HEIGHT)).unwrap();
    assert_eq!(roof.dropped, 0);
    assert_eq!(roof.tile.blocks, floor.blocks);
    assert_eq!((roof.tile.orientation, roof.tile.roof_height), (15, 80));
    let back = reshape_tile(&roof.tile, TileShape::floor()).unwrap();
    assert_eq!(back.tile.blocks, floor.blocks);
    assert_eq!(back.tile.roof_height, 0);
    // inferred shapes
    assert_eq!(TileShape::of_tile(&wall), Some(TileShape::wall(2, 96)));
    assert_eq!(TileShape::of_tile(&roof.tile), Some(TileShape::roof(80)));
    let mut odd = wall.clone();
    odd.orientation = 25;
    assert_eq!(TileShape::of_tile(&odd), None);
}

#[test]
fn template_plain_and_anchor_conversions() {
    let t = template(TileShape::wall(1, 64)).unwrap();
    let mut plain = IndexedImage::new(160, 64, 5, 5);
    plain.pixels[0] = 9;
    plain.alpha[0] = 255;
    let anchored = t.to_anchor(&plain);
    assert_eq!((anchored.offset_x, anchored.offset_y), (0, -64));
    assert_eq!(pixel_at(&anchored, 0, -64), Some((9, 255)));
    let mut back = t.to_plain(&anchored);
    assert_eq!((back.offset_x, back.offset_y), (0, 0));
    back.offset_x = 5;
    back.offset_y = 5;
    assert_eq!(back, plain);
    // a picture larger than the area is cut by to_plain
    let big = paint(
        PixelRect {
            x: -10,
            y: -100,
            width: 200,
            height: 120,
        },
        |_, _| Some(1),
    );
    let cut = t.to_plain(&big);
    assert_eq!((cut.width, cut.height), (160, 64));
    assert!(cut.alpha.iter().all(|&a| a == 255));
    assert!(t.contains(0, -1) && !t.contains(0, 0) && !t.contains(160, -1));
    let f = template(TileShape::floor()).unwrap();
    assert!(f.contains(80, 40) && f.contains(0, 39) && !f.contains(0, 0) && !f.contains(80, 79));
}

#[test]
fn placement_as_is_center_and_fit() {
    let t = template(TileShape::wall(1, 64)).unwrap(); // 160x64 at (0, -64)
    let img = paint(
        PixelRect {
            x: 0,
            y: 0,
            width: 40,
            height: 20,
        },
        |x, y| Some(colour(x, y)),
    );
    let a = place_indexed(&img, &t, Placement::AsIs);
    assert_eq!(
        (a.offset_x, a.offset_y, a.width, a.height),
        (0, -64, 40, 20)
    );
    let c = place_indexed(&img, &t, Placement::Center);
    assert_eq!(
        (c.offset_x, c.offset_y, c.width, c.height),
        (60, -42, 40, 20)
    );
    let f = place_indexed(&img, &t, Placement::Fit);
    // 40x20 * 3.2 = 128x64
    assert_eq!(
        (f.offset_x, f.offset_y, f.width, f.height),
        (16, -64, 128, 64)
    );
    // nearest neighbour: an integer upscale repeats pixels
    let small = paint(
        PixelRect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        },
        |x, _| Some(x as u8 + 1),
    );
    let tf = template(TileShape::wall(1, 32)).unwrap(); // 160x32: 2x1 -> 64x32
    let up = place_indexed(&small, &tf, Placement::Fit);
    assert_eq!((up.width, up.height), (64, 32));
    assert_eq!(up.pixels[0], 1);
    assert_eq!(up.pixels[31], 1);
    assert_eq!(up.pixels[32], 2);
    // too big: Fit shrinks it into the box
    let huge = paint(
        PixelRect {
            x: 0,
            y: 0,
            width: 640,
            height: 640,
        },
        |_, _| Some(3),
    );
    let down = place_indexed(&huge, &tf, Placement::Fit);
    assert_eq!((down.width, down.height), (32, 32));
    let b = build_tile(&down, tf.shape, 0, 0, &BuildOptions::default()).unwrap();
    assert_eq!(b.dropped, 0);
    // RGBA placement matches
    let rgba = RgbaFrame {
        width: 2,
        height: 1,
        offset_x: 0,
        offset_y: 0,
        pixels: vec![10, 20, 30, 255, 40, 50, 60, 255],
    };
    let up = place_rgba(&rgba, &tf, Placement::Fit);
    assert_eq!(
        (up.width, up.height, up.offset_x, up.offset_y),
        (64, 32, 48, -32)
    );
    assert_eq!(&up.pixels[..4], &[10, 20, 30, 255]);
    assert_eq!(&up.pixels[32 * 4..33 * 4], &[40, 50, 60, 255]);
}

fn grey_palette() -> Palette {
    let mut colors = [[0u8; 3]; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = [i as u8; 3];
    }
    Palette { colors }
}

#[test]
fn import_quantizes_and_places_a_png() {
    let mut frame = RgbaFrame::new(32, 16, 0, 0);
    for (i, px) in frame.pixels.chunks_mut(4).enumerate() {
        let v = if i % 32 < 16 { 100 } else { 200 };
        px.copy_from_slice(&[v, v, v, 255]);
    }
    // one transparent pixel
    frame.pixels[3] = 0;
    let png = capy_sprite::image_io::export_png(&frame).unwrap();
    let opts = ImportOptions {
        placement: Placement::Fit,
        ..ImportOptions::default()
    };
    let shape = TileShape::wall(12, 64);
    let img = import_tile_image(&png, ImageKind::Png, &grey_palette(), shape, &opts).unwrap();
    // 32x16 fits 160x64 as 128x64, centred
    assert_eq!(
        (img.width, img.height, img.offset_x, img.offset_y),
        (128, 64, 16, -64)
    );
    assert_eq!(img.pixels[4], 100);
    assert_eq!(img.pixels[127], 200);
    assert_eq!(img.alpha[0], 0);
    let b = build_tile(&img, shape, 0, 0, &BuildOptions::default()).unwrap();
    assert_eq!((b.dropped, b.transparent_zero), (0, 0));
    assert_round_trip(&[b.tile]);
    // not a picture
    assert!(import_tile_image(b"nope", ImageKind::Png, &grey_palette(), shape, &opts).is_err());
    assert!(
        import_tile_image(
            &png,
            ImageKind::Png,
            &grey_palette(),
            TileShape::wall(1, 0),
            &opts
        )
        .is_err()
    );
}

#[test]
fn hostile_pictures_do_not_panic() {
    let shapes = [
        TileShape::floor(),
        TileShape::wall(3, 64),
        TileShape::wall(18, 96),
    ];
    let mut short = IndexedImage::new(4, 4, -2, -70);
    short.width = 100_000;
    short.height = 100_000;
    short.alpha.truncate(3);
    let mut far = IndexedImage::new(2, 2, i32::MAX - 1, i32::MIN);
    far.alpha = vec![255; 4];
    far.pixels = vec![1; 4];
    for shape in shapes {
        for img in [&short, &far] {
            let b = build_tile(img, shape, 0, 0, &BuildOptions::default()).unwrap();
            assert_round_trip(&[b.tile]);
            let t = template(shape).unwrap();
            let _ = place_indexed(img, &t, Placement::Fit);
            let _ = place_indexed(img, &t, Placement::Center);
            let _ = t.to_plain(img);
        }
    }
    let bad = RgbaFrame {
        width: 50_000,
        height: 50_000,
        offset_x: 0,
        offset_y: 0,
        pixels: vec![1; 7],
    };
    let t = template(TileShape::wall(1, 32)).unwrap();
    let placed = place_rgba(&bad, &t, Placement::Fit);
    assert!(placed.width <= 160 && placed.height <= 32);
}

#[test]
fn free_numbers_and_empty_dt1() {
    let empty = new_dt1();
    let bytes = empty.to_bytes().unwrap();
    assert_eq!(bytes.len(), 276);
    assert_eq!(Dt1::parse(&bytes).unwrap(), empty);
    assert_eq!(free_tile_number(&BTreeSet::new(), &[1], None), Some((0, 0)));
    let taken: BTreeSet<TileId> = [
        TileId::new(1, 0, 0),
        TileId::new(1, 0, 1),
        TileId::new(4, 0, 2),
        TileId::new(3, 2, 0),
    ]
    .into();
    assert_eq!(free_tile_number(&taken, &[1], None), Some((0, 2)));
    assert_eq!(free_tile_number(&taken, &[2], None), Some((0, 0)));
    assert_eq!(
        free_tile_number(&taken, &number_orientations(3), None),
        Some((0, 0))
    );
    assert_eq!(free_tile_number(&taken, &[4], None), Some((0, 0)));
    let corner_taken: BTreeSet<TileId> = [TileId::new(4, 0, 0)].into();
    assert_eq!(
        free_tile_number(&corner_taken, &number_orientations(3), None),
        Some((0, 1))
    );
    assert_eq!(free_tile_number(&taken, &[3], Some(2)), Some((2, 1)));
    assert_eq!(free_tile_number(&taken, &[1], Some(99)), Some((0, 2)));
    let full: BTreeSet<TileId> = (0..=MAX_MAIN)
        .flat_map(|m| (0..=MAX_SUB).map(move |s| TileId::new(0, m, s)))
        .collect();
    assert_eq!(free_tile_number(&full, &[0], Some(3)), None);
    assert_eq!(free_tile_number(&full, &[1], None), Some((0, 0)));
}

#[test]
fn created_tiles_go_through_add_to_map() {
    let t = template(TileShape::wall(1, 64)).unwrap();
    let pic = paint(t.rect, |x, y| Some(colour(x, y)));
    let wall = build_tile(&pic, t.shape, 0, 0, &BuildOptions::default())
        .unwrap()
        .tile;
    let corner_pic = paint(
        PixelRect {
            x: 0,
            y: -64,
            width: 160,
            height: 64,
        },
        |x, y| Some(colour(x, y)),
    );
    let (r, l) = build_corner_pair(&corner_pic, 64, 0, 0, &BuildOptions::default()).unwrap();
    let floor = build_tile(
        &paint(template(TileShape::floor()).unwrap().rect, |x, y| {
            Some(colour(x, y))
        }),
        TileShape::floor(),
        0,
        0,
        &BuildOptions::default(),
    )
    .unwrap()
    .tile;
    let special = build_tile(
        &pic,
        TileShape::wall(10, 64),
        0,
        0,
        &BuildOptions::default(),
    )
    .unwrap()
    .tile;
    let tiles = vec![wall, r.tile, l.tile, floor, special];

    // the level type already loads 1/0/0 and 4/0/0
    let lib = Dt1 {
        version: (7, 6),
        tiles: vec![
            Tile {
                orientation: 1,
                ..Tile::default()
            },
            Tile {
                orientation: 4,
                ..Tile::default()
            },
        ],
    };
    let (ty, pr) = (types(2), prest());
    let ds1 = Ds1::new(8, 8, 0);
    let type_dt1s = vec![("a/f1.dt1".to_owned(), &lib)];
    let target = MapTarget {
        map_path: "data/global/tiles/act1/town/x.ds1",
        ds1: &ds1,
        types: &ty,
        prest: &pr,
        type_id: 0,
        prest_def: Some(0),
        type_dt1s: &type_dt1s,
        existing_own: None,
        existing_own_path: None,
    };
    let out = add_new_tiles_to_map(&target, &tiles).unwrap();
    assert_eq!(out.own_path, "a/town_custom.dt1");
    assert_eq!(out.added, 4, "the special tile is skipped");
    assert_eq!(
        out.warnings
            .iter()
            .filter(|w| w.contains("special"))
            .count(),
        1
    );
    // registered: File 3 and its bit in Dt1Mask (3 -> 7), listed in the DS1
    assert_eq!(out.registration.slot, 3);
    assert_eq!(cell(&out.types, 0, "File 3"), "a/town_custom.dt1");
    assert_eq!(cell(&out.prest, 0, "Dt1Mask"), "7");
    assert_eq!(out.ds1.files.len(), 1);
    // renumbered away from the type's numbers; the corner halves together
    let ids = tile_ids(&out.dt1);
    assert!(!ids.contains(&TileId::new(1, 0, 0)));
    assert!(!ids.contains(&TileId::new(4, 0, 0)));
    assert!(
        ids.contains(&TileId::new(0, 0, 0)),
        "the floor keeps its number"
    );
    let corner: Vec<&Tile> = out
        .dt1
        .tiles
        .iter()
        .filter(|t| t.orientation == 3 || t.orientation == 4)
        .collect();
    assert_eq!(corner.len(), 2);
    assert_eq!(
        (corner[0].main_index, corner[0].sub_index),
        (corner[1].main_index, corner[1].sub_index)
    );
    let num = |o| {
        out.placed
            .get(&TileId::new(o, 0, 0))
            .map(|t| (t.main, t.sub))
    };
    assert_eq!(num(3), num(4));
    assert_ne!(num(3), Some((0, 0)));
    assert!(out.remap.contains_key(&TileId::new(1, 0, 0)));
    // the file writes and pixels survive
    assert_round_trip(&out.dt1.tiles);
    let wall_out = out.dt1.tiles.iter().find(|t| t.orientation == 1).unwrap();
    assert_eq!(wall_out.render().unwrap(), tiles[0].render().unwrap());

    // appending more to the existing own DT1 avoids its numbers too
    let own = out.dt1.clone();
    let target2 = MapTarget {
        existing_own: Some(&own),
        existing_own_path: Some(&out.own_path),
        types: &out.types,
        prest: &out.prest,
        ds1: &out.ds1,
        ..target
    };
    let again = add_new_tiles_to_map(&target2, &tiles[..1]).unwrap();
    assert_eq!(again.dt1.tiles.len(), own.tiles.len() + 1);
    assert_eq!(tile_ids(&again.dt1).len(), ids.len() + 1);
    assert!(!again.registration.slot_added);
    // nothing to add
    assert!(add_new_tiles_to_map(&target, &[]).is_err());
}
