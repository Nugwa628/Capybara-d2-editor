//! Type packages: a whole level type as one data structure, for moving it
//! between mods. It holds the type's LvlTypes row, its DT1 list in slot order,
//! the maps of its levels, and the Levels and LvlPrest rows of those levels.
//! Zip packaging (DT1 and DS1 bytes, README) is the caller's job.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::add_map::Edit;
use crate::level_tables::{
    FieldChange, LevelTables, MAX_LAYER, TableName, ensure_type_slots, free_offset, level_act,
    levels_of_type, safe_layer,
};
use crate::lvl_error::{Result, refuse};
use crate::txt_util::{
    append_values, cell, cell_num, data_rows, file_value, mask, rel_key, row_of_record, row_values,
    tiles_rel,
};

/// One table row with its column names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRow {
    /// `LvlTypes.txt`, `Levels.txt` or `LvlPrest.txt`.
    pub table: String,
    pub columns: Vec<String>,
    pub row: Vec<String>,
}

impl PackageRow {
    /// Value of a column (case-insensitive); empty when absent.
    pub fn value(&self, col: &str) -> String {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(col))
            .and_then(|i| self.row.get(i).cloned())
            .unwrap_or_default()
    }
}

/// A DT1 of the type and the File slot (1-32) it sits in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDt1 {
    pub slot: u8,
    /// As stored in LvlTypes (relative to data/global/tiles).
    pub path: String,
}

/// A level type with everything that loads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypePackage {
    pub type_id: u32,
    pub type_name: String,
    pub type_row: PackageRow,
    /// The type's DT1s in slot order.
    pub dt1s: Vec<PackageDt1>,
    /// The maps (LvlPrest File1..6, relative to data/global/tiles) of the type's levels.
    pub maps: Vec<String>,
    pub levels: Vec<PackageRow>,
    pub presets: Vec<PackageRow>,
}

fn package_row(table: &str, doc: &d2_formats::txt::TxtDoc, row: usize) -> PackageRow {
    let (columns, row) = row_values(doc, row).into_iter().unzip();
    PackageRow {
        table: table.to_owned(),
        columns,
        row,
    }
}

/// The maps (LvlPrest File1..6) of every level using the type, once each.
pub fn maps_of_type(tables: &LevelTables, type_id: u32) -> Vec<String> {
    let ids: BTreeSet<i64> = levels_of_type(&tables.levels, type_id, None)
        .into_iter()
        .map(|r| cell_num(&tables.levels, r, "Id"))
        .collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for r in data_rows(&tables.prest) {
        if !ids.contains(&cell_num(&tables.prest, r, "LevelId")) {
            continue;
        }
        for f in 1..=6 {
            if let Some(v) = file_value(&cell(&tables.prest, r, &format!("File{f}"))) {
                let rel = tiles_rel(&v);
                if seen.insert(rel.to_lowercase()) {
                    out.push(rel);
                }
            }
        }
    }
    out
}

/// Exports level type `type_id`.
pub fn export_type_package(tables: &LevelTables, type_id: u32) -> Result<TypePackage> {
    let Some(t_row) = row_of_record(&tables.types, type_id as usize) else {
        return refuse(format!("LvlTypes.txt has no level type {type_id}."));
    };
    let level_rows = levels_of_type(&tables.levels, type_id, None);
    let ids: BTreeSet<i64> = level_rows
        .iter()
        .map(|&r| cell_num(&tables.levels, r, "Id"))
        .collect();
    let dt1s = (1..=32u8)
        .filter_map(|s| {
            file_value(&cell(&tables.types, t_row, &format!("File {s}")))
                .map(|path| PackageDt1 { slot: s, path })
        })
        .collect();
    let presets = data_rows(&tables.prest)
        .into_iter()
        .filter(|&r| ids.contains(&cell_num(&tables.prest, r, "LevelId")))
        .map(|r| package_row("LvlPrest.txt", &tables.prest, r))
        .collect();
    Ok(TypePackage {
        type_id,
        type_name: cell(&tables.types, t_row, "Name"),
        type_row: package_row("LvlTypes.txt", &tables.types, t_row),
        dt1s,
        maps: maps_of_type(tables, type_id),
        levels: level_rows
            .into_iter()
            .map(|r| package_row("Levels.txt", &tables.levels, r))
            .collect(),
        presets,
    })
}

/// The edited tables after a package import.
#[derive(Clone, Debug)]
pub struct ImportOutcome {
    pub tables: LevelTables,
    pub changes: Vec<FieldChange>,
    pub warnings: Vec<String>,
    /// The level type the package's type became here.
    pub type_id: u32,
    /// Package level ids mapped to the ids they have here (only those that differ).
    pub level_ids: BTreeMap<u32, u32>,
}

fn find_by_name(doc: &d2_formats::txt::TxtDoc, name: &str) -> Option<usize> {
    let want = name.trim().to_lowercase();
    data_rows(doc)
        .into_iter()
        .find(|&r| cell(doc, r, "Name").trim().to_lowercase() == want)
}

/// Merges a package into the importer's tables so the game loads its maps the
/// same way, without clobbering rows their other maps rely on:
/// * level type: the row with the same Name is reused (its missing DT1s go into
///   free slots), else it is appended with the next free Id;
/// * levels: the row with the same Name is pointed at the type, else a new level
///   is appended (next Id, a free spot in Act 5, links cleared);
/// * presets: the row already listing the map is updated, else one is appended
///   (next Def). Dt1Mask bits are remapped from the package's slots to this
///   type's slots by DT1 path.
///
/// Importing the same package again changes nothing.
pub fn import_type_package(tables: &LevelTables, pkg: &TypePackage) -> Result<ImportOutcome> {
    crate::add_map::check_record_order(tables)?;
    let mut ed = Edit::new(tables);
    let mut warnings = Vec::new();

    // Level type.
    let t_row = if let Some(r) = find_by_name(&ed.tables.types, &pkg.type_name) {
        r
    } else {
        let id = data_rows(&ed.tables.types).len();
        let mut values: Vec<(String, String)> = pkg
            .type_row
            .columns
            .iter()
            .cloned()
            .zip(pkg.type_row.row.iter().cloned())
            .collect();
        for (k, v) in &mut values {
            if k.eq_ignore_ascii_case("Id") {
                *v = id.to_string();
            }
        }
        let row = append_values(&mut ed.tables.types, &values)?;
        ed.note(
            TableName::LvlTypes,
            &format!("Type {id} \"{}\"", pkg.type_name),
            "(new row)",
            &format!("new level type {id}"),
            None,
        );
        row
    };
    let type_id = cell_num(&ed.tables.types, t_row, "Id") as u32;
    let type_label = format!("Type {type_id} \"{}\"", pkg.type_name);
    let before = ed.tables.types.clone();
    let dt1_paths: Vec<String> = pkg.dt1s.iter().map(|d| d.path.clone()).collect();
    let slots = ensure_type_slots(&mut ed.tables.types, t_row, &dt1_paths)?;
    for a in &slots.added {
        let col = format!("File {}", a.slot);
        ed.changes.push(FieldChange {
            table: TableName::LvlTypes,
            row: type_label.clone(),
            from: file_value(&cell(&before, t_row, &col)).unwrap_or_else(|| "0".into()),
            column: col,
            to: a.value.clone(),
            why: Some("a tile library of the package".into()),
        });
        ed.touched.insert(TableName::LvlTypes);
    }
    // package slot -> this type's slot
    let slot_map: BTreeMap<u8, usize> = pkg
        .dt1s
        .iter()
        .filter_map(|d| slots.slots.get(&rel_key(&d.path)).map(|&s| (d.slot, s)))
        .collect();
    let remap = |m: u32| -> u32 {
        (1..=32u8)
            .filter(|&s| (m >> (s - 1)) & 1 == 1)
            .filter_map(|s| slot_map.get(&s))
            .fold(0u32, |acc, &s| acc | (1 << (s - 1)))
    };

    // Levels.
    let mut level_ids = BTreeMap::new();
    let mut id_of = BTreeMap::new();
    for lv in &pkg.levels {
        let old_id = cell_num_row(lv, "Id") as u32;
        let name = lv.value("Name");
        let id = if let Some(r) = find_by_name(&ed.tables.levels, &name) {
            let id = cell_num(&ed.tables.levels, r, "Id") as u32;
            let label = format!("Level {id} \"{name}\"");
            ed.set(
                TableName::Levels,
                r,
                &label,
                "LevelType",
                &type_id.to_string(),
                None,
            )?;
            id
        } else {
            let id = data_rows(&ed.tables.levels).len() as u32;
            if id >= 1024 {
                return refuse("Levels.txt already has 1024 levels, the most the game allows.");
            }
            let size = (
                cell_num_row(lv, "SizeX").max(200 * i64::from(cell_num_row(lv, "SizeX") == 0)),
                cell_num_row(lv, "SizeY").max(200 * i64::from(cell_num_row(lv, "SizeY") == 0)),
            );
            let off = free_offset(&ed.tables.levels, level_act(id), size, None);
            let mut values: Vec<(String, String)> = lv
                .columns
                .iter()
                .cloned()
                .zip(lv.row.iter().cloned())
                .collect();
            let layer = lv.value("Layer");
            for (k, v) in &mut values {
                let lk = k.to_lowercase();
                match lk.as_str() {
                    "id" => *v = id.to_string(),
                    "leveltype" => *v = type_id.to_string(),
                    "act" => *v = level_act(id).to_string(),
                    "offsetx" => *v = off.0.to_string(),
                    "offsety" => *v = off.1.to_string(),
                    "depend" => *v = "0".into(),
                    "layer" if crate::txt_util::num(&layer) > MAX_LAYER => {
                        *v = safe_layer(&ed.tables.levels).to_string();
                    }
                    s if s.starts_with("vis") && s.len() == 4 => *v = "0".into(),
                    s if s.starts_with("warp") && s.len() == 5 => *v = "-1".into(),
                    _ => {}
                }
            }
            append_values(&mut ed.tables.levels, &values)?;
            ed.note(
                TableName::Levels,
                &format!("Level {id} \"{name}\""),
                "(new row)",
                &format!(
                    "level type {type_id}, Act {}, at {},{}",
                    level_act(id) + 1,
                    off.0,
                    off.1
                ),
                None,
            );
            id
        };
        if id != old_id {
            level_ids.insert(old_id, id);
        }
        id_of.insert(old_id, id);
    }

    // Presets.
    for pr in &pkg.presets {
        let files: Vec<String> = (1..=6)
            .filter_map(|f| file_value(&pr.value(&format!("File{f}"))))
            .collect();
        let old_level = cell_num_row(pr, "LevelId") as u32;
        let level = id_of.get(&old_level).copied().unwrap_or(old_level);
        let new_mask = remap(mask(&pr.value("Dt1Mask")));
        let existing = files
            .first()
            .map(|f| crate::level_tables::preset_rows_for(&ed.tables.prest, f))
            .unwrap_or_default();
        if existing.is_empty() {
            let def = data_rows(&ed.tables.prest).len();
            let mut values: Vec<(String, String)> = pr
                .columns
                .iter()
                .cloned()
                .zip(pr.row.iter().cloned())
                .collect();
            for (k, v) in &mut values {
                if k.eq_ignore_ascii_case("Def") {
                    *v = def.to_string();
                } else if k.eq_ignore_ascii_case("LevelId") {
                    *v = level.to_string();
                } else if k.eq_ignore_ascii_case("Dt1Mask") {
                    *v = new_mask.to_string();
                }
            }
            append_values(&mut ed.tables.prest, &values)?;
            ed.note(
                TableName::LvlPrest,
                &format!("Preset {def} \"{}\"", pr.value("Name")),
                "(new row)",
                &format!("-> level {level}, Dt1Mask {new_mask}"),
                None,
            );
        } else {
            for r in existing {
                let label = format!(
                    "Preset {} \"{}\"",
                    cell(&ed.tables.prest, r, "Def"),
                    cell(&ed.tables.prest, r, "Name")
                );
                let shared = files.len() > 1 || level == 0;
                let old = crate::level_tables::preset_mask(&ed.tables.prest, r);
                let m = if shared { old | new_mask } else { new_mask };
                ed.set(
                    TableName::LvlPrest,
                    r,
                    &label,
                    "Dt1Mask",
                    &m.to_string(),
                    None,
                )?;
                if level != 0 {
                    ed.set(
                        TableName::LvlPrest,
                        r,
                        &label,
                        "LevelId",
                        &level.to_string(),
                        None,
                    )?;
                }
            }
        }
    }
    if pkg.levels.is_empty() && !pkg.presets.is_empty() {
        warnings.push(
            "The package has presets but no levels: they were attached to the level ids they name."
                .into(),
        );
    }
    Ok(ImportOutcome {
        tables: ed.tables,
        changes: ed.changes,
        warnings,
        type_id,
        level_ids,
    })
}

fn cell_num_row(r: &PackageRow, col: &str) -> i64 {
    crate::txt_util::num(&r.value(col))
}
