//! Typed views of the level tables the editor needs: `LvlTypes`, `LvlPrest`
//! and `Levels`.
//!
//! The functions take a [`TxtTable`], so they work on parsed `.txt` files and
//! on the output of [`crate::excel::decode_bin`] alike. Conventions:
//!
//! * `row` is the index into the table (separator rows included).
//! * A row whose cells are all empty is skipped; the `Expansion` separator row
//!   (first cell `Expansion`, the rest empty) is skipped and marks every later
//!   row as an expansion row. A table with an `Expansion` column also marks a
//!   row as expansion when that cell is non-zero.
//! * A blank numeric cell reads as 0 (what the game's compiler writes); a cell
//!   that is neither blank nor a number is an `Invalid` error.
//! * Columns missing from the table are an error only where documented;
//!   otherwise the field takes its zero value. Decoded `.bin` tables never
//!   store `Name` (LvlTypes, LvlPrest), so `name` is empty there.
//! * File columns are accepted with or without the space (`File 1`, `File1`).

use crate::txt::{Row, TxtTable};
use crate::{FormatError, Result};

const FORMAT: &str = "tables";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LvlType {
    pub row: usize,
    pub id: u32,
    pub name: String,
    /// The `Act` column as stored (1-based in `LvlTypes`).
    pub act: u8,
    /// File1..File32 by slot; None for empty or "0".
    pub files: Vec<Option<String>>,
    pub expansion: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LvlPrest {
    pub row: usize,
    pub def: u32,
    pub level_id: i32,
    pub name: String,
    pub size_x: i32,
    pub size_y: i32,
    pub populate: bool,
    pub logicals: bool,
    pub outdoors: bool,
    pub animate: bool,
    pub kill_edge: bool,
    pub fill_blanks: bool,
    pub auto_map: bool,
    pub scan: bool,
    pub pops: i32,
    pub pop_pad: i32,
    /// File1..File6 by slot; None for empty or "0". Paths relative to data\global\tiles.
    pub files: Vec<Option<String>>,
    pub dt1_mask: u32,
    pub expansion: bool,
}

/// One `Levels` row. `name` is the `LevelName` column (the string-table key
/// shown in game, e.g. `Rogue Encampment`), falling back to `Name`; the
/// `.bin` form only stores `LevelName`. `act` is 0-based as in `Levels.txt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub row: usize,
    pub id: u32,
    pub name: String,
    pub act: u8,
    pub pal: u8,
    pub level_type: u32,
    pub drlg_type: u32,
}

fn invalid(what: &'static str, row: usize, detail: String) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset: row,
        detail,
    }
}

fn missing(column: &str) -> FormatError {
    FormatError::MissingColumn {
        format: FORMAT,
        column: column.to_string(),
    }
}

fn has_column(table: &TxtTable, names: &[&str]) -> bool {
    names.iter().any(|n| table.column(n).is_some())
}

fn cell<'a>(row: &Row<'a>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| row.get(n))
}

fn is_blank_row(row: &Row<'_>) -> bool {
    row.cells().iter().all(|c| c.trim().is_empty())
}

fn is_separator(row: &Row<'_>) -> bool {
    let cells = row.cells();
    cells
        .first()
        .is_some_and(|c| c.trim().eq_ignore_ascii_case("Expansion"))
        && cells.iter().skip(1).all(|c| c.trim().is_empty())
}

/// The integer in the first existing column among `names`; blank is 0 and a
/// missing column is `None`.
fn int(row: &Row<'_>, what: &'static str, names: &[&str]) -> Result<Option<i64>> {
    let Some(text) = cell(row, names) else {
        return Ok(None);
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(Some(0));
    }
    text.parse::<i64>()
        .map(Some)
        .map_err(|_| invalid(what, row.index(), format!("{text:?} is not a number")))
}

fn require_int(row: &Row<'_>, what: &'static str, names: &[&str]) -> Result<i64> {
    int(row, what, names)?.ok_or_else(|| missing(names.first().copied().unwrap_or(what)))
}

fn narrow<T: TryFrom<i64>>(row: &Row<'_>, what: &'static str, value: i64) -> Result<T> {
    T::try_from(value).map_err(|_| invalid(what, row.index(), format!("{value} is out of range")))
}

fn opt<T: TryFrom<i64> + Default>(row: &Row<'_>, what: &'static str, names: &[&str]) -> Result<T> {
    match int(row, what, names)? {
        Some(v) => narrow(row, what, v),
        None => Ok(T::default()),
    }
}

fn flag(row: &Row<'_>, what: &'static str, names: &[&str]) -> Result<bool> {
    Ok(int(row, what, names)?.unwrap_or(0) != 0)
}

/// `Some(path)` for a non-empty cell other than the "0" placeholder.
fn file_slot(row: &Row<'_>, n: usize) -> Option<String> {
    let spaced = format!("File {n}");
    let packed = format!("File{n}");
    let text = cell(row, &[spaced.as_str(), packed.as_str()])?.trim();
    (!text.is_empty() && text != "0").then(|| text.to_string())
}

fn expansion(row: &Row<'_>, after_separator: bool, has_col: bool) -> Result<bool> {
    let flagged = if has_col {
        flag(row, "Expansion", &["Expansion"])?
    } else {
        false
    };
    Ok(after_separator || flagged)
}

pub fn lvl_types(table: &TxtTable) -> Result<Vec<LvlType>> {
    if !has_column(table, &["Id"]) {
        return Err(missing("Id"));
    }
    let has_expansion = has_column(table, &["Expansion"]);
    let mut out = Vec::new();
    let mut after_separator = false;
    for row in table.rows() {
        if is_separator(&row) {
            after_separator = true;
            continue;
        }
        if is_blank_row(&row) {
            continue;
        }
        let id = require_int(&row, "Id", &["Id"])?;
        out.push(LvlType {
            row: row.index(),
            id: narrow(&row, "Id", id)?,
            name: row.get("Name").unwrap_or("").to_string(),
            act: opt(&row, "Act", &["Act"])?,
            files: (1..=32).map(|n| file_slot(&row, n)).collect(),
            expansion: expansion(&row, after_separator, has_expansion)?,
        });
    }
    Ok(out)
}

pub fn lvl_prests(table: &TxtTable) -> Result<Vec<LvlPrest>> {
    for required in ["Def", "LevelId", "Dt1Mask"] {
        if !has_column(table, &[required]) {
            return Err(missing(required));
        }
    }
    let has_expansion = has_column(table, &["Expansion"]);
    let mut out = Vec::new();
    let mut after_separator = false;
    for row in table.rows() {
        if is_separator(&row) {
            after_separator = true;
            continue;
        }
        if is_blank_row(&row) {
            continue;
        }
        let def = require_int(&row, "Def", &["Def"])?;
        let level_id = require_int(&row, "LevelId", &["LevelId"])?;
        let mask = require_int(&row, "Dt1Mask", &["Dt1Mask"])?;
        // The mask is a 32-bit pattern; accept it written signed or unsigned.
        let dt1_mask = if (i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(&mask) {
            mask as u32
        } else {
            return Err(invalid(
                "Dt1Mask",
                row.index(),
                format!("{mask} does not fit 32 bits"),
            ));
        };
        out.push(LvlPrest {
            row: row.index(),
            def: narrow(&row, "Def", def)?,
            level_id: narrow(&row, "LevelId", level_id)?,
            name: row.get("Name").unwrap_or("").to_string(),
            size_x: opt(&row, "SizeX", &["SizeX"])?,
            size_y: opt(&row, "SizeY", &["SizeY"])?,
            populate: flag(&row, "Populate", &["Populate"])?,
            logicals: flag(&row, "Logicals", &["Logicals"])?,
            outdoors: flag(&row, "Outdoors", &["Outdoors"])?,
            animate: flag(&row, "Animate", &["Animate"])?,
            kill_edge: flag(&row, "KillEdge", &["KillEdge"])?,
            fill_blanks: flag(&row, "FillBlanks", &["FillBlanks"])?,
            auto_map: flag(&row, "AutoMap", &["AutoMap"])?,
            scan: flag(&row, "Scan", &["Scan"])?,
            pops: opt(&row, "Pops", &["Pops"])?,
            pop_pad: opt(&row, "PopPad", &["PopPad"])?,
            files: (1..=6).map(|n| file_slot(&row, n)).collect(),
            dt1_mask,
            expansion: expansion(&row, after_separator, has_expansion)?,
        });
    }
    Ok(out)
}

/// Needs `Id`, `LevelType` and `DrlgType` columns (a `levels.bin` decoded
/// alone lacks the last two; use [`crate::excel::decode_levels_bin`]).
pub fn levels(table: &TxtTable) -> Result<Vec<Level>> {
    for required in ["Id", "LevelType", "DrlgType"] {
        if !has_column(table, &[required]) {
            return Err(missing(required));
        }
    }
    let mut out = Vec::new();
    for row in table.rows() {
        if is_separator(&row) || is_blank_row(&row) {
            continue;
        }
        let id = require_int(&row, "Id", &["Id"])?;
        let name = match row.get("LevelName") {
            Some(n) if !n.is_empty() => n,
            _ => row.get("Name").unwrap_or(""),
        };
        let level_type = require_int(&row, "LevelType", &["LevelType"])?;
        let drlg_type = require_int(&row, "DrlgType", &["DrlgType"])?;
        out.push(Level {
            row: row.index(),
            id: narrow(&row, "Id", id)?,
            name: name.to_string(),
            act: opt(&row, "Act", &["Act"])?,
            pal: opt(&row, "Pal", &["Pal"])?,
            level_type: narrow(&row, "LevelType", level_type)?,
            drlg_type: narrow(&row, "DrlgType", drlg_type)?,
        });
    }
    Ok(out)
}

impl LvlPrest {
    /// DT1 files of `lvl_type` selected by `dt1_mask` (bit i = File(i+1)), in slot order.
    pub fn dt1_files<'a>(&self, lvl_type: &'a LvlType) -> impl Iterator<Item = &'a str> + use<'a> {
        let mask = self.dt1_mask;
        lvl_type
            .files
            .iter()
            .enumerate()
            .filter_map(move |(slot, file)| {
                let selected = slot < 32 && mask & (1u32 << slot) != 0;
                file.as_deref().filter(|_| selected)
            })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
