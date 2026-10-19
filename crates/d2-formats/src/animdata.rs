//! `AnimData.d2`: per-animation frame counts, speeds and frame events.
//!
//! The file is a run of blocks, `i32 count` followed by `count` 160-byte
//! records; the game uses 256 such blocks bucketed by a name hash, but a
//! loader only needs the records, so blocks are simply walked to the end of
//! the file and flattened in order:
//!
//! ```text
//! 0x00 char[8] cof name (token + mode + weapon class, NUL padded, upper case)
//! 0x08 u32 frames per direction   0x0C u32 speed (256 = one frame per tick)
//! 0x10 u8[144] per-frame event flags
//! ```
//!
//! Vanilla holds 3,558 records (29 names occur twice; the first wins, as in
//! the game's bucket scan) and Median XL 6,057. Lookup ignores ASCII case and
//! a trailing `.cof`. Fewer than 4 trailing bytes after the last block are
//! ignored; a negative count or a block running past the end is an error.

use std::collections::HashMap;

use crate::bytes::{i32_at, slice_at, u32_at};
use crate::{FormatError, Result};

const FORMAT: &str = "animdata";
const RECORD_LEN: usize = 160;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimRecord {
    /// Name up to the first NUL, Latin-1 decoded, as stored (usually upper case).
    pub cof_name: String,
    pub frames_per_direction: u32,
    pub speed: u32,
    pub flags: [u8; 144],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimData {
    records: Vec<AnimRecord>,
    /// upper-cased name -> index of the first record with that name
    index: HashMap<String, usize>,
}

fn key(name: &str) -> String {
    name.to_ascii_uppercase()
}

impl AnimData {
    pub fn parse(bytes: &[u8]) -> Result<AnimData> {
        if bytes.len() < 4 {
            return Err(FormatError::Truncated {
                format: FORMAT,
                offset: bytes.len(),
                needed: 4 - bytes.len(),
            });
        }
        let mut records = Vec::new();
        let mut pos = 0usize;
        while let Some(count) = i32_at(bytes, pos) {
            let at = pos;
            pos += 4;
            let count = usize::try_from(count).map_err(|_| FormatError::Invalid {
                format: FORMAT,
                what: "record count",
                offset: at,
                detail: format!("negative count {count}"),
            })?;
            let size = count.checked_mul(RECORD_LEN);
            let block = size.and_then(|s| slice_at(bytes, pos, s)).ok_or_else(|| {
                let remaining = bytes.len().saturating_sub(pos);
                FormatError::Truncated {
                    format: FORMAT,
                    offset: pos.min(bytes.len()),
                    needed: size.unwrap_or(usize::MAX).saturating_sub(remaining),
                }
            })?;
            for rec in block.as_chunks::<RECORD_LEN>().0 {
                let name_bytes = rec.get(..8).unwrap_or(&[]);
                let end = name_bytes
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(name_bytes.len());
                let cof_name: String = name_bytes
                    .get(..end)
                    .unwrap_or(&[])
                    .iter()
                    .map(|&b| char::from(b))
                    .collect();
                let mut flags = [0u8; 144];
                if let Some(f) = rec.get(16..RECORD_LEN) {
                    flags.copy_from_slice(f);
                }
                records.push(AnimRecord {
                    cof_name,
                    frames_per_direction: u32_at(rec.as_slice(), 8).unwrap_or(0),
                    speed: u32_at(rec.as_slice(), 12).unwrap_or(0),
                    flags,
                });
            }
            pos += block.len();
        }
        let mut index = HashMap::with_capacity(records.len());
        for (i, r) in records.iter().enumerate() {
            index.entry(key(&r.cof_name)).or_insert(i);
        }
        Ok(AnimData { records, index })
    }

    /// Looks a COF name up, ignoring ASCII case and a trailing `.cof`
    /// (`"AnNuHTH"` and `"annuhth.cof"` find the same record). When a name
    /// occurs more than once the first record wins.
    pub fn get(&self, cof_name: &str) -> Option<&AnimRecord> {
        let trimmed = match cof_name.len().checked_sub(4) {
            Some(cut)
                if cof_name
                    .get(cut..)
                    .is_some_and(|ext| ext.eq_ignore_ascii_case(".cof")) =>
            {
                cof_name.get(..cut).unwrap_or(cof_name)
            }
            _ => cof_name,
        };
        self.index
            .get(&key(trimmed))
            .and_then(|&i| self.records.get(i))
    }

    /// Every record in file order (duplicates included).
    pub fn records(&self) -> &[AnimRecord] {
        &self.records
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
