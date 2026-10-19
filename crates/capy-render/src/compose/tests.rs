#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::order::Visibility;
use crate::testutil::{cell, dt1, palette, set_floor, set_shadow, set_wall, tile, tileset};

fn opts(zoom_div: u32) -> RenderOptions {
    RenderOptions {
        zoom_div,
        gamma: 1.0,
        ..RenderOptions::default()
    }
}

#[test]
fn downscale_takes_the_centre_sample() {
    let mut img = RgbaImage::filled(8, 8, [0, 0, 0, 255]);
    // div 4: output (i, j) = input (4i + 2, 4j + 2).
    for (x, y, v) in [(2u32, 2u32, 11u8), (6, 2, 22), (2, 6, 33), (6, 6, 44)] {
        let i = (y as usize * 8 + x as usize) * 4;
        img.pixels[i] = v;
    }
    let out = downscale(&img, 4);
    assert_eq!((out.width, out.height), (2, 2));
    assert_eq!(out.get(0, 0).unwrap()[0], 11);
    assert_eq!(out.get(1, 0).unwrap()[0], 22);
    assert_eq!(out.get(0, 1).unwrap()[0], 33);
    assert_eq!(out.get(1, 1).unwrap()[0], 44);
    assert_eq!(downscale(&img, 1), img);
}

#[test]
fn png_round_trip_keeps_pixels() {
    let mut img = RgbaImage::filled(3, 2, [10, 20, 30, 255]);
    img.pixels[0..4].copy_from_slice(&[1, 2, 3, 0]);
    let bytes = img.to_png().unwrap();
    assert_eq!(RgbaImage::from_png(&bytes).unwrap(), img);
    assert!(RgbaImage::from_png(b"not a png").is_err());
}

#[test]
fn a_floor_renders_at_its_cell() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 40)])]);
    let mut ds1 = Ds1::new(1, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    let out = render(&ds1, &ts, &palette(), &opts(1)).unwrap();
    assert_eq!((out.image.width, out.image.height), (160, 80));
    assert_eq!(out.origin, (0, 0));
    assert!(out.missing.is_empty() && out.warnings.is_empty());
    // Row 7 of the first diamond is fully opaque from x = 0..32.
    assert_eq!(out.image.get(16, 7).unwrap(), [40, 20, 215, 255]);
    // Outside the diamonds nothing is drawn: background.
    assert_eq!(out.image.get(159, 79).unwrap(), [0, 0, 0, 0]);
}

#[test]
fn background_colour_fills_the_canvas() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 40)])]);
    let mut ds1 = Ds1::new(1, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    let mut o = opts(1);
    o.background = [5, 6, 7, 255];
    let out = render(&ds1, &ts, &palette(), &o).unwrap();
    assert_eq!(out.image.get(159, 79).unwrap(), [5, 6, 7, 255]);
}

#[test]
fn gamma_is_applied_to_the_palette() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 100)])]);
    let mut ds1 = Ds1::new(1, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    let flat = render(&ds1, &ts, &palette(), &opts(1)).unwrap();
    let mut o = opts(1);
    o.gamma = 2.0;
    let bright = render(&ds1, &ts, &palette(), &o).unwrap();
    let (a, b) = (
        flat.image.get(16, 7).unwrap(),
        bright.image.get(16, 7).unwrap(),
    );
    assert_eq!(a[..3], [100, 50, 155]);
    assert!(b[0] > a[0] && b[1] > a[1], "{a:?} {b:?}");
}

#[test]
fn shadows_are_blended_at_50_percent_over_the_floor() {
    // A floor whose diamonds sit low in its 160x80 bitmap (rows 60..75), so
    // the shadow bitmap (top at my + 80 - 64 = 16, blocks at row 49) overlaps
    // them.
    let mut floor = tile(0, 1, 0, 0, 200);
    floor.blocks = (0..5)
        .map(|i| crate::testutil::iso_block(i * 32, 60, 200))
        .collect();
    let ts = tileset(vec![dt1(vec![floor, tile(13, 1, 0, 0, 20)])]);
    let mut ds1 = Ds1::new(1, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    set_shadow(&mut ds1, 0, 0, cell(1, 1, 0));
    let out = render(&ds1, &ts, &palette(), &opts(1)).unwrap();
    let floor_px = [200, 100, 55, 255];
    let shadow_rgb = [20, 10, 235];
    let expected = Blend::Alpha50.apply(floor_px, shadow_rgb);
    // Screen row 72: floor diamond row 12 (x 10..22) and shadow diamond row 7.
    assert_eq!(out.image.get(16, 72).unwrap(), expected);
    assert_eq!(expected[3], 255);
    // Where only the floor is, it is untouched (row 62: shadow not there yet).
    assert_eq!(out.image.get(16, 62).unwrap(), floor_px);
}

#[test]
fn roofs_are_drawn_after_floors() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1), tile(15, 1, 0, 0, 2)])]);
    let mut ds1 = Ds1::new(1, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    set_wall(&mut ds1, 0, 0, 0, 15, cell(1, 1, 0));
    let out = render(&ds1, &ts, &palette(), &opts(1)).unwrap();
    // Roof and floor both cover (16, 7); the roof (pass 5) wins.
    assert_eq!(out.image.get(16, 7).unwrap()[..3], palette().colors[2]);
}

#[test]
fn zoomed_render_equals_the_downscaled_1_to_1_render() {
    let ts = tileset(vec![dt1(vec![
        tile(0, 1, 0, 0, 50),
        tile(0, 2, 0, 0, 60),
        tile(1, 1, 0, 0, 70),
        tile(16, 1, 0, 0, 80),
        tile(13, 1, 0, 0, 90),
        {
            let mut roof = tile(15, 1, 0, 0, 100);
            roof.roof_height = 40;
            roof
        },
    ])]);
    let mut ds1 = Ds1::new(3, 2, 0);
    ds1.add_wall_layer().unwrap();
    for y in 0..2 {
        for x in 0..3 {
            set_floor(&mut ds1, 0, x, y, cell(1, 1 + ((x + y) % 2) as u8, 0));
        }
    }
    set_wall(&mut ds1, 0, 1, 0, 1, cell(1, 1, 0));
    set_wall(&mut ds1, 0, 2, 1, 16, cell(1, 1, 0));
    set_wall(&mut ds1, 1, 0, 1, 15, cell(1, 1, 0));
    set_shadow(&mut ds1, 1, 1, cell(1, 1, 0));
    let full = render(&ds1, &ts, &palette(), &opts(1)).unwrap();
    let zoomed = render(&ds1, &ts, &palette(), &opts(4)).unwrap();
    assert_eq!(zoomed.image, downscale(&full.image, 4));
    assert_eq!(zoomed.origin, (full.origin.0 / 4, full.origin.1 / 4));
}

#[test]
fn an_empty_map_renders_a_single_background_pixel() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1)])]);
    let ds1 = Ds1::new(4, 4, 0);
    let (img, missing) = render_rgba(&ds1, &ts, &palette(), &opts(1)).unwrap();
    assert_eq!((img.width, img.height), (1, 1));
    assert!(missing.is_empty());
}

#[test]
fn missing_tiles_do_not_stop_the_render() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1)])]);
    let mut ds1 = Ds1::new(2, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    set_floor(&mut ds1, 0, 1, 0, cell(1, 63, 0));
    let (img, missing) = render_rgba(&ds1, &ts, &palette(), &opts(1)).unwrap();
    assert_eq!(missing.len(), 1);
    assert!(img.width >= 160);
}

#[test]
fn visibility_option_is_respected() {
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1)])]);
    let mut ds1 = Ds1::new(1, 1, 0);
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    let mut o = opts(1);
    o.visibility = Visibility {
        floors: [false, false],
        ..Visibility::default()
    };
    let out = render(&ds1, &ts, &palette(), &o).unwrap();
    // The canvas is still the map's extent; nothing is drawn on it.
    assert_eq!((out.image.width, out.image.height), (160, 80));
    assert!(
        out.image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == 0)
    );
}

#[test]
fn canvas_bounds_follow_the_reference_integer_maths() {
    // 1:16 truncates every size: 160/16 = 10, 80/16 = 5, 96/16 = 6.
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1), tile(1, 1, 0, 0, 2)])]);
    let mut ds1 = Ds1::new(2, 2, 0);
    ds1.add_wall_layer().unwrap();
    set_floor(&mut ds1, 0, 0, 0, cell(1, 1, 0));
    set_wall(&mut ds1, 0, 1, 1, 1, cell(1, 1, 0));
    let (x0, y0, x1, y1) = reference_bounds(&ds1, &ts, 16).unwrap();
    // Cell (1,1): mx = 0, my = 5/2 + 5/2 = 4; wall top = 4 - 96/16 + 5 = 3,
    // bottom = 3 + 96/16 = 9; floor (0,0): 0..10 x 0..5.
    assert_eq!((x0, y0), (0, 0));
    assert_eq!((x1, y1), (10, 9));
}

#[test]
fn cell_corner_matches_the_reference_for_odd_tile_heights() {
    assert_eq!(cell_corner(0, 0, 1), (0, 0));
    assert_eq!(cell_corner(2, 1, 1), (80, 120));
    // 1:16: tile_w 10, tile_h 5: (y * 5) / 2 + (x * 5) / 2 truncates twice.
    assert_eq!(cell_corner(1, 1, 16), (0, 2 + 2));
    assert_eq!(cell_corner(3, 0, 16), (15, 7));
}
