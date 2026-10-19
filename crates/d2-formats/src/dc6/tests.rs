use super::*;
use crate::testutil;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// An independent encoder: image rows -> RLE (skips and literal runs of at
// most 127), rows ordered bottom-up unless `flip`.

#[derive(Clone, Debug)]
struct Img {
    w: usize,
    h: usize,
    /// 0 = transparent
    px: Vec<u8>,
    ox: i32,
    oy: i32,
    flip: bool,
}

fn encode_rows(img: &Img) -> Vec<u8> {
    let mut out = Vec::new();
    let rows: Vec<usize> = if img.flip {
        (0..img.h).collect()
    } else {
        (0..img.h).rev().collect()
    };
    for y in rows {
        let row = &img.px[y * img.w..(y + 1) * img.w];
        let mut x = 0;
        while x < img.w {
            if row[x] == 0 {
                let start = x;
                while x < img.w && row[x] == 0 {
                    x += 1;
                }
                // trailing transparent pixels need no opcode
                if x < img.w {
                    let mut n = x - start;
                    while n > 0 {
                        let step = n.min(127);
                        out.push(0x80 | step as u8);
                        n -= step;
                    }
                }
            } else {
                let start = x;
                while x < img.w && row[x] != 0 && x - start < 127 {
                    x += 1;
                }
                out.push((x - start) as u8);
                out.extend_from_slice(&row[start..x]);
            }
        }
        out.push(0x80);
    }
    out
}

fn build(dirs: u32, fpd: u32, frames: &[Img]) -> Vec<u8> {
    assert_eq!(frames.len(), (dirs * fpd) as usize);
    let mut out = Vec::new();
    for v in [6u32, 1, 0, 0xEEEE_EEEE, dirs, fpd] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let table = out.len();
    out.resize(table + 4 * frames.len(), 0);
    for (i, f) in frames.iter().enumerate() {
        let off = out.len() as u32;
        out[table + 4 * i..table + 4 * i + 4].copy_from_slice(&off.to_le_bytes());
        let rle = encode_rows(f);
        let next = off + 32 + rle.len() as u32 + 3;
        for v in [
            i32::from(f.flip),
            f.w as i32,
            f.h as i32,
            f.ox,
            f.oy,
            0,
            next as i32,
            rle.len() as i32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&rle);
        out.extend_from_slice(&[0xEE; 3]);
    }
    out
}

fn img(w: usize, h: usize, seed: u8, flip: bool) -> Img {
    let px = (0..w * h)
        .map(|i| {
            let v = (i as u8).wrapping_mul(37).wrapping_add(seed);
            if v.is_multiple_of(5) { 0 } else { v | 1 }
        })
        .collect();
    Img {
        w,
        h,
        px,
        ox: -(w as i32) / 2,
        oy: 7,
        flip,
    }
}

fn check(frame: &Dc6Frame, want: &Img) {
    let r = frame.render().unwrap();
    assert_eq!((r.width as usize, r.height as usize), (want.w, want.h));
    assert_eq!(r.offset_x, want.ox);
    assert_eq!(
        r.offset_y,
        want.oy - want.h as i32 + 1,
        "oy is the bottom row"
    );
    assert_eq!(r.pixels, want.px);
    let alpha: Vec<u8> = want
        .px
        .iter()
        .map(|&p| if p == 0 { 0 } else { 255 })
        .collect();
    assert_eq!(r.alpha, alpha);
}

#[test]
fn round_trip_bottom_up_and_top_down() {
    let frames = vec![
        img(5, 4, 1, false),
        img(9, 3, 2, false),
        img(300, 3, 5, false), // skips and literals longer than 127
        img(6, 6, 3, true),
    ];
    let bytes = build(2, 2, &frames);
    let dc6 = Dc6::parse(&bytes).unwrap();
    assert_eq!((dc6.version, dc6.flags, dc6.encoding), (6, 1, 0));
    assert_eq!((dc6.directions, dc6.frames_per_direction), (2, 2));
    assert_eq!(dc6.frames.len(), 4);
    for (i, want) in frames.iter().enumerate() {
        let (d, f) = (i as u32 / 2, i as u32 % 2);
        let frame = dc6.frame(d, f).unwrap();
        assert_eq!(frame.flipped, want.flip);
        assert_eq!(frame.data, encode_rows(want));
        check(frame, want);
    }
    assert!(dc6.frame(2, 0).is_none());
    assert!(dc6.frame(0, 2).is_none());
    assert!(dc6.frame(u32::MAX, u32::MAX).is_none());
}

#[test]
fn row_order_is_bottom_up_by_default() {
    // two rows: the first stream row is the bottom row
    let bytes = {
        let mut f = img(2, 2, 0, false);
        f.px = vec![1, 2, 3, 4]; // top row 1 2, bottom row 3 4
        build(1, 1, &[f])
    };
    let dc6 = Dc6::parse(&bytes).unwrap();
    let data = &dc6.frames[0].data;
    assert_eq!(data[..3], [2, 3, 4], "bottom row first");
    let r = dc6.frames[0].render().unwrap();
    assert_eq!(r.pixels, vec![1, 2, 3, 4]);
}

#[test]
fn empty_and_degenerate_frames() {
    let empty = Img {
        w: 0,
        h: 0,
        px: vec![],
        ox: 3,
        oy: 4,
        flip: false,
    };
    let dc6 = Dc6::parse(&build(1, 1, &[empty])).unwrap();
    let r = dc6.frames[0].render().unwrap();
    assert_eq!((r.width, r.height), (0, 0));
    // no directions at all
    let dc6 = Dc6::parse(&build(0, 0, &[])).unwrap();
    assert!(dc6.frames.is_empty());
    assert!(dc6.frame(0, 0).is_none());
}

#[test]
fn stream_overrun_and_underrun_are_clipped() {
    let mut dc6 = Dc6::parse(&build(1, 1, &[img(4, 2, 1, false)])).unwrap();
    // too many rows and literals running past the width are dropped
    dc6.frames[0].data = vec![6, 1, 2, 3, 4, 5, 6, 0x80, 0x80, 0x80, 2, 9, 9, 0x80];
    let r = dc6.frames[0].render().unwrap();
    assert_eq!(r.pixels.len(), 8);
    assert_eq!(&r.pixels[4..8], &[1, 2, 3, 4]);
    // an empty stream leaves everything transparent
    dc6.frames[0].data = vec![];
    assert!(
        dc6.frames[0]
            .render()
            .unwrap()
            .alpha
            .iter()
            .all(|&a| a == 0)
    );
    // a truncated literal run is clamped
    dc6.frames[0].data = vec![9, 7, 8];
    let r = dc6.frames[0].render().unwrap();
    assert_eq!(&r.pixels[4..6], &[7, 8]);
}

#[test]
fn oversized_frame_is_rejected_at_render() {
    let mut dc6 = Dc6::parse(&build(1, 1, &[img(2, 2, 1, false)])).unwrap();
    dc6.frames[0].width = 100_000;
    assert!(dc6.frames[0].render().is_err());
    dc6.frames[0].width = 4096;
    dc6.frames[0].height = 4096;
    assert!(dc6.frames[0].render().is_ok());
    dc6.frames[0].height = 4097;
    assert!(dc6.frames[0].render().is_err());
}

#[test]
fn corrupt_inputs_error_not_panic() {
    assert!(Dc6::parse(&[]).is_err());
    assert!(Dc6::parse(&[0; 23]).is_err());
    let good = build(1, 2, &[img(4, 4, 1, false), img(4, 4, 2, false)]);
    // wrong version
    let mut v = good.clone();
    v[0] = 5;
    assert!(matches!(
        Dc6::parse(&v),
        Err(FormatError::UnsupportedVersion { version: 5, .. })
    ));
    // huge direction / frame counts
    let mut big = good.clone();
    big[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    big[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Dc6::parse(&big).is_err());
    // frame offset past EOF
    let mut off = good.clone();
    off[24..28].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
    assert!(Dc6::parse(&off).is_err());
    // frame length past EOF
    let first = u32::from_le_bytes(good[24..28].try_into().unwrap()) as usize;
    let mut len = good.clone();
    len[first + 28..first + 32].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
    assert!(Dc6::parse(&len).is_err());
    // negative size
    let mut neg = good.clone();
    neg[first + 4..first + 8].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(Dc6::parse(&neg).is_err());
    for cut in 0..good.len() {
        if let Ok(d) = Dc6::parse(&good[..cut]) {
            for f in &d.frames {
                let _ = f.render();
            }
        }
    }
    assert!(Dc6::parse(&good).is_ok());
}

#[test]
fn shared_frame_data_cannot_amplify() {
    // 200 frames all pointing at one 60 KB frame
    let n = 200u32;
    let mut out = Vec::new();
    for v in [6u32, 1, 0, 0, 1, n] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let frame_at = 24 + 4 * n as usize;
    for _ in 0..n {
        out.extend_from_slice(&(frame_at as u32).to_le_bytes());
    }
    for v in [0i32, 1, 1, 0, 0, 0, 0, 5_000_000] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.resize(out.len() + 5_000_000, 0);
    // 5 MB of data x 200 frames = 1 GB if each frame were copied
    assert!(Dc6::parse(&out).is_err());
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        if let Ok(d) = Dc6::parse(&bytes) {
            for f in &d.frames {
                let _ = f.render();
            }
        }
    }

    #[test]
    fn random_streams_render_without_panic(
        data in prop::collection::vec(any::<u8>(), 0..300),
        w in 0u32..40, h in 0u32..40, flipped in any::<bool>(),
    ) {
        let f = Dc6Frame { flipped, width: w, height: h, offset_x: 0, offset_y: 0, data };
        let r = f.render().unwrap();
        prop_assert_eq!(r.pixels.len(), (w * h) as usize);
    }

    #[test]
    fn encoded_images_round_trip(
        w in 1usize..70, h in 1usize..12, seed in any::<u8>(), flip in any::<bool>(),
        ox in -500i32..500, oy in -500i32..500,
    ) {
        let mut want = img(w, h, seed, flip);
        want.ox = ox;
        want.oy = oy;
        let dc6 = Dc6::parse(&build(1, 1, std::slice::from_ref(&want))).unwrap();
        check(&dc6.frames[0], &want);
    }
}

#[test]
fn corpus_dc6_files_parse_and_render() {
    let mut files = 0usize;
    let mut frames = 0usize;
    let mut flipped = 0usize;
    let mut failures = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::find_files(&root, "dc6") {
            files += 1;
            let name = format!("{profile}:{}", path.strip_prefix(&root).unwrap().display());
            let bytes = std::fs::read(&path).unwrap();
            let dc6 = match Dc6::parse(&bytes) {
                Ok(d) => d,
                Err(e) => {
                    failures.push(format!("{name}: parse: {e}"));
                    continue;
                }
            };
            for (i, f) in dc6.frames.iter().enumerate() {
                frames += 1;
                flipped += usize::from(f.flipped);
                if let Err(e) = f.render() {
                    failures.push(format!("{name}: frame {i}: {e}"));
                }
            }
        }
    }
    println!(
        "dc6 corpus: {files} files, {frames} frames ({flipped} flipped), {} failures",
        failures.len()
    );
    println!("{failures:#?}");
    assert!(failures.is_empty());
}
