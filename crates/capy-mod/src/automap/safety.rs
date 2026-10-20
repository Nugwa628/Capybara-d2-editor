//! Save-time safety checks for `AutoMap.txt`.
//!
//! The game's automap loader asserts that every rule has at least one cel
//! before the `-1` terminator, so a row whose `Cel1` is `-1` stops it at
//! start-up. A piece is hidden by removing its rule, never by writing `-1`.

use d2_formats::txt::TxtDoc;

use super::AutomapError;
use crate::doc_util::text;

/// Rows (0-based document rows; file line = row + 2) with a level and a tile
/// code whose `Cel1` is not a valid picture number (negative, or not an
/// integer). Empty numeric cells count as zero, as the game compiles them.
pub fn invalid_rows(doc: &TxtDoc) -> Result<Vec<usize>, AutomapError> {
    let (Some(level), Some(tile), Some(cel)) = (
        doc.column("LevelName"),
        doc.column("TileName"),
        doc.column("Cel1"),
    ) else {
        return Err(AutomapError::Table(
            "AutoMap.txt needs LevelName, TileName and Cel1 columns.".into(),
        ));
    };
    Ok((0..doc.row_count())
        .filter(|&r| {
            if text(doc, r, Some(level)).is_empty() || text(doc, r, Some(tile)).is_empty() {
                return false;
            }
            let raw = text(doc, r, Some(cel));
            if raw.is_empty() {
                return false;
            }
            raw.parse::<f64>()
                .map_or(true, |v| !v.is_finite() || v.fract() != 0.0 || v < 0.0)
        })
        .collect())
}

/// The fix: removes every row [`invalid_rows`] finds. Idempotent. Returns the
/// number of rows removed.
pub fn remove_invalid_rows(doc: &mut TxtDoc) -> Result<usize, AutomapError> {
    let bad = invalid_rows(doc)?;
    for &r in bad.iter().rev() {
        doc.delete_row(r)?;
    }
    Ok(bad.len())
}

/// Refuses to save a broken `AutoMap.txt`. Other paths are not checked.
pub fn validate_save(path: &str, bytes: &[u8]) -> Result<(), AutomapError> {
    if !path
        .replace('\\', "/")
        .eq_ignore_ascii_case("data/global/excel/automap.txt")
    {
        return Ok(());
    }
    let rows = invalid_rows(&TxtDoc::parse(bytes))?;
    if rows.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = rows.iter().take(12).map(|r| (r + 2).to_string()).collect();
    Err(AutomapError::Invalid(format!(
        "AutoMap.txt contains {} invalid rule(s), on line(s) {}. Cel1 must be a valid picture number: -1 here crashes the game while loading. Clear a piece by removing its matching rule, not by setting all cels to -1. The file was not saved.",
        rows.len(),
        lines.join(", ")
    )))
}
