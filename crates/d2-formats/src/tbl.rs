//! `.tbl` string tables (`data\local\lng\<lang>\*.tbl`).
//!
//! A `.tbl` is a compiled open-addressing hash table (see
//! `docs/guides/tbl-string-tables.md` in the reference tree):
//!
//! ```text
//! 0x00 u16 crc        CRC-16 over [X, S)            (not checked here)
//! 0x02 u16 H          ID-table entries
//! 0x04 u32 N          node slots / hash modulus
//! 0x08 u8             unknown (1 in every shipped file)
//! 0x09 u32 X          offset of the string data
//! 0x0D u32 P          longest probe chain           (unused here)
//! 0x11 u32 S          file size
//! 0x15 H x u16        ID table
//!      N x 17 bytes   nodes: active u8, string id u16, unknown u32,
//!                     key offset u32, text offset u32, text length u16
//!      string data    NUL-terminated keys and texts
//! ```
//!
//! Keys and texts are decoded as Latin-1 (one char per byte), which is
//! lossless for the byte values the western client uses (`0xFF` is the colour
//! escape lead byte); other code pages come out as mojibake but round-trip.
//!
//! A table whose active nodes reference more string bytes in total than
//! eight times the file size (plus 64 KiB) is rejected as `Invalid`: shipped
//! tables reference each string once, so only a crafted file sharing one long
//! string among many nodes gets there.

use std::collections::HashMap;

use crate::{FormatError, Result};

mod write;
pub use write::TblDoc;

const FORMAT: &str = "tbl";
const HEADER_LEN: usize = 0x15;
const NODE_LEN: usize = 17;
/// Decoded key + text bytes allowed per byte of file, on top of
/// [`STRING_BUDGET_FLOOR`]. Shipped tables decode to under one byte per byte
/// (each string is referenced by one node); the cap stops a file whose nodes
/// all point at one long string from multiplying it into gigabytes.
const STRING_BUDGET_PER_BYTE: usize = 8;
const STRING_BUDGET_FLOOR: usize = 1 << 16;

/// The active key/text pairs of one `.tbl`, in node order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringTable {
    entries: Vec<(String, String)>,
    /// String id of each entry (the node's id field), parallel to `entries`.
    ids: Vec<u16>,
    /// String id -> entry (the first node in file order wins on a repeat).
    by_id: HashMap<u16, usize>,
    /// key -> entry the game's hash-and-probe walk would reach first.
    index: HashMap<String, usize>,
}

fn invalid(what: &'static str, offset: usize, detail: impl Into<String>) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset,
        detail: detail.into(),
    }
}

fn u16_at(bytes: &[u8], off: usize) -> Option<u16> {
    let b = bytes.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

fn u32_at(bytes: &[u8], off: usize) -> Option<u32> {
    let b = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([
        *b.first()?,
        *b.get(1)?,
        *b.get(2)?,
        *b.get(3)?,
    ]))
}

/// The ELF/PJW hash the game uses for key lookup.
pub(crate) fn key_hash(key: &[u8], modulus: u32) -> u32 {
    let mut h: u32 = 0;
    for &b in key {
        h = (h << 4).wrapping_add(u32::from(b));
        if h & 0xF000_0000 != 0 {
            h = (((h & 0xF000_0000) >> 24) ^ h) & 0x0FFF_FFFF;
        }
    }
    h.checked_rem(modulus).unwrap_or(0)
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The NUL-terminated string at `off`, without its terminator.
fn cstr_at<'a>(bytes: &'a [u8], off: usize, what: &'static str) -> Result<&'a [u8]> {
    let rest = bytes
        .get(off..)
        .ok_or_else(|| invalid(what, off, "offset is past the end of the file"))?;
    let len = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| invalid(what, off, "string is not terminated"))?;
    rest.get(..len)
        .ok_or_else(|| invalid(what, off, "string is not terminated"))
}

impl StringTable {
    /// Parses a `.tbl`. Inactive slots are skipped; the CRC is not verified.
    pub fn parse(bytes: &[u8]) -> Result<StringTable> {
        if bytes.len() < HEADER_LEN {
            return Err(FormatError::Truncated {
                format: FORMAT,
                offset: 0,
                needed: HEADER_LEN - bytes.len(),
            });
        }
        let id_count = u16_at(bytes, 2).unwrap_or(0) as usize;
        let node_count = u32_at(bytes, 4).unwrap_or(0);
        let nodes_start = HEADER_LEN + id_count * 2;
        let nodes_len = u64::from(node_count) * NODE_LEN as u64;
        let nodes_end = nodes_start as u64 + nodes_len;
        if nodes_end > bytes.len() as u64 {
            return Err(FormatError::Truncated {
                format: FORMAT,
                offset: nodes_start,
                needed: usize::try_from(nodes_end - bytes.len() as u64).unwrap_or(usize::MAX),
            });
        }

        let mut entries = Vec::new();
        let mut ids = Vec::new();
        let mut by_id: HashMap<u16, usize> = HashMap::new();
        let mut best: HashMap<String, (u64, usize)> = HashMap::new();
        let budget = bytes
            .len()
            .saturating_mul(STRING_BUDGET_PER_BYTE)
            .saturating_add(STRING_BUDGET_FLOOR);
        let mut decoded = 0usize;
        for slot in 0..node_count {
            let base = nodes_start + slot as usize * NODE_LEN;
            if bytes.get(base).copied() != Some(1) {
                continue;
            }
            let key_off = u32_at(bytes, base + 7).unwrap_or(0) as usize;
            let text_off = u32_at(bytes, base + 11).unwrap_or(0) as usize;
            let key_bytes = cstr_at(bytes, key_off, "key offset")?;
            let text_bytes = cstr_at(bytes, text_off, "text offset")?;
            decoded = decoded
                .saturating_add(key_bytes.len())
                .saturating_add(text_bytes.len());
            if decoded > budget {
                return Err(invalid(
                    "string data",
                    base,
                    format!("nodes reference more than {budget} bytes of strings"),
                ));
            }
            let key = latin1(key_bytes);
            // Distance from the home bucket along the probe sequence. `slot`
            // and `home` are both below `node_count` (> 0 inside this loop);
            // u64 keeps `slot + node_count` from overflowing.
            let home = key_hash(key_bytes, node_count);
            let n = u64::from(node_count);
            let dist = (u64::from(slot) + n - u64::from(home)) % n;
            let idx = entries.len();
            let id = u16_at(bytes, base + 1).unwrap_or(0);
            ids.push(id);
            by_id.entry(id).or_insert(idx);
            entries.push((key.clone(), latin1(text_bytes)));
            best.entry(key)
                .and_modify(|e| {
                    if dist < e.0 {
                        *e = (dist, idx);
                    }
                })
                .or_insert((dist, idx));
        }
        let index = best.into_iter().map(|(k, (_, i))| (k, i)).collect();
        Ok(StringTable {
            entries,
            ids,
            by_id,
            index,
        })
    }

    /// The text for `key`. If several entries share a key, this is the one the
    /// game's hash-and-probe lookup reaches first.
    pub fn get(&self, key: &str) -> Option<&str> {
        let idx = *self.index.get(key)?;
        self.entries.get(idx).map(|(_, text)| text.as_str())
    }

    /// The `(key, text)` pair with string id `id`: the number compiled excel
    /// tables store instead of a key (`patchstring.tbl` ids are offset by
    /// 10000 and `expansionstring.tbl` ids by 20000 in those tables).
    pub fn entry_by_id(&self, id: u16) -> Option<(&str, &str)> {
        let idx = *self.by_id.get(&id)?;
        self.entries
            .get(idx)
            .map(|(key, text)| (key.as_str(), text.as_str()))
    }

    /// String id of each entry of [`StringTable::entries`], in the same order.
    pub fn entry_ids(&self) -> &[u16] {
        &self.ids
    }

    /// Active `(key, text)` pairs in node-array order.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Test-only compiler: builds a `.tbl` the way the game's tool lays it out
    /// (load factor ~ 1, linear probing, ID table indexed by string id).
    fn build(entries: &[(&[u8], &[u8])], slack: u32) -> Vec<u8> {
        let n = entries.len() as u32 + slack;
        let mut nodes: Vec<Option<usize>> = vec![None; n as usize];
        let mut max_probe = 0u32;
        for (i, (key, _)) in entries.iter().enumerate() {
            let mut slot = key_hash(key, n);
            let mut probes = 0;
            while nodes[slot as usize].is_some() {
                slot = (slot + 1) % n;
                probes += 1;
            }
            max_probe = max_probe.max(probes);
            nodes[slot as usize] = Some(i);
        }
        let h = entries.len();
        let data_start = HEADER_LEN + h * 2 + n as usize * NODE_LEN;
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for (key, text) in entries {
            let k = data_start + data.len();
            data.extend_from_slice(key);
            data.push(0);
            let t = data_start + data.len();
            data.extend_from_slice(text);
            data.push(0);
            offsets.push((k, t, text.len() + 1));
        }
        let total = data_start + data.len();
        let mut out = vec![0u8; HEADER_LEN];
        out[2..4].copy_from_slice(&(h as u16).to_le_bytes());
        out[4..8].copy_from_slice(&n.to_le_bytes());
        out[8] = 1;
        out[9..13].copy_from_slice(&(data_start as u32).to_le_bytes());
        out[13..17].copy_from_slice(&max_probe.to_le_bytes());
        out[17..21].copy_from_slice(&(total as u32).to_le_bytes());
        let mut id_table = vec![0u16; h];
        for (slot, node) in nodes.iter().enumerate() {
            if let Some(i) = node {
                id_table[*i] = slot as u16;
            }
        }
        for id in id_table {
            out.extend_from_slice(&id.to_le_bytes());
        }
        for node in &nodes {
            match node {
                Some(i) => {
                    let (k, t, tl) = offsets[*i];
                    out.push(1);
                    out.extend_from_slice(&(*i as u16).to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&(k as u32).to_le_bytes());
                    out.extend_from_slice(&(t as u32).to_le_bytes());
                    out.extend_from_slice(&(tl as u16).to_le_bytes());
                }
                None => out.extend_from_slice(&[0u8; NODE_LEN]),
            }
        }
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn round_trip_synthetic_table() {
        let entries: Vec<(&[u8], &[u8])> = vec![
            (b"qstsa5q1", b"Siege on Harrogath"),
            (b"9bl", b"Stiletto"),
            (b"AmaOnly", b"(Amazon Only)"),
            (b"color", b"\xFFc5Magic"),
            (b"empty", b""),
        ];
        let bytes = build(&entries, 1);
        let t = StringTable::parse(&bytes).expect("parses");
        assert_eq!(t.entries().len(), entries.len());
        for (k, v) in &entries {
            let key = latin1(k);
            assert_eq!(t.get(&key).map(str::to_owned), Some(latin1(v)), "{key}");
        }
        assert_eq!(t.get("color"), Some("\u{ff}c5Magic"));
        assert_eq!(t.get("missing"), None);
        // the synthetic compiler numbers entries in input order
        for (id, (k, v)) in entries.iter().enumerate() {
            let id = u16::try_from(id).expect("small");
            assert_eq!(
                t.entry_by_id(id),
                Some((latin1(k).as_str(), latin1(v).as_str())),
                "id {id}"
            );
        }
        assert_eq!(t.entry_by_id(5), None);
        let mut ids = t.entry_ids().to_vec();
        ids.sort_unstable();
        assert_eq!(ids, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn heavy_collisions_and_duplicate_keys() {
        // No spare slot at all: every lookup may walk far, and keys repeat.
        let entries: Vec<(&[u8], &[u8])> = vec![
            (b"x", b"K"),
            (b"x", b"M"),
            (b"x", b"B"),
            (b"a", b"A"),
            (b"b", b"B2"),
        ];
        let bytes = build(&entries, 0);
        let t = StringTable::parse(&bytes).expect("parses");
        assert_eq!(t.entries().len(), 5);
        // the first reached by probing is the earliest-placed duplicate
        assert_eq!(t.get("x"), Some("K"));
        assert_eq!(t.get("a"), Some("A"));
        assert_eq!(t.get("b"), Some("B2"));
    }

    #[test]
    fn malformed_input_is_rejected_not_panicked() {
        assert!(StringTable::parse(&[]).is_err());
        assert!(StringTable::parse(&[0; 20]).is_err());
        // header claims an enormous node array
        let mut huge = vec![0u8; HEADER_LEN];
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(StringTable::parse(&huge).is_err());
        // an empty table is fine
        let empty = vec![0u8; HEADER_LEN];
        let t = StringTable::parse(&empty).expect("empty table");
        assert!(t.entries().is_empty());
        // truncating a valid table must never panic
        let valid = build(&[(b"k", b"v"), (b"k2", b"v2")], 1);
        for cut in 0..valid.len() {
            let _ = StringTable::parse(&valid[..cut]);
        }
    }

    #[test]
    fn corpus_tbl_files_parse() {
        let Some(root) = crate::testutil::private_root() else {
            println!("skipped: CAPY_PRIVATE_DIR / testdata/private missing");
            return;
        };
        let mut count = 0usize;
        for dir in ["vanilla", "mxl"] {
            let base = root.join("extracted").join(dir);
            if !base.is_dir() {
                println!("skipped: {} missing", base.display());
                continue;
            }
            // data/local/font/*.tbl are glyph tables, a different format
            let lng = base.join("data").join("local").join("lng");
            for path in crate::testutil::find_files(&lng, "tbl") {
                let bytes = std::fs::read(&path).expect("readable");
                let t = StringTable::parse(&bytes)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                println!("{}: {} entries", path.display(), t.entries().len());
                count += 1;
                if dir == "vanilla" && path.ends_with("eng/expansionstring.tbl") {
                    // worked example from the guide
                    assert_eq!(t.get("qstsa5q1"), Some("Siege on Harrogath"));
                    assert_eq!(t.entries().len(), 2818);
                }
                if dir == "vanilla"
                    && path.ends_with("eng/string.tbl")
                    && !path.to_string_lossy().contains("beta")
                {
                    assert_eq!(t.entries().len(), 5391);
                }
                if dir == "vanilla" && path.ends_with("eng/patchstring.tbl") {
                    // 1,169 entries in 1.13c; the 1.14d install has more
                    assert_eq!(t.get("AmaOnly"), Some("(Amazon Only)"));
                }
            }
        }
        println!("tbl corpus: {count} files parsed");
    }
}
