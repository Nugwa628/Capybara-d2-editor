use super::*;
use crate::testutil;
use proptest::prelude::*;

fn image(w: u32, h: u32, ox: i32, oy: i32, px: Vec<u8>, alpha: Vec<u8>) -> IndexedImage {
    IndexedImage {
        width: w,
        height: h,
        offset_x: ox,
        offset_y: oy,
        pixels: px,
        alpha,
    }
}

/// What the decoder must return for `img`: index 0 and alpha 0 are both
/// transparent, everything else opaque.
fn normalised(img: &IndexedImage) -> IndexedImage {
    let mut out = img.clone();
    for (p, a) in out.pixels.iter_mut().zip(out.alpha.iter_mut()) {
        if *p == 0 || *a == 0 {
            *p = 0;
            *a = 0;
        } else {
            *a = 255;
        }
    }
    out
}

/// Walks an RLE stream and checks the Diablo II limits: every run is at most
/// 127, and every scanline ends with `0x80`.
fn assert_runs_legal(data: &[u8]) {
    let mut pos = 0;
    while pos < data.len() {
        let op = data[pos];
        pos += 1;
        if op == 0x80 {
            continue;
        }
        if op & 0x80 != 0 {
            assert!(op & 0x7F != 0, "zero-length skip");
        } else {
            assert!(op != 0, "zero-length literal run");
            pos += usize::from(op);
        }
    }
    assert_eq!(pos, data.len(), "last literal run overruns the stream");
    assert!(data.is_empty() || data.last() == Some(&0x80));
}

fn round_trip(frames: &[Vec<IndexedImage>]) -> Vec<u8> {
    let dc6 = Dc6::from_frames(frames).unwrap();
    for f in &dc6.frames {
        assert_runs_legal(&f.data);
    }
    let bytes = dc6.to_bytes().unwrap();
    let back = Dc6::parse(&bytes).unwrap();
    assert_eq!(back, dc6);
    assert_eq!((back.version, back.flags, back.encoding), (6, 1, 0));
    assert_eq!(&bytes[12..16], &[0xEE; 4]);
    for (d, dir) in frames.iter().enumerate() {
        for (i, want) in dir.iter().enumerate() {
            let got = back.frame(d as u32, i as u32).unwrap().render().unwrap();
            assert_eq!(got, normalised(want), "dir {d} frame {i}");
        }
    }
    bytes
}

#[test]
fn long_transparent_runs_are_split_at_127() {
    // 300 transparent, one pixel, 255 transparent, one pixel, then the rest
    let mut px = vec![0u8; 600];
    px[300] = 9;
    px[556] = 8;
    let img = image(600, 1, 0, 0, px, vec![255; 600]);
    let frame = Dc6Frame::from_image(&img);
    assert_runs_legal(&frame.data);
    assert_eq!(
        frame.data,
        vec![
            0xFF,
            0xFF,
            0x80 | 46, // 127 + 127 + 46 = 300
            1,
            9, //
            0xFF,
            0xFF,
            0x80 | 1, // 255
            1,
            8,
            0x80
        ]
    );
    round_trip(&[vec![img]]);
}

#[test]
fn long_literal_runs_are_split_at_127() {
    let px: Vec<u8> = (0..300).map(|i| 1 + (i % 200) as u8).collect();
    let img = image(300, 1, 0, 0, px.clone(), vec![255; 300]);
    let frame = Dc6Frame::from_image(&img);
    assert_eq!(&frame.data[..1], &[127]);
    assert_eq!(frame.data[128], 127);
    assert_eq!(frame.data[256], 46);
    round_trip(&[vec![img]]);
}

#[test]
fn rows_are_bottom_up_and_offsets_invert_render() {
    let img = image(2, 2, -5, 7, vec![1, 2, 3, 4], vec![255; 4]);
    let frame = Dc6Frame::from_image(&img);
    assert_eq!(frame.data, vec![2, 3, 4, 0x80, 2, 1, 2, 0x80]);
    assert!(!frame.flipped);
    assert_eq!((frame.offset_x, frame.offset_y), (-5, 8), "bottom row");
    assert_eq!(frame.render().unwrap(), img);
}

#[test]
fn transparent_index_zero_and_alpha_zero() {
    // index 0 with alpha 255 and index 5 with alpha 0 are both transparent
    let img = image(4, 1, 0, 0, vec![0, 5, 6, 7], vec![255, 0, 255, 255]);
    let frame = Dc6Frame::from_image(&img);
    assert_eq!(frame.data, vec![0x82, 2, 6, 7, 0x80]);
    assert_eq!(frame.render().unwrap(), normalised(&img));
}

#[test]
fn trailing_transparency_needs_no_opcode() {
    let img = image(
        5,
        2,
        0,
        0,
        vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![255; 10],
    );
    let frame = Dc6Frame::from_image(&img);
    assert_eq!(frame.data, vec![0x80, 1, 1, 0x80]);
}

#[test]
fn degenerate_frames() {
    let one = image(1, 1, 3, -4, vec![200], vec![255]);
    let clear = image(1, 1, 0, 0, vec![0], vec![0]);
    let empty = IndexedImage::new(0, 0, 3, 4);
    let flat = IndexedImage::new(0, 3, 0, 0);
    let bytes = round_trip(&[vec![one, clear, empty, flat]]);
    let back = Dc6::parse(&bytes).unwrap();
    let e = back.frame(0, 2).unwrap();
    assert_eq!((e.width, e.height, e.data.len()), (0, 0, 0));
    // no directions at all
    let none = Dc6::from_frames(&[]).unwrap();
    assert_eq!((none.directions, none.frames_per_direction), (0, 0));
    assert_eq!(Dc6::parse(&none.to_bytes().unwrap()).unwrap(), none);
}

#[test]
fn from_frames_and_to_bytes_validate_counts() {
    let a = IndexedImage::new(2, 2, 0, 0);
    assert!(Dc6::from_frames(&[vec![a.clone()], vec![a.clone(), a.clone()]]).is_err());
    let mut dc6 = Dc6::from_frames(&[vec![a.clone(), a.clone()]]).unwrap();
    assert!(dc6.to_bytes().is_ok());
    dc6.frames.pop();
    assert!(dc6.to_bytes().is_err());
}

#[test]
fn layout_is_the_documented_one() {
    let a = image(2, 1, 1, 2, vec![1, 2], vec![255; 2]);
    let bytes = Dc6::from_frames(&[vec![a.clone()], vec![a]])
        .unwrap()
        .to_bytes()
        .unwrap();
    let word = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    assert_eq!(
        [word(0), word(4), word(8), word(12), word(16), word(20)],
        [6, 1, 0, 0xEEEE_EEEE, 2, 1]
    );
    assert_eq!(word(24), 32);
    let first_len = 32 + 4 + 3; // header + [2,1,2,0x80] + slack
    assert_eq!(word(28) as usize, 32 + first_len);
    assert_eq!(&bytes[32 + 32 + 4..32 + 32 + 4 + 3], &[0xEE; 3]);
    assert_eq!(bytes.len(), 32 + 2 * first_len);
    // frame header: flip, w, h, x, bottom y
    assert_eq!(word(32), 0);
    assert_eq!((word(36), word(40), word(44), word(48)), (2, 1, 1, 2));
}

proptest! {
    #[test]
    fn random_images_round_trip(
        w in 1u32..40, h in 1u32..10, ox in -300i32..300, oy in -300i32..300,
        seed in any::<u64>(), density in 0u8..=255,
    ) {
        let mut state = seed | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let n = (w * h) as usize;
        let px: Vec<u8> = (0..n).map(|_| {
            if (next() & 255) as u8 >= density { 0 } else { (next() & 255) as u8 }
        }).collect();
        let alpha: Vec<u8> = (0..n).map(|_| if next() % 7 == 0 { 0 } else { 255 }).collect();
        round_trip(&[vec![image(w, h, ox, oy, px, alpha)]]);
    }

    #[test]
    fn full_width_transparent_lines(
        w in 128u32..700, rows in 1u32..6, opaque_row in 0u32..6, x in 0u32..700,
    ) {
        let mut px = vec![0u8; (w * rows) as usize];
        let r = opaque_row % rows;
        px[(r * w + x % w) as usize] = 77;
        let img = image(w, rows, 0, 0, px, vec![255; (w * rows) as usize]);
        let frame = Dc6Frame::from_image(&img);
        assert_runs_legal(&frame.data);
        round_trip(&[vec![img]]);
    }

    #[test]
    fn single_pixels(p in any::<u8>(), a in any::<u8>(), ox in any::<i16>(), oy in any::<i16>()) {
        round_trip(&[vec![image(1, 1, i32::from(ox), i32::from(oy), vec![p], vec![a])]]);
    }

    #[test]
    fn several_directions(dirs in 1usize..4, frames in 1usize..4, seed in any::<u8>()) {
        let all: Vec<Vec<IndexedImage>> = (0..dirs).map(|d| {
            (0..frames).map(|f| {
                let w = 1 + (d + f) as u32 * 3;
                let px = (0..w * 2).map(|i| (i as u8).wrapping_mul(seed | 1)).collect();
                image(w, 2, d as i32, f as i32, px, vec![255; (w * 2) as usize])
            }).collect()
        }).collect();
        round_trip(&all);
    }
}

#[test]
fn corpus_dc6_reencode_round_trip() {
    let mut files = 0usize;
    let mut frames = 0usize;
    let mut identical_data = 0usize;
    let mut flipped = 0usize;
    let mut problems = Vec::new();
    let (mut orig_bytes, mut our_bytes) = (0usize, 0usize);
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::sample(testutil::find_files(&root, "dc6"), 2) {
            let name = format!("{profile}:{}", path.strip_prefix(&root).unwrap().display());
            let bytes = std::fs::read(&path).unwrap();
            let Ok(original) = Dc6::parse(&bytes) else {
                problems.push(format!("{name}: does not parse"));
                continue;
            };
            files += 1;
            let mut dirs: Vec<Vec<IndexedImage>> = Vec::new();
            let mut skip = false;
            for d in 0..original.directions {
                let mut dir = Vec::new();
                for i in 0..original.frames_per_direction {
                    match original.frame(d, i).map(Dc6Frame::render) {
                        Some(Ok(img)) => dir.push(img),
                        other => {
                            problems.push(format!("{name}: frame {d}/{i}: {other:?}"));
                            skip = true;
                        }
                    }
                }
                dirs.push(dir);
            }
            if skip {
                continue;
            }
            let rebuilt = Dc6::from_frames(&dirs).unwrap();
            let out = rebuilt.to_bytes().unwrap();
            let back = match Dc6::parse(&out) {
                Ok(b) => b,
                Err(e) => {
                    problems.push(format!("{name}: re-parse: {e}"));
                    continue;
                }
            };
            orig_bytes += bytes.len();
            our_bytes += out.len();
            if back.directions != original.directions
                || back.frames_per_direction != original.frames_per_direction
            {
                problems.push(format!("{name}: shape changed"));
                continue;
            }
            for (i, (a, b)) in original.frames.iter().zip(&back.frames).enumerate() {
                frames += 1;
                flipped += usize::from(a.flipped);
                identical_data += usize::from(a.data == b.data);
                let (ra, rb) = (a.render().unwrap(), b.render().unwrap());
                if ra != rb {
                    problems.push(format!("{name}: frame {i} differs after re-encode"));
                }
                assert_runs_legal(&b.data);
            }
        }
    }
    println!(
        "dc6 re-encode: {files} files, {frames} frames identical after decode ({identical_data} \
         with byte-identical RLE data, {flipped} originally flipped); bytes {our_bytes} vs \
         {orig_bytes}; {} problems",
        problems.len()
    );
    for p in problems.iter().take(20) {
        println!("  {p}");
    }
    assert!(problems.is_empty());
}
