use super::*;
use crate::dc6::Dc6;
use crate::testutil;
use proptest::prelude::*;

fn parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = work(item);
                    results.lock().unwrap().push(r);
                }
            });
        }
    });
    results.into_inner().unwrap()
}

fn frames_of(dcc: &Dcc) -> Vec<Vec<IndexedImage>> {
    dcc.directions.iter().map(|d| d.frames.clone()).collect()
}

fn img(w: u32, h: u32, ox: i32, oy: i32, px: Vec<u8>) -> IndexedImage {
    let alpha = px.iter().map(|&p| if p == 0 { 0 } else { 255 }).collect();
    IndexedImage {
        width: w,
        height: h,
        offset_x: ox,
        offset_y: oy,
        pixels: px,
        alpha,
    }
}

/// Encodes, decodes and checks that the decoder returns exactly the frames
/// `from_frames` stored.
fn round_trip(frames: &[Vec<IndexedImage>]) -> (Dcc, Vec<u8>) {
    let dcc = Dcc::from_frames(frames).unwrap();
    let bytes = dcc.to_bytes().unwrap();
    let (back, warnings) = parse_checked(&bytes).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.version, 6);
    assert_eq!(back.frames_per_direction, dcc.frames_per_direction);
    assert_eq!(back, dcc, "decoded file differs from the encoded frames");
    (dcc, bytes)
}

#[test]
fn single_flat_frames() {
    let a = img(1, 1, 0, 0, vec![7]);
    let (dcc, _) = round_trip(&[vec![a.clone()]]);
    assert_eq!(dcc.directions[0].frames[0], a);
    let t = img(1, 1, -3, 9, vec![0]);
    round_trip(&[vec![t]]);
    // all transparent, and all opaque of one colour
    round_trip(&[vec![img(9, 7, 2, -2, vec![0; 63])]]);
    round_trip(&[vec![img(9, 7, 2, -2, vec![200; 63])]]);
}

#[test]
fn empty_frames_become_one_transparent_pixel() {
    let empty = IndexedImage::new(0, 0, 4, -5);
    let (dcc, _) = round_trip(&[vec![empty, img(3, 3, 0, 0, vec![1; 9])]]);
    let f = &dcc.directions[0].frames[0];
    assert_eq!((f.width, f.height, f.offset_x, f.offset_y), (1, 1, 4, -5));
    assert_eq!(f.alpha, vec![0]);
}

#[test]
fn rejects_bad_shapes() {
    let a = img(2, 2, 0, 0, vec![1; 4]);
    assert!(Dcc::from_frames(&[]).is_err());
    assert!(Dcc::from_frames(&[vec![]]).is_err());
    assert!(Dcc::from_frames(&[vec![a.clone()], vec![a.clone(), a.clone()]]).is_err());
    let too_many: Vec<Vec<IndexedImage>> = (0..33).map(|_| vec![a.clone()]).collect();
    assert!(Dcc::from_frames(&too_many).is_err());
    let mut bad = a.clone();
    bad.pixels.pop();
    assert!(Dcc::from_frames(&[vec![bad]]).is_err());
    // box wider than the decoder accepts
    let wide = img(2049, 1, 0, 0, vec![1; 2049]);
    assert!(Dcc::from_frames(&[vec![wide]]).is_err());
}

#[test]
fn alpha_zero_means_transparent_and_partial_alpha_opaque() {
    let mut a = img(2, 1, 0, 0, vec![5, 6]);
    a.alpha = vec![0, 100];
    let dcc = Dcc::from_frames(&[vec![a]]).unwrap();
    let f = &dcc.directions[0].frames[0];
    assert_eq!(f.pixels, vec![0, 6]);
    assert_eq!(f.alpha, vec![0, 255]);
}

#[test]
fn crowded_cell_keeps_silhouette_and_most_frequent_colours() {
    // one 4x4 cell with 8 colours and some transparency
    let mut px = vec![0u8; 16];
    for (i, p) in px.iter_mut().enumerate().skip(2) {
        *p = if i < 8 { 10 } else { 20 + (i as u8 - 8) };
    }
    let dcc = Dcc::from_frames(&[vec![img(4, 4, 0, 0, px.clone())]]).unwrap();
    let out = &dcc.directions[0].frames[0].pixels;
    for (a, b) in px.iter().zip(out) {
        assert_eq!(*a == 0, *b == 0, "silhouette changed");
    }
    let mut distinct = out.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() <= 4);
    assert!(distinct.contains(&10), "the most frequent colour survives");
    // and the file round-trips
    round_trip(&[vec![img(4, 4, 0, 0, px)]]);
    // a custom distance decides where dropped colours go
    let px2: Vec<u8> = (1..=16).collect();
    let dcc = Dcc::from_frames_with(&[vec![img(4, 4, 0, 0, px2)]], &|_, b| {
        // pretend everything is closest to the largest kept colour
        255 - u32::from(b)
    })
    .unwrap();
    let out = &dcc.directions[0].frames[0].pixels;
    let kept: std::collections::BTreeSet<u8> = out.iter().copied().collect();
    assert_eq!(kept.len(), 4);
    assert_eq!(out.iter().filter(|&&p| p == 4).count(), 16 - 3);
}

#[test]
fn animation_round_trips_with_every_stream() {
    // a moving blob over 12 frames, odd sizes and offsets
    let frames: Vec<IndexedImage> = (0..12)
        .map(|i| {
            let (w, h) = (9 + (i % 3) as u32, 7 + (i % 2) as u32);
            let px = (0..w * h)
                .map(|k| {
                    let (x, y) = (k % w, k / w);
                    if (x + y + i as u32) % 5 < 2 {
                        0
                    } else {
                        1 + ((x / 3 + y / 3) % 3) as u8 * 40
                    }
                })
                .collect();
            img(w, h, -4 + i, -20 + (i % 4), px)
        })
        .collect();
    let dirs = vec![frames.clone(), frames.iter().rev().cloned().collect()];
    round_trip(&dirs);
}

#[test]
fn identical_frames_compress_to_almost_nothing() {
    let one = img(40, 40, 0, 0, (0..1600).map(|i| (i % 7) as u8).collect());
    let frames: Vec<IndexedImage> = (0..30).map(|_| one.clone()).collect();
    let (_, bytes) = round_trip(&[frames]);
    let single = Dcc::from_frames(&[vec![one]]).unwrap().to_bytes().unwrap();
    assert!(
        bytes.len() < single.len() * 2,
        "30 equal frames cost {} bytes, one frame {}",
        bytes.len(),
        single.len()
    );
}

#[test]
fn no_pixels_leak_between_frames() {
    // frame 0 is full, frame 1 is empty (transparent), frame 2 a single dot
    let full = img(16, 16, 0, 0, vec![9; 256]);
    let none = img(16, 16, 0, 0, vec![0; 256]);
    let mut dot = vec![0u8; 256];
    dot[255] = 3;
    let dot = img(16, 16, 0, 0, dot);
    let (dcc, _) = round_trip(&[vec![full, none.clone(), dot, none]]);
    assert!(dcc.directions[0].frames[1].pixels.iter().all(|&p| p == 0));
    assert!(dcc.directions[0].frames[3].pixels.iter().all(|&p| p == 0));
}

#[test]
fn header_figures_match_the_dc6_equivalent() {
    let a = img(300, 2, 0, 0, {
        let mut v = vec![0u8; 600];
        v[299] = 4;
        v[300] = 5;
        v
    });
    let (dcc, bytes) = round_trip(&[vec![a.clone()]]);
    assert_eq!(bytes[0], 0x74);
    assert_eq!(bytes[1], 6);
    assert_eq!(bytes[2], 1);
    let dc6 = Dc6::from_frames(&[vec![a]]).unwrap();
    let dc6_len = dc6.to_bytes().unwrap().len() as u32;
    let final_size = u32::from_le_bytes(bytes[11..15].try_into().unwrap());
    assert_eq!(final_size, dc6_len);
    assert_eq!(dcc.directions[0].bbox.width, 300);
}

#[test]
fn big_offsets_and_wide_fields() {
    let a = img(5, 5, -1500, 1000, vec![3; 25]);
    let b = img(5, 5, 900, -900, vec![4; 25]);
    // box would be too large for the decoder: refused, not truncated
    assert!(Dcc::from_frames(&[vec![a, b]]).is_err());
    let a = img(5, 5, -1000, 700, vec![3; 25]);
    let b = img(5, 5, 900, 1500, vec![4; 25]);
    round_trip(&[vec![a, b]]);
}

// ---------------------------------------------------------------------------
// random images

/// A small pseudo-random generator (proptest supplies the seed).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Random animation; `colours` palette values (index 0 transparent) are used.
fn random_dirs(seed: u64, palette_size: u64) -> Vec<Vec<IndexedImage>> {
    let mut rng = Lcg(seed);
    let dirs = 1 + rng.below(3) as usize;
    let frames = 1 + rng.below(5) as usize;
    let palette: Vec<u8> = (0..palette_size)
        .map(|_| rng.below(256) as u8)
        .chain([0])
        .collect();
    (0..dirs)
        .map(|_| {
            let mut prev: Option<IndexedImage> = None;
            (0..frames)
                .map(|_| {
                    let w = 1 + rng.below(30) as u32;
                    let h = 1 + rng.below(30) as u32;
                    let (ox, oy) = (rng.below(40) as i32 - 20, rng.below(40) as i32 - 30);
                    let mut px = vec![0u8; (w * h) as usize];
                    for (i, p) in px.iter_mut().enumerate() {
                        // smooth-ish: copy the previous frame sometimes
                        let from_prev = prev
                            .as_ref()
                            .filter(|q| q.pixels.len() > i && rng.below(3) > 0)
                            .map(|q| q.pixels[i]);
                        *p = from_prev.unwrap_or_else(|| {
                            if rng.below(4) == 0 {
                                0
                            } else {
                                palette[rng.below(palette.len() as u64) as usize]
                            }
                        });
                    }
                    let f = img(w, h, ox, oy, px);
                    prev = Some(f.clone());
                    f
                })
                .collect()
        })
        .collect()
}

proptest! {
    #[test]
    fn random_animations_round_trip(seed in any::<u64>(), palette in 1u64..40) {
        round_trip(&random_dirs(seed, palette));
    }

    #[test]
    fn reduction_keeps_the_silhouette(seed in any::<u64>()) {
        let input = random_dirs(seed, 200);
        let dcc = Dcc::from_frames(&input).unwrap();
        for (d, dir) in input.iter().enumerate() {
            for (f, want) in dir.iter().enumerate() {
                let got = &dcc.directions[d].frames[f];
                prop_assert_eq!(
                    (got.width, got.height, got.offset_x, got.offset_y),
                    (want.width, want.height, want.offset_x, want.offset_y)
                );
                for (a, b) in want.pixels.iter().zip(&got.pixels) {
                    prop_assert_eq!(*a == 0, *b == 0);
                }
            }
        }
    }

    #[test]
    fn few_colour_images_are_lossless(seed in any::<u64>()) {
        // 3 colours + transparency never exceeds four per cell
        let input = random_dirs(seed, 3);
        let dcc = Dcc::from_frames(&input).unwrap();
        for (d, dir) in input.iter().enumerate() {
            for (f, want) in dir.iter().enumerate() {
                prop_assert_eq!(&dcc.directions[d].frames[f], want);
            }
        }
    }

    #[test]
    fn long_transparent_lines_and_single_pixels(
        w in 1u32..400, h in 1u32..6, dot in any::<u16>(), seed in any::<u64>()
    ) {
        let mut px = vec![0u8; (w * h) as usize];
        let at = dot as usize % px.len();
        px[at] = 1 + (seed % 255) as u8;
        round_trip(&[vec![img(w, h, 0, 0, px)]]);
    }
}

// ---------------------------------------------------------------------------
// corpus

#[derive(Default)]
struct FileResult {
    ok: bool,
    original: usize,
    ours: usize,
    problem: Option<String>,
    reduced: bool,
}

#[test]
fn corpus_dcc_reencode_round_trip() {
    let mut paths = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for p in testutil::sample(testutil::find_files(&root, "dcc"), 20) {
            paths.push((profile, root.clone(), p));
        }
    }
    let results = parallel(&paths, |(profile, root, path)| {
        let name = format!("{profile}:{}", path.strip_prefix(root).unwrap().display());
        let bytes = std::fs::read(path).unwrap();
        let mut r = FileResult {
            original: bytes.len(),
            ..FileResult::default()
        };
        let original = match Dcc::parse(&bytes) {
            Ok(d) => d,
            Err(e) => {
                r.problem = Some(format!("{name}: parse: {e}"));
                return r;
            }
        };
        let rebuilt = match Dcc::from_frames(&frames_of(&original)) {
            Ok(d) => d,
            Err(e) => {
                r.problem = Some(format!("{name}: from_frames: {e}"));
                return r;
            }
        };
        r.reduced = rebuilt != original && frames_of(&rebuilt) != frames_of(&original);
        let out = match rebuilt.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                r.problem = Some(format!("{name}: to_bytes: {e}"));
                return r;
            }
        };
        r.ours = out.len();
        match parse_checked(&out) {
            Err(e) => r.problem = Some(format!("{name}: re-parse: {e}")),
            Ok((back, warnings)) => {
                if let Some(w) = warnings.first() {
                    r.problem = Some(format!("{name}: warning {w}"));
                } else if frames_of(&back) != frames_of(&original) {
                    r.problem = Some(format!("{name}: frames differ after re-encode"));
                } else if back
                    .directions
                    .iter()
                    .zip(&original.directions)
                    .any(|(a, b)| a.bbox != b.bbox)
                {
                    r.problem = Some(format!("{name}: bbox differs"));
                } else {
                    r.ok = true;
                }
            }
        }
        r
    });
    let files = results.len();
    let ok = results.iter().filter(|r| r.ok).count();
    let (orig, ours) = results
        .iter()
        .filter(|r| r.ok)
        .fold((0usize, 0usize), |a, r| (a.0 + r.original, a.1 + r.ours));
    let mut ratios: Vec<f64> = results
        .iter()
        .filter(|r| r.ok && r.original > 0)
        .map(|r| r.ours as f64 / r.original as f64)
        .collect();
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| {
        ratios
            .get(((ratios.len() as f64 - 1.0) * p) as usize)
            .copied()
    };
    println!(
        "dcc re-encode: {ok}/{files} files identical after decode; bytes {ours} vs {orig} \
         (ratio {:.4}); per-file ratio min {:?} median {:?} p95 {:?} max {:?}",
        ours as f64 / orig.max(1) as f64,
        pct(0.0),
        pct(0.5),
        pct(0.95),
        pct(1.0)
    );
    let problems: Vec<&String> = results.iter().filter_map(|r| r.problem.as_ref()).collect();
    for p in problems.iter().take(40) {
        println!("  {p}");
    }
    assert!(
        problems.is_empty(),
        "{} problems, first: {:?}",
        problems.len(),
        problems.first()
    );
    let _ = results.iter().filter(|r| r.reduced).count();
}
