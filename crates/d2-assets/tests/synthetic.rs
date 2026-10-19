//! Synthetic MPQs built by the test-only writer in `common`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Builder, FileSpec, Pack, noise, text, write_temp};
use d2_assets::{AssetError, MpqArchive};

/// "AIAIAIAIAIAIA" as a bare PKWARE DCL stream (from the `explode` crate docs).
const IMPLODED: [u8; 8] = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F];

fn open(builder: &Builder) -> (tempfile::TempDir, MpqArchive) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_temp(dir.path(), "t.mpq", &builder.build());
    let archive = MpqArchive::open(&path).expect("open");
    (dir, archive)
}

fn open_bytes(bytes: &[u8]) -> Result<(tempfile::TempDir, MpqArchive), AssetError> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_temp(dir.path(), "t.mpq", bytes);
    MpqArchive::open(&path).map(|a| (dir, a))
}

fn assert_reads(archive: &MpqArchive, spec: &FileSpec) {
    let got = archive
        .read(&spec.name)
        .unwrap_or_else(|e| panic!("{}: {e}", spec.name))
        .unwrap_or_else(|| panic!("{} missing", spec.name));
    assert_eq!(got.len(), spec.data.len(), "{} length", spec.name);
    assert!(got == spec.data, "{} content", spec.name);
}

#[test]
fn v1_every_storage_kind() {
    let specs = vec![
        FileSpec::new("data\\plain.txt", &text(100)),
        FileSpec::new("data\\empty.txt", b""),
        FileSpec::new("data\\zlib.txt", &text(3000)).pack(Pack::Zlib),
        FileSpec::new("data\\bzip2.txt", &text(3000)).pack(Pack::Bzip2),
        FileSpec::new("data\\enc.bin", &noise(1000, 1)).encrypted(),
        FileSpec::new("data\\fixkey.bin", &noise(1001, 2)).fix_key(),
        FileSpec::new("data\\unit.txt", &text(2500)).single_unit(),
        FileSpec::new("data\\unit_zlib.txt", &text(2500))
            .pack(Pack::Zlib)
            .single_unit(),
        FileSpec::new("data\\unit_bz.txt", &text(2500))
            .pack(Pack::Bzip2)
            .single_unit(),
        FileSpec::new("data\\unit_fix.bin", &text(2500))
            .pack(Pack::Zlib)
            .fix_key()
            .single_unit(),
        // 10 000 bytes over 4096-byte sectors: three sectors, last partial.
        FileSpec::new("data\\multi.txt", &text(10_000)),
        FileSpec::new("data\\multi_zlib.txt", &text(10_000)).pack(Pack::Zlib),
        FileSpec::new("data\\multi_bz.txt", &text(10_000)).pack(Pack::Bzip2),
        FileSpec::new("data\\multi_enc.txt", &text(10_000))
            .pack(Pack::Zlib)
            .encrypted(),
        FileSpec::new("data\\multi_fix.txt", &text(10_000))
            .pack(Pack::Bzip2)
            .fix_key(),
        FileSpec::new("data\\multi_plain_enc.bin", &noise(9000, 3)).fix_key(),
        // Incompressible data in a compressed file: sectors are stored raw.
        FileSpec::new("data\\noise_zlib.bin", &noise(9000, 4)).pack(Pack::Zlib),
        FileSpec::new("data\\noise_unit.bin", &noise(3000, 5))
            .pack(Pack::Zlib)
            .single_unit(),
        // Implode (flag and mask forms).
        FileSpec::new("data\\implode_flag.txt", b"AIAIAIAIAIAIA")
            .pack(Pack::ImplodeFlag(IMPLODED.to_vec())),
        FileSpec::new("data\\implode_mask.txt", b"AIAIAIAIAIAIA")
            .pack(Pack::Mask(0x08, IMPLODED.to_vec())),
        FileSpec::new("data\\implode_enc.txt", b"AIAIAIAIAIAIA")
            .pack(Pack::ImplodeFlag(IMPLODED.to_vec()))
            .fix_key(),
        FileSpec::new("data\\implode_unit.txt", b"AIAIAIAIAIAIA")
            .pack(Pack::Mask(0x08, IMPLODED.to_vec()))
            .single_unit(),
    ];
    let mut builder = Builder::new();
    for s in &specs {
        builder = builder.file(s.clone());
    }
    let (_dir, archive) = open(&builder);
    assert_eq!(archive.header().format_version, 0);
    assert_eq!(archive.header().sector_size, 4096);
    assert_eq!(archive.header().archive_offset, 0);
    assert_eq!(archive.file_count(), specs.len());
    for spec in &specs {
        assert_reads(&archive, spec);
        // case-insensitive, either slash
        let alt = spec.name.to_uppercase().replace('\\', "/");
        assert!(archive.contains(&alt), "{alt}");
    }
    assert_eq!(archive.read("data\\absent.txt").unwrap(), None);
    assert!(!archive.contains("data\\absent.txt"));
    assert!(archive.listfile().unwrap().is_none());
}

#[test]
fn compression_masks_are_reported() {
    let builder = Builder::new()
        .file(FileSpec::new("a", &text(5000)).pack(Pack::Zlib))
        .file(FileSpec::new("b", &text(5000)).pack(Pack::Bzip2).fix_key())
        .file(FileSpec::new("c", b"AIAIAIAIAIAIA").pack(Pack::ImplodeFlag(IMPLODED.to_vec())))
        .file(
            FileSpec::new("d", &text(300))
                .single_unit()
                .pack(Pack::Zlib)
                .encrypted(),
        )
        .file(FileSpec::new("e", &text(300)));
    let (_dir, archive) = open(&builder);
    assert_eq!(archive.compression_mask("a").unwrap(), Some(0x02));
    assert_eq!(archive.compression_mask("b").unwrap(), Some(0x10));
    assert_eq!(archive.compression_mask("c").unwrap(), Some(0x08));
    assert_eq!(archive.compression_mask("d").unwrap(), Some(0x02));
    assert_eq!(archive.compression_mask("e").unwrap(), None);
    assert_eq!(archive.compression_mask("nope").unwrap(), None);
}

#[test]
fn small_sectors() {
    let mut builder = Builder::new();
    builder.sector_shift = 0; // 512-byte sectors
    let spec = FileSpec::new("x.bin", &text(5000))
        .pack(Pack::Zlib)
        .encrypted();
    let (_dir, archive) = open(&builder.file(spec.clone()));
    assert_eq!(archive.header().sector_size, 512);
    assert_reads(&archive, &spec);
}

#[test]
fn archive_at_512_byte_offset() {
    for offset in [512usize, 1024, 0x10000, 0x10200] {
        let mut builder = Builder::new();
        builder.archive_offset = offset;
        let spec = FileSpec::new("data\\f.txt", &text(6000))
            .pack(Pack::Zlib)
            .fix_key();
        let (_dir, archive) = open(&builder.file(spec.clone()));
        assert_eq!(archive.header().archive_offset, offset as u64);
        assert_reads(&archive, &spec);
    }
}

#[test]
fn v2_v3_v4_headers_open() {
    for version in 1..=3u16 {
        let mut builder = Builder::new();
        builder.version = version;
        let spec = FileSpec::new("data\\f.txt", &text(6000))
            .pack(Pack::Zlib)
            .encrypted();
        let (_dir, archive) = open(&builder.file(spec.clone()));
        assert_eq!(archive.header().format_version, version);
        assert_reads(&archive, &spec);
    }
}

#[test]
fn v2_hi_block_table_is_honoured() {
    // Build a v2 archive, then add a hi-block table of zeros and point at it.
    let mut builder = Builder::new();
    builder.version = 1;
    let spec = FileSpec::new("f", &text(100));
    let mut bytes = builder.file(spec.clone()).build();
    let hi_pos = bytes.len() as u64;
    bytes.extend_from_slice(&[0u8; 2]); // one block, hi bits 0
    bytes[32..40].copy_from_slice(&hi_pos.to_le_bytes());
    let (_dir, archive) = open_bytes(&bytes).expect("open");
    assert_reads(&archive, &spec);

    // A non-zero high half moves the file far past the end: error, no panic.
    let n = bytes.len();
    bytes[n - 2..].copy_from_slice(&1u16.to_le_bytes());
    let (_dir, archive) = open_bytes(&bytes).expect("open");
    assert!(archive.read("f").is_err());

    // A hi-block table past EOF fails to open.
    bytes[32..40].copy_from_slice(&(n as u64 * 4).to_le_bytes());
    assert!(open_bytes(&bytes).is_err());
}

#[test]
fn listfile_parsing_and_coverage() {
    let builder = Builder::new()
        .file(
            FileSpec::new(
                "(listfile)",
                b"data\\a.txt\r\ndata\\b.txt;;\n\r\ndata\\ghost.txt\r\n",
            )
            .pack(Pack::Zlib),
        )
        .file(FileSpec::new("data\\a.txt", b"a"))
        .file(FileSpec::new("data\\b.txt", b"b"))
        .file(FileSpec::new("data\\hidden.txt", b"h"));
    let (_dir, archive) = open(&builder);
    let list = archive.listfile().unwrap().expect("listfile");
    assert_eq!(list, ["data\\a.txt", "data\\b.txt", "data\\ghost.txt"]);
    assert_eq!(archive.file_count(), 4);
    let cov = archive.coverage(list.iter().map(String::as_str));
    assert_eq!((cov.total, cov.named), (4, 2));
    let cov = archive.coverage([
        "DATA/A.TXT",
        "data\\a.txt",
        "data\\hidden.txt",
        "(listfile)",
    ]);
    assert_eq!((cov.total, cov.named), (4, 3));
}

#[test]
fn locale_zero_is_preferred() {
    let builder = Builder::new()
        .file(FileSpec::new("data\\l.txt", b"german").locale(0x407))
        .file(FileSpec::new("data\\l.txt", b"neutral").locale(0))
        .file(FileSpec::new("data\\l2.txt", b"german2").locale(0x407))
        .file(FileSpec::new("data\\l2.txt", b"french2").locale(0x40C));
    let (_dir, archive) = open(&builder);
    assert_eq!(archive.read("data\\l.txt").unwrap().unwrap(), b"neutral");
    // No neutral entry: the first match wins.
    assert_eq!(archive.read("data\\l2.txt").unwrap().unwrap(), b"german2");
}

#[test]
fn deleted_hash_slots_are_probed_past() {
    let mut builder = Builder::new()
        .file(FileSpec::new("data\\one.txt", b"1"))
        .file(FileSpec::new("data\\two.txt", b"2"))
        .file(FileSpec::new("data\\three.txt", b"3"));
    builder.deleted_before_each = true;
    builder.hash_entries = Some(64);
    let (_dir, archive) = open(&builder);
    for (n, v) in [("one", b"1"), ("two", b"2"), ("three", b"3")] {
        assert_eq!(archive.read(&format!("data\\{n}.txt")).unwrap().unwrap(), v);
    }
    assert_eq!(archive.read("data\\four.txt").unwrap(), None);
}

#[test]
fn crowded_hash_table_with_collisions() {
    // Smallest table that fits: lots of probing, and a miss must terminate.
    let mut builder = Builder::new();
    builder.hash_entries = Some(16);
    let mut specs = Vec::new();
    for i in 0..15 {
        let s = FileSpec::new(&format!("data\\f{i}.txt"), format!("file {i}").as_bytes());
        specs.push(s.clone());
        builder = builder.file(s);
    }
    let (_dir, archive) = open(&builder);
    for s in &specs {
        assert_reads(&archive, s);
    }
    assert_eq!(archive.read("data\\missing.txt").unwrap(), None);

    // Completely full table: a miss must still terminate.
    let mut full = Builder::new();
    full.hash_entries = Some(4);
    for i in 0..4 {
        full = full.file(FileSpec::new(&format!("n{i}"), b"x"));
    }
    let (_dir, archive) = open(&full);
    assert_eq!(archive.read("n3").unwrap().unwrap(), b"x");
    assert_eq!(archive.read("missing").unwrap(), None);
}

#[test]
fn unsupported_codecs_are_errors_not_panics() {
    let builder = Builder::new()
        .file(FileSpec::new("huff", b"abcdefgh").pack(Pack::Mask(0x01, vec![1, 2, 3])))
        .file(FileSpec::new("adpcm", b"abcdefgh").pack(Pack::Mask(0x41, vec![1, 2, 3])))
        .file(FileSpec::new("adpcm2", b"abcdefgh").pack(Pack::Mask(0x81, vec![1, 2, 3])))
        .file(FileSpec::new("junk", b"abcdefgh").pack(Pack::Mask(0x04, vec![1, 2, 3])));
    let (_dir, archive) = open(&builder);
    for name in ["huff", "adpcm", "adpcm2", "junk"] {
        match archive.read(name) {
            Err(AssetError::Decompress { file, method, .. }) => {
                assert_eq!(file, name);
                let _ = method;
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn wrong_decompressed_length_is_an_error() {
    // Claim 13 bytes but the implode stream yields 13; claim 20 instead.
    let builder = Builder::new().file(
        FileSpec::new("liar", b"AIAIAIAIAIAIAxxxxxxx").pack(Pack::ImplodeFlag(IMPLODED.to_vec())),
    );
    let (_dir, archive) = open(&builder);
    assert!(matches!(
        archive.read("liar"),
        Err(AssetError::Decompress { .. })
    ));
}

// ------------------------------------------------------------ corrupt input

fn sample() -> Vec<u8> {
    Builder::new()
        .file(FileSpec::new("(listfile)", b"a.txt\r\n"))
        .file(FileSpec::new("a.txt", &text(5000)).pack(Pack::Zlib))
        .build()
}

#[test]
fn garbage_and_empty_files_are_errors() {
    assert!(open_bytes(b"").is_err());
    assert!(open_bytes(b"MPQ").is_err());
    assert!(open_bytes(&noise(5000, 9)).is_err());
    assert!(open_bytes(&[0u8; 4096]).is_err());
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        MpqArchive::open(&dir.path().join("missing.mpq")),
        Err(AssetError::Io { .. })
    ));
}

#[test]
fn truncated_header_is_an_error() {
    let bytes = sample();
    for cut in [4usize, 8, 16, 31] {
        assert!(open_bytes(&bytes[..cut]).is_err(), "cut {cut}");
    }
    // Header claims 0x44 bytes but the file ends sooner.
    let mut v3 = Builder::new();
    v3.version = 2;
    let b = v3.file(FileSpec::new("x", b"x")).build();
    assert!(open_bytes(&b[..0x30]).is_err());
}

#[test]
fn bad_header_fields_are_errors() {
    let good = sample();
    let patch = |at: usize, v: u32| {
        let mut b = good.clone();
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        b
    };
    assert!(open_bytes(&patch(4, 8)).is_err(), "header size too small");
    assert!(
        open_bytes(&patch(4, 0x1000)).is_err(),
        "header size too big"
    );
    let mut b = good.clone();
    b[14..16].copy_from_slice(&40u16.to_le_bytes());
    assert!(open_bytes(&b).is_err(), "sector shift");
}

#[test]
fn tables_past_eof_are_errors() {
    let good = sample();
    let patch = |at: usize, v: u32| {
        let mut b = good.clone();
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        b
    };
    assert!(
        open_bytes(&patch(16, 0x7FFF_0000)).is_err(),
        "hash table position"
    );
    assert!(
        open_bytes(&patch(20, 0x7FFF_0000)).is_err(),
        "block table position"
    );
    assert!(
        open_bytes(&patch(24, 0x7FFF_0000)).is_err(),
        "hash table count"
    );
    assert!(
        open_bytes(&patch(28, 0xFFFF_FFFF)).is_err(),
        "block table count"
    );
    // Truncated file: tables are cut off.
    assert!(open_bytes(&good[..good.len() - 20]).is_err());
    assert!(open_bytes(&good[..good.len() / 2]).is_err());
}

#[test]
fn absurd_block_sizes_error_without_huge_allocations() {
    // Rebuild with a hand-written block entry.
    let builder = Builder::new().file(FileSpec::new("f", &text(5000)).pack(Pack::Zlib));
    let good = builder.build();
    let block_pos = u32::from_le_bytes(good[20..24].try_into().unwrap()) as usize;
    let key = d2_assets::mpq::hash_string("(block table)", 3);
    let patch = |csize: u32, fsize: u32, pos: Option<u32>| {
        let mut b = good.clone();
        // One block: the whole block table is exactly this 16-byte entry.
        let mut raw = Vec::new();
        let header_size = builder.header_size() as u32;
        raw.extend_from_slice(&pos.unwrap_or(header_size).to_le_bytes());
        raw.extend_from_slice(&csize.to_le_bytes());
        raw.extend_from_slice(&fsize.to_le_bytes());
        raw.extend_from_slice(&(common::FLAG_EXISTS | common::FLAG_COMPRESS).to_le_bytes());
        common::encrypt(&mut raw, key);
        b[block_pos..block_pos + 16].copy_from_slice(&raw);
        b
    };
    // fsize 4 GiB-1 with a tiny stored size.
    let (_d, a) = open_bytes(&patch(20, u32::MAX, None)).expect("open");
    assert!(a.read("f").is_err());
    // fsize beyond the sanity cap even with a big csize.
    let (_d, a) = open_bytes(&patch(4000, 0xC000_0000, None)).expect("open");
    assert!(a.read("f").is_err());
    // Stored size runs past EOF.
    let (_d, a) = open_bytes(&patch(u32::MAX, 5000, None)).expect("open");
    assert!(a.read("f").is_err());
    // Position past EOF.
    let (_d, a) = open_bytes(&patch(100, 5000, Some(u32::MAX - 10))).expect("open");
    assert!(a.read("f").is_err());
    // Sanity: the unpatched archive reads.
    let (_d, a) = open_bytes(&good).expect("open");
    assert_eq!(a.read("f").unwrap().unwrap(), text(5000));
}

#[test]
fn mangled_archives_never_panic() {
    let good = Builder::new()
        .file(FileSpec::new("(listfile)", b"a.txt\r\nb.bin\r\n"))
        .file(FileSpec::new("a.txt", &text(9000)).pack(Pack::Zlib))
        .file(
            FileSpec::new("b.bin", &noise(9000, 7))
                .pack(Pack::Bzip2)
                .fix_key(),
        )
        .file(FileSpec::new("c.txt", b"AIAIAIAIAIAIA").pack(Pack::ImplodeFlag(IMPLODED.to_vec())))
        .build();
    let dir = tempfile::tempdir().unwrap();
    let mut seed = 12345u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as usize
    };
    for round in 0..400 {
        let mut b = good.clone();
        for _ in 0..1 + round % 6 {
            let at = next() % b.len();
            b[at] = next() as u8;
        }
        if round % 5 == 0 {
            b.truncate(next() % b.len());
        }
        let path = write_temp(dir.path(), "m.mpq", &b);
        if let Ok(archive) = MpqArchive::open(&path) {
            let _ = archive.listfile();
            for name in ["a.txt", "b.bin", "c.txt", "(listfile)", "zzz"] {
                let _ = archive.read(name);
            }
            let _ = archive.coverage(["a.txt", "b.bin"]);
        }
    }
}
