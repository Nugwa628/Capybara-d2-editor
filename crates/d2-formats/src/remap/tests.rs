use super::*;
use crate::dc6::Dc6Frame;
use crate::dt1::Dt1;
use crate::testutil;
use proptest::prelude::*;
use std::collections::BTreeSet;

mod corpus;
mod fuzz;

// ---------------------------------------------------------------------------
// tables

fn identity() -> IndexTable {
    std::array::from_fn(|i| i as u8)
}

/// Act-5-to-act-1 style: identity except 225..=254, which move to nonzero
/// indices below 225.
fn act_table() -> IndexTable {
    let mut t = identity();
    for (i, v) in t.iter_mut().enumerate().skip(225).take(30) {
        *v = (1 + (i * 37) % 224) as u8;
    }
    t
}

/// A valid table from a seed: every nonzero index goes to some nonzero index.
fn random_table(seed: u64) -> IndexTable {
    let mut rng = testutil::Rng::new(seed);
    let mut t = [0u8; 256];
    for v in t.iter_mut().skip(1) {
        *v = 1 + rng.below(255) as u8;
    }
    t
}

/// `hist` pushed through `table`: what the histogram of the remapped file
/// must be.
fn mapped_hist(hist: &[u64; 256], table: &IndexTable) -> [u64; 256] {
    let mut out = [0u64; 256];
    for (i, &n) in hist.iter().enumerate() {
        out[usize::from(table[i])] += n;
    }
    out
}

fn map_img(img: &IndexedImage, table: &IndexTable) -> IndexedImage {
    let mut out = img.clone();
    for p in &mut out.pixels {
        *p = table[usize::from(*p)];
    }
    out
}

/// `mask[i]`: byte `i` is a pixel literal according to `walk`.
fn pixel_mask(kind: SpriteKind, bytes: &[u8]) -> Vec<bool> {
    let mut mask = vec![false; bytes.len()];
    walk(kind, bytes, &mut |start, len, role| {
        if role == Role::Pixels {
            mask[start..start + len].fill(true);
        }
    })
    .unwrap();
    mask
}

fn mask_of(set: &BTreeSet<usize>, len: usize) -> Vec<bool> {
    (0..len).map(|i| set.contains(&i)).collect()
}

/// The DT1/DC6 contract against `out = remap(bytes, table)`: same length,
/// pixel bytes mapped, every other byte unchanged, histogram mapped.
fn check_patch(kind: SpriteKind, bytes: &[u8], out: &[u8], table: &IndexTable, pixels: &[bool]) {
    assert_eq!(out.len(), bytes.len());
    assert_eq!(pixels.len(), bytes.len());
    for (i, ((&a, &b), &px)) in bytes.iter().zip(out).zip(pixels).enumerate() {
        if px {
            assert_eq!(b, table[usize::from(a)], "pixel byte {i:#x}");
        } else {
            assert_eq!(a, b, "non-pixel byte {i:#x} changed");
        }
    }
    let before = index_histogram(kind, bytes).unwrap();
    let after = index_histogram(kind, out).unwrap();
    assert_eq!(after, mapped_hist(&before, table));
}

// ---------------------------------------------------------------------------
// basics

#[test]
fn kind_from_path() {
    use SpriteKind::*;
    assert_eq!(
        SpriteKind::from_path(r"data\global\tiles\act1\town\floor.dt1"),
        Some(Dt1)
    );
    assert_eq!(
        SpriteKind::from_path("data/global/ui/PANEL/invchar6.DC6"),
        Some(Dc6)
    );
    assert_eq!(
        SpriteKind::from_path(r"C:\mods\x\BaHdLitA1HTH.Dcc"),
        Some(Dcc)
    );
    assert_eq!(SpriteKind::from_path("floor.dt1"), Some(Dt1));
    assert_eq!(SpriteKind::from_path("floor.ds1"), None);
    assert_eq!(SpriteKind::from_path("dt1"), None);
    assert_eq!(SpriteKind::from_path(r"a.dt1\floor"), None);
    assert_eq!(SpriteKind::from_path("a.dcc/b"), None);
    assert_eq!(SpriteKind::from_path(""), None);
    assert_eq!(SpriteKind::from_path("x.dt1.bak"), None);
}

#[test]
fn bad_tables_are_invalid() {
    let dt1 = Dt1Fixture::new().bytes;
    let dc6 = dc6_fixture().0;
    let dcc = dcc_fixture();
    let mut t0 = identity();
    t0[0] = 5;
    let mut t1 = identity();
    t1[17] = 0;
    let mut t2 = identity();
    t2[255] = 0;
    for table in [t0, t1, t2] {
        for (kind, bytes) in [
            (SpriteKind::Dt1, &dt1),
            (SpriteKind::Dc6, &dc6),
            (SpriteKind::Dcc, &dcc),
        ] {
            let err = remap_indices(kind, bytes, &table).unwrap_err();
            assert!(
                matches!(
                    err,
                    FormatError::Invalid {
                        what: "index table",
                        ..
                    }
                ),
                "{err}"
            );
        }
    }
    // the table is checked before the data
    assert!(matches!(
        remap_indices(SpriteKind::Dt1, &[], &t1),
        Err(FormatError::Invalid {
            what: "index table",
            ..
        })
    ));
}

#[test]
fn unparsable_files_are_errors() {
    for kind in [SpriteKind::Dt1, SpriteKind::Dc6, SpriteKind::Dcc] {
        assert!(remap_indices(kind, &[], &identity()).is_err());
        assert!(index_histogram(kind, &[]).is_err());
    }
    // a 4.1 DT1 (major version 4) is the older layout parse refuses
    let mut old = Dt1Fixture::new().bytes;
    old[0..4].copy_from_slice(&4i32.to_le_bytes());
    old[4..8].copy_from_slice(&1i32.to_le_bytes());
    assert!(matches!(
        remap_indices(SpriteKind::Dt1, &old, &identity()),
        Err(FormatError::UnsupportedVersion { version: 4, .. })
    ));
    assert!(index_histogram(SpriteKind::Dt1, &old).is_err());
}

// ---------------------------------------------------------------------------
// DT1 fixture: an independent builder that records where the pixel literals
// are. Filler bytes in headers, padding and the tail use 0xE0..0xEF, inside
// the act-specific range, so a remap that touched them would show.

struct Dt1Fixture {
    bytes: Vec<u8>,
    /// Offsets of pixel literal bytes (each once, even when shared).
    pixels: BTreeSet<usize>,
    /// Literal bytes per index, counted once per block that draws them.
    hist: [u64; 256],
}

enum Rle {
    Run(u8, Vec<u8>),
    Skip(u8),
    Eol,
    /// A run whose count claims more literals than the block holds (must
    /// be the last item).
    Cut(u8, Vec<u8>),
    /// A lone trailing byte (must be the last item).
    Odd(u8),
}

/// A tile's own blocks, or `Some((tile, k))`: share the first `k` sub-block
/// headers of an earlier tile.
type TileDef = (Vec<BlockDef>, Option<(usize, usize)>);

struct BlockDef {
    x: i16,
    y: i16,
    format: i16,
    data: Vec<u8>,
    /// Offsets inside `data` of literal bytes.
    literals: Vec<usize>,
}

fn rle(items: &[Rle], x: i16, y: i16, format: i16) -> BlockDef {
    let mut data = Vec::new();
    let mut literals = Vec::new();
    for item in items {
        match item {
            Rle::Run(skip, px) => {
                data.extend([*skip, px.len() as u8]);
                literals.extend(data.len()..data.len() + px.len());
                data.extend(px);
            }
            Rle::Skip(n) => data.extend([*n, 0]),
            Rle::Eol => data.extend([0, 0]),
            Rle::Cut(skip, px) => {
                data.extend([*skip, px.len() as u8 + 9]);
                literals.extend(data.len()..data.len() + px.len());
                data.extend(px);
            }
            Rle::Odd(b) => data.push(*b),
        }
    }
    BlockDef {
        x,
        y,
        format,
        data,
        literals,
    }
}

fn iso(seed: u8) -> BlockDef {
    BlockDef {
        x: 0,
        y: 0,
        format: 1,
        // every index, including 0 and the act range
        data: (0..256u32).map(|i| (i as u8).wrapping_add(seed)).collect(),
        literals: (0..256).collect(),
    }
}

impl Dt1Fixture {
    /// Tile 0: an isometric block, an RLE block and a 100-byte isometric
    /// block (draws nothing). Tile 1 shares tile 0's first two sub-block
    /// headers (and so their data). Tile 2 holds an RLE block whose last run
    /// is cut short.
    fn new() -> Dt1Fixture {
        let a_rle = rle(
            &[
                Rle::Run(3, vec![1, 230, 0, 254]),
                Rle::Eol,
                Rle::Run(0, vec![240, 241]),
                Rle::Skip(5),
                Rle::Run(2, vec![225, 7, 0]),
                Rle::Eol,
                Rle::Run(31, vec![255]),
                Rle::Eol,
                Rle::Odd(0xE5),
            ],
            0,
            20,
            0x1001,
        );
        let a_bad_iso = BlockDef {
            x: 40,
            y: 0,
            format: 1,
            data: vec![0xE8; 100],
            literals: Vec::new(),
        };
        let c_rle = rle(
            &[
                Rle::Run(1, vec![9, 233]),
                Rle::Eol,
                Rle::Cut(0, vec![250, 251, 0]),
            ],
            -8,
            -4,
            0,
        );
        // (blocks, shares the header array of tile `n` with `k` blocks)
        let tiles: Vec<TileDef> = vec![
            (vec![iso(3), a_rle, a_bad_iso], None),
            (Vec::new(), Some((0, 2))),
            (vec![c_rle], None),
        ];
        let mut out = vec![0u8; 276];
        out[0..4].copy_from_slice(&7i32.to_le_bytes());
        out[4..8].copy_from_slice(&6i32.to_le_bytes());
        out[8..20].copy_from_slice(b"synth\xE1\xE2\xE3\xE4\xEE\xEF\xFE");
        out[268..272].copy_from_slice(&(tiles.len() as i32).to_le_bytes());
        out[272..276].copy_from_slice(&276i32.to_le_bytes());
        out.resize(276 + 96 * tiles.len(), 0);
        let mut pixels = BTreeSet::new();
        let mut hist = [0u64; 256];
        let mut placed: Vec<(usize, usize)> = Vec::new(); // (ptr, count) per tile
        for (t, (blocks, share)) in tiles.iter().enumerate() {
            let th = 276 + 96 * t;
            // unread header bytes carry filler
            for o in [16usize, 17, 18, 19, 36, 37, 38, 39, 65, 70, 84, 95] {
                out[th + o] = 0xE0 + (o % 16) as u8;
            }
            out[th + 8..th + 12].copy_from_slice(&(-80i32).to_le_bytes());
            out[th + 12..th + 16].copy_from_slice(&160i32.to_le_bytes());
            out[th + 24..th + 28].copy_from_slice(&(t as i32).to_le_bytes());
            out[th + 32..th + 36].copy_from_slice(&1i32.to_le_bytes());
            let (ptr, count) = if let Some((src, k)) = share {
                let (ptr, _) = placed[*src];
                // the shared blocks draw again
                let (src_blocks, _) = &tiles[*src];
                for b in src_blocks.iter().take(*k) {
                    for &l in &b.literals {
                        hist[usize::from(b.data[l])] += 1;
                    }
                }
                (ptr, *k)
            } else {
                // two bytes of padding before the header array
                out.extend([0xE9, 0xEA]);
                let ptr = out.len();
                let n = blocks.len();
                out.resize(ptr + 20 * n, 0);
                for (s, b) in blocks.iter().enumerate() {
                    // padding between blocks
                    out.extend([0xEB; 3]);
                    let data_at = out.len();
                    let h = ptr + 20 * s;
                    out[h..h + 2].copy_from_slice(&b.x.to_le_bytes());
                    out[h + 2..h + 4].copy_from_slice(&b.y.to_le_bytes());
                    out[h + 4] = 0xEC;
                    out[h + 5] = 0xED;
                    out[h + 8..h + 10].copy_from_slice(&b.format.to_le_bytes());
                    out[h + 10..h + 12].copy_from_slice(&(b.data.len() as u16).to_le_bytes());
                    out[h + 12..h + 16].copy_from_slice(&[0xE6; 4]);
                    out[h + 16..h + 20].copy_from_slice(&((data_at - ptr) as i32).to_le_bytes());
                    out.extend(&b.data);
                    for &l in &b.literals {
                        pixels.insert(data_at + l);
                        hist[usize::from(b.data[l])] += 1;
                    }
                }
                (ptr, n)
            };
            out[th + 72..th + 76].copy_from_slice(&(ptr as i32).to_le_bytes());
            out[th + 80..th + 84].copy_from_slice(&(count as i32).to_le_bytes());
            placed.push((ptr, count));
        }
        out.extend([0xEE; 13]);
        Dt1Fixture {
            bytes: out,
            pixels,
            hist,
        }
    }
}

#[test]
fn dt1_fixture_parses_as_designed() {
    let fx = Dt1Fixture::new();
    let dt1 = Dt1::parse(&fx.bytes).unwrap();
    assert_eq!(dt1.tiles.len(), 3);
    assert_eq!(dt1.tiles[0].blocks.len(), 3);
    assert_eq!(dt1.tiles[1].blocks, dt1.tiles[0].blocks[..2]);
    assert_eq!(
        pixel_mask(SpriteKind::Dt1, &fx.bytes),
        mask_of(&fx.pixels, fx.bytes.len())
    );
}

#[test]
fn dt1_identity_is_byte_identical() {
    let fx = Dt1Fixture::new();
    let out = remap_indices(SpriteKind::Dt1, &fx.bytes, &identity()).unwrap();
    assert_eq!(out, fx.bytes);
}

#[test]
fn dt1_histogram_counts_literals_and_matches_renders() {
    let fx = Dt1Fixture::new();
    let hist = index_histogram(SpriteKind::Dt1, &fx.bytes).unwrap();
    assert_eq!(hist, fx.hist);
    // the fixture's blocks do not overlap, so every non-zero literal is one
    // opaque rendered pixel
    let dt1 = Dt1::parse(&fx.bytes).unwrap();
    let opaque: u64 = dt1
        .tiles
        .iter()
        .map(|t| {
            t.render()
                .unwrap()
                .alpha
                .iter()
                .filter(|&&a| a != 0)
                .count() as u64
        })
        .sum();
    assert_eq!(hist[1..].iter().sum::<u64>(), opaque);
    assert!(hist[0] > 0, "literal zeros count under 0");
}

#[test]
fn dt1_remap_changes_only_pixel_bytes() {
    let fx = Dt1Fixture::new();
    let before = Dt1::parse(&fx.bytes).unwrap();
    for seed in 0..64u64 {
        let table = if seed == 0 {
            act_table()
        } else {
            random_table(seed)
        };
        let out = remap_indices(SpriteKind::Dt1, &fx.bytes, &table).unwrap();
        check_patch(
            SpriteKind::Dt1,
            &fx.bytes,
            &out,
            &table,
            &mask_of(&fx.pixels, fx.bytes.len()),
        );
        let after = Dt1::parse(&out).unwrap();
        for (a, b) in before.tiles.iter().zip(&after.tiles) {
            assert_eq!(b.render().unwrap(), map_img(&a.render().unwrap(), &table));
            assert_eq!(a.key(), b.key());
            assert_eq!(a.subtile_flags, b.subtile_flags);
        }
    }
}

#[test]
fn dt1_shared_data_is_mapped_once() {
    let fx = Dt1Fixture::new();
    // a table with no fixed points above 0: mapping twice would show
    let table: IndexTable = std::array::from_fn(|i| if i == 0 { 0 } else { (i % 255 + 1) as u8 });
    let out = remap_indices(SpriteKind::Dt1, &fx.bytes, &table).unwrap();
    for &p in &fx.pixels {
        assert_eq!(out[p], table[usize::from(fx.bytes[p])]);
    }
}

#[test]
fn dt1_pixels_overlapping_a_header_are_refused_only_when_they_change() {
    // one isometric block whose data offset 0 points at its own header
    let mut out = vec![0u8; 276 + 96];
    out[0..4].copy_from_slice(&7i32.to_le_bytes());
    out[268..272].copy_from_slice(&1i32.to_le_bytes());
    out[272..276].copy_from_slice(&276i32.to_le_bytes());
    let ptr = out.len();
    out[276 + 72..276 + 76].copy_from_slice(&(ptr as i32).to_le_bytes());
    out[276 + 80..276 + 84].copy_from_slice(&1i32.to_le_bytes());
    out.resize(ptr + 256, 7);
    out[ptr..ptr + 20].fill(0);
    out[ptr + 8..ptr + 10].copy_from_slice(&1i16.to_le_bytes());
    out[ptr + 10..ptr + 12].copy_from_slice(&256u16.to_le_bytes());
    assert_eq!(
        remap_indices(SpriteKind::Dt1, &out, &identity()).unwrap(),
        out
    );
    // changing 7 only touches pure pixel bytes
    let mut t = identity();
    t[7] = 8;
    let patched = remap_indices(SpriteKind::Dt1, &out, &t).unwrap();
    assert_eq!(patched[ptr..ptr + 20], out[ptr..ptr + 20]);
    assert!(patched[ptr + 20..].iter().all(|&b| b == 8));
    // changing 1 would rewrite the block's own format word
    let mut t = identity();
    t[1] = 2;
    assert!(matches!(
        remap_indices(SpriteKind::Dt1, &out, &t),
        Err(FormatError::Invalid {
            what: "pixel data",
            ..
        })
    ));
}

// ---------------------------------------------------------------------------
// DC6 fixture: a raw builder with literal zeros, a flipped frame, two table
// entries sharing one frame record and an unreferenced frame record.

enum Op {
    Lit(Vec<u8>),
    Skip(u8),
    Eol,
}

struct FrameDef {
    flip: bool,
    width: i32,
    height: i32,
    ops: Vec<Op>,
}

/// `(bytes, pixel offsets, literal histogram counted per table entry)`.
fn dc6_fixture() -> (Vec<u8>, BTreeSet<usize>, [u64; 256]) {
    let frames = [
        FrameDef {
            flip: false,
            width: 4,
            height: 2,
            ops: vec![
                Op::Lit(vec![5, 0]),
                Op::Skip(1),
                Op::Lit(vec![230]),
                Op::Eol,
                Op::Lit(vec![0, 7, 254, 225]),
                Op::Eol,
            ],
        },
        FrameDef {
            flip: true,
            width: 3,
            height: 3,
            ops: vec![
                Op::Skip(2),
                Op::Lit(vec![238]),
                Op::Eol,
                Op::Eol,
                Op::Lit(vec![1, 2, 3]),
                Op::Eol,
            ],
        },
        // never referenced: its bytes must stay as they are
        FrameDef {
            flip: false,
            width: 1,
            height: 1,
            ops: vec![Op::Lit(vec![240]), Op::Eol],
        },
    ];
    // directions = 1, frames per direction = 3: entries 0, 1, 1 (shared)
    let entries = [0usize, 1, 1];
    let mut out = Vec::new();
    for v in [6u32, 1, 0, 0xEEEE_EEEE, 1, entries.len() as u32] {
        out.extend(v.to_le_bytes());
    }
    let table_at = out.len();
    out.resize(table_at + 4 * entries.len(), 0);
    let mut records = Vec::new();
    let mut pixels = BTreeSet::new();
    let mut lit_hist: Vec<[u64; 256]> = Vec::new();
    for f in &frames {
        let at = out.len();
        records.push(at);
        let mut data = Vec::new();
        let mut lits = Vec::new();
        for op in &f.ops {
            match op {
                Op::Lit(px) => {
                    data.push(px.len() as u8);
                    lits.extend(data.len()..data.len() + px.len());
                    data.extend(px);
                }
                Op::Skip(n) => data.push(0x80 | n),
                Op::Eol => data.push(0x80),
            }
        }
        for v in [
            i32::from(f.flip),
            f.width,
            f.height,
            -1,
            f.height - 1,
            0x7EE0_E1E2,
            0x7EEE_EEEE,
            data.len() as i32,
        ] {
            out.extend(v.to_le_bytes());
        }
        let data_at = out.len();
        out.extend(&data);
        out.extend([0xEE; 3]);
        let mut h = [0u64; 256];
        for &l in &lits {
            h[usize::from(data[l])] += 1;
        }
        lit_hist.push(h);
        if entries.contains(&(records.len() - 1)) {
            pixels.extend(lits.iter().map(|l| data_at + l));
        }
    }
    let mut hist = [0u64; 256];
    for (i, &e) in entries.iter().enumerate() {
        out[table_at + 4 * i..table_at + 4 * i + 4]
            .copy_from_slice(&(records[e] as u32).to_le_bytes());
        for (sum, n) in hist.iter_mut().zip(&lit_hist[e]) {
            *sum += n;
        }
    }
    out.extend([0xE4; 5]);
    (out, pixels, hist)
}

fn dc6_images(bytes: &[u8]) -> Vec<IndexedImage> {
    Dc6::parse(bytes)
        .unwrap()
        .frames
        .iter()
        .map(|f| f.render().unwrap())
        .collect()
}

#[test]
fn dc6_fixture_parses_as_designed() {
    let (bytes, pixels, _) = dc6_fixture();
    let dc6 = Dc6::parse(&bytes).unwrap();
    assert_eq!(dc6.frames.len(), 3);
    assert!(dc6.frames[1].flipped);
    assert_eq!(dc6.frames[1], dc6.frames[2]);
    assert_eq!(
        pixel_mask(SpriteKind::Dc6, &bytes),
        mask_of(&pixels, bytes.len())
    );
}

#[test]
fn dc6_identity_is_byte_identical() {
    let (bytes, _, _) = dc6_fixture();
    assert_eq!(
        remap_indices(SpriteKind::Dc6, &bytes, &identity()).unwrap(),
        bytes
    );
}

#[test]
fn dc6_histogram_counts_literals_and_matches_renders() {
    let (bytes, _, expected) = dc6_fixture();
    let hist = index_histogram(SpriteKind::Dc6, &bytes).unwrap();
    assert_eq!(hist, expected);
    let opaque: u64 = dc6_images(&bytes)
        .iter()
        .map(|img| img.alpha.iter().filter(|&&a| a != 0).count() as u64)
        .sum();
    assert_eq!(hist[1..].iter().sum::<u64>(), opaque);
    assert_eq!(hist[0], 2);
}

#[test]
fn dc6_remap_changes_only_pixel_bytes() {
    let (bytes, pixels, _) = dc6_fixture();
    let before = dc6_images(&bytes);
    for seed in 0..64u64 {
        let table = if seed == 0 {
            act_table()
        } else {
            random_table(seed)
        };
        let out = remap_indices(SpriteKind::Dc6, &bytes, &table).unwrap();
        check_patch(
            SpriteKind::Dc6,
            &bytes,
            &out,
            &table,
            &mask_of(&pixels, bytes.len()),
        );
        let after = dc6_images(&out);
        let expected: Vec<_> = before.iter().map(|i| map_img(i, &table)).collect();
        assert_eq!(after, expected);
    }
}

/// For a file the DC6 encoder wrote, patching equals re-encoding the mapped
/// frames: the encoder's layout depends only on which pixels are opaque.
#[test]
fn dc6_patch_equals_reencoding_encoder_output() {
    let mut rng = testutil::Rng::new(7);
    let mut frames = Vec::new();
    for d in 0..2 {
        let mut dir = Vec::new();
        for f in 0..3u32 {
            let (w, h) = (3 + f * 5, 2 + f * 3 + d);
            let mut img = IndexedImage::new(w, h, -(f as i32), 4);
            for (p, a) in img.pixels.iter_mut().zip(img.alpha.iter_mut()) {
                if rng.below(4) != 0 {
                    *p = 1 + rng.below(255) as u8;
                    *a = 255;
                }
            }
            dir.push(img);
        }
        frames.push(dir);
    }
    let bytes = Dc6::from_frames(&frames).unwrap().to_bytes().unwrap();
    for seed in 0..16u64 {
        let table = random_table(seed);
        let mapped: Vec<Vec<IndexedImage>> = frames
            .iter()
            .map(|d| d.iter().map(|i| map_img(i, &table)).collect())
            .collect();
        let expected = Dc6::from_frames(&mapped).unwrap().to_bytes().unwrap();
        assert_eq!(
            remap_indices(SpriteKind::Dc6, &bytes, &table).unwrap(),
            expected
        );
    }
    // and the frame helper agrees with what the file holds
    assert_eq!(
        Dc6Frame::from_image(&frames[0][0]),
        Dc6::parse(&bytes).unwrap().frames[0]
    );
}

// ---------------------------------------------------------------------------
// DCC

/// Frames with at most four colours per 4x4 cell, drawn from the act range
/// and the shared range.
fn dcc_frames() -> Vec<Vec<IndexedImage>> {
    let mut frames = Vec::new();
    for d in 0..2i32 {
        let mut dir = Vec::new();
        for f in 0..3i32 {
            let (w, h) = (9 + f as u32 * 3, 7 + d as u32 * 2);
            let mut img = IndexedImage::new(w, h, -4 + f, -10 + d);
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) as usize;
                    // three colours per frame, so no 4x4 cell can hold more
                    // than four (with transparency)
                    let px = match (x + 2 * y + f as u32) % 4 {
                        0 => 0,
                        1 => (225 + (5 * d + f) % 30) as u8,
                        2 => (10 + 7 * d + f) as u8,
                        _ => (100 + 3 * f - d) as u8,
                    };
                    img.pixels[i] = px;
                    img.alpha[i] = if px == 0 { 0 } else { 255 };
                }
            }
            dir.push(img);
        }
        frames.push(dir);
    }
    frames
}

fn dcc_fixture() -> Vec<u8> {
    Dcc::from_frames(&dcc_frames()).unwrap().to_bytes().unwrap()
}

fn dcc_decoded(bytes: &[u8]) -> Vec<Vec<IndexedImage>> {
    Dcc::parse(bytes)
        .unwrap()
        .directions
        .iter()
        .map(|d| d.frames.clone())
        .collect()
}

#[test]
fn dcc_remap_decodes_to_the_mapped_frames() {
    let bytes = dcc_fixture();
    let before = dcc_decoded(&bytes);
    assert_eq!(before, dcc_frames(), "fixture round-trips");
    let mut merge_all = [1u8; 256];
    merge_all[0] = 0;
    let tables = [
        identity(),
        act_table(),
        random_table(1),
        random_table(2),
        merge_all,
    ];
    for table in tables {
        let out = remap_indices(SpriteKind::Dcc, &bytes, &table).unwrap();
        let expected: Vec<Vec<IndexedImage>> = before
            .iter()
            .map(|d| d.iter().map(|i| map_img(i, &table)).collect())
            .collect();
        assert_eq!(dcc_decoded(&out), expected);
        let hist = index_histogram(SpriteKind::Dcc, &bytes).unwrap();
        assert_eq!(
            index_histogram(SpriteKind::Dcc, &out).unwrap(),
            mapped_hist(&hist, &table)
        );
    }
}

#[test]
fn dcc_histogram_counts_opaque_pixels() {
    let bytes = dcc_fixture();
    let hist = index_histogram(SpriteKind::Dcc, &bytes).unwrap();
    let opaque: u64 = dcc_frames()
        .iter()
        .flatten()
        .map(|i| i.alpha.iter().filter(|&&a| a != 0).count() as u64)
        .sum();
    assert_eq!(hist[0], 0);
    assert_eq!(hist.iter().sum::<u64>(), opaque);
    assert!(hist[225..=254].iter().sum::<u64>() > 0);
}

proptest! {
    /// Random valid tables on the fixtures: the patch contract holds.
    #[test]
    fn prop_random_tables_keep_the_contract(seed in any::<u64>()) {
        let table = random_table(seed);
        let fx = Dt1Fixture::new();
        let out = remap_indices(SpriteKind::Dt1, &fx.bytes, &table).unwrap();
        check_patch(SpriteKind::Dt1, &fx.bytes, &out, &table, &mask_of(&fx.pixels, fx.bytes.len()));
        let (bytes, pixels, _) = dc6_fixture();
        let out = remap_indices(SpriteKind::Dc6, &bytes, &table).unwrap();
        check_patch(SpriteKind::Dc6, &bytes, &out, &table, &mask_of(&pixels, bytes.len()));
    }
}
