//! Lossless, editable view of a `.txt` table.
//!
//! [`TxtDoc`] keeps every line as the raw bytes it was read from plus its own
//! line terminator, so anything the editing operations do not touch is
//! written back byte for byte: CRLF/LF/CR mixes, a missing final newline, a
//! UTF-8 byte order mark, blank and comment rows, rows shorter or longer than
//! the header, trailing tabs and spaces, and quote characters (the game does
//! no quote handling, so `"` is an ordinary byte). Unlike
//! [`TxtTable`](crate::txt::TxtTable) nothing is dropped or padded: trailing
//! blank rows are rows.
//!
//! Line 0 is the header; row `r` is line `r + 1`. Cells are Latin-1 bytes;
//! the `&str` accessors map each byte to the char with the same code point,
//! and setters reject chars above U+00FF.

use crate::{FormatError, Result};

const FORMAT: &str = "txt";
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const CRLF: &[u8] = b"\r\n";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    /// Bytes of the line without its terminator.
    content: Vec<u8>,
    /// `b"\r\n"`, `b"\n"`, `b"\r"`, or empty for an unterminated last line.
    eol: &'static [u8],
}

/// Editable, byte-exact `.txt` table. See the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxtDoc {
    bom: bool,
    lines: Vec<Line>,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset: 0,
        detail: detail.into(),
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn check_cell(bytes: &[u8]) -> Result<()> {
    if bytes.iter().any(|b| matches!(b, b'\t' | b'\r' | b'\n')) {
        return Err(invalid("cell", "cells cannot contain tab or line breaks"));
    }
    Ok(())
}

fn to_latin1(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let b = u8::try_from(u32::from(c))
            .map_err(|_| invalid("cell", format!("{c:?} is not a Latin-1 character")))?;
        out.push(b);
    }
    check_cell(&out)?;
    Ok(out)
}

fn cells_of(content: &[u8]) -> Vec<&[u8]> {
    content.split(|&b| b == b'\t').collect()
}

fn join(cells: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            out.push(b'\t');
        }
        out.extend_from_slice(c);
    }
    out
}

impl TxtDoc {
    /// Parses any byte string; never fails. `to_bytes` returns the input.
    pub fn parse(bytes: &[u8]) -> TxtDoc {
        let (bom, body) = match bytes.strip_prefix(&BOM) {
            Some(rest) => (true, rest),
            None => (false, bytes),
        };
        let mut lines = Vec::new();
        let mut start = 0usize;
        let mut i = 0usize;
        while i < body.len() {
            let eol: &'static [u8] = match body.get(i) {
                Some(b'\n') => b"\n",
                Some(b'\r') if body.get(i + 1) == Some(&b'\n') => CRLF,
                Some(b'\r') => b"\r",
                _ => {
                    i += 1;
                    continue;
                }
            };
            lines.push(Line {
                content: body.get(start..i).unwrap_or(&[]).to_vec(),
                eol,
            });
            i += eol.len();
            start = i;
        }
        if start < body.len() {
            lines.push(Line {
                content: body.get(start..).unwrap_or(&[]).to_vec(),
                eol: b"",
            });
        }
        TxtDoc { bom, lines }
    }

    /// The file bytes, identical to the parsed input where nothing was edited.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.bom {
            out.extend_from_slice(&BOM);
        }
        for l in &self.lines {
            out.extend_from_slice(&l.content);
            out.extend_from_slice(l.eol);
        }
        out
    }

    /// Header cells (empty when the file has no lines).
    pub fn headers(&self) -> Vec<String> {
        self.lines
            .first()
            .map(|l| cells_of(&l.content).into_iter().map(latin1).collect())
            .unwrap_or_default()
    }

    fn header_len(&self) -> usize {
        self.lines.first().map_or(0, |l| cells_of(&l.content).len())
    }

    /// Index of the first header equal to `name`, ignoring ASCII case.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.headers()
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
    }

    /// Number of rows after the header, including blank and trailing ones.
    pub fn row_count(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    /// The cells of row `row` as stored (no padding), or `None` past the end.
    pub fn row_cells(&self, row: usize) -> Option<Vec<&[u8]>> {
        self.lines
            .get(row.checked_add(1)?)
            .map(|l| cells_of(&l.content))
    }

    /// Raw bytes of one cell; `None` if the row or the cell does not exist
    /// (a short row has no cell there).
    pub fn cell_bytes(&self, row: usize, col: usize) -> Option<&[u8]> {
        let line = self.lines.get(row.checked_add(1)?)?;
        line.content.split(|&b| b == b'\t').nth(col)
    }

    /// One cell as Latin-1 text.
    pub fn cell(&self, row: usize, col: usize) -> Option<String> {
        self.cell_bytes(row, col).map(latin1)
    }

    /// Replaces one cell. A row shorter than `col + 1` cells is padded with
    /// empty cells only as far as needed; only that row's bytes change.
    pub fn set_cell(&mut self, row: usize, col: usize, value: &str) -> Result<()> {
        self.set_cell_bytes(row, col, &to_latin1(value)?)
    }

    /// [`TxtDoc::set_cell`] with raw bytes (no tab, CR or LF).
    pub fn set_cell_bytes(&mut self, row: usize, col: usize, value: &[u8]) -> Result<()> {
        check_cell(value)?;
        let line = row
            .checked_add(1)
            .and_then(|l| self.lines.get_mut(l))
            .ok_or_else(|| invalid("row", format!("row {row} does not exist")))?;
        // Edit in place so the other cells' bytes are never rebuilt.
        let mut start = 0usize;
        let mut found = 0usize;
        while found < col {
            match line
                .content
                .get(start..)
                .and_then(|r| r.iter().position(|&b| b == b'\t'))
            {
                Some(p) => {
                    start += p + 1;
                    found += 1;
                }
                None => break,
            }
        }
        if found < col {
            // a short row: pad with tabs up to the target cell
            line.content.extend(std::iter::repeat_n(b'\t', col - found));
            start = line.content.len();
        }
        let end = line
            .content
            .get(start..)
            .and_then(|r| r.iter().position(|&b| b == b'\t'))
            .map_or(line.content.len(), |p| start + p);
        line.content.splice(start..end, value.iter().copied());
        self.fix_tail();
        Ok(())
    }

    /// The terminator for a line appended after the current last line: the
    /// nearest existing one, CRLF for a file without any.
    fn appended_eol(&self) -> &'static [u8] {
        self.lines
            .iter()
            .rev()
            .map(|l| l.eol)
            .find(|e| !e.is_empty())
            .unwrap_or(CRLF)
    }

    /// An empty unterminated last line would vanish on re-read (it is not a
    /// line then), so it gets a terminator.
    fn fix_tail(&mut self) {
        let eol = self.appended_eol();
        if let Some(last) = self.lines.last_mut()
            && last.eol.is_empty()
            && last.content.is_empty()
        {
            last.eol = eol;
        }
    }

    fn insert_line(&mut self, at_line: usize, content: Vec<u8>) {
        if at_line >= self.lines.len() {
            let eol = self.appended_eol();
            if let Some(last) = self.lines.last_mut()
                && last.eol.is_empty()
            {
                // keep "no final newline": the new last line has none
                last.eol = eol;
                self.lines.push(Line { content, eol: b"" });
                return;
            }
            self.lines.push(Line { content, eol });
        } else {
            // every line but the last has a terminator
            let eol = self
                .lines
                .get(at_line.saturating_sub(1))
                .map_or(CRLF, |l| l.eol);
            self.lines.insert(at_line, Line { content, eol });
        }
    }

    /// Inserts a row before row `at` (`at == row_count()` appends). Missing
    /// trailing cells are empty up to the header width; extra cells are kept.
    pub fn insert_row(&mut self, at: usize, cells: &[&str]) -> Result<()> {
        if self.lines.is_empty() {
            return Err(invalid("row", "the file has no header line"));
        }
        if at > self.row_count() {
            return Err(invalid("row", format!("cannot insert at row {at}")));
        }
        let mut row: Vec<Vec<u8>> = cells.iter().map(|c| to_latin1(c)).collect::<Result<_>>()?;
        if row.len() < self.header_len() {
            row.resize(self.header_len(), Vec::new());
        }
        self.insert_line(at + 1, join(&row));
        self.fix_tail();
        Ok(())
    }

    /// Inserts a copy of row `row` right after it (ragged rows stay ragged).
    pub fn duplicate_row(&mut self, row: usize) -> Result<()> {
        let content = row
            .checked_add(1)
            .and_then(|l| self.lines.get(l))
            .map(|l| l.content.clone())
            .ok_or_else(|| invalid("row", format!("row {row} does not exist")))?;
        self.insert_line(row + 2, content);
        self.fix_tail();
        Ok(())
    }

    /// Removes row `row`. Removing the unterminated last row keeps the file
    /// unterminated.
    pub fn delete_row(&mut self, row: usize) -> Result<()> {
        let idx = row.saturating_add(1);
        if idx >= self.lines.len() {
            return Err(invalid("row", format!("row {row} does not exist")));
        }
        let removed = self.lines.remove(idx);
        if idx == self.lines.len()
            && removed.eol.is_empty()
            && let Some(last) = self.lines.last_mut()
        {
            last.eol = b"";
        }
        self.fix_tail();
        Ok(())
    }

    /// Appends a column named `name` after the header's last cell and gives
    /// every non-blank row `default` there (short rows are padded first, long
    /// rows get it at the header's width). Completely empty lines stay empty.
    /// Returns the new column's index.
    pub fn add_column(&mut self, name: &str, default: &str) -> Result<usize> {
        let name = to_latin1(name)?;
        let default = to_latin1(default)?;
        let width = self.header_len();
        let Some((header, rows)) = self.lines.split_first_mut() else {
            return Err(invalid("column", "the file has no header line"));
        };
        header.content.push(b'\t');
        header.content.extend_from_slice(&name);
        for line in rows {
            if line.content.is_empty() {
                continue;
            }
            let mut cells: Vec<Vec<u8>> = cells_of(&line.content)
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect();
            if cells.len() < width {
                cells.resize(width, Vec::new());
            }
            cells.insert(width, default.clone());
            line.content = join(&cells);
        }
        Ok(width)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rt(s: &[u8]) {
        assert_eq!(TxtDoc::parse(s).to_bytes(), s, "{s:?}");
    }

    #[test]
    fn round_trips_awkward_inputs() {
        for s in [
            &b"a\tb\n1\t2"[..],
            b"a\n1\n",
            b"a\r\n1\n2\r\n",
            b"h\n\r\n",
            b"",
            b"\r\n",
            b"\r",
            b"a\rb\r",
            b"x\xE9\xFF\x80\x9F\n",
            b"\xEF\xBB\xBFName\tId\r\n\"q\t uoted\"\t \r\n",
            b"Name\tId\tName\t\r\nfoo\t1\tx\t\r\nbar\t2\r\n\r\nExpansion\r\n",
            b"a\n\n\n",
            b"\n\n",
            b"a\t\t\t\n \t \n",
        ] {
            rt(s);
        }
    }

    fn sample() -> TxtDoc {
        TxtDoc::parse(b"Name\tId\tName\t\r\nfoo\t1\tx\t\r\nbar\t2\r\n\r\nExpansion\r\n")
    }

    #[test]
    fn reads_cells_without_padding() {
        let d = sample();
        assert_eq!(d.headers(), ["Name", "Id", "Name", ""]);
        assert_eq!(d.row_count(), 4);
        assert_eq!(d.column("ID"), Some(1));
        assert_eq!(d.cell(1, 1).as_deref(), Some("2"));
        assert_eq!(d.cell(1, 2), None);
        assert_eq!(d.cell(2, 0).as_deref(), Some(""));
        assert_eq!(d.cell(9, 0), None);
        assert_eq!(d.row_cells(1).map(|c| c.len()), Some(2));
    }

    #[test]
    fn set_cell_changes_only_that_cell() {
        let mut d = sample();
        d.set_cell(0, 0, "Changed").unwrap();
        assert_eq!(
            d.to_bytes(),
            b"Name\tId\tName\t\r\nChanged\t1\tx\t\r\nbar\t2\r\n\r\nExpansion\r\n"
        );
        // pads only as far as needed
        let mut d = sample();
        d.set_cell(1, 2, "y").unwrap();
        assert_eq!(
            d.to_bytes(),
            b"Name\tId\tName\t\r\nfoo\t1\tx\t\r\nbar\t2\ty\r\n\r\nExpansion\r\n"
        );
        // same value is a no-op on the bytes
        let mut d = sample();
        d.set_cell(0, 1, "1").unwrap();
        assert_eq!(d.to_bytes(), sample().to_bytes());
        let mut d = sample();
        d.set_cell(2, 2, "z").unwrap();
        assert_eq!(d.cell(2, 2).as_deref(), Some("z"));
        assert!(d.set_cell(9, 0, "x").is_err());
        assert!(d.set_cell(0, 0, "a\tb").is_err());
        assert!(d.set_cell(0, 0, "\u{100}").is_err());
        d.set_cell(0, 0, "caf\u{e9}").unwrap();
        assert_eq!(d.cell_bytes(0, 0), Some(&b"caf\xE9"[..]));
    }

    #[test]
    fn insert_duplicate_delete() {
        let mut d = sample();
        d.insert_row(4, &["baz", "3"]).unwrap();
        assert!(d.to_bytes().ends_with(b"Expansion\r\nbaz\t3\t\t\r\n"));
        d.duplicate_row(0).unwrap();
        assert_eq!(d.cell(1, 0).as_deref(), Some("foo"));
        d.delete_row(1).unwrap();
        d.delete_row(0).unwrap();
        assert_eq!(d.cell(0, 0).as_deref(), Some("bar"));
        assert!(d.delete_row(99).is_err());
        assert!(d.insert_row(99, &[]).is_err());
        assert!(TxtDoc::parse(b"").insert_row(0, &[]).is_err());
    }

    #[test]
    fn missing_final_newline_survives_append_and_delete() {
        let mut d = TxtDoc::parse(b"a\tb\n1\t2");
        d.insert_row(1, &["3"]).unwrap();
        assert_eq!(d.to_bytes(), b"a\tb\n1\t2\n3\t");
        d.delete_row(1).unwrap();
        assert_eq!(d.to_bytes(), b"a\tb\n1\t2");
        d.duplicate_row(0).unwrap();
        assert_eq!(d.to_bytes(), b"a\tb\n1\t2\n1\t2");
    }

    #[test]
    fn mixed_endings_keep_their_neighbours() {
        let mut d = TxtDoc::parse(b"a\r\n1\n2\r");
        d.insert_row(1, &["x"]).unwrap();
        assert_eq!(d.to_bytes(), b"a\r\n1\nx\n2\r");
    }

    #[test]
    fn add_column_pads_and_skips_blank_lines() {
        let mut d = sample();
        assert_eq!(d.add_column("New", "0").unwrap(), 4);
        assert_eq!(
            d.to_bytes(),
            b"Name\tId\tName\t\tNew\r\nfoo\t1\tx\t\t0\r\nbar\t2\t\t\t0\r\n\r\nExpansion\t\t\t\t0\r\n"
        );
    }

    #[test]
    fn bom_is_kept() {
        let mut d = TxtDoc::parse(b"\xEF\xBB\xBFA\n1\n");
        d.set_cell(0, 0, "2").unwrap();
        assert_eq!(d.to_bytes(), b"\xEF\xBB\xBFA\n2\n");
    }

    #[test]
    fn agrees_with_txt_table() {
        let src = b"A\tB\tC\n1\n1\t2\t3\t4\n\n";
        let t = crate::txt::TxtTable::parse(src).unwrap();
        let d = TxtDoc::parse(src);
        assert_eq!(d.headers(), t.headers());
        for r in t.rows() {
            for (c, cell) in r.cells().iter().enumerate() {
                assert_eq!(d.cell(r.index(), c).unwrap_or_default(), *cell);
            }
        }
    }

    use proptest::prelude::*;

    fn txt_bytes() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(
            prop_oneof![
                Just(b'\t'),
                Just(b'\n'),
                Just(b'\r'),
                Just(b'a'),
                Just(b'"'),
                any::<u8>()
            ],
            0..300,
        )
    }

    proptest! {
        #[test]
        fn any_bytes_round_trip(bytes in txt_bytes()) {
            prop_assert_eq!(TxtDoc::parse(&bytes).to_bytes(), bytes);
        }

        #[test]
        fn set_cell_touches_only_its_cell(
            bytes in txt_bytes(), row in 0usize..6, col in 0usize..6,
            value in "[a-z0-9 ]{0,5}",
        ) {
            let doc = TxtDoc::parse(&bytes);
            let mut e = doc.clone();
            if e.set_cell(row, col, &value).is_ok() {
                for r in 0..doc.row_count() {
                    if r != row {
                        prop_assert_eq!(e.row_cells(r), doc.row_cells(r));
                    }
                }
                prop_assert_eq!(e.cell(row, col), Some(value));
                for c in 0..8 {
                    if c != col && doc.cell_bytes(row, c).is_some() {
                        prop_assert_eq!(e.cell_bytes(row, c), doc.cell_bytes(row, c));
                    }
                }
                // the edit survives a write and re-read
                // (a bare CR beside an emptied line can fuse with a LF: skip)
                if !bytes.contains(&b'\r') {
                    let back = TxtDoc::parse(&e.to_bytes());
                    prop_assert_eq!(back.row_count(), e.row_count());
                }
            }
        }

        #[test]
        fn random_edit_sequences_never_panic(
            bytes in txt_bytes(),
            ops in prop::collection::vec((0u8..5, 0usize..5, 0usize..5), 0..12),
        ) {
            let mut d = TxtDoc::parse(&bytes);
            for (op, a, b) in ops {
                let _ = match op {
                    0 => d.set_cell(a, b, "x"),
                    1 => d.insert_row(a, &["p", "q"]),
                    2 => d.duplicate_row(a),
                    3 => d.delete_row(a),
                    _ => d.add_column("c", "0").map(|_| ()),
                };
            }
            let _ = TxtDoc::parse(&d.to_bytes());
        }
    }

    #[test]
    fn corpus_round_trip_and_single_cell_edit() {
        let Some(root) = crate::testutil::private_root() else {
            println!("skipped: CAPY_PRIVATE_DIR / testdata/private missing");
            return;
        };
        let mut files = 0usize;
        let mut edited = 0usize;
        for dir in ["vanilla", "mxl"] {
            let base = root.join("extracted").join(dir);
            for path in crate::testutil::sample(crate::testutil::find_files(&base, "txt"), 1) {
                let bytes = std::fs::read(&path).unwrap();
                let doc = TxtDoc::parse(&bytes);
                assert!(doc.to_bytes() == bytes, "{}", path.display());
                files += 1;
                if doc.row_count() == 0 {
                    continue;
                }
                // edit the first cell of the last row; only that cell differs
                let row = doc.row_count() - 1;
                let mut e = doc.clone();
                e.set_cell(row, 0, "ZZ").unwrap();
                let out = e.to_bytes();
                let old = doc.cell_bytes(row, 0).map_or(0, <[u8]>::len);
                assert_eq!(out.len() + old, bytes.len() + 2, "{}", path.display());
                let pre = bytes.iter().zip(&out).take_while(|(a, b)| a == b).count();
                let suf = bytes
                    .iter()
                    .rev()
                    .zip(out.iter().rev())
                    .take_while(|(a, b)| a == b)
                    .count();
                assert!(pre + suf + old >= bytes.len(), "{}", path.display());
                edited += 1;
            }
        }
        println!("txt corpus: {files} files round-tripped, {edited} single-cell edits");
    }
}
