//! Bringing `AutoMap.txt` rows from another mod. Its `LevelName` column names
//! a level type by the number (or name) it has in THAT mod, which is usually
//! another level type here (46 is one mod's Guild and another's Poisoned
//! Well). Each source `LevelName` is mapped to one of this mod's level types
//! before the rows are added.

use std::collections::HashMap;

use d2_formats::txt::TxtDoc;

use super::{AutomapError, Change, EditSummary};

/// Rows read from another mod's table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomapSource {
    /// Header cells, by position (AutoMap.txt names a column twice).
    pub columns: Vec<String>,
    /// Data rows (blank lines skipped), cells as written.
    pub rows: Vec<Vec<String>>,
    /// Each `LevelName` with its row count and a few of its tile codes.
    pub levels: Vec<SourceLevel>,
}

/// One level name found in a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLevel {
    /// `LevelName` as written.
    pub name: String,
    /// Rows under it.
    pub rows: usize,
    /// Up to 8 distinct tile codes.
    pub codes: Vec<String>,
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Reads AutoMap rows (a whole table, or a header and some rows). `None`
/// when the data is not an AutoMap table.
pub fn read_rows(bytes: &[u8]) -> Option<AutomapSource> {
    let text = latin1(bytes);
    let mut lines = text.lines();
    let columns: Vec<String> = lines.next()?.split('\t').map(str::to_string).collect();
    let at = columns.iter().position(|c| c == "LevelName")?;
    let code = columns.iter().position(|c| c == "TileName")?;
    let rows: Vec<Vec<String>> = lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    let mut levels: Vec<SourceLevel> = Vec::new();
    for r in &rows {
        let name = r.get(at).map_or("", |s| s.trim()).to_string();
        let i = levels
            .iter()
            .position(|l| l.name == name)
            .unwrap_or_else(|| {
                levels.push(SourceLevel {
                    name,
                    rows: 0,
                    codes: Vec::new(),
                });
                levels.len() - 1
            });
        levels[i].rows += 1;
        if let Some(c) = r.get(code).map(|s| s.trim()).filter(|s| !s.is_empty())
            && !levels[i].codes.iter().any(|x| x == c)
            && levels[i].codes.len() < 8
        {
            levels[i].codes.push(c.to_string());
        }
    }
    Some(AutomapSource {
        columns,
        rows,
        levels,
    })
}

/// Whether the table names level types by number (PD2) rather than by name
/// (the game's own).
pub fn names_by_number(target: &TxtDoc) -> bool {
    let Some(at) = target.column("LevelName") else {
        return false;
    };
    let (mut num, mut text) = (0, 0);
    for r in 0..target.row_count() {
        let v = target.cell(r, at).unwrap_or_default();
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        if v.bytes().all(|b| b.is_ascii_digit()) {
            num += 1;
        } else {
            text += 1;
        }
    }
    num > text
}

/// Result of [`merge_rows`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeResult {
    /// Rows added.
    pub added: usize,
    /// Rows skipped because the table already had them.
    pub already: usize,
    /// Row insertions made, in order.
    pub summary: EditSummary,
}

fn is_blank(doc: &TxtDoc, row: usize) -> bool {
    doc.row_cells(row)
        .is_none_or(|cs| cs.iter().all(|c| c.iter().all(u8::is_ascii_whitespace)))
}

/// Adds the source rows to `target`: each source `LevelName` becomes
/// `mapping[name]` (the value to write; `None` or absent leaves those rows
/// out). Rows already there (same cells, trimmed, over the shared columns) are
/// skipped, so importing twice adds nothing the second time. New rows go after
/// the last data row; existing bytes are untouched.
pub fn merge_rows(
    target: &mut TxtDoc,
    src: &AutomapSource,
    mapping: &HashMap<String, Option<String>>,
) -> Result<MergeResult, AutomapError> {
    let Some(at) = src.columns.iter().position(|c| c == "LevelName") else {
        return Err(AutomapError::Table("source has no LevelName column".into()));
    };
    let header = target.headers();
    let same_header = header.len() == src.columns.len() && header == src.columns;
    // Source cell index for each target column.
    let map: Vec<Option<usize>> = header
        .iter()
        .enumerate()
        .map(|(i, h)| {
            if same_header {
                Some(i)
            } else {
                src.columns.iter().position(|c| c == h)
            }
        })
        .collect();
    let mut out = MergeResult::default();
    for r in &src.rows {
        let to = match mapping.get(r.get(at).map_or("", |s| s.trim())) {
            Some(Some(to)) => to.clone(),
            _ => continue,
        };
        let mut row = r.clone();
        row.resize(src.columns.len(), String::new());
        row[at] = to;
        let cells: Vec<String> = map
            .iter()
            .map(|m| m.and_then(|i| row.get(i).cloned()).unwrap_or_default())
            .collect();
        if cells.iter().any(|c| c.contains(['\t', '\r', '\n'])) {
            return Err(AutomapError::Invalid(
                "row contains tabs or line breaks".into(),
            ));
        }
        let exists = (0..target.row_count()).any(|t| {
            !is_blank(target, t)
                && map.iter().enumerate().all(|(c, m)| {
                    m.is_none() || target.cell(t, c).unwrap_or_default().trim() == cells[c].trim()
                })
        });
        if exists {
            out.already += 1;
            continue;
        }
        let mut pos = target.row_count();
        while pos > 0 && is_blank(target, pos - 1) {
            pos -= 1;
        }
        let refs: Vec<&str> = cells.iter().map(String::as_str).collect();
        target.insert_row(pos, &refs)?;
        out.summary.changes.push(Change::RowInserted { row: pos });
        out.added += 1;
    }
    out.summary.summary = format!(
        "AutoMap.txt: {} row(s) imported, {} already there",
        out.added, out.already
    );
    Ok(out)
}
