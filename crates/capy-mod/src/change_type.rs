//! Changing the level type of a map's level (Levels `LevelType`), with
//! everything the game needs to load it.
//!
//! * A preset level (DrlgType 2) is built from the FIRST LvlPrest row whose
//!   `LevelId` is the level; its rooms load `LvlTypes[LevelType]."File N"` for
//!   every bit N-1 set in that row's `Dt1Mask`, so every set bit must name a
//!   filled slot holding a DT1 the map needs.
//! * Maze (1) and outdoor (3) levels pick pieces by level-type number: refused.
//! * A type other levels use keeps its tile list: its free slots are only
//!   filled when nothing else could load them, otherwise the level gets a copy
//!   of the type of its own.
//!
//! AutoMap.txt rows follow the level type in the reference editor; that part
//! belongs to the automap module and is not done here.

use crate::add_map::Edit;
use crate::level_tables::{
    FieldChange, LevelTables, TableName, append_own_type, ensure_type_slots, filled_slots,
    levels_of_type, mask_of, preset_rows_for, tile_path_problem,
};
use crate::lvl_error::{Result, refuse};
use crate::txt_util::{
    cell, cell_num, data_rows, file_value, mask, rel_key, row_of_record, tiles_rel,
};

/// What to change.
#[derive(Clone, Debug)]
pub struct ChangeTypeInput {
    /// The map, relative to data/global/tiles.
    pub map_rel: String,
    /// The level, when the map belongs to several.
    pub level_id: Option<u32>,
    pub target_type: u32,
    /// `false`: the level uses that type. `true`: a new type of its own, a copy of it.
    pub copy: bool,
    /// Paths of the DT1s the map's tiles come from.
    pub used_dt1s: Vec<String>,
    /// Name for a copied type (default: the level's name).
    pub copy_name: Option<String>,
}

/// The edited tables.
#[derive(Clone, Debug)]
pub struct ChangeTypeOutcome {
    pub tables: LevelTables,
    pub changes: Vec<FieldChange>,
    pub warnings: Vec<String>,
    pub type_id: u32,
    pub level_id: u32,
    /// Whether a copy was made (asked for, or needed).
    pub copied: bool,
}

/// A level type the level could change to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelTypeChoice {
    pub id: u32,
    pub name: String,
    pub act: u8,
    pub files: usize,
    /// Levels using it, other than this one.
    pub users: usize,
    /// Why it can only be used as a copy (None: it can be used as it is).
    pub copy_only: Option<String>,
}

/// The level a map belongs to.
#[derive(Clone, Copy, Debug)]
pub struct MapLevel {
    pub p_row: usize,
    pub l_row: usize,
    pub level_id: u32,
    pub type_id: u32,
}

/// The level the map belongs to, or why its type can't be changed here.
pub fn map_level(tables: &LevelTables, map_rel: &str, want: Option<u32>) -> Result<MapLevel> {
    let rows = preset_rows_for(&tables.prest, map_rel);
    if rows.is_empty() {
        return refuse(
            "This map is not in the game yet (no LvlPrest.txt row lists it). Add it to the game first.",
        );
    }
    let mut ids: Vec<i64> = rows
        .iter()
        .map(|&r| cell_num(&tables.prest, r, "LevelId"))
        .collect();
    ids.dedup();
    let mut seen = std::collections::BTreeSet::new();
    ids.retain(|i| seen.insert(*i));
    let mut p_row = rows[0];
    if let Some(w) = want {
        match rows
            .iter()
            .find(|&&r| cell_num(&tables.prest, r, "LevelId") == i64::from(w))
        {
            Some(&r) => p_row = r,
            None => return refuse(format!("No LvlPrest.txt row lists this map for level {w}.")),
        }
    } else if ids.len() > 1 {
        return refuse(format!(
            "This map belongs to {} levels ({}): choose which one.",
            ids.len(),
            ids.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let level_id = cell_num(&tables.prest, p_row, "LevelId");
    if level_id == 0 {
        return refuse(
            "This map is a shared preset (LvlPrest LevelId 0): the game places it inside other levels, which use their own level type. Change the level type of those levels instead.",
        );
    }
    let Some(l_row) = row_of_record(&tables.levels, level_id as usize) else {
        return refuse(format!("Levels.txt has no level {level_id}."));
    };
    let drlg = cell_num(&tables.levels, l_row, "DrlgType");
    if drlg != 2 {
        let kind = match drlg {
            1 => "a maze level".to_owned(),
            3 => "an outdoor level".to_owned(),
            n => format!("DrlgType {n}"),
        };
        return refuse(format!(
            "Level {level_id} \"{}\" is {kind}: the game picks its building pieces by level-type number, so changing its type is not safe.",
            cell(&tables.levels, l_row, "Name")
        ));
    }
    let first = data_rows(&tables.prest)
        .into_iter()
        .find(|&r| cell_num(&tables.prest, r, "LevelId") == level_id);
    if first != Some(p_row) {
        return refuse(format!(
            "The game builds level {level_id} from the first LvlPrest row claiming it ({}), not from this map.",
            first.map_or(String::new(), |f| cell(&tables.prest, f, "Name"))
        ));
    }
    Ok(MapLevel {
        p_row,
        l_row,
        level_id: level_id as u32,
        type_id: cell_num(&tables.levels, l_row, "LevelType") as u32,
    })
}

/// The libraries the level's preset row must load: the map's, plus, when the
/// row lists several maps, every library it loads now. Returns the list and
/// how many other maps the row lists.
fn required_dt1s(tables: &LevelTables, lvl: &MapLevel, used: &[String]) -> (Vec<String>, usize) {
    let files = (1..=6)
        .filter(|f| file_value(&cell(&tables.prest, lvl.p_row, &format!("File{f}"))).is_some())
        .count();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |p: &str| {
        let k = rel_key(p);
        if !out.iter().any(|(e, _)| *e == k) {
            out.push((k, p.to_owned()));
        }
    };
    for u in used {
        add(u);
    }
    if files > 1
        && let Some(t_row) = row_of_record(&tables.types, lvl.type_id as usize)
    {
        let m = crate::level_tables::preset_mask(&tables.prest, lvl.p_row);
        for i in 1..=32usize {
            if (m >> (i - 1)) & 1 == 1
                && let Some(v) = file_value(&cell(&tables.types, t_row, &format!("File {i}")))
            {
                add(&v);
            }
        }
    }
    (
        out.into_iter().map(|(_, p)| p).collect(),
        files.saturating_sub(1),
    )
}

/// Why filling these slots of a type could make another level load them.
fn slots_reached_by_others(
    tables: &LevelTables,
    type_id: u32,
    except_level: u32,
    slots: &[usize],
) -> Option<String> {
    if slots.is_empty() {
        return None;
    }
    let bits = slots.iter().fold(0u32, |m, &s| m | (1 << (s - 1)));
    for lr in levels_of_type(&tables.levels, type_id, Some(except_level)) {
        let id = cell_num(&tables.levels, lr, "Id");
        let lname = cell(&tables.levels, lr, "Name");
        if cell_num(&tables.levels, lr, "DrlgType") != 2 {
            return Some(format!(
                "level {id} \"{lname}\" uses it and isn't a preset level"
            ));
        }
        for r in data_rows(&tables.prest) {
            if cell_num(&tables.prest, r, "LevelId") == id
                && crate::level_tables::preset_mask(&tables.prest, r) & bits != 0
            {
                return Some(format!("level {id} \"{lname}\" would load the new slots"));
            }
        }
    }
    for r in data_rows(&tables.prest) {
        if cell_num(&tables.prest, r, "LevelId") == 0
            && crate::level_tables::preset_mask(&tables.prest, r) & bits != 0
        {
            return Some(format!(
                "shared preset \"{}\" would load the new slots wherever it is placed",
                cell(&tables.prest, r, "Name")
            ));
        }
    }
    if let Some(sub) = &tables.lvl_sub {
        for r in data_rows(sub) {
            if mask(&cell(sub, r, "Dt1Mask")) & bits != 0 {
                return Some(format!(
                    "LvlSub \"{}\" would load the new slots",
                    cell(sub, r, "Name")
                ));
            }
        }
    }
    None
}

/// The level types the map's level could change to, and which only as a copy.
pub fn level_type_choices(
    tables: &LevelTables,
    map_rel: &str,
    used_dt1s: &[String],
    level_id: Option<u32>,
) -> Result<Vec<LevelTypeChoice>> {
    let lvl = map_level(tables, map_rel, level_id)?;
    let (want, _) = required_dt1s(tables, &lvl, used_dt1s);
    let want: std::collections::BTreeSet<String> = want.iter().map(|p| rel_key(p)).collect();
    let mut out = Vec::new();
    for r in data_rows(&tables.types) {
        let name = cell(&tables.types, r, "Name");
        if name.is_empty() || name.eq_ignore_ascii_case("expansion") {
            continue;
        }
        let id = cell_num(&tables.types, r, "Id") as u32;
        let filled = filled_slots(&tables.types, r);
        let have: std::collections::BTreeSet<String> = filled
            .iter()
            .map(|&s| rel_key(&cell(&tables.types, r, &format!("File {s}"))))
            .collect();
        let missing = want.iter().filter(|p| !have.contains(*p)).count();
        let users = levels_of_type(&tables.levels, id, Some(lvl.level_id)).len();
        let mut copy_only = None;
        if missing > 0 && users > 0 {
            copy_only = Some(format!(
                "it lacks {missing} of the map's tile libraries and {users} other level{} it (their tile list stays as it is)",
                if users == 1 { " uses" } else { "s use" }
            ));
        } else if missing > 0 {
            let free: Vec<usize> = (1..=32)
                .filter(|i| !filled.contains(i))
                .take(missing)
                .collect();
            if free.len() < missing {
                copy_only = Some(format!("no room for {missing} more tile libraries"));
            } else if let Some(why) = slots_reached_by_others(tables, id, lvl.level_id, &free) {
                copy_only = Some(format!(
                    "it lacks {missing} of the map's tile libraries and {why}"
                ));
            }
        }
        out.push(LevelTypeChoice {
            id,
            name,
            act: cell_num(&tables.types, r, "Act") as u8,
            files: filled.len(),
            users,
            copy_only,
        });
    }
    Ok(out)
}

/// Everything that changing the level type writes, checked against the rules
/// above. Doing it again for the same target changes nothing more (refused as
/// "already uses").
pub fn change_level_type(
    tables: &LevelTables,
    input: &ChangeTypeInput,
) -> Result<ChangeTypeOutcome> {
    let lvl = map_level(tables, &input.map_rel, input.level_id)?;
    let level_name = cell(&tables.levels, lvl.l_row, "Name");
    let level_label = format!("Level {} \"{level_name}\"", lvl.level_id);
    let Some(t_row) = row_of_record(&tables.types, input.target_type as usize) else {
        return refuse(format!(
            "LvlTypes.txt has no level type {}.",
            input.target_type
        ));
    };
    let target_name = cell(&tables.types, t_row, "Name");
    let (dt1s, others) = required_dt1s(tables, &lvl, &input.used_dt1s);
    if dt1s.is_empty() {
        return refuse("The map uses no tile libraries.");
    }
    for p in &dt1s {
        if let Some(issue) = tile_path_problem(&tiles_rel(p)) {
            return refuse(issue);
        }
    }
    let choices = level_type_choices(tables, &input.map_rel, &input.used_dt1s, input.level_id)?;
    let Some(choice) = choices.iter().find(|c| c.id == input.target_type) else {
        return refuse(format!("Level type {} can't be chosen.", input.target_type));
    };
    let copy = input.copy || choice.copy_only.is_some();
    if !copy && input.target_type == lvl.type_id {
        return refuse(format!(
            "{level_label} already uses level type {} \"{target_name}\".",
            lvl.type_id
        ));
    }
    let mut ed = Edit::new(tables);
    let mut warnings = Vec::new();
    if others > 0 {
        warnings.push(format!(
            "The preset row \"{}\" also builds {others} other map{} (the game picks one): every tile library it loads now stays loaded, so {} their tiles.",
            cell(&tables.prest, lvl.p_row, "Name"),
            if others == 1 { "" } else { "s" },
            if others == 1 { "that map keeps" } else { "those maps keep" }
        ));
    }
    if !input.copy
        && let Some(why) = &choice.copy_only
    {
        warnings.push(format!(
            "Type {} \"{target_name}\" can't be used as it is ({why}), so the level gets a copy of it of its own.",
            input.target_type
        ));
    }

    let (type_id, slots, type_row);
    if copy {
        let n = input
            .copy_name
            .clone()
            .unwrap_or_else(|| level_name.clone());
        let name = if n.trim().is_empty() {
            level_name.clone()
        } else {
            n.trim().to_owned()
        };
        let own = append_own_type(&mut ed.tables.types, t_row, &name, &dt1s)?;
        type_id = own.type_id;
        type_row = own.type_row;
        slots = own.slots;
        let label = format!("Type {type_id} \"{name}\"");
        ed.note(
            TableName::LvlTypes,
            &label,
            "(new row)",
            &format!(
                "copy of {} \"{target_name}\" (Act, Expansion), appended as record {type_id}",
                input.target_type
            ),
            Some("a level type of its own for this level, with just the map's tile libraries"),
        );
        let mut by_slot: Vec<_> = slots.values().copied().collect();
        by_slot.sort_unstable();
        for s in by_slot {
            let v = cell(&ed.tables.types, type_row, &format!("File {s}"));
            ed.note(TableName::LvlTypes, &label, &format!("File {s}"), &v, None);
        }
    } else {
        type_id = input.target_type;
        type_row = t_row;
        let before = ed.tables.types.clone();
        let r = ensure_type_slots(&mut ed.tables.types, t_row, &dt1s)?;
        slots = r.slots;
        for a in r.added {
            let col = format!("File {}", a.slot);
            ed.changes.push(FieldChange {
                table: TableName::LvlTypes,
                row: format!("Type {type_id} \"{target_name}\""),
                from: file_value(&cell(&before, t_row, &col)).unwrap_or_else(|| "0".into()),
                column: col,
                to: a.value,
                why: Some("a tile library the map uses".into()),
            });
            ed.touched.insert(TableName::LvlTypes);
        }
    }
    ed.touched.insert(TableName::Levels);
    ed.set(
        TableName::Levels,
        lvl.l_row,
        &level_label,
        "LevelType",
        &type_id.to_string(),
        Some(&if copy {
            "its own copy of the chosen type".to_owned()
        } else {
            format!("type \"{target_name}\"")
        }),
    )?;
    let new_mask = mask_of(
        &dt1s
            .iter()
            .filter_map(|p| slots.get(&rel_key(p)).map(|&s| (rel_key(p), s)))
            .collect(),
    );
    let mut used_slots: Vec<usize> = slots.values().copied().collect();
    used_slots.sort_unstable();
    used_slots.dedup();
    let p_label = format!(
        "Preset {} \"{}\"",
        cell(&tables.prest, lvl.p_row, "Def"),
        cell(&tables.prest, lvl.p_row, "Name")
    );
    ed.set(
        TableName::LvlPrest,
        lvl.p_row,
        &p_label,
        "Dt1Mask",
        &new_mask.to_string(),
        Some(&format!(
            "File {} of the new type",
            used_slots
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    )?;

    // Final check: every mask bit names a filled slot, with no gap before it.
    let filled = filled_slots(&ed.tables.types, type_row);
    for i in 1..=32usize {
        if (new_mask >> (i - 1)) & 1 == 1 && !filled.contains(&i) {
            return refuse(format!(
                "Internal check failed: Dt1Mask bit for File {i}, which is empty."
            ));
        }
    }
    if let Some(&max) = filled.iter().max() {
        for i in 1..max {
            if !filled.contains(&i) {
                warnings.push(format!(
                    "LvlTypes \"{}\" has an empty File {i} before File {max}: the game's tile preload stops there (the level still loads its tiles when its rooms are built).",
                    cell(&ed.tables.types, type_row, "Name")
                ));
            }
        }
    }
    if copy {
        warnings.push(format!(
            "AutoMap.txt rows follow the level type: add automap rows for type {type_id} (Automap editor)."
        ));
    }
    Ok(ChangeTypeOutcome {
        tables: ed.tables,
        changes: ed.changes,
        warnings,
        type_id,
        level_id: lvl.level_id,
        copied: copy,
    })
}
