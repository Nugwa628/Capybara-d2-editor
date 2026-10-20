//! `.tbl` writer for the classic format.
//!
//! [`TblDoc`] holds a table as the ordered list of its strings, indexed by
//! string id (entry `i` has id `i`, which is what compiled excel tables
//! store), plus the hash-table size and the version byte of the file it came
//! from. Writing lays the table out the way the game's compiler does:
//!
//! * the hash table keeps its node count (`N`), growing only when there are
//!   more strings than slots; each string goes to its home slot
//!   (`key_hash(key, N)`) or the next free one (linear probing, wrapping), in
//!   id order, so for duplicate keys the lowest id owns the home slot;
//! * the ID table has one `u16` per string holding the string's node slot;
//! * `P` (header offset 0x0D) is the longest probe sequence plus one;
//! * each node stores its hash in the 4 bytes after the id (the field
//!   [`StringTable`] does not interpret);
//! * string data is `key NUL text NUL` per id, and the header CRC is
//!   CRC-16/CCITT (poly 0x1021, init 0xFFFF) over `[X, S)`.
//!
//! Strings are Latin-1 (one char per byte), as [`StringTable`] reads them.
//!
//! A [`TblDoc`] that was loaded and not edited writes back the original bytes
//! exactly, whatever layout the source tool used; once any edit is made the
//! table is re-laid out as above.

use super::{HEADER_LEN, NODE_LEN, StringTable};
use crate::{FormatError, Result};

const FORMAT: &str = "tbl";

fn invalid(what: &'static str, detail: impl Into<String>) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset: 0,
        detail: detail.into(),
    }
}

fn to_latin1(s: &str, what: &'static str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let b = u8::try_from(u32::from(c))
            .map_err(|_| invalid(what, format!("{c:?} is not a Latin-1 character")))?;
        if b == 0 {
            return Err(invalid(what, "strings cannot contain NUL"));
        }
        out.push(b);
    }
    Ok(out)
}

/// CRC-16/CCITT (poly 0x1021, init 0xFFFF, not reflected).
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// An editable `.tbl`: strings by id, plus layout parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TblDoc {
    /// `(key, text)` indexed by string id.
    entries: Vec<(String, String)>,
    hash_size: u32,
    version: u8,
    /// The source bytes while the table is unedited.
    original: Option<Vec<u8>>,
}

impl Default for TblDoc {
    fn default() -> Self {
        TblDoc::new()
    }
}

impl TblDoc {
    /// An empty table (version byte 1, hash table grows with the strings).
    pub fn new() -> TblDoc {
        TblDoc {
            entries: Vec::new(),
            hash_size: 0,
            version: 1,
            original: None,
        }
    }

    /// Loads a `.tbl`. Fails if [`StringTable::parse`] does, or if the string
    /// ids of the active nodes are not exactly `0..n` (a table the writer
    /// could not re-lay out without renumbering).
    pub fn parse(bytes: &[u8]) -> Result<TblDoc> {
        let table = StringTable::parse(bytes)?;
        let n = table.entries().len();
        let mut slots: Vec<Option<(String, String)>> = vec![None; n];
        for ((key, text), &id) in table.entries().iter().zip(table.entry_ids()) {
            let slot = slots
                .get_mut(usize::from(id))
                .ok_or_else(|| invalid("string id", format!("id {id} is outside 0..{n}")))?;
            if slot.replace((key.clone(), text.clone())).is_some() {
                return Err(invalid("string id", format!("id {id} is used twice")));
            }
        }
        let entries = slots.into_iter().flatten().collect();
        Ok(TblDoc {
            entries,
            hash_size: super::u32_at(bytes, 4).unwrap_or(0),
            version: bytes.get(8).copied().unwrap_or(1),
            original: Some(bytes.to_vec()),
        })
    }

    /// `(key, text)` pairs in string-id order.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// The text of the first entry (lowest id) with `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, t)| t.as_str())
    }

    /// True once the table has been changed since it was loaded.
    pub fn is_modified(&self) -> bool {
        self.original.is_none()
    }

    fn check(key: &str, text: &str) -> Result<()> {
        let k = to_latin1(key, "key")?;
        let t = to_latin1(text, "text")?;
        if k.is_empty() {
            return Err(invalid("key", "keys cannot be empty"));
        }
        // the node's text length field is a u16 counting the NUL
        if t.len() >= usize::from(u16::MAX) {
            return Err(invalid("text", "text is too long for a node"));
        }
        Ok(())
    }

    /// Sets the text of `key`, keeping its id; a new key is appended with the
    /// next free id. Returns the id.
    pub fn set(&mut self, key: &str, text: &str) -> Result<u16> {
        Self::check(key, text)?;
        let id = match self.entries.iter().position(|(k, _)| k == key) {
            Some(i) => {
                if let Some(e) = self.entries.get_mut(i) {
                    e.1 = text.to_owned();
                }
                i
            }
            None => {
                self.entries.push((key.to_owned(), text.to_owned()));
                self.entries.len() - 1
            }
        };
        self.original = None;
        u16::try_from(id).map_err(|_| invalid("string id", "more than 65535 strings"))
    }

    /// Appends a string even if `key` already exists (the game finds the
    /// lowest id's). Returns the new id.
    pub fn add(&mut self, key: &str, text: &str) -> Result<u16> {
        Self::check(key, text)?;
        self.entries.push((key.to_owned(), text.to_owned()));
        self.original = None;
        u16::try_from(self.entries.len() - 1)
            .map_err(|_| invalid("string id", "more than 65535 strings"))
    }

    /// Removes the first entry with `key`. Ids above it shift down by one
    /// (the table has no gaps), so compiled excel tables that store string
    /// ids need re-exporting. Returns whether a key was removed.
    pub fn remove(&mut self, key: &str) -> bool {
        match self.entries.iter().position(|(k, _)| k == key) {
            Some(i) => {
                self.entries.remove(i);
                self.original = None;
                true
            }
            None => false,
        }
    }

    /// The file bytes: the original unchanged, or the laid-out table.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        match &self.original {
            Some(o) => Ok(o.clone()),
            None => self.layout(),
        }
    }

    /// Always lays the table out from its entries, even when unedited.
    pub fn layout(&self) -> Result<Vec<u8>> {
        let n = self.entries.len();
        let h = u16::try_from(n).map_err(|_| invalid("string id", "more than 65535 strings"))?;
        let size = usize::try_from(self.hash_size).unwrap_or(usize::MAX).max(n);
        // slots are named by u16 in the ID table
        if size > usize::from(u16::MAX) + 1 {
            return Err(invalid("hash size", "more than 65536 node slots"));
        }
        let size32 = size as u32;
        let mut slot_of = vec![0usize; n];
        let mut node: Vec<Option<usize>> = vec![None; size];
        let mut max_probes = 0u32;
        let mut keys = Vec::with_capacity(n);
        let mut texts = Vec::with_capacity(n);
        for (i, (key, text)) in self.entries.iter().enumerate() {
            Self::check(key, text)?;
            let k = to_latin1(key, "key")?;
            let mut slot = super::key_hash(&k, size32) as usize;
            let mut probes = 0u32;
            while node.get(slot).is_some_and(Option::is_some) {
                slot = (slot + 1) % size;
                probes += 1;
            }
            max_probes = max_probes.max(probes + 1);
            if let Some(s) = node.get_mut(slot) {
                *s = Some(i);
            }
            if let Some(s) = slot_of.get_mut(i) {
                *s = slot;
            }
            keys.push(k);
            texts.push(to_latin1(text, "text")?);
        }
        let data_start = HEADER_LEN + n * 2 + size * NODE_LEN;
        let mut data = Vec::new();
        let mut offsets = Vec::with_capacity(n);
        for (k, t) in keys.iter().zip(&texts) {
            let key_off = data_start + data.len();
            data.extend_from_slice(k);
            data.push(0);
            let text_off = data_start + data.len();
            data.extend_from_slice(t);
            data.push(0);
            offsets.push((key_off, text_off));
        }
        let total = data_start + data.len();
        let to_u32 = |v: usize| {
            u32::try_from(v).map_err(|_| invalid("file size", "table is larger than 4 GiB"))
        };
        let mut out = vec![0u8; HEADER_LEN];
        out[2..4].copy_from_slice(&h.to_le_bytes());
        out[4..8].copy_from_slice(&size32.to_le_bytes());
        out[8] = self.version;
        out[9..13].copy_from_slice(&to_u32(data_start)?.to_le_bytes());
        out[13..17].copy_from_slice(&max_probes.to_le_bytes());
        out[17..21].copy_from_slice(&to_u32(total)?.to_le_bytes());
        for &slot in &slot_of {
            out.extend_from_slice(&(slot as u16).to_le_bytes());
        }
        for entry in &node {
            match entry {
                &Some(i) => {
                    let (key_off, text_off) = offsets.get(i).copied().unwrap_or((0, 0));
                    let key = keys.get(i).map_or(&[][..], Vec::as_slice);
                    let text_len = texts.get(i).map_or(0, Vec::len) + 1;
                    out.push(1);
                    out.extend_from_slice(&(i as u16).to_le_bytes());
                    out.extend_from_slice(&super::key_hash(key, size32).to_le_bytes());
                    out.extend_from_slice(&to_u32(key_off)?.to_le_bytes());
                    out.extend_from_slice(&to_u32(text_off)?.to_le_bytes());
                    out.extend_from_slice(&(text_len as u16).to_le_bytes());
                }
                None => out.extend_from_slice(&[0u8; NODE_LEN]),
            }
        }
        out.extend_from_slice(&data);
        let crc = crc16(out.get(data_start..).unwrap_or(&[]));
        out[0..2].copy_from_slice(&crc.to_le_bytes());
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The game's lookup: hash, then up to P consecutive slots (an empty slot
    /// does not end the walk), comparing keys.
    fn lookup(bytes: &[u8], key: &str) -> Option<String> {
        let n = super::super::u32_at(bytes, 4)?;
        let p = super::super::u32_at(bytes, 13)?;
        let h = usize::from(super::super::u16_at(bytes, 2)?);
        let k = to_latin1(key, "key").ok()?;
        let home = super::super::key_hash(&k, n);
        for t in 0..p {
            let slot = (home + t) % n;
            let base = HEADER_LEN + h * 2 + slot as usize * NODE_LEN;
            if bytes.get(base) != Some(&1) {
                continue;
            }
            let ko = super::super::u32_at(bytes, base + 7)? as usize;
            let to = super::super::u32_at(bytes, base + 11)? as usize;
            let kb = super::super::cstr_at(bytes, ko, "key").ok()?;
            if kb == k.as_slice() {
                let tb = super::super::cstr_at(bytes, to, "text").ok()?;
                return Some(tb.iter().map(|&b| char::from(b)).collect());
            }
        }
        None
    }

    fn assert_consistent(bytes: &[u8], doc: &TblDoc) {
        let t = StringTable::parse(bytes).unwrap();
        // same ids -> same strings
        for (id, (k, v)) in doc.entries().iter().enumerate() {
            assert_eq!(t.entry_by_id(id as u16), Some((k.as_str(), v.as_str())));
        }
        // the game's lookup finds the lowest id for each key
        let mut first: HashMap<&str, &str> = HashMap::new();
        for (k, v) in doc.entries() {
            first.entry(k).or_insert(v);
        }
        for (k, v) in first {
            assert_eq!(lookup(bytes, k).as_deref(), Some(v), "{k}");
            assert_eq!(t.get(k), Some(v), "{k}");
        }
        let n = super::super::u32_at(bytes, 4).unwrap() as usize;
        let h = doc.entries().len();
        let start = super::super::u32_at(bytes, 9).unwrap() as usize;
        assert_eq!(start, HEADER_LEN + h * 2 + n * NODE_LEN);
        assert_eq!(
            usize::from(super::super::u16_at(bytes, 0).unwrap()),
            usize::from(crc16(&bytes[start..]))
        );
        assert_eq!(
            super::super::u32_at(bytes, 17).unwrap() as usize,
            bytes.len()
        );
    }

    #[test]
    fn crc_check_value() {
        // CRC-16/CCITT-FALSE check value
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn build_edit_and_reread() {
        let mut d = TblDoc::new();
        d.set("qstsa5q1", "Siege on Harrogath").unwrap();
        d.set("9bl", "Stiletto").unwrap();
        d.set("color", "\u{ff}c5Magic").unwrap();
        d.set("empty", "").unwrap();
        let b = d.to_bytes().unwrap();
        assert_consistent(&b, &d);
        let back = TblDoc::parse(&b).unwrap();
        assert_eq!(back.entries(), d.entries());
        // an unedited reload writes the same bytes, and so does a relayout
        assert_eq!(back.to_bytes().unwrap(), b);
        assert_eq!(back.layout().unwrap(), b);

        let mut e = back.clone();
        assert!(!e.is_modified());
        assert_eq!(e.set("9bl", "Dagger").unwrap(), 1);
        assert_eq!(e.set("newkey", "New").unwrap(), 4);
        assert!(e.remove("empty"));
        assert!(!e.remove("empty"));
        assert!(e.is_modified());
        let b2 = e.to_bytes().unwrap();
        assert_consistent(&b2, &e);
        assert_eq!(e.get("9bl"), Some("Dagger"));
        assert_eq!(lookup(&b2, "newkey").as_deref(), Some("New"));
        assert_eq!(lookup(&b2, "empty"), None);
    }

    #[test]
    fn dense_table_with_collisions_and_duplicates() {
        let mut d = TblDoc::new();
        for i in 0..300 {
            d.add(&format!("k{}", i % 250), &format!("v{i}")).unwrap();
        }
        let b = d.to_bytes().unwrap();
        assert_consistent(&b, &d);
        // fully packed: no spare slots, so probes are long
        assert_eq!(super::super::u32_at(&b, 4), Some(300));
        assert_eq!(lookup(&b, "k0").as_deref(), Some("v0"));
    }

    #[test]
    fn hash_size_is_kept_and_grows_on_demand() {
        let mut d = TblDoc::parse(&{
            let mut s = TblDoc::new();
            s.hash_size = 50;
            s.set("a", "1").unwrap();
            s.layout().unwrap()
        })
        .unwrap();
        assert_eq!(super::super::u32_at(&d.layout().unwrap(), 4), Some(50));
        for i in 0..60 {
            d.set(&format!("x{i}"), "t").unwrap();
        }
        assert_eq!(super::super::u32_at(&d.to_bytes().unwrap(), 4), Some(61));
    }

    #[test]
    fn rejects_unwritable_strings() {
        let mut d = TblDoc::new();
        assert!(d.set("", "x").is_err());
        assert!(d.set("k\0", "x").is_err());
        assert!(d.set("k", "\u{100}").is_err());
        assert!(d.set("k", &"x".repeat(70_000)).is_err());
        assert!(d.entries().is_empty());
        assert_eq!(TblDoc::new().to_bytes().unwrap().len(), HEADER_LEN);
    }

    #[test]
    fn parse_rejects_bad_ids() {
        let mut d = TblDoc::new();
        d.set("a", "1").unwrap();
        d.set("b", "2").unwrap();
        let mut b = d.to_bytes().unwrap();
        // point both nodes at id 0
        let base = HEADER_LEN + 2 * 2;
        for slot in 0..2 {
            let off = base + slot * NODE_LEN + 1;
            b[off..off + 2].copy_from_slice(&0u16.to_le_bytes());
        }
        assert!(TblDoc::parse(&b).is_err());
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn random_tables_lay_out_consistently(
            entries in prop::collection::vec(("[a-c0-9_]{1,6}", "[ -~\u{ff}]{0,12}"), 0..60),
            slack in 0u32..20,
        ) {
            let mut d = TblDoc::new();
            d.hash_size = u32::try_from(entries.len()).unwrap() + slack;
            for (k, v) in &entries {
                d.add(k, v).unwrap();
            }
            let b = d.layout().unwrap();
            assert_consistent(&b, &d);
            let back = TblDoc::parse(&b).unwrap();
            prop_assert_eq!(back.entries(), d.entries());
            prop_assert_eq!(back.to_bytes().unwrap(), b);
        }

        #[test]
        fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
            if let Ok(d) = TblDoc::parse(&bytes) {
                let _ = d.layout();
            }
        }

        #[test]
        fn mutated_tables_never_panic(seed in any::<u64>()) {
            let mut d = TblDoc::new();
            for i in 0..12 {
                d.add(&format!("key{i}"), &format!("text {i}")).unwrap();
            }
            let base = d.layout().unwrap();
            let mut rng = crate::testutil::Rng::new(seed);
            let bad = crate::testutil::mutate(&base, &mut rng);
            if let Ok(mut m) = TblDoc::parse(&bad) {
                let _ = m.set("zz", "yy");
                let _ = m.to_bytes();
                m.remove("key3");
                let _ = m.to_bytes();
            }
        }
    }

    #[test]
    fn corpus_round_trip_and_edit() {
        let Some(root) = crate::testutil::private_root() else {
            println!("skipped: CAPY_PRIVATE_DIR / testdata/private missing");
            return;
        };
        let (mut files, mut relayout_same, mut skipped) = (0usize, 0usize, 0usize);
        for dir in ["vanilla", "mxl"] {
            let base = root.join("extracted").join(dir);
            for path in crate::testutil::sample(crate::testutil::find_files(&base, "tbl"), 1) {
                let bytes = std::fs::read(&path).unwrap();
                let Ok(mut doc) = TblDoc::parse(&bytes) else {
                    // glyph tables (data/local/font) and the like
                    println!("not a string table: {}", path.display());
                    skipped += 1;
                    continue;
                };
                assert!(doc.to_bytes().unwrap() == bytes, "{}", path.display());
                files += 1;
                if doc.layout().unwrap() == bytes {
                    relayout_same += 1;
                }
                // edit, relayout, and check the key -> string map
                let before: Vec<_> = doc.entries().to_vec();
                doc.set("capy_test_key", "Capy test").unwrap();
                if let Some((k, _)) = before.first() {
                    doc.set(k, "changed").unwrap();
                }
                let out = doc.to_bytes().unwrap();
                assert_consistent(&out, &doc);
                let re = TblDoc::parse(&out).unwrap();
                assert_eq!(re.entries(), doc.entries(), "{}", path.display());
                assert_eq!(re.entries().len(), before.len() + 1);
            }
        }
        println!(
            "tbl corpus: {files} files round-tripped ({relayout_same} also re-lay out identically), {skipped} not string tables"
        );
    }
}
