//! A minimal, dependency-free ZIP reader and writer (stored and deflate, no
//! zip64, no encryption), built on `flate2`'s raw deflate. Enough for map
//! packages; entries are written with a fixed timestamp so equal input gives
//! equal bytes.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::error::{HistoryError, Result};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
/// 1980-01-01 00:00:00 in DOS time/date.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x0021;
/// Refuse entries that inflate to more than this (zip-bomb guard).
const MAX_ENTRY: u64 = 1 << 30;

fn bad(detail: impl Into<String>) -> HistoryError {
    HistoryError::Package(detail.into())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(bytes);
    c.sum()
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Builds a zip in memory.
#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl ZipWriter {
    pub fn new() -> ZipWriter {
        ZipWriter::default()
    }

    /// Adds one file (deflated when that is smaller, else stored).
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let too_big = || bad(format!("{name}: too large for a package"));
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(data).map_err(|e| bad(e.to_string()))?;
        let deflated = enc.finish().map_err(|e| bad(e.to_string()))?;
        let (method, body): (u16, &[u8]) = if deflated.len() < data.len() {
            (8, &deflated)
        } else {
            (0, data)
        };
        let crc = crc32(data);
        let offset = u32::try_from(self.out.len()).map_err(|_| too_big())?;
        let csize = u32::try_from(body.len()).map_err(|_| too_big())?;
        let usize_ = u32::try_from(data.len()).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        // local header
        let o = &mut self.out;
        o.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        o.extend_from_slice(&20u16.to_le_bytes()); // version needed
        o.extend_from_slice(&0x0800u16.to_le_bytes()); // utf-8 names
        o.extend_from_slice(&method.to_le_bytes());
        o.extend_from_slice(&DOS_TIME.to_le_bytes());
        o.extend_from_slice(&DOS_DATE.to_le_bytes());
        o.extend_from_slice(&crc.to_le_bytes());
        o.extend_from_slice(&csize.to_le_bytes());
        o.extend_from_slice(&usize_.to_le_bytes());
        o.extend_from_slice(&name_len.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(name.as_bytes());
        o.extend_from_slice(body);
        // central directory entry
        let c = &mut self.central;
        c.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes()); // version made by
        c.extend_from_slice(&20u16.to_le_bytes()); // version needed
        c.extend_from_slice(&0x0800u16.to_le_bytes());
        c.extend_from_slice(&method.to_le_bytes());
        c.extend_from_slice(&DOS_TIME.to_le_bytes());
        c.extend_from_slice(&DOS_DATE.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&csize.to_le_bytes());
        c.extend_from_slice(&usize_.to_le_bytes());
        c.extend_from_slice(&name_len.to_le_bytes());
        c.extend_from_slice(&[0u8; 8]); // extra len, comment len, disk, internal attrs
        c.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| bad("too many files"))?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<Vec<u8>> {
        let cd_offset = u32::try_from(self.out.len()).map_err(|_| bad("package too large"))?;
        let cd_size = u32::try_from(self.central.len()).map_err(|_| bad("package too large"))?;
        self.out.extend_from_slice(&self.central);
        let o = &mut self.out;
        o.extend_from_slice(&EOCD_SIG.to_le_bytes());
        o.extend_from_slice(&[0u8; 4]); // disk numbers
        o.extend_from_slice(&self.count.to_le_bytes());
        o.extend_from_slice(&self.count.to_le_bytes());
        o.extend_from_slice(&cd_size.to_le_bytes());
        o.extend_from_slice(&cd_offset.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.out)
    }
}

/// Reads every file entry of a zip (directories are skipped). CRCs are
/// verified.
pub fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let eocd = (0..=bytes.len().saturating_sub(22))
        .rev()
        .take(65_557)
        .find(|&i| u32_at(bytes, i) == Some(EOCD_SIG))
        .ok_or_else(|| bad("not a zip file"))?;
    let count = u16_at(bytes, eocd + 10).ok_or_else(|| bad("truncated zip"))?;
    let mut at = usize::try_from(u32_at(bytes, eocd + 16).ok_or_else(|| bad("truncated zip"))?)
        .map_err(|_| bad("bad zip"))?;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32_at(bytes, at) != Some(CENTRAL_SIG) {
            return Err(bad("damaged zip directory"));
        }
        let t = || bad("truncated zip");
        let flags = u16_at(bytes, at + 8).ok_or_else(t)?;
        let method = u16_at(bytes, at + 10).ok_or_else(t)?;
        let crc = u32_at(bytes, at + 16).ok_or_else(t)?;
        let csize = u64::from(u32_at(bytes, at + 20).ok_or_else(t)?);
        let usize_ = u64::from(u32_at(bytes, at + 24).ok_or_else(t)?);
        let nlen = usize::from(u16_at(bytes, at + 28).ok_or_else(t)?);
        let xlen = usize::from(u16_at(bytes, at + 30).ok_or_else(t)?);
        let clen = usize::from(u16_at(bytes, at + 32).ok_or_else(t)?);
        let local = usize::try_from(u32_at(bytes, at + 42).ok_or_else(t)?).map_err(|_| t())?;
        let name_bytes = bytes.get(at + 46..at + 46 + nlen).ok_or_else(t)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        at += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(bad(format!("{name}: encrypted entries are not supported")));
        }
        if csize > MAX_ENTRY || usize_ > MAX_ENTRY {
            return Err(bad(format!("{name}: entry too large")));
        }
        if u32_at(bytes, local) != Some(LOCAL_SIG) {
            return Err(bad(format!("{name}: damaged local header")));
        }
        let lname = usize::from(u16_at(bytes, local + 26).ok_or_else(t)?);
        let lextra = usize::from(u16_at(bytes, local + 28).ok_or_else(t)?);
        let start = local + 30 + lname + lextra;
        let body = bytes.get(start..start + csize as usize).ok_or_else(t)?;
        let data = match method {
            0 => body.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_ as usize);
                DeflateDecoder::new(body)
                    .take(MAX_ENTRY + 1)
                    .read_to_end(&mut v)
                    .map_err(|e| bad(format!("{name}: {e}")))?;
                v
            }
            m => return Err(bad(format!("{name}: unsupported compression method {m}"))),
        };
        if data.len() as u64 != usize_ || crc32(&data) != crc {
            return Err(bad(format!("{name}: checksum mismatch")));
        }
        out.push((name, data));
    }
    Ok(out)
}
