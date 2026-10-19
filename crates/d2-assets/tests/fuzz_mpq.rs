//! Fuzz-style robustness tests for the MPQ reader.
//!
//! Small synthetic archives (built by the independent writer in `common`)
//! are mutated structurally (header fields, decrypted hash/block entries and
//! sector offset tables set to extreme values, then re-encrypted) and
//! byte-wise (bit flips, overwrites, splices, truncation). Opening the
//! archive and reading every file must never panic, and a successful read
//! must return exactly the block's file size.
//!
//! Knobs: the proptest cases follow `PROPTEST_CASES` (default 256); the
//! deterministic extreme-value sweeps are exhaustive over a fixed value list.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use common::{
    Builder, FLAG_COMPRESS, FLAG_ENCRYPTED, FLAG_FIX_KEY, FLAG_IMPLODE, FLAG_SINGLE_UNIT, FileSpec,
    Pack, decrypt, encrypt, noise, text,
};
use d2_assets::MpqArchive;
use d2_assets::mpq::hash_string;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

/// "AIAIAIAIAIAIA" as a bare PKWARE DCL stream (from the `explode` crate docs).
const IMPLODED: [u8; 8] = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F];

const FLAG_EXISTS: u32 = 0x8000_0000;
const FLAG_PATCH: u32 = 0x0004_0000;
const FLAG_DELETE: u32 = 0x0200_0000;

/// Values every field gets tried with: boundaries, sizes around the header
/// and sector sizes, sign bits, the hash-table sentinels and flag mixes.
const EXTREMES: &[u32] = &[
    0,
    1,
    2,
    3,
    4,
    8,
    0xF,
    0x10,
    0x11,
    0x1F,
    0x20,
    0x2C,
    0x44,
    0xD0,
    0x1FF,
    0x200,
    0x1000,
    0xFFFF,
    0x1_0000,
    0x10_0000,
    0x4000_0000,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFD,
    0xFFFF_FFFE,
    0xFFFF_FFFF,
    FLAG_EXISTS | FLAG_COMPRESS,
    FLAG_EXISTS | FLAG_IMPLODE,
    FLAG_EXISTS | FLAG_SINGLE_UNIT | FLAG_COMPRESS,
    FLAG_EXISTS | FLAG_ENCRYPTED | FLAG_FIX_KEY,
    FLAG_EXISTS | FLAG_ENCRYPTED | FLAG_FIX_KEY | FLAG_COMPRESS | FLAG_SINGLE_UNIT,
    FLAG_EXISTS | FLAG_PATCH,
    FLAG_EXISTS | FLAG_DELETE,
    FLAG_EXISTS | FLAG_IMPLODE | FLAG_COMPRESS,
];

/// Header fields as `(offset, width)` relative to the archive start.
const HEADER_FIELDS: &[(usize, usize)] = &[
    (4, 4),  // header size
    (8, 4),  // archive size
    (12, 2), // format version
    (14, 2), // sector size shift
    (16, 4), // hash table position
    (20, 4), // block table position
    (24, 4), // hash table entries
    (28, 4), // block table entries
    (32, 4), // hi-block table position (low)
    (36, 4), // hi-block table position (high)
    (40, 2), // hash table position high
    (42, 2), // block table position high
];

#[derive(Clone, Copy, Debug)]
struct Block {
    pos: u32,
    file_size: u32,
    flags: u32,
}

/// One seed archive and where its structures are.
struct Seed {
    bytes: Vec<u8>,
    names: Vec<String>,
    archive_offset: usize,
    sector_size: usize,
    hash_pos: usize,
    hash_entries: usize,
    block_pos: usize,
    blocks: Vec<Block>,
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

impl Seed {
    fn new(builder: &Builder) -> Seed {
        let bytes = builder.build();
        let base = builder.archive_offset;
        let hash_pos = base + le32(&bytes, base + 16) as usize;
        let block_pos = base + le32(&bytes, base + 20) as usize;
        let hash_entries = le32(&bytes, base + 24) as usize;
        let block_entries = le32(&bytes, base + 28) as usize;
        let mut raw = bytes[block_pos..block_pos + block_entries * 16].to_vec();
        decrypt(&mut raw, hash_string("(block table)", 3));
        let blocks = raw
            .as_chunks::<16>()
            .0
            .iter()
            .map(|c| Block {
                pos: le32(c, 0),
                file_size: le32(c, 8),
                flags: le32(c, 12),
            })
            .collect();
        let mut names: Vec<String> = builder.files.iter().map(|f| f.name.clone()).collect();
        names.push("zzz-not-there".to_owned());
        Seed {
            bytes,
            names,
            archive_offset: base,
            sector_size: builder.sector_size(),
            hash_pos,
            hash_entries,
            block_pos,
            blocks,
        }
    }

    /// The file key of block `i` (its spec has the same index).
    fn key(&self, i: usize) -> u32 {
        let block = self.blocks[i];
        let name = &self.names[i];
        let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let mut key = hash_string(base, 3);
        if block.flags & FLAG_FIX_KEY != 0 {
            key = key.wrapping_add(block.pos) ^ block.file_size;
        }
        key
    }
}

fn seeds() -> Vec<Seed> {
    let listfile = b"a.txt\r\nb.bin\r\nc.txt\r\nd.bin\r\ne.txt\r\nempty\r\nsub\\f.txt\r\n";
    let v0 = Builder::new()
        .file(FileSpec::new("(listfile)", listfile))
        .file(FileSpec::new("a.txt", &text(9000)).pack(Pack::Zlib))
        .file(
            FileSpec::new("b.bin", &noise(9000, 7))
                .pack(Pack::Bzip2)
                .fix_key(),
        )
        .file(FileSpec::new("c.txt", b"AIAIAIAIAIAIA").pack(Pack::ImplodeFlag(IMPLODED.to_vec())))
        .file(FileSpec::new("d.bin", &noise(5000, 3)).encrypted())
        .file(
            FileSpec::new("e.txt", &text(3000))
                .pack(Pack::Zlib)
                .single_unit()
                .encrypted(),
        )
        .file(FileSpec::new("empty", b""))
        .file(
            FileSpec::new("sub\\f.txt", &text(6000))
                .pack(Pack::Bzip2)
                .encrypted(),
        );

    let mut v1 = Builder::new()
        .file(FileSpec::new("(listfile)", b"a.txt;b.bin"))
        .file(
            FileSpec::new("a.txt", &text(3000))
                .pack(Pack::Zlib)
                .encrypted(),
        )
        .file(FileSpec::new("b.bin", &noise(2000, 5)).pack(Pack::Bzip2));
    v1.version = 1;
    v1.sector_shift = 0;
    v1.archive_offset = 512;

    let mut v3 = Builder::new()
        .file(
            FileSpec::new("x", &text(700))
                .pack(Pack::Zlib)
                .locale(0x409),
        )
        .file(FileSpec::new("y", &noise(300, 1)))
        .file(FileSpec::new("z", &text(1500)).single_unit());
    v3.version = 3;
    v3.deleted_before_each = true;
    v3.hash_entries = Some(8);

    [v0, v1, v3].iter().map(Seed::new).collect()
}

#[derive(Clone, Debug)]
enum Mutation {
    Byte {
        at: usize,
        value: u8,
    },
    Flip {
        at: usize,
        bit: u8,
    },
    Truncate {
        keep: usize,
    },
    Splice {
        at: usize,
        bytes: Vec<u8>,
    },
    Header {
        field: usize,
        value: u32,
    },
    Block {
        entry: usize,
        field: usize,
        value: u32,
    },
    Hash {
        entry: usize,
        field: usize,
        value: u32,
    },
    Sector {
        file: usize,
        index: usize,
        value: u32,
    },
}

/// Rewrite one 4-byte word of an encrypted table in place.
fn patch_encrypted(bytes: &mut [u8], start: usize, len: usize, key: u32, word: usize, value: u32) {
    let Some(table) = bytes.get_mut(start..start + len) else {
        return;
    };
    decrypt(table, key);
    if let Some(slot) = table.get_mut(word * 4..word * 4 + 4) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    encrypt(table, key);
}

fn apply(seed: &Seed, mutations: &[Mutation]) -> Vec<u8> {
    let mut b = seed.bytes.clone();
    for m in mutations {
        match m {
            Mutation::Byte { at, value } => {
                if !b.is_empty() {
                    let at = at % b.len();
                    b[at] = *value;
                }
            }
            Mutation::Flip { at, bit } => {
                if !b.is_empty() {
                    let at = at % b.len();
                    b[at] ^= 1 << (bit % 8);
                }
            }
            Mutation::Truncate { keep } => {
                let keep = keep % (b.len() + 1);
                b.truncate(keep);
            }
            Mutation::Splice { at, bytes } => {
                let at = at % (b.len() + 1);
                b.splice(at..at, bytes.iter().copied());
            }
            Mutation::Header { field, value } => {
                let (offset, width) = HEADER_FIELDS[field % HEADER_FIELDS.len()];
                let at = seed.archive_offset + offset;
                if let Some(slot) = b.get_mut(at..at + width) {
                    slot.copy_from_slice(&value.to_le_bytes()[..width]);
                }
            }
            Mutation::Block {
                entry,
                field,
                value,
            } => {
                let n = seed.blocks.len();
                let word = (entry % n) * 4 + field % 4;
                let key = hash_string("(block table)", 3);
                patch_encrypted(&mut b, seed.block_pos, n * 16, key, word, *value);
            }
            Mutation::Hash {
                entry,
                field,
                value,
            } => {
                let n = seed.hash_entries;
                let word = (entry % n) * 4 + field % 4;
                let key = hash_string("(hash table)", 3);
                patch_encrypted(&mut b, seed.hash_pos, n * 16, key, word, *value);
            }
            Mutation::Sector { file, index, value } => {
                let i = file % seed.blocks.len();
                let block = seed.blocks[i];
                let has_table = block.flags & (FLAG_COMPRESS | FLAG_IMPLODE) != 0
                    && block.flags & FLAG_SINGLE_UNIT == 0
                    && block.file_size > 0;
                if !has_table {
                    continue;
                }
                let entries = (block.file_size as usize).div_ceil(seed.sector_size) + 1;
                let start = seed.archive_offset + block.pos as usize;
                let word = index % entries;
                if block.flags & FLAG_ENCRYPTED != 0 {
                    let key = seed.key(i).wrapping_sub(1);
                    patch_encrypted(&mut b, start, entries * 4, key, word, *value);
                } else if let Some(slot) = b.get_mut(start + word * 4..start + word * 4 + 4) {
                    slot.copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
    b
}

/// How far the cases got, printed so a run shows the fuzz reaches the reads.
#[derive(Debug, Default)]
struct Reach {
    cases: usize,
    opened: usize,
    reads_ok: usize,
    reads_err: usize,
}

/// Open and read everything; panics only on a broken invariant (or a bug).
fn exercise(path: &Path, names: &[String], reach: &mut Reach) {
    reach.cases += 1;
    let Ok(archive) = MpqArchive::open(path) else {
        return;
    };
    reach.opened += 1;
    let _ = archive.header();
    let _ = archive.file_count();
    let mut all = names.to_vec();
    if let Ok(Some(listed)) = archive.listfile() {
        all.extend(listed.into_iter().take(32));
    }
    for name in &all {
        let _ = archive.contains(name);
        let info = archive.block_info(name);
        let _ = archive.compression_mask(name);
        match archive.read(name) {
            Ok(Some(data)) => {
                reach.reads_ok += 1;
                let (size, _) = info.expect("a readable file has a block entry");
                assert_eq!(data.len() as u64, u64::from(size), "{name}: length");
            }
            Ok(None) => {}
            Err(_) => reach.reads_err += 1,
        }
    }
    let _ = archive.coverage(all.iter().map(String::as_str));
}

fn value() -> impl Strategy<Value = u32> {
    prop_oneof![
        3 => prop::sample::select(EXTREMES),
        2 => 0u32..0x1_0000,
        1 => any::<u32>(),
    ]
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        2 => (any::<usize>(), any::<u8>()).prop_map(|(at, value)| Mutation::Byte { at, value }),
        2 => (any::<usize>(), 0u8..8).prop_map(|(at, bit)| Mutation::Flip { at, bit }),
        1 => any::<usize>().prop_map(|keep| Mutation::Truncate { keep }),
        1 => (any::<usize>(), prop::collection::vec(any::<u8>(), 1..48))
            .prop_map(|(at, bytes)| Mutation::Splice { at, bytes }),
        3 => (any::<usize>(), value()).prop_map(|(field, value)| Mutation::Header { field, value }),
        4 => (any::<usize>(), 0usize..4, value())
            .prop_map(|(entry, field, value)| Mutation::Block { entry, field, value }),
        3 => (any::<usize>(), 0usize..4, value())
            .prop_map(|(entry, field, value)| Mutation::Hash { entry, field, value }),
        3 => (any::<usize>(), any::<usize>(), value())
            .prop_map(|(file, index, value)| Mutation::Sector { file, index, value }),
    ]
}

fn runner() -> TestRunner {
    TestRunner::new(Config {
        failure_persistence: None,
        ..Config::default()
    })
}

#[test]
fn seeds_are_valid() {
    let dir = tempfile::tempdir().unwrap();
    for (i, seed) in seeds().iter().enumerate() {
        let path = common::write_temp(dir.path(), &format!("seed{i}.mpq"), &seed.bytes);
        let archive = MpqArchive::open(&path).unwrap();
        for name in &seed.names[..seed.names.len() - 1] {
            assert!(archive.read(name).unwrap().is_some(), "seed {i}: {name}");
        }
    }
}

#[test]
fn mutated_archives_never_panic() {
    let seeds = seeds();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fuzz.mpq");
    let mut runner = runner();
    // The runner takes an `Fn` closure.
    let reach = std::cell::RefCell::new(Reach::default());
    let strategy = (0..seeds.len(), prop::collection::vec(mutation(), 1..6));
    let result = runner.run(&strategy, |(seed, mutations)| {
        let seed = &seeds[seed];
        std::fs::write(&path, apply(seed, &mutations)).unwrap();
        exercise(&path, &seed.names, &mut reach.borrow_mut());
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
    println!("mutated_archives_never_panic: {:?}", reach.borrow());
}

/// Whole block entries replaced by random/extreme words (all four fields at
/// once), plus a few byte-level mutations: the read path sees arbitrary
/// position/size/flag combinations behind names that do resolve.
#[test]
fn random_block_entries_never_panic() {
    let seeds = seeds();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blocks.mpq");
    let mut runner = runner();
    let reach = std::cell::RefCell::new(Reach::default());
    let strategy = (
        0..seeds.len(),
        any::<usize>(),
        prop::collection::vec(value(), 4),
        prop::collection::vec(mutation(), 0..3),
    );
    let result = runner.run(&strategy, |(seed, entry, words, extra)| {
        let seed = &seeds[seed];
        let mut mutations: Vec<Mutation> = words
            .iter()
            .enumerate()
            .map(|(field, &value)| Mutation::Block {
                entry,
                field,
                value,
            })
            .collect();
        mutations.extend(extra);
        std::fs::write(&path, apply(seed, &mutations)).unwrap();
        exercise(&path, &seed.names, &mut reach.borrow_mut());
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
    println!("random_block_entries_never_panic: {:?}", reach.borrow());
}

/// Random bytes, half of them behind a header-shaped prefix (magic plus
/// random header fields) so the table code is reached too.
#[test]
fn random_bytes_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("random.mpq");
    let mut runner = runner();
    // The runner takes an `Fn` closure.
    let reach = std::cell::RefCell::new(Reach::default());
    // Mostly in-file table positions and small entry counts, so the random
    // body is decrypted into hash and block entries and read through.
    let position = || prop_oneof![3 => 0u32..256, 1 => value()];
    let count = || prop_oneof![3 => 0u32..12, 1 => value()];
    let header = (
        prop::sample::select(&[0x20u32, 0x2C, 0x44, 0xD0, 0, 0xFFFF_FFFF][..]),
        value(),
        prop_oneof![3 => 0u16..4, 1 => any::<u16>()],
        prop_oneof![3 => 0u16..4, 1 => any::<u16>()],
        (position(), position(), count(), count()),
    )
        .prop_map(|(size, archive_size, version, shift, (hp, bp, hn, bn))| {
            let mut h = b"MPQ\x1a".to_vec();
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&archive_size.to_le_bytes());
            h.extend_from_slice(&version.to_le_bytes());
            h.extend_from_slice(&shift.to_le_bytes());
            for f in [hp, bp, hn, bn] {
                h.extend_from_slice(&f.to_le_bytes());
            }
            h
        });
    let strategy = (
        prop::option::of(header),
        prop::collection::vec(any::<u8>(), 0..2048),
    );
    let result = runner.run(&strategy, |(header, body)| {
        let mut bytes = header.unwrap_or_default();
        bytes.extend(body);
        std::fs::write(&path, &bytes).unwrap();
        exercise(
            &path,
            &["a".to_owned(), "(listfile)".to_owned()],
            &mut reach.borrow_mut(),
        );
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
    println!("random_bytes_never_panic: {:?}", reach.borrow());
}

/// Every header field, block entry field, hash entry field and sector offset
/// set to every value in [`EXTREMES`], one at a time.
#[test]
fn extreme_field_values_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sweep.mpq");
    let mut reach = Reach::default();
    for seed in seeds() {
        let mut run = |m: Mutation| {
            std::fs::write(&path, apply(&seed, std::slice::from_ref(&m))).unwrap();
            exercise(&path, &seed.names, &mut reach);
        };
        for &value in EXTREMES {
            for field in 0..HEADER_FIELDS.len() {
                run(Mutation::Header { field, value });
            }
            for entry in 0..seed.blocks.len() {
                for field in 0..4 {
                    run(Mutation::Block {
                        entry,
                        field,
                        value,
                    });
                }
            }
            // Hash A, hash B and the block index (the locale word matters less).
            for entry in 0..seed.hash_entries {
                for field in [0, 1, 3] {
                    run(Mutation::Hash {
                        entry,
                        field,
                        value,
                    });
                }
            }
            for file in 0..seed.blocks.len() {
                for index in 0..4 {
                    run(Mutation::Sector { file, index, value });
                }
            }
        }
    }
    println!("extreme_field_values_never_panic: {reach:?}");
}

/// Claims far beyond the archive's size fail fast instead of allocating.
#[test]
fn absurd_claims_in_a_tiny_archive_are_errors() {
    let builder = Builder::new().file(FileSpec::new("f", &text(600)).pack(Pack::Zlib));
    let seed = Seed::new(&builder);
    assert!(seed.bytes.len() < 1024, "{}", seed.bytes.len());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tiny.mpq");
    let started = std::time::Instant::now();
    for (field, value) in [
        (2usize, u32::MAX), // file size 4 GiB
        (2, 0x4000_0000),   // file size 1 GiB
        (1, u32::MAX),      // stored size 4 GiB
        (1, 0x7FFF_FFFF),   // stored size 2 GiB
        (0, u32::MAX - 3),  // position at the end of the address space
    ] {
        let bytes = apply(
            &seed,
            &[Mutation::Block {
                entry: 0,
                field,
                value,
            }],
        );
        std::fs::write(&path, bytes).unwrap();
        let archive = MpqArchive::open(&path).unwrap();
        assert!(archive.read("f").is_err(), "field {field} = {value:#x}");
    }
    // A header that claims 2^32 - 1 hash entries is rejected before any allocation.
    for field in [6usize, 7] {
        let bytes = apply(
            &seed,
            &[Mutation::Header {
                field,
                value: u32::MAX,
            }],
        );
        std::fs::write(&path, bytes).unwrap();
        assert!(MpqArchive::open(&path).is_err(), "header field {field}");
    }
    // A sector table entry pointing 4 GiB away.
    let bytes = apply(
        &seed,
        &[Mutation::Sector {
            file: 0,
            index: 1,
            value: u32::MAX,
        }],
    );
    std::fs::write(&path, bytes).unwrap();
    assert!(MpqArchive::open(&path).unwrap().read("f").is_err());
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}
