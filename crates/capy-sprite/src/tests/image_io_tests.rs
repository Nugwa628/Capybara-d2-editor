use std::io::Cursor;

use image::{ExtendedColorType, ImageEncoder};

use crate::error::SpriteError;
use crate::image_io::{ImageKind, export_png, import_image};
use crate::model::RgbaFrame;

fn gradient(w: u32, h: u32) -> RgbaFrame {
    let mut f = RgbaFrame::new(w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            f.put(
                x,
                y,
                [
                    (x * 255 / w.max(1)) as u8,
                    (y * 255 / h.max(1)) as u8,
                    (x ^ y) as u8,
                    if (x + y) % 7 == 0 { 0 } else { 255 },
                ],
            );
        }
    }
    f
}

#[test]
fn png_round_trip_is_lossless() {
    let f = gradient(13, 9);
    let bytes = export_png(&f).unwrap();
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
    let back = import_image(&bytes, ImageKind::Png).unwrap();
    assert_eq!(back, f);
}

#[test]
fn png_export_rejects_empty_and_mismatched_frames() {
    assert!(export_png(&RgbaFrame::new(0, 4, 0, 0)).is_err());
    let mut bad = RgbaFrame::new(2, 2, 0, 0);
    bad.pixels.pop();
    assert!(matches!(export_png(&bad), Err(SpriteError::Invalid(_))));
}

#[test]
fn bmp_and_gif_are_read() {
    let f = gradient(10, 6);
    let rgb: Vec<u8> = f
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut bmp = Vec::new();
    image::codecs::bmp::BmpEncoder::new(&mut Cursor::new(&mut bmp))
        .write_image(&rgb, 10, 6, ExtendedColorType::Rgb8)
        .unwrap();
    let back = import_image(&bmp, ImageKind::Bmp).unwrap();
    assert_eq!((back.width, back.height), (10, 6));
    for (a, b) in back
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(f.pixels.as_chunks::<4>().0)
    {
        assert_eq!(a[..3], b[..3]);
        assert_eq!(a[3], 255);
    }

    let mut gif = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
        let frame = image::Frame::new(image::RgbaImage::from_raw(10, 6, f.pixels.clone()).unwrap());
        enc.encode_frame(frame).unwrap();
    }
    let back = import_image(&gif, ImageKind::Gif).unwrap();
    assert_eq!((back.width, back.height), (10, 6));
    assert_eq!(back.pixels.len(), 10 * 6 * 4);
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    for kind in [
        ImageKind::Png,
        ImageKind::Bmp,
        ImageKind::Gif,
        ImageKind::Pcx,
    ] {
        assert!(matches!(
            import_image(&[1, 2, 3, 4, 5], kind),
            Err(SpriteError::Image(_))
        ));
        assert!(import_image(&[], kind).is_err());
    }
}

#[test]
fn kinds_from_extensions() {
    assert_eq!(ImageKind::from_extension("PNG"), Some(ImageKind::Png));
    assert_eq!(ImageKind::from_extension("pcx"), Some(ImageKind::Pcx));
    assert_eq!(ImageKind::from_extension("dc6"), None);
}

// ---------------------------------------------------------------------------
// PCX

fn rle(row: &[u8], out: &mut Vec<u8>) {
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

fn pcx_header(w: u16, h: u16, bpp: u8, planes: u8, bytes_per_line: u16) -> Vec<u8> {
    let mut hd = vec![0u8; 128];
    hd[0] = 0x0A;
    hd[1] = 5;
    hd[2] = 1;
    hd[3] = bpp;
    hd[8..10].copy_from_slice(&(w - 1).to_le_bytes());
    hd[10..12].copy_from_slice(&(h - 1).to_le_bytes());
    hd[65] = planes;
    hd[66..68].copy_from_slice(&bytes_per_line.to_le_bytes());
    hd
}

#[test]
fn pcx_8_bit_with_palette() {
    // 5x3, bytes per line 6 (even), with a run and a byte >= 0xC0
    let (w, h, bpl) = (5u16, 3u16, 6u16);
    let rows: [[u8; 6]; 3] = [
        [1, 1, 1, 1, 2, 0],
        [0xC1, 0xC1, 3, 4, 5, 0],
        [7, 8, 9, 10, 11, 0],
    ];
    let mut file = pcx_header(w, h, 8, 1, bpl);
    for r in &rows {
        rle(r, &mut file);
    }
    file.push(0x0C);
    for i in 0..256usize {
        file.extend_from_slice(&[i as u8, (255 - i) as u8, (i / 2) as u8]);
    }
    let img = import_image(&file, ImageKind::Pcx).unwrap();
    assert_eq!(
        (img.width, img.height, img.offset_x, img.offset_y),
        (5, 3, 0, 0)
    );
    let px = |x, y| img.get(x, y).unwrap();
    assert_eq!(px(0, 0), [1, 254, 0, 255]);
    assert_eq!(px(4, 0), [2, 253, 1, 255]);
    assert_eq!(px(0, 1), [0xC1, 255 - 0xC1, 0xC1 / 2, 255]);
    assert_eq!(px(4, 2), [11, 244, 5, 255]);
}

#[test]
fn pcx_8_bit_without_palette_is_grey() {
    let mut file = pcx_header(2, 1, 8, 1, 2);
    rle(&[10, 200], &mut file);
    let img = import_image(&file, ImageKind::Pcx).unwrap();
    assert_eq!(img.get(0, 0), Some([10, 10, 10, 255]));
    assert_eq!(img.get(1, 0), Some([200, 200, 200, 255]));
}

#[test]
fn pcx_24_bit_planar() {
    let (w, h, bpl) = (3u16, 2u16, 4u16);
    let mut file = pcx_header(w, h, 8, 3, bpl);
    for y in 0..h as u8 {
        rle(&[10 + y, 11 + y, 12 + y, 0], &mut file); // red plane
        rle(&[20 + y, 21 + y, 22 + y, 0], &mut file); // green
        rle(&[30 + y, 31 + y, 32 + y, 0], &mut file); // blue
    }
    let img = import_image(&file, ImageKind::Pcx).unwrap();
    assert_eq!(img.get(0, 0), Some([10, 20, 30, 255]));
    assert_eq!(img.get(2, 1), Some([13, 23, 33, 255]));
}

#[test]
fn pcx_one_bit_mono_and_four_bit_planes() {
    // 1 bit, 1 plane: 10 pixels in 2 bytes
    let mut file = pcx_header(10, 1, 1, 1, 2);
    rle(&[0b1010_0000, 0b0100_0000], &mut file);
    let img = import_image(&file, ImageKind::Pcx).unwrap();
    let on: Vec<bool> = (0..10).map(|x| img.get(x, 0).unwrap()[0] == 255).collect();
    assert_eq!(
        on,
        vec![
            true, false, true, false, false, false, false, false, false, true
        ]
    );

    // 1 bit per pixel, 4 planes: index = bit0 | bit1<<1 | ...; header palette
    let mut file = pcx_header(8, 1, 1, 4, 2);
    file[16 + 3 * 5..16 + 3 * 5 + 3].copy_from_slice(&[9, 8, 7]);
    // pixel 0 has plane bits 1 and 4 set (0b0101 = 5)
    rle(&[0b1000_0000, 0], &mut file);
    rle(&[0, 0], &mut file);
    rle(&[0b1000_0000, 0], &mut file);
    rle(&[0, 0], &mut file);
    let img = import_image(&file, ImageKind::Pcx).unwrap();
    assert_eq!(img.get(0, 0), Some([9, 8, 7, 255]));
    assert_eq!(img.get(1, 0), Some([0, 0, 0, 255]));
}

#[test]
fn pcx_rejects_bad_files() {
    let good = {
        let mut f = pcx_header(2, 2, 8, 1, 2);
        rle(&[1, 2, 3, 4], &mut f);
        f
    };
    assert!(import_image(&good, ImageKind::Pcx).is_ok());
    // truncated header, bad magic, truncated data
    assert!(import_image(&good[..100], ImageKind::Pcx).is_err());
    let mut bad = good.clone();
    bad[0] = 0;
    assert!(import_image(&bad, ImageKind::Pcx).is_err());
    assert!(import_image(&good[..good.len() - 1], ImageKind::Pcx).is_err());
    // inverted window, absurd size, bad plane count, bytes per line too small
    let mut inv = good.clone();
    inv[8..10].copy_from_slice(&0u16.to_le_bytes());
    inv[4..6].copy_from_slice(&5u16.to_le_bytes());
    assert!(import_image(&inv, ImageKind::Pcx).is_err());
    let mut huge = good.clone();
    huge[8..10].copy_from_slice(&60000u16.to_le_bytes());
    huge[10..12].copy_from_slice(&60000u16.to_le_bytes());
    assert!(import_image(&huge, ImageKind::Pcx).is_err());
    let mut planes = good.clone();
    planes[65] = 9;
    assert!(import_image(&planes, ImageKind::Pcx).is_err());
    let mut bpl = good;
    bpl[66..68].copy_from_slice(&1u16.to_le_bytes());
    assert!(import_image(&bpl, ImageKind::Pcx).is_err());
}

#[test]
fn pcx_random_bytes_never_panic() {
    let mut state = 0x1234_5678_9abc_def1u64;
    for _ in 0..300 {
        let n = (state >> 59) as usize * 20;
        let mut bytes: Vec<u8> = (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 16) as u8
            })
            .collect();
        if bytes.len() > 4 {
            bytes[0] = 0x0A;
        }
        let _ = import_image(&bytes, ImageKind::Pcx);
    }
}
