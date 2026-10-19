//! Property and mutation tests: arbitrary bytes, and bit-flipped or
//! truncated valid files, must never panic a parser; structurally valid
//! inputs must decode.

use crate::excel::{self, ExcelTableKind, ExcelVersion};
use crate::tbl::StringTable;
use crate::txt::TxtTable;
use proptest::prelude::*;

proptest! {
    #[test]
    fn txt_random_bytes_never_panic_and_rows_cover_headers(
        bytes in prop::collection::vec(any::<u8>(), 0..200)
    ) {
        let t = TxtTable::parse(&bytes).unwrap();
        for row in t.rows() {
            prop_assert!(row.cells().len() >= t.headers().len());
        }
    }

    #[test]
    fn tbl_random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = StringTable::parse(&bytes);
    }

    #[test]
    fn tbl_plausible_header_with_random_body_never_panics(
        h in 0u16..4,
        n in 0u32..6,
        body in prop::collection::vec(any::<u8>(), 0..200),
    ) {
        let mut bytes = vec![0u8; 0x15];
        bytes[2..4].copy_from_slice(&h.to_le_bytes());
        bytes[4..8].copy_from_slice(&n.to_le_bytes());
        bytes.extend_from_slice(&body);
        let _ = StringTable::parse(&bytes);
    }

    #[test]
    fn excel_random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        for kind in ExcelTableKind::all() {
            let _ = excel::decode_bin(*kind, ExcelVersion::V113c, &bytes);
            let _ = excel::detect_version(*kind, &bytes);
        }
        let _ = excel::decode_levels_bin(ExcelVersion::V114d, &bytes, &bytes);
    }

    #[test]
    fn excel_valid_count_with_random_records_decodes(
        records in 0usize..4,
        fill in prop::collection::vec(any::<u8>(), 1..64),
    ) {
        for kind in ExcelTableKind::all() {
            let size = excel::record_size(*kind, ExcelVersion::V113c);
            let mut bytes = (records as u32).to_le_bytes().to_vec();
            for i in 0..records * size {
                bytes.push(fill[i % fill.len()]);
            }
            let t = excel::decode_bin(*kind, ExcelVersion::V113c, &bytes).unwrap();
            prop_assert_eq!(t.len(), records);
            prop_assert_eq!(
                excel::detect_version(*kind, &bytes),
                Some(ExcelVersion::V113c)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mutation fuzzing (wave 2). Every test asserts only "no panic, no abort":
// each parser must return `Ok` or `Err` for whatever it is fed. Proptest
// tests honour `PROPTEST_CASES`; the corpus loops honour `CAPY_FUZZ_ITERS`.

use crate::animdata::AnimData;
use crate::cof::Cof;
use crate::palette::{Palette, Pl2};
use crate::tables;
use crate::testutil::{self, Rng};

/// A structurally valid COF: `layers` layers, `frames` frames, `dirs`
/// directions, with a permutation priority table.
fn synth_cof(layers: u8, frames: u8, dirs: u8) -> Vec<u8> {
    let mut out = vec![0u8; 28];
    out[..4].copy_from_slice(&[layers, frames, dirs, 20]);
    for l in 0..layers {
        out.extend_from_slice(&[l % 16, 1, 1, 0, 0]);
        out.extend_from_slice(b"HTH\0");
    }
    out.extend((0..frames).map(|f| f % 2));
    for _ in 0..usize::from(dirs) * usize::from(frames) {
        out.extend((0..layers).map(|l| l % 16));
    }
    out
}

/// A structurally valid AnimData: `blocks` blocks of 0..3 records.
fn synth_animdata(blocks: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for b in 0..blocks {
        let n = b % 3;
        out.extend_from_slice(&(n as i32).to_le_bytes());
        for r in 0..n {
            let mut rec = vec![0u8; 160];
            let name = format!("A{b}R{r}HTH");
            let len = name.len().min(8);
            rec[..len].copy_from_slice(&name.as_bytes()[..len]);
            rec[8..12].copy_from_slice(&16u32.to_le_bytes());
            rec[12..16].copy_from_slice(&256u32.to_le_bytes());
            rec[16 + r] = 1;
            out.extend_from_slice(&rec);
        }
    }
    out
}

/// A valid `.tbl` built with linear probing over `slack` spare slots.
fn synth_tbl(entries: &[(&[u8], &[u8])], slack: u32) -> Vec<u8> {
    let n = entries.len() as u32 + slack;
    let mut nodes: Vec<Option<usize>> = vec![None; n as usize];
    for (i, (key, _)) in entries.iter().enumerate() {
        let mut slot = crate::tbl::key_hash(key, n);
        while nodes[slot as usize].is_some() {
            slot = (slot + 1) % n;
        }
        nodes[slot as usize] = Some(i);
    }
    let h = entries.len();
    let data_start = 0x15 + h * 2 + n as usize * 17;
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for (key, text) in entries {
        let k = data_start + data.len();
        data.extend_from_slice(key);
        data.push(0);
        let t = data_start + data.len();
        data.extend_from_slice(text);
        data.push(0);
        offsets.push((k as u32, t as u32, text.len() as u16 + 1));
    }
    let mut out = vec![0u8; 0x15];
    out[2..4].copy_from_slice(&(h as u16).to_le_bytes());
    out[4..8].copy_from_slice(&n.to_le_bytes());
    out[8] = 1;
    out[9..13].copy_from_slice(&(data_start as u32).to_le_bytes());
    out[17..21].copy_from_slice(&((data_start + data.len()) as u32).to_le_bytes());
    for i in 0..h {
        out.extend_from_slice(&(i as u16).to_le_bytes());
    }
    for node in &nodes {
        match node {
            Some(i) => {
                let (k, t, tl) = offsets[*i];
                out.push(1);
                out.extend_from_slice(&(*i as u16).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&k.to_le_bytes());
                out.extend_from_slice(&t.to_le_bytes());
                out.extend_from_slice(&tl.to_le_bytes());
            }
            None => out.extend_from_slice(&[0u8; 17]),
        }
    }
    out.extend_from_slice(&data);
    out
}

fn sample_tbl() -> Vec<u8> {
    synth_tbl(
        &[
            (b"qstsa5q1", b"Siege on Harrogath"),
            (b"9bl", b"Stiletto"),
            (b"x", b"K"),
            (b"x", b"M"),
            (b"color", b"\xFFc5Magic"),
            (b"empty", b""),
        ],
        2,
    )
}

/// A `.bin` with `count` records of `size` bytes filled from `seed`.
fn synth_bin_size(size: usize, count: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut out = (count as u32).to_le_bytes().to_vec();
    out.extend((0..count * size).map(|_| match rng.below(4) {
        0 => 0,
        1 => b'a' + rng.below(26) as u8,
        _ => rng.next_u64() as u8,
    }));
    out
}

/// A valid `.bin` of `kind` with `count` records filled from `seed`.
fn synth_bin(kind: ExcelTableKind, count: usize, seed: u64) -> Vec<u8> {
    synth_bin_size(excel::record_size(kind, ExcelVersion::V113c), count, seed)
}

const BOTH: [ExcelVersion; 2] = [ExcelVersion::V113c, ExcelVersion::V114d];

/// Runs every consumer of a parsed table; none may panic.
fn exercise_table(t: &TxtTable) {
    let types = tables::lvl_types(t);
    let prests = tables::lvl_prests(t);
    let _ = tables::levels(t);
    if let (Ok(types), Ok(prests)) = (types, prests) {
        for p in prests.iter().take(8) {
            for ty in types.iter().take(8) {
                let _ = p.dt1_files(ty).count();
            }
        }
    }
    for row in t.rows().take(64) {
        for h in t.headers().iter().take(64) {
            let _ = row.get_i64(h);
        }
    }
}

/// Every excel entry point on `bytes`, every kind and version.
fn exercise_excel(bytes: &[u8]) {
    for kind in ExcelTableKind::all() {
        let _ = excel::detect_version(*kind, bytes);
        for v in BOTH {
            if let Ok(t) = excel::decode_bin(*kind, v, bytes) {
                exercise_table(&t);
            }
        }
    }
    for v in BOTH {
        let _ = excel::decode_levels_bin(v, bytes, bytes);
    }
}

fn exercise_cof(bytes: &[u8]) {
    if let Ok(cof) = Cof::parse(bytes) {
        for d in 0..=cof.directions.saturating_add(1) {
            for f in 0..=cof.frames_per_direction.saturating_add(1) {
                let _ = cof.draw_order(d, f);
            }
        }
        for l in &cof.layers {
            let _ = (l.component_code(), l.weapon_class_token());
            let _ = cof.component_layer(l.component);
        }
    }
}

fn exercise_animdata(bytes: &[u8]) {
    if let Ok(a) = AnimData::parse(bytes) {
        for r in a.records().iter().take(32) {
            let _ = a.get(&r.cof_name);
            let _ = a.get(&format!("{}.cof", r.cof_name));
        }
        let _ = a.get("");
        let _ = a.get("\u{e9}\u{e9}");
        let _ = a.get("x\u{e9}.cof");
    }
}

fn exercise_tbl(bytes: &[u8]) {
    if let Ok(t) = StringTable::parse(bytes) {
        for (i, (k, _)) in t.entries().iter().enumerate().take(64) {
            let _ = t.get(k);
            let _ = t.entry_by_id(i as u16);
        }
        let _ = t.entry_ids().len();
    }
}

fn exercise_txt(bytes: &[u8]) {
    if let Ok(t) = TxtTable::parse(bytes) {
        for row in t.rows() {
            assert!(row.cells().len() >= t.headers().len());
        }
        exercise_table(&t);
    }
}

fn exercise_palette(bytes: &[u8]) {
    if let Ok(p) = Palette::parse_dat(bytes) {
        let _ = p.with_gamma(1.3);
    }
    let _ = Pl2::parse(bytes);
}

/// Mutates every corpus seed `CAPY_FUZZ_ITERS` times (default `iters`)
/// through `run`. Skips when the corpus is absent.
fn fuzz_corpus(
    label: &str,
    ext: &str,
    filter: &str,
    max_seeds: usize,
    iters: usize,
    run: fn(&[u8]),
) {
    let seeds = testutil::corpus_seeds(ext, filter, max_seeds);
    if seeds.is_empty() {
        println!("skipped {label}: no private corpus seeds");
        return;
    }
    let iters = testutil::fuzz_iters(label, iters);
    for (n, path) in seeds.iter().enumerate() {
        let Ok(base) = std::fs::read(path) else {
            continue;
        };
        run(&base);
        let mut rng = Rng::new(0x00C0_FFEE ^ n as u64);
        for _ in 0..iters {
            run(&testutil::mutate(&base, &mut rng));
        }
    }
    println!("{label}: {} seeds fuzzed", seeds.len());
}

/// Cells that stress the typed table readers.
const WEIRD_CELLS: [&str; 16] = [
    "",
    "0",
    "1",
    "-1",
    " 12 ",
    "abc",
    "99999999999999999999",
    "4294967295",
    "4294967296",
    "-2147483648",
    "-2147483649",
    "9223372036854775807",
    "-9223372036854775808",
    "Expansion",
    "\u{e9}\u{ff}",
    "0x10",
];

/// Headers the typed table readers look for, plus noise.
const TABLE_HEADERS: [&str; 20] = [
    "Id",
    "Name",
    "Act",
    "File 1",
    "File2",
    "File 32",
    "Expansion",
    "Def",
    "LevelId",
    "Dt1Mask",
    "SizeX",
    "SizeY",
    "Pops",
    "PopPad",
    "LevelType",
    "DrlgType",
    "LevelName",
    "Pal",
    "Populate",
    "id",
];

const TXT_SAMPLE: &[u8] =
    b"Name\tId\tAct\tFile 1\tFile2\tExpansion\tDef\tLevelId\tDt1Mask\tLevelType\tDrlgType\r\n\
Act 1 - Town\t1\t1\tA1\\Town.dt1\t0\t\t1\t1\t3\t1\t2\r\n\
Expansion\r\n\
\t\t\t\t\t\t\t\t\t\t\r\n\
Siege\t32\t5\tA5\\x.dt1\tA5\\y.dt1\t1\t2\t3\t4294967295\t2\t1\r\n";

proptest! {
    #[test]
    fn cof_mutated_valid_never_panics(
        layers in 0u8..12, frames in 0u8..12, dirs in 0u8..12, seed in any::<u64>(),
    ) {
        let base = synth_cof(layers, frames, dirs);
        prop_assert!(Cof::parse(&base).is_ok());
        exercise_cof(&base);
        let mut rng = Rng::new(seed);
        exercise_cof(&testutil::mutate(&base, &mut rng));
    }

    #[test]
    fn cof_any_header_with_long_tail_never_panics(
        header in prop::array::uniform3(any::<u8>()),
        tail in prop::collection::vec(any::<u8>(), 0..6000),
    ) {
        let mut bytes = vec![0u8; 28];
        bytes[..3].copy_from_slice(&header);
        bytes.extend_from_slice(&tail);
        exercise_cof(&bytes);
    }

    #[test]
    fn animdata_mutated_valid_never_panics(blocks in 0usize..10, seed in any::<u64>()) {
        let base = synth_animdata(blocks);
        if blocks > 0 {
            prop_assert!(AnimData::parse(&base).is_ok());
        }
        let mut rng = Rng::new(seed);
        exercise_animdata(&testutil::mutate(&base, &mut rng));
    }

    #[test]
    fn animdata_plausible_counts_never_panic(
        count in prop_oneof![0i32..4, any::<i32>()],
        body in prop::collection::vec(any::<u8>(), 0..1200),
    ) {
        let mut bytes = count.to_le_bytes().to_vec();
        bytes.extend_from_slice(&body);
        exercise_animdata(&bytes);
    }

    #[test]
    fn palette_dat_random_and_truncated_never_panic(
        bytes in prop::collection::vec(any::<u8>(), 0..1600),
        gamma in any::<f32>(),
    ) {
        if let Ok(p) = Palette::parse_dat(&bytes) {
            let _ = p.with_gamma(gamma);
        }
        let _ = Pl2::parse(&bytes);
    }

    #[test]
    fn pl2_truncated_and_mutated_never_panics(cut in 0usize..=0x6C327 + 64, seed in any::<u64>()) {
        let mut base: Vec<u8> = (0..0x6C327usize).map(|i| (i % 251) as u8).collect();
        base.resize(cut, 7);
        let _ = Pl2::parse(&base);
        let mut rng = Rng::new(seed);
        let _ = Pl2::parse(&testutil::mutate(&base, &mut rng));
    }

    #[test]
    fn txt_random_bytes_long_never_panic(
        bytes in prop::collection::vec(
            prop_oneof![Just(b'\t'), Just(b'\n'), Just(b'\r'), any::<u8>()], 0..3000),
    ) {
        exercise_txt(&bytes);
    }

    #[test]
    fn txt_mutated_valid_never_panics(seed in any::<u64>()) {
        exercise_txt(TXT_SAMPLE);
        let mut rng = Rng::new(seed);
        exercise_txt(&testutil::mutate(TXT_SAMPLE, &mut rng));
    }

    #[test]
    fn typed_tables_on_weird_cells_never_panic(
        headers in prop::collection::vec(prop::sample::select(TABLE_HEADERS.to_vec()), 0..12),
        cells in prop::collection::vec(
            prop::collection::vec(prop::sample::select(WEIRD_CELLS.to_vec()), 0..14), 0..8),
    ) {
        let headers: Vec<String> = headers.into_iter().map(str::to_string).collect();
        let rows: Vec<Vec<String>> = cells
            .into_iter()
            .map(|r| r.into_iter().map(str::to_string).collect())
            .collect();
        exercise_table(&TxtTable::from_parts(headers, rows));
    }

    #[test]
    fn tbl_mutated_valid_never_panics(seed in any::<u64>()) {
        let base = sample_tbl();
        prop_assert!(StringTable::parse(&base).is_ok());
        let mut rng = Rng::new(seed);
        exercise_tbl(&testutil::mutate(&base, &mut rng));
    }

    #[test]
    fn tbl_offsets_pointing_anywhere_never_panic(
        h in 0u16..8,
        n in prop_oneof![0u32..40, any::<u32>()],
        nodes in prop::collection::vec(
            (0u8..3, any::<u16>(), prop_oneof![0u32..600, any::<u32>()],
             prop_oneof![0u32..600, any::<u32>()]), 0..24),
        data in prop::collection::vec(prop_oneof![Just(0u8), any::<u8>()], 0..300),
    ) {
        let mut bytes = vec![0u8; 0x15];
        bytes[2..4].copy_from_slice(&h.to_le_bytes());
        bytes[4..8].copy_from_slice(&n.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0u8, usize::from(h) * 2));
        for (active, id, key, text) in &nodes {
            bytes.push(*active);
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&key.to_le_bytes());
            bytes.extend_from_slice(&text.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }
        bytes.extend_from_slice(&data);
        exercise_tbl(&bytes);
        // the same nodes with the node count that matches them
        bytes[4..8].copy_from_slice(&(nodes.len() as u32).to_le_bytes());
        exercise_tbl(&bytes);
    }

    #[test]
    fn excel_random_bytes_longer_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2400)) {
        exercise_excel(&bytes);
    }

    #[test]
    fn excel_counts_against_body_never_panic(
        count in prop_oneof![0u32..6, any::<u32>()],
        kind in prop::sample::select(ExcelTableKind::all().to_vec()),
        records in 0usize..4,
        extra in -8isize..8,
        seed in any::<u64>(),
    ) {
        // A declared count against a body of whole records plus or minus a
        // few bytes: truncated records, a huge count, trailing junk.
        let size = excel::record_size(kind, ExcelVersion::V113c);
        let mut bytes = synth_bin(kind, records, seed);
        bytes[..4].copy_from_slice(&count.to_le_bytes());
        let len = (bytes.len() as isize + extra).max(0) as usize;
        bytes.resize(len, 0xAB);
        let fits = bytes.len() >= 4 && u64::from(count) * size as u64 + 4 == bytes.len() as u64;
        for v in BOTH {
            prop_assert_eq!(excel::decode_bin(kind, v, &bytes).is_ok(), fits);
        }
        exercise_excel(&bytes);
    }

    #[test]
    fn excel_mutated_valid_never_panics(
        kind in prop::sample::select(ExcelTableKind::all().to_vec()),
        records in 0usize..4,
        seed in any::<u64>(),
    ) {
        let base = synth_bin(kind, records, seed);
        let t = excel::decode_bin(kind, ExcelVersion::V114d, &base).unwrap();
        exercise_table(&t);
        let mut rng = Rng::new(seed);
        exercise_excel(&testutil::mutate(&base, &mut rng));
        // levels + leveldefs, mutated independently
        let defs = synth_bin_size(excel::leveldefs_record_size(ExcelVersion::V113c), records, seed);
        let levels = synth_bin(ExcelTableKind::Levels, records, seed ^ 1);
        let t = excel::decode_levels_bin(ExcelVersion::V113c, &levels, &defs).unwrap();
        exercise_table(&t);
        for v in BOTH {
            let _ = excel::decode_levels_bin(v, &levels, &testutil::mutate(&defs, &mut rng));
            let _ = excel::decode_levels_bin(v, &testutil::mutate(&levels, &mut rng), &defs);
        }
    }
}

#[test]
fn truncations_of_valid_files_never_panic() {
    let cof = synth_cof(4, 3, 8);
    for cut in 0..=cof.len() {
        exercise_cof(&cof[..cut]);
    }
    let anim = synth_animdata(6);
    for cut in 0..=anim.len() {
        exercise_animdata(&anim[..cut]);
    }
    let tbl = sample_tbl();
    for cut in 0..=tbl.len() {
        exercise_tbl(&tbl[..cut]);
    }
    for cut in 0..=TXT_SAMPLE.len() {
        exercise_txt(&TXT_SAMPLE[..cut]);
    }
    for kind in ExcelTableKind::all() {
        let bin = synth_bin(*kind, 2, 7);
        for cut in 0..=bin.len() {
            for v in BOTH {
                let _ = excel::decode_bin(*kind, v, &bin[..cut]);
            }
        }
    }
    let dat: Vec<u8> = (0..800u32).map(|i| i as u8).collect();
    for cut in 0..=dat.len() {
        exercise_palette(&dat[..cut]);
    }
}

#[test]
fn tbl_shared_string_bomb_is_rejected() {
    // 20,000 active nodes all pointing at one 64 KiB string: decoding them
    // would copy 2.6 GB; the string budget refuses instead.
    let nodes = 20_000u32;
    let data_start = 0x15 + nodes as usize * 17;
    let mut bytes = vec![0u8; 0x15];
    bytes[4..8].copy_from_slice(&nodes.to_le_bytes());
    for _ in 0..nodes {
        bytes.push(1);
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&(data_start as u32).to_le_bytes());
        bytes.extend_from_slice(&(data_start as u32).to_le_bytes());
        bytes.extend_from_slice(&[0; 2]);
    }
    bytes.extend(std::iter::repeat_n(b'k', 65_536));
    bytes.push(0);
    let err = StringTable::parse(&bytes).unwrap_err();
    assert!(
        matches!(
            err,
            crate::FormatError::Invalid {
                what: "string data",
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn txt_padding_bomb_is_rejected() {
    // 300k header columns over 300k blank interior rows would pad to 9e10
    // cells; the budget refuses instead.
    let mut bytes = vec![b'\t'; 300_000];
    bytes.extend(std::iter::repeat_n(b'\n', 300_000));
    bytes.push(b'x');
    let err = TxtTable::parse(&bytes).unwrap_err();
    assert!(
        matches!(
            err,
            crate::FormatError::Invalid {
                what: "row widths",
                ..
            }
        ),
        "{err}"
    );
    // a wide header over a few short rows is still padded
    let mut ok = vec![b'\t'; 300_000];
    ok.extend_from_slice(b"\nx\ny\n");
    let t = TxtTable::parse(&ok).unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(t.row(1).unwrap().cells().len(), 300_001);
}

#[test]
fn corpus_cof_mutations_never_panic() {
    fuzz_corpus("cof corpus", "cof", "", 40, 50, exercise_cof);
}

#[test]
fn corpus_animdata_mutations_never_panic() {
    fuzz_corpus(
        "animdata corpus",
        "d2",
        "animdata",
        2,
        40,
        exercise_animdata,
    );
}

#[test]
fn corpus_palette_mutations_never_panic() {
    fuzz_corpus("pal.dat corpus", "dat", "palette", 6, 30, exercise_palette);
    fuzz_corpus("pal.pl2 corpus", "pl2", "palette", 2, 10, |b| {
        let _ = Pl2::parse(b);
    });
}

#[test]
fn corpus_txt_mutations_never_panic() {
    fuzz_corpus("txt corpus", "txt", "excel", 12, 20, exercise_txt);
}

#[test]
fn corpus_tbl_mutations_never_panic() {
    fuzz_corpus("tbl corpus", "tbl", "lng", 4, 10, exercise_tbl);
}

#[test]
fn corpus_excel_bin_mutations_never_panic() {
    fuzz_corpus("excel bin corpus", "bin", "excel", 16, 10, exercise_excel);
}
