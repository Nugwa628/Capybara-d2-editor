//! Small helpers over [`TxtDoc`] shared by the level-tool modules.

use d2_formats::txt::TxtDoc;

use crate::lvl_error::Result;

/// JavaScript's `Number(s) || 0`, truncated.
pub(crate) fn num(s: &str) -> i64 {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .map_or(0, |f| f as i64)
}

/// A bit mask cell (`>>> 0`).
pub(crate) fn mask(s: &str) -> u32 {
    num(s) as u64 as u32
}

pub(crate) fn has(doc: &TxtDoc, col: &str) -> bool {
    doc.column(col).is_some()
}

/// Cell text by column name; empty when the row or column is missing.
pub(crate) fn cell(doc: &TxtDoc, row: usize, col: &str) -> String {
    doc.column(col)
        .and_then(|c| doc.cell(row, c))
        .unwrap_or_default()
}

pub(crate) fn cell_num(doc: &TxtDoc, row: usize, col: &str) -> i64 {
    num(&cell(doc, row, col))
}

/// Sets a cell by column name. Returns false (nothing done) when the column is
/// missing or the value is already there.
pub(crate) fn set(doc: &mut TxtDoc, row: usize, col: &str, value: &str) -> Result<bool> {
    let Some(c) = doc.column(col) else {
        return Ok(false);
    };
    if doc.cell(row, c).unwrap_or_default() == value {
        return Ok(false);
    }
    doc.set_cell(row, c, value)?;
    Ok(true)
}

fn blank(doc: &TxtDoc, row: usize) -> bool {
    doc.row_cells(row)
        .is_none_or(|cells| cells.iter().all(|c| c.iter().all(u8::is_ascii_whitespace)))
}

/// Rows the game compiles into records: not blank and not the `Expansion`
/// separator (the reference editor's rule).
pub(crate) fn data_rows(doc: &TxtDoc) -> Vec<usize> {
    (0..doc.row_count())
        .filter(|&r| {
            !blank(doc, r)
                && doc
                    .cell(r, 0)
                    .is_none_or(|c| !c.trim().eq_ignore_ascii_case("expansion"))
        })
        .collect()
}

/// Row index of the record with the given index.
pub(crate) fn row_of_record(doc: &TxtDoc, record: usize) -> Option<usize> {
    data_rows(doc).get(record).copied()
}

/// Appends a full row before trailing blank lines (so it stays a record).
pub(crate) fn append_row(doc: &mut TxtDoc, cells: &[String]) -> Result<usize> {
    let mut at = doc.row_count();
    while at > 0 && blank(doc, at - 1) {
        at -= 1;
    }
    let refs: Vec<&str> = cells.iter().map(String::as_str).collect();
    doc.insert_row(at, &refs)?;
    Ok(at)
}

/// Appends a row of empty cells with `values` set by column name.
pub(crate) fn append_values(doc: &mut TxtDoc, values: &[(String, String)]) -> Result<usize> {
    let headers = doc.headers();
    let mut row = vec![String::new(); headers.len()];
    for (k, v) in values {
        if let Some(c) = doc.column(k) {
            row[c] = v.clone();
        }
    }
    append_row(doc, &row)
}

/// A copy of row `row` as (column name, value) pairs.
pub(crate) fn row_values(doc: &TxtDoc, row: usize) -> Vec<(String, String)> {
    doc.headers()
        .into_iter()
        .enumerate()
        .map(|(i, h)| (h, doc.cell(row, i).unwrap_or_default()))
        .collect()
}

/// A tile path relative to `data/global/tiles`, forward slashes, case kept.
pub(crate) fn tiles_rel(p: &str) -> String {
    let s = p.replace('\\', "/");
    let s = s.trim_start_matches('/');
    if s.to_lowercase().starts_with("data/global/tiles/") {
        s["data/global/tiles/".len()..].to_owned()
    } else {
        s.to_owned()
    }
}

/// `tiles_rel` normalised for comparison.
pub(crate) fn rel_key(p: &str) -> String {
    tiles_rel(p).to_lowercase()
}

/// Value of a File slot (`''` and `0` count as empty).
pub(crate) fn file_value(v: &str) -> Option<String> {
    let t = v.trim();
    (!t.is_empty() && t != "0").then(|| t.to_owned())
}
