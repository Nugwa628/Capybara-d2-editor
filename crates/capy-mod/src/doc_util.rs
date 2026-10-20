//! Small helpers over [`TxtDoc`] shared by the automap and compatibility
//! modules: trimmed cell reads, JS-like number parsing and data-row lists.

use d2_formats::txt::TxtDoc;

/// Trimmed text of a cell; empty when the row, column or cell is missing.
pub(crate) fn text(doc: &TxtDoc, row: usize, col: Option<usize>) -> String {
    col.and_then(|c| doc.cell(row, c))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// `Number(x) || 0` as the reference tool reads numeric cells.
pub(crate) fn parse_num(s: &str) -> i64 {
    let s = s.trim();
    s.parse::<i64>()
        .ok()
        .or_else(|| {
            s.parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(|f| f as i64)
        })
        .unwrap_or(0)
}

/// Numeric cell by column name (0 when absent or not a number).
pub(crate) fn num(doc: &TxtDoc, row: usize, col: &str) -> i64 {
    parse_num(&text(doc, row, doc.column(col)))
}

/// Trimmed cell by column name.
pub(crate) fn get(doc: &TxtDoc, row: usize, col: &str) -> String {
    text(doc, row, doc.column(col))
}

/// Rows the game compiles into records: not blank and not the "Expansion"
/// separator. The record number of a row is its position in this list.
pub(crate) fn data_rows(doc: &TxtDoc) -> Vec<usize> {
    (0..doc.row_count())
        .filter(|&r| {
            let cells = doc.row_cells(r).unwrap_or_default();
            cells.iter().any(|c| !c.iter().all(u8::is_ascii_whitespace))
                && !cells.first().is_some_and(|c| {
                    String::from_utf8_lossy(c)
                        .trim()
                        .eq_ignore_ascii_case("expansion")
                })
        })
        .collect()
}

/// Sets a cell by column name when it differs (trimmed). Returns the old value
/// when a change was made; `None` when the column is missing or unchanged.
pub(crate) fn set_if_different(
    doc: &mut TxtDoc,
    row: usize,
    col: &str,
    value: &str,
) -> Option<String> {
    let c = doc.column(col)?;
    let old = doc.cell(row, c).unwrap_or_default();
    if old.trim() == value {
        return None;
    }
    doc.set_cell(row, c, value).ok()?;
    Some(old)
}
