use super::*;
use crate::testutil;
use proptest::prelude::*;
use std::collections::BTreeSet;

mod fuzz;

// ---------------------------------------------------------------------------
// An independent DCC encoder, written from the format description in the
// module docs. It cuts frames into cells by the plain "4-pixel chunks, a lone
// trailing pixel merges into the previous chunk" rule (checked against
// `axis_cells` below) and models the "unchanged cell" semantics itself.

#[derive(Default)]
struct BitWriter {
    bits: Vec<bool>,
}

impl BitWriter {
    fn put(&mut self, value: u64, n: u32) {
        for i in 0..n {
            self.bits.push(value >> i & 1 != 0);
        }
    }
    fn append(&mut self, other: &BitWriter) {
        self.bits.extend_from_slice(&other.bits);
    }
    fn len(&self) -> usize {
        self.bits.len()
    }
    fn into_bytes(self) -> Vec<u8> {
        let mut out = vec![0u8; self.bits.len().div_ceil(8)];
        for (i, b) in self.bits.iter().enumerate() {
            if *b {
                out[i / 8] |= 1 << (i % 8);
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
struct Frame {
    xmin: i32,
    ymin: i32,
    w: usize,
    h: usize,
    /// palette indices, row-major, 0 = transparent
    px: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct Mode {
    equal_cells: bool, // flag bit 1
    raw: bool,         // flag bit 0 (encoding-type + raw-pixel streams)
}

fn chunks(rel: i32, len: usize) -> Vec<(usize, usize)> {
    // (start relative to the frame, length)
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut first = true;
    while pos < len {
        let size = if first {
            (4 - rel.rem_euclid(4) as usize).min(len - pos)
        } else {
            4.min(len - pos)
        };
        out.push((pos, size));
        pos += size;
        first = false;
    }
    if out.len() >= 2 && out.last().map(|c| c.1) == Some(1) {
        let (_, last_len) = out.pop().unwrap();
        out.last_mut().unwrap().1 += last_len;
    }
    out
}

fn delta_nibbles(mut d: u32, w: &mut BitWriter) {
    while d >= 15 {
        w.put(15, 4);
        d -= 15;
    }
    w.put(u64::from(d), 4);
}

#[derive(Clone)]
struct Stored {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

fn encode_direction(frames: &[Frame], mode: Mode) -> Vec<u8> {
    let x0 = frames.iter().map(|f| f.xmin).min().unwrap();
    let y0 = frames.iter().map(|f| f.ymin).min().unwrap();
    // palette-usage key
    let mut used: BTreeSet<u8> = BTreeSet::new();
    for f in frames {
        used.extend(f.px.iter().copied());
    }
    let code_of = |v: u8| used.iter().position(|&u| u == v).unwrap() as u32;
    let flag = u64::from(mode.equal_cells) << 1 | u64::from(mode.raw);

    let mut equal = BitWriter::default();
    let mut mask = BitWriter::default();
    let mut enc_type = BitWriter::default();
    let mut raw = BitWriter::default();
    let mut codes = BitWriter::default();
    let mut pixel_bits = BitWriter::default();

    let mut stored: std::collections::HashMap<(i32, i32), Stored> = Default::default();
    let mut seen: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut cell_counter = 0usize;
    for f in frames {
        let rel_x = f.xmin - x0;
        let rel_y = f.ymin - y0;
        for (cy_start, ch) in chunks(rel_y, f.h) {
            for (cx_start, cw) in chunks(rel_x, f.w) {
                let key = ((rel_x + cx_start as i32) / 4, (rel_y + cy_start as i32) / 4);
                let mut content = Vec::with_capacity(cw * ch);
                for y in 0..ch {
                    for x in 0..cw {
                        content.push(f.px[(cy_start + y) * f.w + cx_start + x]);
                    }
                }
                let first_time = seen.insert(key);
                if !first_time {
                    let prev = stored.get(&key);
                    // what the decoder would show for an "unchanged" cell
                    let kept: Vec<u8> = match prev {
                        Some(p) if p.w == cw && p.h == ch => p.px.clone(),
                        _ => vec![0; cw * ch],
                    };
                    if mode.equal_cells {
                        let same = kept == content;
                        equal.put(u64::from(same), 1);
                        if same {
                            stored.insert(
                                key,
                                Stored {
                                    w: cw,
                                    h: ch,
                                    px: content,
                                },
                            );
                            continue;
                        }
                    }
                    mask.put(0xF, 4);
                }
                // new pixel-buffer entry
                let mut distinct: Vec<u32> = content.iter().map(|&v| code_of(v)).collect();
                distinct.sort_unstable();
                distinct.dedup();
                assert!(
                    distinct.len() <= 4,
                    "test image has too many colours in a cell"
                );
                let nonzero: Vec<u32> = distinct.iter().copied().filter(|&c| c != 0).collect();
                let use_raw = mode.raw && cell_counter % 2 == 1;
                cell_counter += 1;
                if mode.raw {
                    enc_type.put(u64::from(use_raw), 1);
                }
                let mut last = 0u32;
                for &c in &nonzero {
                    if use_raw {
                        raw.put(u64::from(c), 8);
                    } else {
                        delta_nibbles(c - last, &mut codes);
                    }
                    last = c;
                }
                if nonzero.len() < 4 {
                    // terminator: a code equal to the previous one
                    if use_raw {
                        raw.put(u64::from(last), 8);
                    } else {
                        codes.put(0, 4);
                    }
                }
                // slot values: decoded codes descending, then zeros
                let mut val = [0u32; 4];
                for (i, &c) in nonzero.iter().rev().enumerate() {
                    val[i] = c;
                }
                if val[0] != val[1] {
                    let bits = if val[1] == val[2] { 1 } else { 2 };
                    for &v in &content {
                        let c = code_of(v);
                        let k = val.iter().position(|&s| s == c).unwrap();
                        pixel_bits.put(k as u64, bits);
                    }
                }
                stored.insert(
                    key,
                    Stored {
                        w: cw,
                        h: ch,
                        px: content,
                    },
                );
            }
        }
    }

    let mut dir = BitWriter::default();
    dir.put(0xDEAD, 32);
    dir.put(flag, 2);
    // width codes: variable0 0 bits, width/height/x/y 16 bits (code 9),
    // optional bytes 0 bits, coded bytes 0 bits
    for code in [0u64, 9, 9, 9, 9, 0, 0] {
        dir.put(code, 4);
    }
    for f in frames {
        dir.put(0, 0);
        dir.put(f.w as u64, 16);
        dir.put(f.h as u64, 16);
        dir.put(f.xmin as u16 as u64, 16);
        dir.put((f.ymin + f.h as i32 - 1) as u16 as u64, 16); // top-down: y is the bottom row
        dir.put(0, 1); // bottom_up = 0
    }
    if mode.equal_cells {
        dir.put(equal.len() as u64, 20);
    }
    dir.put(mask.len() as u64, 20);
    if mode.raw {
        dir.put(enc_type.len() as u64, 20);
        dir.put(raw.len() as u64, 20);
    }
    for v in 0..=255u8 {
        dir.put(u64::from(used.contains(&v)), 1);
    }
    if mode.equal_cells {
        dir.append(&equal);
    }
    dir.append(&mask);
    if mode.raw {
        dir.append(&enc_type);
        dir.append(&raw);
    }
    dir.append(&codes);
    dir.append(&pixel_bits);
    dir.into_bytes()
}

fn build_file(directions: &[Vec<Frame>], mode: Mode) -> Vec<u8> {
    let blobs: Vec<Vec<u8>> = directions
        .iter()
        .map(|d| encode_direction(d, mode))
        .collect();
    let fpd = directions.first().map_or(0, Vec::len) as u32;
    let mut out = vec![0x74, 6, directions.len() as u8];
    out.extend_from_slice(&fpd.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut offset = 15 + 4 * directions.len();
    for b in &blobs {
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += b.len();
    }
    for b in &blobs {
        out.extend_from_slice(b);
    }
    out
}

fn assert_round_trip(directions: &[Vec<Frame>], mode: Mode) {
    let bytes = build_file(directions, mode);
    let (dcc, warnings) = parse_checked(&bytes).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(dcc.version, 6);
    assert_eq!(dcc.directions.len(), directions.len());
    for (d, (got, want)) in dcc.directions.iter().zip(directions).enumerate() {
        assert_eq!(got.frames.len(), want.len());
        let x0 = want.iter().map(|f| f.xmin).min().unwrap();
        let y0 = want.iter().map(|f| f.ymin).min().unwrap();
        let x1 = want.iter().map(|f| f.xmin + f.w as i32 - 1).max().unwrap();
        let y1 = want.iter().map(|f| f.ymin + f.h as i32 - 1).max().unwrap();
        assert_eq!(
            got.bbox,
            DccBox {
                x: x0,
                y: y0,
                width: (x1 - x0 + 1) as u32,
                height: (y1 - y0 + 1) as u32
            }
        );
        for (i, (g, w)) in got.frames.iter().zip(want).enumerate() {
            assert_eq!(
                (g.offset_x, g.offset_y),
                (w.xmin, w.ymin),
                "dir {d} frame {i}"
            );
            assert_eq!(
                (g.width as usize, g.height as usize),
                (w.w, w.h),
                "dir {d} frame {i}"
            );
            assert_eq!(g.pixels, w.px, "dir {d} frame {i} ({mode:?})");
            let alpha: Vec<u8> = w.px.iter().map(|&p| if p == 0 { 0 } else { 255 }).collect();
            assert_eq!(g.alpha, alpha);
        }
    }
}

const MODES: [Mode; 4] = [
    Mode {
        equal_cells: false,
        raw: false,
    },
    Mode {
        equal_cells: true,
        raw: false,
    },
    Mode {
        equal_cells: false,
        raw: true,
    },
    Mode {
        equal_cells: true,
        raw: true,
    },
];

// ---------------------------------------------------------------------------

#[test]
fn bit_reader_is_lsb_first() {
    let data = [0b1010_0101u8, 0xFF, 0x01];
    let mut b = Bits::new(&data, 0, 24);
    assert_eq!(b.read(1), Some(1));
    assert_eq!(b.read(3), Some(0b010));
    assert_eq!(b.read(4), Some(0b1010));
    assert_eq!(b.read(12), Some(0x1FF));
    assert_eq!(b.read(4), Some(0));
    assert_eq!(b.read(1), None, "past the end");
    let mut c = Bits::new(&data, 4, 12);
    assert_eq!(c.read(8), Some(0xFA), "starts mid-byte");
    assert_eq!(c.read(5), None);
    let mut s = Bits::new(&data, 0, 24);
    assert_eq!(s.read_signed(4), Some(5));
    assert_eq!(s.read_signed(4), Some(-6)); // 0b1010
    assert_eq!(Bits::new(&[0xFF; 8], 0, 64).read(32), Some(u32::MAX));
    assert_eq!(Bits::new(&[0xFF; 8], 0, 64).read_signed(32), Some(-1));
    assert_eq!(Bits::new(&[], 0, 0).read(0), Some(0));
    assert_eq!(Bits::new(&[1], 0, 8).read(33), None);
}

#[test]
fn axis_cells_matches_the_chunk_rule() {
    for rel in -3..12 {
        for len in 1..40usize {
            let want: Vec<(i64, i64)> = chunks(rel, len)
                .into_iter()
                .map(|(s, l)| (i64::from(rel) + s as i64, l as i64))
                .collect();
            // `rel` is frame_min - dir_min and must be >= 0 for the decoder
            if rel >= 0 {
                assert_eq!(
                    axis_cells(i64::from(rel), 0, len as i64),
                    want,
                    "rel {rel} len {len}"
                );
            }
        }
    }
}

fn solid(xmin: i32, ymin: i32, w: usize, h: usize, v: u8) -> Frame {
    Frame {
        xmin,
        ymin,
        w,
        h,
        px: vec![v; w * h],
    }
}

#[test]
fn flat_and_empty_cells() {
    for mode in MODES {
        assert_round_trip(&[vec![solid(0, 0, 4, 4, 9)]], mode);
        assert_round_trip(&[vec![solid(-8, -20, 8, 8, 0)]], mode);
        assert_round_trip(&[vec![solid(3, 5, 1, 1, 77)]], mode);
        assert_round_trip(&[vec![solid(0, 0, 5, 5, 200)]], mode); // single 5x5 cell
        assert_round_trip(&[vec![solid(0, 0, 13, 9, 4), solid(0, 0, 13, 9, 4)]], mode);
    }
}

#[test]
fn two_colour_and_four_colour_cells() {
    // a 4x4 cell with colours 0, 10 and 250
    let px: Vec<u8> = (0..16).map(|i| [0u8, 10, 250][i % 3]).collect();
    let f = Frame {
        xmin: 0,
        ymin: 0,
        w: 4,
        h: 4,
        px,
    };
    // four distinct colours incl. transparent
    let px4: Vec<u8> = (0..16).map(|i| [0u8, 1, 2, 3][i % 4]).collect();
    let g = Frame {
        xmin: 0,
        ymin: 0,
        w: 4,
        h: 4,
        px: px4,
    };
    // four non-zero colours far apart (delta chains of 15+)
    let px5: Vec<u8> = (0..16).map(|i| [20u8, 40, 160, 255][(i / 3) % 4]).collect();
    let h = Frame {
        xmin: 0,
        ymin: 0,
        w: 4,
        h: 4,
        px: px5,
    };
    for mode in MODES {
        assert_round_trip(&[vec![f.clone()]], mode);
        assert_round_trip(&[vec![g.clone()]], mode);
        assert_round_trip(&[vec![h.clone()]], mode);
    }
}

#[test]
fn animation_with_equal_cells_and_moving_boxes() {
    // frame 1 repeats frame 0 except one cell, frame 2 changes the box
    let base: Vec<u8> = (0..8 * 8)
        .map(|i| if (i / 8) < 4 { 11 } else { 22 })
        .collect();
    let f0 = Frame {
        xmin: 0,
        ymin: 0,
        w: 8,
        h: 8,
        px: base.clone(),
    };
    let mut p1 = base.clone();
    for p in p1.iter_mut().skip(4).take(4) {
        *p = 33;
    }
    let f1 = Frame {
        xmin: 0,
        ymin: 0,
        w: 8,
        h: 8,
        px: p1,
    };
    // shifted by a whole cell and smaller: same buffer cells, new sizes
    let f2 = Frame {
        xmin: 4,
        ymin: 0,
        w: 4,
        h: 8,
        px: vec![44; 32],
    };
    // different phase
    let f3 = Frame {
        xmin: 1,
        ymin: 2,
        w: 9,
        h: 5,
        px: (0..45).map(|i| [0u8, 5][i % 2]).collect(),
    };
    for mode in MODES {
        assert_round_trip(
            &[vec![f0.clone(), f1.clone(), f2.clone(), f3.clone()]],
            mode,
        );
    }
}

#[test]
fn multiple_directions_and_negative_offsets() {
    let d0 = vec![solid(-10, -30, 6, 6, 5), solid(-9, -29, 6, 6, 6)];
    let d1 = vec![solid(0, 0, 12, 3, 9), solid(2, 1, 7, 7, 8)];
    for mode in MODES {
        assert_round_trip(&[d0.clone(), d1.clone()], mode);
    }
}

#[test]
fn unchanged_cell_of_a_new_size_is_cleared() {
    // frame 1's cell covers the same buffer cell but is 3 wide instead of 4,
    // so an "unchanged" marker clears it (all-zero content).
    let f0 = solid(0, 0, 4, 4, 9);
    let f1 = Frame {
        xmin: 0,
        ymin: 0,
        w: 3,
        h: 4,
        px: vec![0; 12],
    };
    assert_round_trip(
        &[vec![f0, f1]],
        Mode {
            equal_cells: true,
            raw: false,
        },
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: crate::ds1::fuzz_support::cases(96),
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_animations_round_trip(seed in any::<u64>(), mode_idx in 0usize..4) {
        let mut rng = Lcg(seed | 1);
        let ndirs = 1 + rng.below(2) as usize;
        let nframes = 1 + rng.below(4) as usize;
        let mut palette: Vec<u8> = (0..30).map(|_| rng.below(256) as u8).collect();
        palette.sort_unstable();
        palette.dedup();
        let dirs: Vec<Vec<Frame>> = (0..ndirs).map(|_| random_direction(&mut rng, nframes, &palette)).collect();
        assert_round_trip(&dirs, MODES[mode_idx]);
    }
}

struct Lcg(u64);
impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

fn random_direction(rng: &mut Lcg, nframes: usize, palette: &[u8]) -> Vec<Frame> {
    // boxes first, so the cell phase is known
    let mut boxes = Vec::new();
    for _ in 0..nframes {
        boxes.push((
            rng.below(9) as i32 - 3,
            rng.below(9) as i32 - 3,
            1 + rng.below(22) as usize,
            1 + rng.below(22) as usize,
        ));
    }
    let x0 = boxes.iter().map(|b| b.0).min().unwrap();
    let y0 = boxes.iter().map(|b| b.1).min().unwrap();
    let mut prev_cells: std::collections::HashMap<(i32, i32), (usize, usize, Vec<u8>)> =
        Default::default();
    let mut frames = Vec::new();
    for &(xmin, ymin, w, h) in &boxes {
        let mut px = vec![0u8; w * h];
        for (cy, ch) in chunks(ymin - y0, h) {
            for (cx, cw) in chunks(xmin - x0, w) {
                let key = ((xmin - x0 + cx as i32) / 4, (ymin - y0 + cy as i32) / 4);
                let cell: Vec<u8> = match prev_cells.get(&key) {
                    Some((pw, ph, p)) if *pw == cw && *ph == ch && rng.below(2) == 0 => p.clone(),
                    _ => {
                        let n = 1 + rng.below(4) as usize;
                        let colours: Vec<u8> = (0..n)
                            .map(|_| {
                                if rng.below(4) == 0 {
                                    0
                                } else {
                                    palette[rng.below(palette.len() as u64) as usize]
                                }
                            })
                            .collect();
                        (0..cw * ch)
                            .map(|_| colours[rng.below(n as u64) as usize])
                            .collect()
                    }
                };
                for y in 0..ch {
                    for x in 0..cw {
                        px[(cy + y) * w + cx + x] = cell[y * cw + x];
                    }
                }
                prev_cells.insert(key, (cw, ch, cell));
            }
        }
        frames.push(Frame {
            xmin,
            ymin,
            w,
            h,
            px,
        });
    }
    frames
}

#[test]
fn bottom_up_frames_use_the_offset_as_the_top() {
    // hand-build: one frame, bottom_up = 1, y offset = top row
    let mut dir = BitWriter::default();
    dir.put(0, 32);
    dir.put(0, 2);
    for code in [0u64, 9, 9, 9, 9, 0, 0] {
        dir.put(code, 4);
    }
    dir.put(4, 16); // width
    dir.put(4, 16); // height
    dir.put(0, 16); // x
    dir.put(10, 16); // y
    dir.put(1, 1); // bottom_up
    dir.put(0, 20); // pixel mask stream size: empty (the only cell is new)
    for v in 0..=255u8 {
        dir.put(u64::from(v == 5), 1);
    }
    // the one new cell decodes no codes (terminator nibble), so it is flat
    dir.put(0, 4);
    let blob = dir.into_bytes();
    let mut file = vec![0x74, 6, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    file.extend_from_slice(&15u32.wrapping_add(4).to_le_bytes());
    file.extend_from_slice(&blob);
    let dcc = Dcc::parse(&file).unwrap();
    let f = &dcc.directions[0].frames[0];
    assert_eq!((f.offset_x, f.offset_y, f.width, f.height), (0, 10, 4, 4));
}

#[test]
fn headers_and_limits() {
    assert!(Dcc::parse(&[]).is_err());
    assert!(Dcc::parse(&[0x74; 14]).is_err());
    // zero directions parse to an empty file
    let empty = Dcc::parse(&[0x74, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert!(empty.directions.is_empty());
    // too many directions / frames
    let mut many = vec![0x74, 6, 33];
    many.extend_from_slice(&[0; 12 + 4 * 33]);
    assert!(Dcc::parse(&many).is_err());
    let mut frames = vec![0x74, 6, 1];
    frames.extend_from_slice(&5000u32.to_le_bytes());
    frames.extend_from_slice(&[0; 8 + 4 + 64]);
    assert!(Dcc::parse(&frames).is_err());
    // direction offset outside the file
    let good = build_file(&[vec![solid(0, 0, 4, 4, 9)]], MODES[0]);
    let mut off = good.clone();
    off[15..19].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
    assert!(Dcc::parse(&off).is_err());
    // box limits: a frame header claiming a 60000-pixel-wide frame
    let mut wide = BitWriter::default();
    wide.put(0, 32);
    wide.put(0, 2);
    for code in [0u64, 9, 9, 9, 9, 0, 0] {
        wide.put(code, 4);
    }
    wide.put(60000, 16);
    wide.put(10, 16);
    wide.put(0, 16);
    wide.put(0, 16);
    wide.put(0, 1);
    let blob = wide.into_bytes();
    let mut file = vec![0x74, 6, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    file.extend_from_slice(&19u32.to_le_bytes());
    file.extend_from_slice(&blob);
    assert!(Dcc::parse(&file).is_err());
}

#[test]
fn truncation_and_bit_flips_never_panic() {
    let dirs = vec![
        vec![
            Frame {
                xmin: 0,
                ymin: 0,
                w: 9,
                h: 7,
                px: (0..63).map(|i| [0u8, 5, 9][i % 3]).collect(),
            },
            solid(1, 1, 6, 6, 5),
        ],
        vec![solid(-4, 2, 10, 10, 3), solid(-4, 2, 10, 10, 3)],
    ];
    for mode in MODES {
        let good = build_file(&dirs, mode);
        assert!(Dcc::parse(&good).is_ok());
        for cut in 0..good.len() {
            let _ = Dcc::parse(&good[..cut]);
        }
        for byte in 0..good.len() {
            for bit in 0..8 {
                let mut bad = good.clone();
                bad[byte] ^= 1 << bit;
                let _ = Dcc::parse(&bad);
            }
        }
    }
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..500)) {
        let _ = Dcc::parse(&bytes);
    }

    #[test]
    fn plausible_header_random_body_never_panics(
        dirs in 1u8..4, fpd in 0u32..6,
        body in prop::collection::vec(any::<u8>(), 0..400),
    ) {
        let mut bytes = vec![0x74, 6, dirs];
        bytes.extend_from_slice(&fpd.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        let first = 15 + 4 * u32::from(dirs);
        for i in 0..u32::from(dirs) {
            bytes.extend_from_slice(&(first + i * 20).to_le_bytes());
        }
        bytes.extend_from_slice(&body);
        let _ = Dcc::parse(&bytes);
    }

    #[test]
    fn mutated_valid_files_never_panic(seed in any::<u64>(), flips in 1usize..6) {
        let mut rng = Lcg(seed | 1);
        let palette: Vec<u8> = (1..20).collect();
        let dirs = vec![random_direction(&mut rng, 3, &palette)];
        let mut bytes = build_file(&dirs, MODES[(seed % 4) as usize]);
        for _ in 0..flips {
            let at = rng.below(bytes.len() as u64) as usize;
            bytes[at] ^= 1 << rng.below(8);
        }
        let _ = Dcc::parse(&bytes);
    }
}

// ---------------------------------------------------------------------------
// corpus

/// Runs `work` over `items` on all cores.
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

#[derive(Default)]
struct FileStats {
    frames: usize,
    pixels: u64,
    max_box: (u32, u32),
    directions: usize,
    problems: Vec<String>,
}

#[test]
fn corpus_dcc_files_decode() {
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
        let mut stats = FileStats::default();
        let name = format!("{profile}:{}", path.strip_prefix(root).unwrap().display());
        let bytes = std::fs::read(path).unwrap();
        match parse_checked(&bytes) {
            Err(e) => stats.problems.push(format!("{name}: {e}")),
            Ok((dcc, warnings)) => {
                for w in warnings {
                    stats.problems.push(format!("{name}: {w}"));
                }
                stats.directions = dcc.directions.len();
                for (d, dir) in dcc.directions.iter().enumerate() {
                    let b = dir.bbox;
                    for (i, f) in dir.frames.iter().enumerate() {
                        stats.frames += 1;
                        stats.pixels += u64::from(f.width) * u64::from(f.height);
                        stats.max_box =
                            (stats.max_box.0.max(b.width), stats.max_box.1.max(b.height));
                        let inside = f.offset_x >= b.x
                            && f.offset_y >= b.y
                            && i64::from(f.offset_x) + i64::from(f.width)
                                <= i64::from(b.x) + i64::from(b.width)
                            && i64::from(f.offset_y) + i64::from(f.height)
                                <= i64::from(b.y) + i64::from(b.height);
                        if !inside {
                            stats
                                .problems
                                .push(format!("{name}: dir {d} frame {i} outside its bbox"));
                        }
                        if f.pixels.len() != f.width as usize * f.height as usize {
                            stats
                                .problems
                                .push(format!("{name}: dir {d} frame {i} bad buffer"));
                        }
                    }
                }
            }
        }
        stats
    });
    let files = results.len();
    let frames: usize = results.iter().map(|r| r.frames).sum();
    let directions: usize = results.iter().map(|r| r.directions).sum();
    let max_pixels = results.iter().map(|r| r.pixels).max().unwrap_or(0);
    let max_w = results.iter().map(|r| r.max_box.0).max().unwrap_or(0);
    let max_h = results.iter().map(|r| r.max_box.1).max().unwrap_or(0);
    println!("largest file: {max_pixels} decoded pixels; largest direction box {max_w}x{max_h}");
    let problems: Vec<&String> = results.iter().flat_map(|r| &r.problems).collect();
    println!(
        "dcc corpus: {files} files, {directions} directions, {frames} frames, {} problems",
        problems.len()
    );
    for p in problems.iter().take(40) {
        println!("  {p}");
    }
    assert!(
        problems.is_empty(),
        "{} problems, first: {:?}",
        problems.len(),
        problems.first()
    );
}

#[test]
fn corpus_dcc_mutations_never_panic() {
    let Some(root) = testutil::extracted("vanilla") else {
        println!("skipped: extracted/vanilla missing");
        return;
    };
    let all = testutil::find_files(&root, "dcc");
    let sample: Vec<_> = all
        .iter()
        .step_by((all.len() / 40).max(1))
        .take(40)
        .collect();
    let mut rng = Lcg(0x5eed);
    let mut decoded = 0usize;
    for path in sample {
        let bytes = std::fs::read(path).unwrap();
        if bytes.len() > 40_000 {
            continue;
        }
        for _ in 0..6 {
            let mut bad = bytes.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(bad.len() as u64) as usize;
                bad[at] ^= 1 << rng.below(8);
            }
            decoded += usize::from(Dcc::parse(&bad).is_ok());
        }
        let cut = rng.below(bytes.len() as u64) as usize;
        let _ = Dcc::parse(&bytes[..cut]);
    }
    println!("mutated corpus files: {decoded} still decoded");
}

// ---------------------------------------------------------------------------
// Cross-check against the reference editor's stage 2 (dcc.c:1064-1249), which
// keeps one direction-sized bitmap alive across frames and copies "unchanged"
// cells between positions with alpha blending (transparent source pixels do
// not overwrite). Our decoder keeps pixels per buffer cell instead; this
// re-implements the reference model from the same stage-1 output and compares.

/// Frames of `dir` as the reference stage 2 would produce them, cropped to
/// each frame box.
fn reference_stage2(dir: &mut Direction<'_>, entries: &[Entry]) -> Vec<Vec<u8>> {
    let (dw, dh) = (dir.width as usize, dir.height as usize);
    let mut bmp = vec![0u8; dw * dh];
    // last (w, h, x0, y0) per buffer cell
    let mut last: Vec<Option<(usize, usize, usize, usize)>> = vec![None; dir.buf_cells];
    let mut cursor = 0usize;
    let mut out = Vec::new();
    for f in 0..dir.headers.len() {
        let h = &dir.headers[f];
        let grid = &dir.grids[f];
        let columns = grid.cols.len();
        let mut frm = vec![0u8; dw * dh];
        for (cy, &(ry, ch)) in grid.rows.iter().enumerate() {
            for (cx, &(rx, cw)) in grid.cols.iter().enumerate() {
                let slot = dir.buffer_slot(f, cx, cy).unwrap();
                let (x0, y0) = (rx as usize, ry as usize);
                let (cw, ch) = (cw as usize, ch as usize);
                let hit = entries
                    .get(cursor)
                    .filter(|e| e.frame == f as u32 && e.cell == (cx + cy * columns) as u32);
                if let Some(e) = hit {
                    cursor += 1;
                    let val = e.val;
                    let mut px = vec![0u8; cw * ch];
                    if val[0] == val[1] {
                        px.fill(val[0]);
                    } else {
                        let nb = if val[1] == val[2] { 1 } else { 2 };
                        for p in &mut px {
                            *p = val[dir.pcd.read(nb).unwrap() as usize];
                        }
                    }
                    for y in 0..ch {
                        for x in 0..cw {
                            let v = px[y * cw + x];
                            bmp[(y0 + y) * dw + x0 + x] = v;
                            if v != 0 {
                                frm[(y0 + y) * dw + x0 + x] = v;
                            }
                        }
                    }
                } else if let Some(Some((lw, lh, lx, ly))) = last.get(slot).copied()
                    && lw == cw
                    && lh == ch
                {
                    // copy the old cell to its new position, blended
                    let src: Vec<u8> = (0..ch)
                        .flat_map(|y| (0..cw).map(move |x| (x, y)))
                        .map(|(x, y)| bmp[(ly + y) * dw + lx + x])
                        .collect();
                    for y in 0..ch {
                        for x in 0..cw {
                            let v = src[y * cw + x];
                            if v != 0 {
                                bmp[(y0 + y) * dw + x0 + x] = v;
                            }
                        }
                    }
                    for y in 0..ch {
                        for x in 0..cw {
                            let v = bmp[(y0 + y) * dw + x0 + x];
                            if v != 0 {
                                frm[(y0 + y) * dw + x0 + x] = v;
                            }
                        }
                    }
                } else {
                    for y in 0..ch {
                        for x in 0..cw {
                            bmp[(y0 + y) * dw + x0 + x] = 0;
                        }
                    }
                }
                last[slot] = Some((cw, ch, x0, y0));
            }
        }
        // crop to the frame box
        let (fx, fy) = ((h.xmin - dir.x0) as usize, (h.ymin - dir.y0) as usize);
        let (fw, fh) = (h.width as usize, h.height as usize);
        let mut crop = Vec::with_capacity(fw * fh);
        for y in 0..fh {
            for x in 0..fw {
                crop.push(frm[(fy + y) * dw + fx + x]);
            }
        }
        out.push(crop);
    }
    out
}

#[test]
fn corpus_dcc_matches_reference_stage2_model() {
    let mut paths = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        let all = testutil::find_files(&root, "dcc");
        paths.extend(all.into_iter().step_by(50));
    }
    let results = parallel(&paths, |path| {
        let bytes = std::fs::read(path).unwrap();
        let Ok(dcc) = Dcc::parse(&bytes) else {
            return (0usize, 0usize, 0usize);
        };
        let dir_count = usize::from(bytes[2]);
        let offsets: Vec<usize> = (0..dir_count)
            .map(|i| u32_at(&bytes, 15 + 4 * i).unwrap() as usize)
            .collect();
        let (mut frames, mut differing, mut dirs_differing) = (0usize, 0usize, 0usize);
        for (d, &start) in offsets.iter().enumerate() {
            let end = offsets.get(d + 1).copied().unwrap_or(bytes.len());
            let mut budget = MAX_TOTAL_PIXELS;
            let mut dir = read_direction(&bytes[start..end], dcc.frames_per_direction, &mut budget)
                .unwrap()
                .unwrap();
            let entries = stage1(&mut dir).unwrap();
            let reference = reference_stage2(&mut dir, &entries);
            let mut any = false;
            for (mine, theirs) in dcc.directions[d].frames.iter().zip(&reference) {
                frames += 1;
                if &mine.pixels != theirs {
                    differing += 1;
                    any = true;
                }
            }
            dirs_differing += usize::from(any);
        }
        (frames, differing, dirs_differing)
    });
    let frames: usize = results.iter().map(|r| r.0).sum();
    let differing: usize = results.iter().map(|r| r.1).sum();
    let dirs: usize = results.iter().map(|r| r.2).sum();
    println!(
        "stage-2 cross-check: {} files sampled, {frames} frames, {differing} differ from the \
         reference model ({dirs} directions)",
        paths.len()
    );
    // The two models agree except where the reference's shared bitmap keeps
    // stale pixels (a handful per frame); that is rare.
    assert!(differing * 100 <= frames.max(1), "{differing} of {frames}");
}
