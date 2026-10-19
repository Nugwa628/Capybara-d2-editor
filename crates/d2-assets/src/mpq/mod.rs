//! Read-only MPQ archive reader.
//!
//! Our own implementation. Existing crates were read and rejected on API and
//! design grounds: `mpq` 0.8 loads the whole archive into memory, needs
//! `&mut self` for every read and `expect`s on I/O errors; `wow-mpq` 0.7
//! needs `&mut self` for `read_file`/`list` and pulls in rsa, rayon, tokio,
//! lzma and `pklib` (clap, indicatif). Neither fits the shared, lock-free,
//! positional-read, never-panic contract; the format is small, so the
//! reader lives here with `explode`, `flate2` and `bzip2` as codecs.
//! Supports header versions 0 to 3 (the on-disk `format_version`
//! field; "v1" to "v4" in StormLib terms) using their v1/v2-compatible
//! fields, archives at any 512-byte offset, user-data headers, encrypted and
//! FIX_KEY files, single-unit files, sector tables, and uncompressed,
//! implode, zlib and bzip2 storage.

mod crypto;
mod decompress;

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};

pub use crypto::hash_string;
use crypto::{HASH_FILE_KEY, HASH_NAME_A, HASH_NAME_B, HASH_TABLE_OFFSET, decrypt_bytes};

use crate::error::{AssetError, Result};

const MAGIC: [u8; 4] = *b"MPQ\x1a";
const USER_DATA_MAGIC: [u8; 4] = *b"MPQ\x1b";
const HEADER_MIN: usize = 0x20;
const HEADER_MAX: usize = 0xD0;
const SEARCH_STEP: u64 = 0x200;

const HASH_ENTRY_SIZE: u64 = 16;
const BLOCK_ENTRY_SIZE: u64 = 16;
const HASH_EMPTY: u32 = 0xFFFF_FFFF;
const HASH_DELETED: u32 = 0xFFFF_FFFE;

const FLAG_IMPLODE: u32 = 0x0000_0100;
const FLAG_COMPRESS: u32 = 0x0000_0200;
const FLAG_ENCRYPTED: u32 = 0x0001_0000;
const FLAG_FIX_KEY: u32 = 0x0002_0000;
const FLAG_PATCH_FILE: u32 = 0x0004_0000;
const FLAG_SINGLE_UNIT: u32 = 0x0100_0000;
const FLAG_DELETE_MARKER: u32 = 0x0200_0000;
const FLAG_EXISTS: u32 = 0x8000_0000;

/// No single game file comes close to this; it only bounds corrupt sizes.
const MAX_FILE_SIZE: u64 = 1 << 30;
/// Largest sector shift accepted (512 << 16 = 32 MiB sectors).
const MAX_SECTOR_SHIFT: u16 = 16;

/// What the archive header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MpqHeader {
    /// The raw `format_version` field: 0 = original ("v1"), 1 = "v2", 2 = "v3", 3 = "v4".
    pub format_version: u16,
    /// Where the header was found (0 for a plain archive).
    pub archive_offset: u64,
    pub sector_size: u32,
    pub hash_table_entries: u32,
    pub block_table_entries: u32,
}

/// How much of an archive a set of names accounts for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MpqCoverage {
    /// Files in the archive (block entries with the EXISTS flag).
    pub total: usize,
    /// Distinct files reachable through the given names.
    pub named: usize,
}

#[derive(Clone, Copy, Debug)]
struct HashEntry {
    hash_a: u32,
    hash_b: u32,
    locale: u16,
    block: u32,
}

#[derive(Clone, Copy, Debug)]
struct BlockEntry {
    pos: u64,
    compressed_size: u32,
    file_size: u32,
    flags: u32,
}

impl BlockEntry {
    fn exists(&self) -> bool {
        self.flags & FLAG_EXISTS != 0 && self.flags & FLAG_DELETE_MARKER == 0
    }
}

/// Read-only MPQ archive. `Send + Sync`; reads are positional, no lock is
/// held between calls, and the file is opened with full share mode.
pub struct MpqArchive {
    path: PathBuf,
    file: File,
    file_len: u64,
    header: MpqHeader,
    hashes: Vec<HashEntry>,
    blocks: Vec<BlockEntry>,
}

impl std::fmt::Debug for MpqArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MpqArchive")
            .field("path", &self.path)
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

fn le_u16(buf: &[u8], at: usize) -> u16 {
    let mut a = [0u8; 2];
    if let Some(s) = buf.get(at..at.saturating_add(2)) {
        a.copy_from_slice(s);
    }
    u16::from_le_bytes(a)
}

fn le_u32(buf: &[u8], at: usize) -> u32 {
    let mut a = [0u8; 4];
    if let Some(s) = buf.get(at..at.saturating_add(4)) {
        a.copy_from_slice(s);
    }
    u32::from_le_bytes(a)
}

/// Positional read of exactly `buf.len()` bytes; the file cursor is never relied on.
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    let mut done = 0usize;
    while done < buf.len() {
        let Some(rest) = buf.get_mut(done..) else {
            break;
        };
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(file, rest, offset + done as u64)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(file, rest, offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

impl MpqArchive {
    pub fn open(path: &Path) -> Result<MpqArchive> {
        let file = File::open(path).map_err(|e| AssetError::io(path, e))?;
        let file_len = file.metadata().map_err(|e| AssetError::io(path, e))?.len();
        let bad = |detail: String| AssetError::Mpq {
            archive: path.to_path_buf(),
            detail,
        };
        let io = |e: std::io::Error| AssetError::io(path, e);

        let archive_offset = find_header(&file, file_len)
            .map_err(io)?
            .ok_or_else(|| bad("no MPQ header found".into()))?;

        // Header.
        let avail = file_len - archive_offset;
        if avail < HEADER_MIN as u64 {
            return Err(bad(format!(
                "truncated header at offset {archive_offset:#x}"
            )));
        }
        let mut raw = vec![0u8; avail.min(HEADER_MAX as u64) as usize];
        read_exact_at(&file, &mut raw, archive_offset).map_err(io)?;
        let header_size = le_u32(&raw, 4) as usize;
        if !(HEADER_MIN..=HEADER_MAX).contains(&header_size) {
            return Err(bad(format!("invalid header size {header_size:#x}")));
        }
        if header_size > raw.len() {
            return Err(bad(format!(
                "truncated header at offset {archive_offset:#x}"
            )));
        }
        let format_version = le_u16(&raw, 12);
        let shift = le_u16(&raw, 14);
        if shift > MAX_SECTOR_SHIFT {
            return Err(bad(format!("invalid sector size shift {shift}")));
        }
        let sector_size = 512u32 << shift;
        let mut hash_pos = u64::from(le_u32(&raw, 16));
        let mut block_pos = u64::from(le_u32(&raw, 20));
        let hash_entries = le_u32(&raw, 24);
        let block_entries = le_u32(&raw, 28);
        let mut hi_block_pos = 0u64;
        if format_version >= 1 && header_size >= 0x2C {
            hi_block_pos = le_u32(&raw, 32) as u64 | (le_u32(&raw, 36) as u64) << 32;
            hash_pos |= u64::from(le_u16(&raw, 40)) << 32;
            block_pos |= u64::from(le_u16(&raw, 42)) << 32;
        }

        let read_table = |pos: u64, entries: u64, entry_size: u64, what: &str| -> Result<Vec<u8>> {
            let size = entries
                .checked_mul(entry_size)
                .ok_or_else(|| bad(format!("{what} size overflows")))?;
            let start = archive_offset
                .checked_add(pos)
                .ok_or_else(|| bad(format!("{what} position overflows")))?;
            let end = start
                .checked_add(size)
                .ok_or_else(|| bad(format!("{what} size overflows")))?;
            if end > file_len {
                return Err(bad(format!(
                    "{what} ({entries} entries at {start:#x}) extends past the end of the file ({file_len:#x} bytes)"
                )));
            }
            let mut buf = vec![0u8; size as usize];
            read_exact_at(&file, &mut buf, start).map_err(io)?;
            Ok(buf)
        };

        let mut hash_raw = read_table(
            hash_pos,
            u64::from(hash_entries),
            HASH_ENTRY_SIZE,
            "hash table",
        )?;
        decrypt_bytes(&mut hash_raw, hash_string("(hash table)", HASH_FILE_KEY));
        let hashes: Vec<HashEntry> = hash_raw
            .as_chunks::<{ HASH_ENTRY_SIZE as usize }>()
            .0
            .iter()
            .map(|c| HashEntry {
                hash_a: le_u32(c, 0),
                hash_b: le_u32(c, 4),
                locale: le_u16(c, 8),
                block: le_u32(c, 12),
            })
            .collect();

        let mut block_raw = read_table(
            block_pos,
            u64::from(block_entries),
            BLOCK_ENTRY_SIZE,
            "block table",
        )?;
        decrypt_bytes(&mut block_raw, hash_string("(block table)", HASH_FILE_KEY));
        let hi_raw = if hi_block_pos != 0 {
            Some(read_table(
                hi_block_pos,
                u64::from(block_entries),
                2,
                "hi-block table",
            )?)
        } else {
            None
        };
        let blocks: Vec<BlockEntry> = block_raw
            .as_chunks::<{ BLOCK_ENTRY_SIZE as usize }>()
            .0
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let hi = hi_raw.as_ref().map_or(0, |h| u64::from(le_u16(h, i * 2)));
                BlockEntry {
                    pos: u64::from(le_u32(c, 0)) | hi << 32,
                    compressed_size: le_u32(c, 4),
                    file_size: le_u32(c, 8),
                    flags: le_u32(c, 12),
                }
            })
            .collect();

        Ok(MpqArchive {
            path: path.to_path_buf(),
            file,
            file_len,
            header: MpqHeader {
                format_version,
                archive_offset,
                sector_size,
                hash_table_entries: hash_entries,
                block_table_entries: block_entries,
            },
            hashes,
            blocks,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn header(&self) -> &MpqHeader {
        &self.header
    }

    /// Block index of `name`, preferring locale 0, else the first match.
    fn find(&self, name: &str) -> Option<usize> {
        let n = self.hashes.len();
        if n == 0 {
            return None;
        }
        let hash_a = hash_string(name, HASH_NAME_A);
        let hash_b = hash_string(name, HASH_NAME_B);
        let valid = |e: &HashEntry| -> Option<usize> {
            if e.hash_a == hash_a && e.hash_b == hash_b && e.block < HASH_DELETED {
                let idx = e.block as usize;
                if self.blocks.get(idx).is_some_and(BlockEntry::exists) {
                    return Some(idx);
                }
            }
            None
        };
        let mut first: Option<usize> = None;
        let mut consider = |e: &HashEntry| -> Option<usize> {
            let idx = valid(e)?;
            if e.locale == 0 {
                return Some(idx);
            }
            first.get_or_insert(idx);
            None
        };
        if n.is_power_of_two() {
            let mask = n - 1;
            let start = hash_string(name, HASH_TABLE_OFFSET) as usize & mask;
            let mut i = start;
            loop {
                let e = self.hashes.get(i)?;
                if e.block == HASH_EMPTY {
                    break;
                }
                if e.block != HASH_DELETED
                    && let Some(idx) = consider(e)
                {
                    return Some(idx);
                }
                i = (i + 1) & mask;
                if i == start {
                    break;
                }
            }
        } else {
            // Not a Storm-style table; fall back to a full scan.
            for e in &self.hashes {
                if let Some(idx) = consider(e) {
                    return Some(idx);
                }
            }
        }
        first
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// Decompressed file contents, `None` when the archive has no such file.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        match self.find(name) {
            None => Ok(None),
            Some(idx) => self.read_block(idx, name).map(Some),
        }
    }

    /// `(file_size, flags)` from the block table, for diagnostics and tests.
    #[doc(hidden)]
    pub fn block_info(&self, name: &str) -> Option<(u32, u32)> {
        let idx = self.find(name)?;
        self.blocks.get(idx).map(|b| (b.file_size, b.flags))
    }

    /// Compression mask byte of the file's first unit (0x08 for the IMPLODE
    /// flag); `None` for absent or uncompressed files. Diagnostics only.
    #[doc(hidden)]
    pub fn compression_mask(&self, name: &str) -> Result<Option<u8>> {
        let Some(idx) = self.find(name) else {
            return Ok(None);
        };
        let Some(block) = self.blocks.get(idx).copied() else {
            return Ok(None);
        };
        if block.flags & FLAG_IMPLODE != 0 {
            return Ok(Some(decompress::METHOD_PKWARE));
        }
        if block.flags & FLAG_COMPRESS == 0 || block.file_size == 0 {
            return Ok(None);
        }
        let start = self.header.archive_offset.saturating_add(block.pos);
        let io = |e: std::io::Error| AssetError::io(&self.path, e);
        let encrypted = block.flags & FLAG_ENCRYPTED != 0;
        let mut key = 0;
        if encrypted {
            key = hash_string(file_key_name(name), HASH_FILE_KEY);
            if block.flags & FLAG_FIX_KEY != 0 {
                key = key.wrapping_add((block.pos & 0xFFFF_FFFF) as u32) ^ block.file_size;
            }
        }
        let mut word = [0u8; 8];
        if block.flags & FLAG_SINGLE_UNIT != 0 {
            if block.compressed_size >= block.file_size {
                return Ok(None);
            }
            read_exact_at(&self.file, &mut word[..4], start).map_err(io)?;
            if encrypted {
                decrypt_bytes(&mut word[..4], key);
            }
            return Ok(Some(word[0]));
        }
        // Sector 0: offsets [0] and [1] tell where it is and whether it is stored.
        read_exact_at(&self.file, &mut word, start).map_err(io)?;
        if encrypted {
            decrypt_bytes(&mut word, key.wrapping_sub(1));
        }
        let (from, to) = (le_u32(&word, 0), le_u32(&word, 4));
        let first = u64::from(block.file_size).min(u64::from(self.header.sector_size));
        if u64::from(to.saturating_sub(from)) >= first {
            return Ok(None);
        }
        let mut head = [0u8; 4];
        read_exact_at(&self.file, &mut head, start + u64::from(from)).map_err(io)?;
        if encrypted {
            decrypt_bytes(&mut head, key);
        }
        Ok(Some(head[0]))
    }

    /// `(listfile)` split on `;`, CR and LF, empty entries dropped.
    pub fn listfile(&self) -> Result<Option<Vec<String>>> {
        let Some(bytes) = self.read("(listfile)")? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes);
        Ok(Some(
            text.split([';', '\r', '\n'])
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
        ))
    }

    /// Block entries with the EXISTS flag.
    pub fn file_count(&self) -> usize {
        self.blocks.iter().filter(|b| b.exists()).count()
    }

    /// How many existing files the given names account for.
    pub fn coverage<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> MpqCoverage {
        let named: HashSet<usize> = names.into_iter().filter_map(|n| self.find(n)).collect();
        MpqCoverage {
            total: self.file_count(),
            named: named.len(),
        }
    }

    fn mpq_err(&self, detail: impl Into<String>) -> AssetError {
        AssetError::Mpq {
            archive: self.path.clone(),
            detail: detail.into(),
        }
    }

    fn decompress_err(&self, name: &str, (method, detail): (u8, String)) -> AssetError {
        AssetError::Decompress {
            archive: self.path.clone(),
            file: name.to_owned(),
            method,
            detail,
        }
    }

    fn read_block(&self, idx: usize, name: &str) -> Result<Vec<u8>> {
        let Some(block) = self.blocks.get(idx).copied() else {
            return Err(self.mpq_err(format!("{name}: block index {idx} out of range")));
        };
        if block.flags & FLAG_PATCH_FILE != 0 {
            return Err(self.mpq_err(format!("{name}: patch files are not supported")));
        }
        let file_size = u64::from(block.file_size);
        if file_size == 0 {
            return Ok(Vec::new());
        }
        let csize = u64::from(block.compressed_size);
        let compressed = block.flags & (FLAG_COMPRESS | FLAG_IMPLODE) != 0;
        let implode = block.flags & FLAG_IMPLODE != 0;

        // Size sanity before any allocation.
        if file_size > MAX_FILE_SIZE {
            return Err(self.mpq_err(format!("{name}: implausible file size {file_size}")));
        }
        if csize > MAX_FILE_SIZE * 2 {
            return Err(self.mpq_err(format!("{name}: implausible stored size {csize}")));
        }
        if compressed {
            if csize.saturating_mul(1 << 16).saturating_add(4096) < file_size {
                return Err(self.mpq_err(format!(
                    "{name}: implausible sizes (stored {csize}, unpacked {file_size})"
                )));
            }
        } else if csize < file_size {
            return Err(self.mpq_err(format!(
                "{name}: stored size {csize} is smaller than file size {file_size}"
            )));
        }
        let start = self
            .header
            .archive_offset
            .checked_add(block.pos)
            .ok_or_else(|| self.mpq_err(format!("{name}: file position overflows")))?;
        let end = start
            .checked_add(csize)
            .ok_or_else(|| self.mpq_err(format!("{name}: file extent overflows")))?;
        if end > self.file_len {
            return Err(self.mpq_err(format!(
                "{name}: data at {start:#x}..{end:#x} extends past the end of the file"
            )));
        }

        let mut data = vec![0u8; csize as usize];
        read_exact_at(&self.file, &mut data, start).map_err(|e| AssetError::io(&self.path, e))?;

        let encrypted = block.flags & FLAG_ENCRYPTED != 0;
        let key = if encrypted {
            let base = file_key_name(name);
            let mut key = hash_string(base, HASH_FILE_KEY);
            if block.flags & FLAG_FIX_KEY != 0 {
                key = key.wrapping_add((block.pos & 0xFFFF_FFFF) as u32) ^ block.file_size;
            }
            key
        } else {
            0
        };
        let expected_total = file_size as usize;

        if block.flags & FLAG_SINGLE_UNIT != 0 {
            if encrypted {
                decrypt_bytes(&mut data, key);
            }
            return self.finish_unit(
                name,
                data,
                expected_total,
                compressed && csize < file_size,
                implode,
            );
        }

        let sector_size = self.header.sector_size as usize;
        let sectors = expected_total.div_ceil(sector_size);
        let mut out = Vec::with_capacity(expected_total.min(64 << 20));

        // Offsets of each sector inside `data`.
        let offsets: Vec<usize> = if compressed {
            let table_len = (sectors + 1) * 4;
            if data.len() < table_len {
                return Err(self.mpq_err(format!("{name}: sector offset table is truncated")));
            }
            let Some(table) = data.get_mut(..table_len) else {
                return Err(self.mpq_err(format!("{name}: sector offset table is truncated")));
            };
            if encrypted {
                decrypt_bytes(table, key.wrapping_sub(1));
            }
            table
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c) as usize)
                .collect()
        } else {
            (0..=sectors)
                .map(|i| (i * sector_size).min(expected_total))
                .collect()
        };

        for i in 0..sectors {
            let (Some(&from), Some(&to)) = (offsets.get(i), offsets.get(i + 1)) else {
                return Err(self.mpq_err(format!("{name}: sector table is truncated")));
            };
            if from > to || to > data.len() {
                return Err(self.mpq_err(format!(
                    "{name}: sector {i} has invalid extent {from:#x}..{to:#x}"
                )));
            }
            let expected = sector_size.min(expected_total - i * sector_size);
            let mut sector = data
                .get(from..to)
                .ok_or_else(|| self.mpq_err(format!("{name}: sector {i} out of range")))?
                .to_vec();
            if encrypted {
                decrypt_bytes(&mut sector, key.wrapping_add(i as u32));
            }
            let unit = self.finish_unit(
                name,
                sector,
                expected,
                compressed && to - from < expected,
                implode,
            )?;
            out.extend_from_slice(&unit);
        }
        Ok(out)
    }

    /// Turn one stored unit (sector or whole file) into exactly `expected` bytes.
    fn finish_unit(
        &self,
        name: &str,
        mut unit: Vec<u8>,
        expected: usize,
        is_compressed: bool,
        implode: bool,
    ) -> Result<Vec<u8>> {
        if is_compressed {
            return decompress::decompress(&unit, expected, implode)
                .map_err(|e| self.decompress_err(name, e));
        }
        if unit.len() < expected {
            return Err(self.mpq_err(format!(
                "{name}: stored unit has {} bytes, expected {expected}",
                unit.len()
            )));
        }
        unit.truncate(expected);
        Ok(unit)
    }
}

/// Storm derives file keys from the last path component only.
fn file_key_name(name: &str) -> &str {
    name.rsplit(['\\', '/']).next().unwrap_or(name)
}

/// Offset of the archive header: the first 512-byte boundary holding the MPQ
/// magic, following user-data headers to the real one.
fn find_header(file: &File, file_len: u64) -> std::io::Result<Option<u64>> {
    const CHUNK: u64 = 1 << 16;
    let mut chunk_start = 0u64;
    let mut buf = vec![0u8; CHUNK as usize + 12];
    while chunk_start + 4 <= file_len {
        let want = (CHUNK + 12).min(file_len - chunk_start) as usize;
        let Some(window) = buf.get_mut(..want) else {
            break;
        };
        read_exact_at(file, window, chunk_start)?;
        let mut rel = 0u64;
        while rel < CHUNK && rel + 4 <= want as u64 {
            let at = rel as usize;
            let magic = window.get(at..at + 4);
            if magic == Some(&MAGIC) {
                return Ok(Some(chunk_start + rel));
            }
            if magic == Some(&USER_DATA_MAGIC) && rel + 12 <= want as u64 {
                let header_off = u64::from(le_u32(window, at + 8));
                let target = chunk_start + rel + header_off;
                if target + 4 <= file_len {
                    let mut m = [0u8; 4];
                    read_exact_at(file, &mut m, target)?;
                    if m == MAGIC {
                        return Ok(Some(target));
                    }
                }
            }
            rel += SEARCH_STEP;
        }
        chunk_start += CHUNK;
    }
    Ok(None)
}
