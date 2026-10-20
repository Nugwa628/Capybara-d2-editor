//! Export checks: palette files, indexed PNG, animated GIF, onion skin.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use capy_paint::{
    GifOptions, PaintError, animated_gif, animated_gif_with, delay_for_fps, indexed_png,
    nearest_index, onion_composite, palette_act, palette_gpl, palette_jasc, rgba_to_indexed,
};
use common::*;
use d2_formats::IndexedImage;

#[test]
fn palette_files() {
    let pal = test_palette();
    let act = palette_act(&pal);
    assert_eq!(act.len(), 768);
    assert_eq!(&act[3..6], &[60, 10, 10], "index 1 is RGB");
    assert_eq!(&act[765..], &[250, 250, 250]);

    let gpl = palette_gpl(&pal, "act1\nsneaky");
    let mut lines = gpl.lines();
    assert_eq!(lines.next(), Some("GIMP Palette"));
    assert_eq!(
        lines.next(),
        Some("Name: act1 sneaky"),
        "newlines cannot break the header"
    );
    assert_eq!(lines.next(), Some("Columns: 16"));
    assert_eq!(lines.next(), Some("#"));
    assert_eq!(lines.next(), Some("  0   0   0	Index 0"));
    assert_eq!(gpl.lines().count(), 4 + 256);
    assert!(gpl.contains(" 60  10  10\tIndex 1\n"));

    let jasc = palette_jasc(&pal);
    assert!(jasc.starts_with("JASC-PAL\r\n0100\r\n256\r\n0 0 0\r\n60 10 10\r\n"));
    assert_eq!(jasc.matches("\r\n").count(), 3 + 256);
}

fn sample() -> IndexedImage {
    let mut img = image(&["1.3", "249", ".z7"]);
    img.offset_x = -1;
    img.offset_y = -2;
    img
}

#[test]
fn indexed_png_decodes_to_the_same_indices_and_palette() {
    let pal = test_palette();
    let img = sample();
    let bytes = indexed_png(&img, &pal).unwrap();

    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = decoder.read_info().unwrap();
    let info = reader.info();
    assert_eq!((info.width, info.height), (3, 3));
    assert_eq!(info.color_type, png::ColorType::Indexed);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    let plte = info.palette.as_ref().expect("PLTE").to_vec();
    let expected_plte: Vec<u8> = pal.colors.iter().flatten().copied().collect();
    assert_eq!(plte, expected_plte, "same palette");
    let trns = info.trns.as_ref().expect("tRNS").to_vec();
    assert_eq!(trns, [0], "index 0 is transparent, everything else opaque");

    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut buf).unwrap();
    let indices = &buf[..frame.buffer_size()];
    let want: Vec<u8> = img
        .pixels
        .iter()
        .zip(&img.alpha)
        .map(|(&p, &a)| if a == 0 { 0 } else { p })
        .collect();
    assert_eq!(indices, want.as_slice(), "same indices");
}

#[test]
fn indexed_png_rejects_empty_images() {
    let img = IndexedImage::new(0, 3, 0, 0);
    assert!(matches!(
        indexed_png(&img, &test_palette()),
        Err(PaintError::Encode(_))
    ));
}

fn decode_gif(bytes: &[u8]) -> (Vec<u8>, u16, u16, Vec<gif::Frame<'static>>) {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = opts.read_info(std::io::Cursor::new(bytes)).unwrap();
    let palette = dec.global_palette().unwrap().to_vec();
    let (w, h) = (dec.width(), dec.height());
    let mut frames = Vec::new();
    while let Some(f) = dec.read_next_frame().unwrap() {
        frames.push(f.clone());
    }
    (palette, w, h, frames)
}

#[test]
fn animated_gif_has_the_right_frames_and_pixels() {
    let pal = test_palette();
    // Two frames with different sizes and offsets share one canvas.
    let mut a = image(&["12", "34"]);
    a.offset_x = 0;
    a.offset_y = 0;
    let mut b = image(&["5.", ".6", "78"]);
    b.offset_x = 1;
    b.offset_y = -1;
    let bytes = animated_gif(&[a, b], &pal, 7).unwrap();
    let (gpal, w, h, frames) = decode_gif(&bytes);
    // Union box: x 0..3, y -1..2 -> 3 x 3 canvas.
    assert_eq!((w, h), (3, 3));
    assert_eq!(
        gpal,
        pal.colors.iter().flatten().copied().collect::<Vec<u8>>()
    );
    assert_eq!(frames.len(), 2);
    for f in &frames {
        assert_eq!((f.width, f.height, f.left, f.top), (3, 3, 0, 0));
        assert_eq!(f.delay, 7);
        assert_eq!(f.transparent, Some(0));
    }
    // Canvas origin is (0, -1). Frame a sits at canvas (0, 1).
    assert_eq!(&frames[0].buffer[..], &[0, 0, 0, 1, 2, 0, 3, 4, 0]);
    // Frame b sits at canvas (1, 0); its transparent pixels are index 0.
    assert_eq!(&frames[1].buffer[..], &[0, 5, 0, 0, 0, 6, 0, 7, 8]);
}

#[test]
fn animated_gif_roundtrips_a_full_frame_and_rejects_nothing() {
    let pal = test_palette();
    let img = sample();
    let bytes = animated_gif(std::slice::from_ref(&img), &pal, 4).unwrap();
    let (_, w, h, frames) = decode_gif(&bytes);
    assert_eq!((w, h, frames.len()), (3, 3, 1));
    let want: Vec<u8> = img
        .pixels
        .iter()
        .zip(&img.alpha)
        .map(|(&p, &a)| if a == 0 { 0 } else { p })
        .collect();
    assert_eq!(&frames[0].buffer[..], want.as_slice());

    assert!(matches!(
        animated_gif(&[], &pal, 4),
        Err(PaintError::Encode(_))
    ));
    assert!(animated_gif(&[IndexedImage::new(0, 0, 0, 0)], &pal, 4).is_err());
}

#[test]
fn gif_options_scale_loop_background_and_delay() {
    let pal = test_palette();
    let img = image(&["1.", ".2"]);
    // Defaults equal the plain call.
    assert_eq!(
        animated_gif_with(std::slice::from_ref(&img), &pal, &GifOptions::new(5)).unwrap(),
        animated_gif(std::slice::from_ref(&img), &pal, 5).unwrap()
    );
    let opts = GifOptions {
        delay_cs: 9,
        looping: false,
        scale: 3,
        background: Some([1, 2, 3]),
    };
    let bytes = animated_gif_with(&[img.clone(), img.clone()], &pal, &opts).unwrap();
    let mut o = gif::DecodeOptions::new();
    o.set_color_output(gif::ColorOutput::Indexed);
    let dec = o.read_info(std::io::Cursor::new(&bytes)).unwrap();
    assert_eq!(dec.repeat(), gif::Repeat::Finite(0), "no loop extension");
    let (table, w, h, frames) = decode_gif(&bytes);
    assert_eq!((w, h, frames.len()), (6, 6, 2));
    assert_eq!(&table[..3], &[1, 2, 3], "colour 0 is the background");
    assert_eq!(frames[0].transparent, None);
    assert_eq!(frames[0].delay, 9);
    // The transparent corner shows the background; scaled 3x nearest.
    assert_eq!(frames[0].buffer[3], 0);
    assert_eq!(frames[0].buffer[0], 1);
    assert_eq!(frames[0].buffer[2 * 6 + 2], 1);
    assert_eq!(frames[0].buffer[3 * 6 + 3], 2);
    assert_eq!(frames[0].buffer[5 * 6 + 5], 2);
    // Looping is the default.
    let looped = animated_gif(&[img], &pal, 5).unwrap();
    let mut o2 = gif::DecodeOptions::new();
    o2.set_color_output(gif::ColorOutput::Indexed);
    let dec = o2.read_info(std::io::Cursor::new(&looped)).unwrap();
    assert_eq!(dec.repeat(), gif::Repeat::Infinite);
}

#[test]
fn delay_and_rgba_helpers() {
    assert_eq!(delay_for_fps(25.0), 4);
    assert_eq!(delay_for_fps(12.5), 8);
    assert_eq!(delay_for_fps(1000.0), 2);
    assert_eq!(delay_for_fps(0.0), 1000);
    let pal = test_palette();
    // Exact colour, a near colour, and a transparent pixel.
    let rgba = [
        60, 10, 10, 255, //
        61, 12, 9, 255, //
        0, 0, 0, 0, //
        10, 10, 228, 255,
    ];
    let img = rgba_to_indexed(2, 2, -1, -2, &rgba, &pal);
    assert_eq!((img.offset_x, img.offset_y), (-1, -2));
    assert_eq!(img.pixels, vec![1, 1, 0, 24]);
    assert_eq!(img.alpha, vec![255, 255, 0, 255]);
    assert_eq!(nearest_index(&pal, [0, 0, 0]), 1, "index 0 is never chosen");
}

#[test]
fn onion_composite_layers_current_over_faded_neighbours() {
    let pal = test_palette();
    let cur = image(&["1."]);
    let prev = image(&["22"]);
    let next = {
        let mut n = image(&[".3"]);
        n.offset_x = 1;
        n
    };
    let (w, h, rgba) = onion_composite(&cur, Some(&prev), Some(&next), &pal, 0.5);
    assert_eq!((w, h), (3, 1));
    assert_eq!(rgba.len(), 12);
    // Pixel 0: current, fully opaque red 1.
    assert_eq!(&rgba[0..4], &[60, 10, 10, 255]);
    // Pixel 1: only prev (index 2), faded to half alpha.
    assert_eq!(&rgba[4..8], &[84, 10, 10, 128]);
    // Pixel 2: only next (index 3), faded.
    assert_eq!(&rgba[8..12], &[108, 10, 10, 128]);
    // Without neighbours it is just the current frame.
    let (w, h, rgba) = onion_composite(&cur, None, None, &pal, 0.5);
    assert_eq!((w, h), (2, 1));
    assert_eq!(&rgba[4..8], &[0, 0, 0, 0]);
    // fade 0 hides the neighbours.
    let (_, _, rgba) = onion_composite(&cur, Some(&prev), None, &pal, 0.0);
    assert_eq!(rgba[7], 0);
}

#[test]
fn absurd_offsets_are_errors_not_panics() {
    let pal = test_palette();
    let mut a = image(&["1"]);
    a.offset_x = i32::MIN;
    let mut b = image(&["2"]);
    b.offset_x = i32::MAX;
    assert!(animated_gif(&[a.clone(), b.clone()], &pal, 1).is_err());
    let (w, h, rgba) = onion_composite(&a, Some(&b), None, &pal, 0.5);
    assert_eq!((w, h, rgba.len()), (0, 0, 0));
}
