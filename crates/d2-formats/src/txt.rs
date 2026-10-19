//! Tab-separated excel tables (`data\global\excel\*.txt`).
//!
//! The files are Latin-1 (every byte maps to the char with the same code
//! point), tab-separated, CRLF or LF terminated. The first line names the
//! columns. Rows shorter than the header are padded with empty strings; longer
//! rows keep their extra cells. Trailing all-empty rows are dropped, interior
//! empty rows are kept so row indices match the file. A leading UTF-8 byte
//! order mark (which the game would not accept but editors add) is ignored.
//!
//! Padding is capped: a file whose short rows would need more than four
//! empty cells per byte of input (plus about a million) is rejected as
//! `Invalid`. Shipped tables pad well under one cell per byte; the cap stops
//! a header of a million tabs followed by a million blank lines from
//! allocating a trillion cells.

use crate::{FormatError, Result};

const FORMAT: &str = "txt";
/// Padding cells allowed per input byte, on top of [`PAD_BUDGET_FLOOR`].
const PAD_BUDGET_PER_BYTE: usize = 4;
const PAD_BUDGET_FLOOR: usize = 1 << 20;

/// Tab-separated excel table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxtTable {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl TxtTable {
    /// Parses a `.txt` table. An empty input yields a table with no headers
    /// and no rows. The only error is `Invalid` for a file whose short rows
    /// would need an absurd number of padding cells (see the module docs).
    pub fn parse(bytes: &[u8]) -> Result<TxtTable> {
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        let mut lines = split_lines(bytes).into_iter();
        let headers = lines.next().unwrap_or_default();
        let mut rows: Vec<Vec<String>> = lines.collect();
        while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
            rows.pop();
        }
        let padding = rows.iter().fold(0usize, |acc, row| {
            acc.saturating_add(headers.len().saturating_sub(row.len()))
        });
        let budget = bytes
            .len()
            .saturating_mul(PAD_BUDGET_PER_BYTE)
            .saturating_add(PAD_BUDGET_FLOOR);
        if padding > budget {
            return Err(FormatError::Invalid {
                format: FORMAT,
                what: "row widths",
                offset: 0,
                detail: format!(
                    "padding {} rows to {} columns needs {padding} empty cells (limit {budget})",
                    rows.len(),
                    headers.len()
                ),
            });
        }
        for row in &mut rows {
            if row.len() < headers.len() {
                row.resize(headers.len(), String::new());
            }
        }
        Ok(TxtTable { headers, rows })
    }

    /// Builds a table from parts; short rows are padded like [`TxtTable::parse`].
    pub fn from_parts(headers: Vec<String>, mut rows: Vec<Vec<String>>) -> TxtTable {
        for row in &mut rows {
            if row.len() < headers.len() {
                row.resize(headers.len(), String::new());
            }
        }
        TxtTable { headers, rows }
    }

    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    /// Index of the first header equal to `name`, ignoring ASCII case.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
    }

    /// Number of data rows (the header is not counted).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, index: usize) -> Option<Row<'_>> {
        (index < self.rows.len()).then_some(Row { table: self, index })
    }

    pub fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        (0..self.rows.len()).map(|index| Row { table: self, index })
    }
}

/// A borrowed data row.
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    table: &'a TxtTable,
    index: usize,
}

impl<'a> Row<'a> {
    /// Zero-based index among the data rows.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The cell under header `column` (ASCII case-insensitive, first match).
    pub fn get(&self, column: &str) -> Option<&'a str> {
        let col = self.table.column(column)?;
        self.cells().get(col).map(String::as_str)
    }

    /// The cell parsed as an integer; `None` when the column is missing, the
    /// cell is empty or it is not a number. Surrounding whitespace is ignored.
    pub fn get_i64(&self, column: &str) -> Option<i64> {
        self.get(column)?.trim().parse().ok()
    }

    pub fn cells(&self) -> &'a [String] {
        self.table
            .rows
            .get(self.index)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Splits into lines (CRLF, LF or lone CR) of Latin-1 tab-separated cells.
fn split_lines(bytes: &[u8]) -> Vec<Vec<String>> {
    let mut lines = Vec::new();
    if bytes.is_empty() {
        return lines;
    }
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut iter = bytes.iter().copied().peekable();
    while let Some(b) = iter.next() {
        match b {
            b'\t' => row.push(std::mem::take(&mut cell)),
            b'\r' | b'\n' => {
                if b == b'\r' && iter.peek() == Some(&b'\n') {
                    iter.next();
                }
                row.push(std::mem::take(&mut cell));
                lines.push(std::mem::take(&mut row));
            }
            other => cell.push(char::from(other)),
        }
    }
    // A final line without a terminator.
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        lines.push(row);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> TxtTable {
        TxtTable::parse(bytes).expect("txt parse never fails")
    }

    #[test]
    fn crlf_and_lf_agree() {
        let crlf = parse(b"Name\tId\r\nfoo\t1\r\nbar\t2\r\n");
        let lf = parse(b"Name\tId\nfoo\t1\nbar\t2\n");
        assert_eq!(crlf, lf);
        assert_eq!(crlf.len(), 2);
        assert_eq!(crlf.headers(), ["Name", "Id"]);
        let row = crlf.row(1).expect("row 1");
        assert_eq!(row.get("name"), Some("bar"));
        assert_eq!(row.get_i64("ID"), Some(2));
        assert_eq!(row.index(), 1);
    }

    #[test]
    fn trailing_blank_lines_are_dropped() {
        let t = parse(b"A\tB\r\n1\t2\r\n\r\n\t\r\n");
        assert_eq!(t.len(), 1);
        let t = parse(b"A\tB\n1\t2");
        assert_eq!(t.len(), 1);
        assert_eq!(t.row(0).and_then(|r| r.get("B")), Some("2"));
    }

    #[test]
    fn interior_blank_rows_are_kept() {
        let t = parse(b"A\tB\n1\t2\n\n3\t4\n");
        assert_eq!(t.len(), 3);
        assert_eq!(
            t.row(1).map(|r| r.cells().to_vec()),
            Some(vec![String::new(); 2])
        );
    }

    #[test]
    fn ragged_rows_are_padded() {
        let t = parse(b"A\tB\tC\n1\n1\t2\t3\t4\n");
        assert_eq!(t.row(0).map(|r| r.cells().len()), Some(3));
        assert_eq!(t.row(0).and_then(|r| r.get("C")), Some(""));
        assert_eq!(t.row(0).and_then(|r| r.get_i64("C")), None);
        // extra cells are kept
        assert_eq!(t.row(1).map(|r| r.cells().len()), Some(4));
    }

    #[test]
    fn latin1_bytes_decode_one_to_one() {
        let t = parse(b"N\ncaf\xE9\n\xFF\xFE\n");
        assert_eq!(t.row(0).and_then(|r| r.get("N")), Some("caf\u{e9}"));
        assert_eq!(t.row(1).and_then(|r| r.get("N")), Some("\u{ff}\u{fe}"));
    }

    #[test]
    fn duplicate_headers_first_match_wins() {
        let t = parse(b"X\tx\tY\n1\t2\t3\n");
        assert_eq!(t.column("x"), Some(0));
        assert_eq!(t.row(0).and_then(|r| r.get_i64("X")), Some(1));
        assert_eq!(t.column("y"), Some(2));
        assert_eq!(t.column("nope"), None);
    }

    #[test]
    fn empty_tables() {
        let t = parse(b"");
        assert!(t.is_empty());
        assert!(t.headers().is_empty());
        assert!(t.row(0).is_none());
        let t = parse(b"A\tB\r\n");
        assert!(t.is_empty());
        assert_eq!(t.headers().len(), 2);
        let t = parse(b"\n\n");
        assert!(t.is_empty());
    }

    #[test]
    fn numbers_and_missing_columns() {
        let t = parse(b"A\tB\n 12 \tx\n-5\t\n");
        let r0 = t.row(0).expect("row");
        assert_eq!(r0.get_i64("A"), Some(12));
        assert_eq!(r0.get_i64("B"), None);
        assert_eq!(r0.get_i64("Z"), None);
        assert_eq!(t.row(1).and_then(|r| r.get_i64("A")), Some(-5));
        assert_eq!(t.rows().count(), 2);
    }

    #[test]
    fn utf8_bom_is_ignored() {
        let t = parse(b"\xEF\xBB\xBFName\nx\n");
        assert_eq!(t.headers(), ["Name"]);
    }

    #[test]
    fn from_parts_pads() {
        let t = TxtTable::from_parts(vec!["A".into(), "B".into()], vec![vec!["1".into()]]);
        assert_eq!(t.row(0).and_then(|r| r.get("B")), Some(""));
    }
}
