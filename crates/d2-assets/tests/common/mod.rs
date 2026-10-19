//! Shared test support: install-dir lookup and a minimal MPQ *writer*.
//!
//! The writer exists only to build synthetic archives in memory or in temp
//! dirs. It is independent of the crate's reader on purpose (own crypt
//! table, own layout code), so the two cross-check each other.

#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;

use d2_assets::mpq::hash_string;

pub const DEFAULT_VANILLA: &str = "C:\\Program Files (x86)\\Diablo II";
pub const DEFAULT_MXL: &str = "C:\\games\\median-xl";

pub fn vanilla_dir() -> PathBuf {
    std::env::var_os("CAPY_VANILLA_DIR")
        .map_or_else(|| PathBuf::from(DEFAULT_VANILLA), PathBuf::from)
}

pub fn mxl_dir() -> PathBuf {
    std::env::var_os("CAPY_MXL_DIR").map_or_else(|| PathBuf::from(DEFAULT_MXL), PathBuf::from)
}

/// Print the standard skip line and return false when `dir` is absent.
pub fn dir_available(dir: &std::path::Path) -> bool {
    if dir.is_dir() {
        true
    } else {
        println!("skipped: {} missing", dir.display());
        false
    }
}

/// Every `*.mpq` directly inside `dir`, sorted.
pub fn mpqs_in(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mpq"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// `testdata\private` of the main checkout (gitignored, so absent in
/// worktrees); `CAPY_PRIVATE_DIR` overrides it.
pub fn private_dir() -> PathBuf {
    std::env::var_os("CAPY_PRIVATE_DIR").map_or_else(
        || PathBuf::from(r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor\testdata\private"),
        PathBuf::from,
    )
}

/// Iteration count for hand-rolled fuzz loops: `CAPY_FUZZ_ITERS`, else
/// `PROPTEST_CASES`, else `default`.
pub fn fuzz_iters(default: usize) -> usize {
    ["CAPY_FUZZ_ITERS", "PROPTEST_CASES"]
        .iter()
        .find_map(|var| std::env::var(var).ok()?.trim().parse().ok())
        .unwrap_or(default)
}

// ---------------------------------------------------------------- writer

fn crypt_table() -> Vec<u32> {
    let mut table = vec![0u32; 0x500];
    let mut seed: u32 = 0x0010_0001;
    for i in 0..0x100usize {
        let mut j = i;
        for _ in 0..5 {
            seed = (seed * 125 + 3) % 0x2AAAAB;
            let hi = (seed & 0xFFFF) << 16;
            seed = (seed * 125 + 3) % 0x2AAAAB;
            let lo = seed & 0xFFFF;
            table[j] = hi | lo;
            j += 0x100;
        }
    }
    table
}

pub fn encrypt(data: &mut [u8], key: u32) {
    let table = crypt_table();
    let mut key = key;
    let mut seed2: u32 = 0xEEEE_EEEE;
    for chunk in data.as_chunks_mut::<4>().0 {
        seed2 = seed2.wrapping_add(table[0x400 + (key & 0xFF) as usize]);
        let plain = u32::from_le_bytes(*chunk);
        *chunk = (plain ^ key.wrapping_add(seed2)).to_le_bytes();
        key = ((!key) << 0x15).wrapping_add(0x1111_1111) | (key >> 0x0B);
        seed2 = plain
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
}

/// Inverse of [`encrypt`].
pub fn decrypt(data: &mut [u8], key: u32) {
    let table = crypt_table();
    let mut key = key;
    let mut seed2: u32 = 0xEEEE_EEEE;
    for chunk in data.as_chunks_mut::<4>().0 {
        seed2 = seed2.wrapping_add(table[0x400 + (key & 0xFF) as usize]);
        let plain = u32::from_le_bytes(*chunk) ^ key.wrapping_add(seed2);
        *chunk = plain.to_le_bytes();
        key = ((!key) << 0x15).wrapping_add(0x1111_1111) | (key >> 0x0B);
        seed2 = plain
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
}

/// How a file's units are stored.
#[derive(Clone, Debug)]
pub enum Pack {
    Stored,
    /// COMPRESS flag, mask 0x02 (kept raw when it does not shrink).
    Zlib,
    /// COMPRESS flag, mask 0x10.
    Bzip2,
    /// COMPRESS flag with the given mask byte and an already-packed payload
    /// (single unit / single sector only).
    Mask(u8, Vec<u8>),
    /// IMPLODE flag with an already-imploded payload (single sector only).
    ImplodeFlag(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct FileSpec {
    pub name: String,
    /// The bytes the reader must return.
    pub data: Vec<u8>,
    pub pack: Pack,
    pub encrypt: bool,
    pub fix_key: bool,
    pub single_unit: bool,
    pub locale: u16,
}

impl FileSpec {
    pub fn new(name: &str, data: &[u8]) -> FileSpec {
        FileSpec {
            name: name.to_owned(),
            data: data.to_vec(),
            pack: Pack::Stored,
            encrypt: false,
            fix_key: false,
            single_unit: false,
            locale: 0,
        }
    }
    pub fn pack(mut self, pack: Pack) -> FileSpec {
        self.pack = pack;
        self
    }
    pub fn encrypted(mut self) -> FileSpec {
        self.encrypt = true;
        self
    }
    pub fn fix_key(mut self) -> FileSpec {
        self.encrypt = true;
        self.fix_key = true;
        self
    }
    pub fn single_unit(mut self) -> FileSpec {
        self.single_unit = true;
        self
    }
    pub fn locale(mut self, locale: u16) -> FileSpec {
        self.locale = locale;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Builder {
    /// Raw header `format_version` field: 0, 1, 2 or 3.
    pub version: u16,
    pub sector_shift: u16,
    /// Zero bytes in front of the header (a multiple of 512 to be findable).
    pub archive_offset: usize,
    pub files: Vec<FileSpec>,
    /// Put a "deleted" hash entry on each file's home slot first.
    pub deleted_before_each: bool,
    /// Override the hash table size (must be a power of two).
    pub hash_entries: Option<usize>,
}

pub const FLAG_IMPLODE: u32 = 0x100;
pub const FLAG_COMPRESS: u32 = 0x200;
pub const FLAG_ENCRYPTED: u32 = 0x1_0000;
pub const FLAG_FIX_KEY: u32 = 0x2_0000;
pub const FLAG_SINGLE_UNIT: u32 = 0x100_0000;
pub const FLAG_EXISTS: u32 = 0x8000_0000;

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).expect("zlib write");
    enc.finish().expect("zlib finish")
}

fn bzip2(data: &[u8]) -> Vec<u8> {
    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    enc.write_all(data).expect("bzip2 write");
    enc.finish().expect("bzip2 finish")
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            version: 0,
            sector_shift: 3, // 4096-byte sectors
            archive_offset: 0,
            files: Vec::new(),
            deleted_before_each: false,
            hash_entries: None,
        }
    }

    pub fn file(mut self, spec: FileSpec) -> Builder {
        self.files.push(spec);
        self
    }

    pub fn header_size(&self) -> usize {
        match self.version {
            0 => 0x20,
            1 => 0x2C,
            2 => 0x44,
            _ => 0xD0,
        }
    }

    pub fn sector_size(&self) -> usize {
        512usize << self.sector_shift
    }

    /// Encode one file: returns `(bytes on disk, flags)`; `pos` is the
    /// block's position relative to the archive start.
    fn encode(&self, f: &FileSpec, pos: usize) -> (Vec<u8>, u32) {
        let mut flags = FLAG_EXISTS;
        let compressed = !matches!(f.pack, Pack::Stored);
        match f.pack {
            Pack::ImplodeFlag(_) => flags |= FLAG_IMPLODE,
            Pack::Stored => {}
            _ => flags |= FLAG_COMPRESS,
        }
        if f.single_unit {
            flags |= FLAG_SINGLE_UNIT;
        }
        let mut key = 0;
        if f.encrypt {
            flags |= FLAG_ENCRYPTED;
            let base = f.name.rsplit(['\\', '/']).next().unwrap_or(&f.name);
            key = hash_string(base, 3);
            if f.fix_key {
                flags |= FLAG_FIX_KEY;
                key = key.wrapping_add(pos as u32) ^ f.data.len() as u32;
            }
        }

        let sector = self.sector_size();
        let chunks: Vec<&[u8]> = if f.single_unit {
            vec![&f.data[..]]
        } else {
            f.data.chunks(sector).collect()
        };
        let mut units: Vec<Vec<u8>> = Vec::new();
        for chunk in &chunks {
            let unit = match &f.pack {
                Pack::Stored => chunk.to_vec(),
                Pack::Zlib | Pack::Bzip2 => {
                    let (mask, body) = if matches!(f.pack, Pack::Zlib) {
                        (0x02u8, zlib(chunk))
                    } else {
                        (0x10u8, bzip2(chunk))
                    };
                    if body.len() + 1 < chunk.len() {
                        let mut u = vec![mask];
                        u.extend(body);
                        u
                    } else {
                        chunk.to_vec()
                    }
                }
                Pack::Mask(mask, payload) => {
                    let mut u = vec![*mask];
                    u.extend_from_slice(payload);
                    u
                }
                Pack::ImplodeFlag(stream) => stream.clone(),
            };
            units.push(unit);
        }

        let mut out = Vec::new();
        if f.single_unit {
            let mut unit = units.remove(0);
            if f.encrypt {
                encrypt(&mut unit, key);
            }
            out = unit;
        } else if compressed {
            let table_len = (units.len() + 1) * 4;
            let mut offsets = vec![table_len as u32];
            for u in &units {
                offsets.push(offsets.last().copied().unwrap_or(0) + u.len() as u32);
            }
            let mut table = Vec::new();
            for o in &offsets {
                le32(&mut table, *o);
            }
            if f.encrypt {
                encrypt(&mut table, key.wrapping_sub(1));
            }
            out.extend(table);
            for (i, mut u) in units.into_iter().enumerate() {
                if f.encrypt {
                    encrypt(&mut u, key.wrapping_add(i as u32));
                }
                out.extend(u);
            }
        } else {
            for (i, mut u) in units.into_iter().enumerate() {
                if f.encrypt {
                    encrypt(&mut u, key.wrapping_add(i as u32));
                }
                out.extend(u);
            }
        }
        (out, flags)
    }

    pub fn build(&self) -> Vec<u8> {
        let header_size = self.header_size();
        let mut body = Vec::new(); // everything after the header
        let mut blocks: Vec<(u32, u32, u32, u32)> = Vec::new(); // pos, csize, fsize, flags
        for f in &self.files {
            let pos = header_size + body.len();
            let (bytes, flags) = self.encode(f, pos);
            blocks.push((pos as u32, bytes.len() as u32, f.data.len() as u32, flags));
            body.extend(bytes);
        }

        // Hash table.
        let n = self
            .hash_entries
            .unwrap_or_else(|| (self.files.len() * 2).max(4).next_power_of_two());
        assert!(n.is_power_of_two());
        let mask = n - 1;
        // (hash_a, hash_b, locale, platform, block)
        let mut hashes = vec![
            (
                0xFFFF_FFFFu32,
                0xFFFF_FFFFu32,
                0xFFFFu16,
                0u16,
                0xFFFF_FFFFu32
            );
            n
        ];
        for (i, f) in self.files.iter().enumerate() {
            let mut idx = hash_string(&f.name, 0) as usize & mask;
            if self.deleted_before_each && hashes[idx].4 == 0xFFFF_FFFF {
                hashes[idx] = (0, 0, 0, 0, 0xFFFF_FFFE);
                idx = (idx + 1) & mask;
            }
            while hashes[idx].4 != 0xFFFF_FFFF {
                idx = (idx + 1) & mask;
            }
            hashes[idx] = (
                hash_string(&f.name, 1),
                hash_string(&f.name, 2),
                f.locale,
                0,
                i as u32,
            );
        }
        let mut hash_bytes = Vec::new();
        for (a, b, locale, platform, block) in &hashes {
            le32(&mut hash_bytes, *a);
            le32(&mut hash_bytes, *b);
            hash_bytes.extend_from_slice(&locale.to_le_bytes());
            hash_bytes.extend_from_slice(&platform.to_le_bytes());
            le32(&mut hash_bytes, *block);
        }
        encrypt(&mut hash_bytes, hash_string("(hash table)", 3));

        let mut block_bytes = Vec::new();
        for (pos, csize, fsize, flags) in &blocks {
            le32(&mut block_bytes, *pos);
            le32(&mut block_bytes, *csize);
            le32(&mut block_bytes, *fsize);
            le32(&mut block_bytes, *flags);
        }
        encrypt(&mut block_bytes, hash_string("(block table)", 3));

        let hash_pos = header_size + body.len();
        let block_pos = hash_pos + hash_bytes.len();
        let archive_size = block_pos + block_bytes.len();

        let mut header = Vec::new();
        header.extend_from_slice(b"MPQ\x1a");
        le32(&mut header, header_size as u32);
        le32(&mut header, archive_size as u32);
        header.extend_from_slice(&self.version.to_le_bytes());
        header.extend_from_slice(&self.sector_shift.to_le_bytes());
        le32(&mut header, hash_pos as u32);
        le32(&mut header, block_pos as u32);
        le32(&mut header, n as u32);
        le32(&mut header, blocks.len() as u32);
        header.resize(header_size, 0); // v2+ extension fields are all zero

        let mut out = vec![0u8; self.archive_offset];
        out.extend(header);
        out.extend(body);
        out.extend(hash_bytes);
        out.extend(block_bytes);
        out
    }
}

impl Default for Builder {
    fn default() -> Builder {
        Builder::new()
    }
}

pub fn write_temp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write temp archive");
    path
}

/// Deterministic incompressible-ish bytes.
pub fn noise(len: usize, mut seed: u32) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect()
}

/// Compressible text-like bytes.
pub fn text(len: usize) -> Vec<u8> {
    (0..len).map(|i| b"abcdefgh"[(i / 3) % 8]).collect()
}
