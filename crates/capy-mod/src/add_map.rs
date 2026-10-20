//! "Add to game": the table rows that make the game load a DS1 map.
//!
//! Rules (from the game's loaders, as the reference editor documents them):
//! * Levels, LvlPrest and LvlTypes are read by row position, so rows are only
//!   appended; `Id`/`Def` equal the data-row index.
//! * A preset level (Levels `DrlgType` 2) uses the FIRST LvlPrest row whose
//!   `LevelId` is the level; the game loads that preset by its `Def`.
//! * The act comes from the level number (109 on is Act 5), so new levels are
//!   Act 5; `Pal` keeps the colours of the act the tiles were drawn for.
//! * The level's size is the DS1's size minus one; LvlPrest `SizeX/SizeY` stay
//!   0 so the preset fills the level.
//! * A level type other levels use keeps its tile list: a map that needs
//!   libraries it lacks gets a level type of its own.

use std::collections::BTreeSet;

use crate::level_tables::{
    DEFAULT_ENTRY_FILE, FieldChange, LevelTables, MAX_LAYER, MAX_LEVEL_STRING, SlotAdd, TableName,
    append_own_type, ensure_type_slots, free_offset, level_act, levels_of_type, mask_for, mask_of,
    preset_rows_for, safe_layer, tile_path_problem, type_act,
};
use crate::lvl_error::{Result, refuse};
use crate::txt_util::{
    append_values, cell, cell_num, data_rows, has, row_of_record, row_values, set, tiles_rel,
};

/// Whether the map becomes a new level or an existing preset level's map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMode {
    /// A new level (appended), copying settings from `level_id`.
    New,
    /// The preset level `level_id` now loads this map.
    Existing,
}

/// What to register.
#[derive(Clone, Debug)]
pub struct AddMapInput {
    pub mode: AddMode,
    /// `New`: the level to copy settings from. `Existing`: the preset level that uses this map.
    pub level_id: u32,
    pub name: String,
    /// Map path relative to data/global/tiles, with its own capitals.
    pub map_rel: String,
    /// DS1 size in cells as the editor reads it (the file stores one less).
    pub width: u32,
    pub height: u32,
    /// Paths (full or tiles-relative) of the DT1s the map's tiles come from.
    pub used_dt1s: Vec<String>,
    /// Roof hide areas in the map.
    pub pop_count: u32,
    /// `New`: the act whose palette the level uses (0-4); default: the act of the template's tiles.
    pub pal_act: Option<u8>,
    /// A level type of its own even when the template's type already lists every library.
    pub new_type: bool,
}

/// The edited tables with the change list.
#[derive(Clone, Debug)]
pub struct AddMapOutcome {
    pub tables: LevelTables,
    pub changes: Vec<FieldChange>,
    pub warnings: Vec<String>,
    /// The level that now loads the map.
    pub level_id: u32,
    pub new_level_id: Option<u32>,
    /// The level type the level ends with.
    pub type_id: u32,
    /// Whether the map got a level type of its own.
    pub own_type: bool,
    /// LvlPrest `Def` of the map's preset row.
    pub preset_def: u32,
    pub touched: BTreeSet<TableName>,
}

/// Cell edits with a change list (shared by the level tools).
pub(crate) struct Edit {
    pub tables: LevelTables,
    pub changes: Vec<FieldChange>,
    pub touched: BTreeSet<TableName>,
}

impl Edit {
    pub fn new(tables: &LevelTables) -> Edit {
        Edit {
            tables: tables.clone(),
            changes: Vec::new(),
            touched: BTreeSet::new(),
        }
    }

    fn doc(&mut self, t: TableName) -> &mut d2_formats::txt::TxtDoc {
        match t {
            TableName::Levels => &mut self.tables.levels,
            TableName::LvlPrest => &mut self.tables.prest,
            TableName::LvlTypes => &mut self.tables.types,
        }
    }

    /// Sets a cell and records it when it changed (a missing column is skipped).
    pub fn set(
        &mut self,
        t: TableName,
        row: usize,
        label: &str,
        col: &str,
        to: &str,
        why: Option<&str>,
    ) -> Result<()> {
        let from = cell(self.doc(t), row, col);
        if set(self.doc(t), row, col, to)? {
            self.changes.push(FieldChange {
                table: t,
                row: label.to_owned(),
                column: col.to_owned(),
                from,
                to: to.to_owned(),
                why: why.map(str::to_owned),
            });
            self.touched.insert(t);
        }
        Ok(())
    }

    pub fn note(&mut self, t: TableName, label: &str, column: &str, to: &str, why: Option<&str>) {
        self.changes.push(FieldChange {
            table: t,
            row: label.to_owned(),
            column: column.to_owned(),
            from: String::new(),
            to: to.to_owned(),
            why: why.map(str::to_owned),
        });
        self.touched.insert(t);
    }
}

fn level_name_of(levels: &d2_formats::txt::TxtDoc, id: u32) -> String {
    row_of_record(levels, id as usize).map_or_else(String::new, |r| cell(levels, r, "Name"))
}

/// Record check: `Id`/`Def` must count up 0, 1, 2... or every id we write is off.
pub(crate) fn check_record_order(t: &LevelTables) -> Result<()> {
    for (table, doc, col) in [
        ("Levels.txt", &t.levels, "Id"),
        ("LvlPrest.txt", &t.prest, "Def"),
        ("LvlTypes.txt", &t.types, "Id"),
    ] {
        for (i, r) in data_rows(doc).into_iter().enumerate() {
            let v = cell(doc, r, col);
            if v.trim() != i.to_string() {
                let shown = if v.is_empty() { "(empty)" } else { &v };
                return refuse(format!(
                    "{table}: record {i} has {col} {shown}. The game reads these tables by row position, so {col} must count up 0, 1, 2... Put the rows back in order first."
                ));
            }
        }
    }
    Ok(())
}

/// Registers a DS1 with the game's tables. The inputs are not changed; the
/// outcome holds the edited tables. Registering an `Existing` map twice changes
/// nothing the second time; a `New` map already listed in LvlPrest is refused.
pub fn add_map_to_game(tables: &LevelTables, input: &AddMapInput) -> Result<AddMapOutcome> {
    let name = input.name.trim().to_owned();
    if name.is_empty() {
        return refuse("Give the level a name.");
    }
    if let Some(p) = tile_path_problem(&input.map_rel) {
        return refuse(p);
    }
    check_record_order(tables)?;
    for p in &input.used_dt1s {
        if let Some(issue) = tile_path_problem(&tiles_rel(p)) {
            return refuse(format!("LvlTypes: {issue}"));
        }
    }
    if input.mode == AddMode::New && !preset_rows_for(&tables.prest, &input.map_rel).is_empty() {
        return refuse(
            "This map is already listed in LvlPrest.txt. Use the existing level instead of making a new one.",
        );
    }
    let mut ed = Edit::new(tables);
    let mut warnings = Vec::new();

    let Some(level_row) = row_of_record(&ed.tables.levels, input.level_id as usize) else {
        return refuse("Pick a level.");
    };
    let mut type_id = cell_num(&ed.tables.levels, level_row, "LevelType") as u32;
    let Some(mut type_row) = row_of_record(&ed.tables.types, type_id as usize) else {
        return refuse(format!("LvlTypes.txt has no level type {type_id}."));
    };
    let mut type_name = cell(&ed.tables.types, type_row, "Name");
    let size_x = (i64::from(input.width) - 1).to_string();
    let size_y = (i64::from(input.height) - 1).to_string();

    // Tile libraries.
    let sharers = levels_of_type(
        &ed.tables.levels,
        type_id,
        (input.mode == AddMode::Existing).then_some(input.level_id),
    );
    let before_types = ed.tables.types.clone();
    let mut trial = ed.tables.types.clone();
    let slots_try = ensure_type_slots(&mut trial, type_row, &input.used_dt1s)?;
    let mut own_type = false;
    let slots;
    if (!slots_try.added.is_empty() || input.new_type) && !sharers.is_empty() {
        let own = append_own_type(&mut ed.tables.types, type_row, &name, &input.used_dt1s)?;
        let users: Vec<String> = sharers
            .iter()
            .map(|&x| {
                format!(
                    "{} {}",
                    cell(&ed.tables.levels, x, "Id"),
                    cell(&ed.tables.levels, x, "Name")
                )
            })
            .collect();
        let more = if users.len() > 3 {
            format!(" and {} more", users.len() - 3)
        } else {
            String::new()
        };
        warnings.push(format!(
            "Level type {type_id} \"{type_name}\" is also used by {}{more}, so the map gets its own level type {} \"{name}\" with just its {} tile libraries. The level loses that type's AutoMap.txt entries: add its own with the Automap editor.",
            users.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
            own.type_id,
            own.slots.len()
        ));
        let label = format!("Type {} \"{name}\"", own.type_id);
        ed.note(
            TableName::LvlTypes,
            &label,
            "(new row)",
            &format!(
                "copy of {type_id} \"{type_name}\" (Act, Expansion), appended as record {}",
                own.type_id
            ),
            None,
        );
        let mut by_slot: Vec<_> = own.slots.values().copied().collect();
        by_slot.sort_unstable();
        for s in by_slot {
            let v = cell(&ed.tables.types, own.type_row, &format!("File {s}"));
            ed.note(
                TableName::LvlTypes,
                &label,
                &format!("File {s}"),
                &v,
                (s == 1).then_some("the tile libraries the map uses"),
            );
        }
        type_row = own.type_row;
        type_id = own.type_id;
        type_name = name.clone();
        slots = own.slots;
        own_type = true;
    } else {
        ed.tables.types = trial;
        slots = slots_try.slots.clone();
        for SlotAdd { slot, value } in &slots_try.added {
            let col = format!("File {slot}");
            ed.changes.push(FieldChange {
                table: TableName::LvlTypes,
                row: format!("Type {type_id} \"{type_name}\""),
                column: col.clone(),
                from: cell(&before_types, type_row, &col),
                to: value.clone(),
                why: Some("a tile library the map uses".into()),
            });
            ed.touched.insert(TableName::LvlTypes);
        }
    }
    if own_type {
        ed.touched.insert(TableName::LvlTypes);
    }

    let mut new_level_id = None;
    let mut target = input.level_id;
    let prest_rows = data_rows(&ed.tables.prest);

    match input.mode {
        AddMode::New => {
            if cell_num(&ed.tables.levels, level_row, "DrlgType") == 0 {
                return refuse("Pick a real level to copy from.");
            }
            let id = data_rows(&ed.tables.levels).len() as u32;
            if id >= 1024 {
                return refuse("Levels.txt already has 1024 levels, the most the game allows.");
            }
            if id > 255 {
                warnings.push(format!("The new level is number {id}. Levels.txt's Id column is stored in one byte, so numbers above 255 need a mod that supports them (PD2 does)."));
            }
            let values = row_values(&ed.tables.levels, level_row);
            let row = append_values(&mut ed.tables.levels, &values)?;
            let label = format!("Level {id} \"{name}\"");
            ed.note(
                TableName::Levels,
                &label,
                "(new row)",
                &format!(
                    "copy of {} {}, appended as record {id}",
                    input.level_id,
                    level_name_of(&ed.tables.levels, input.level_id)
                ),
                None,
            );
            let tiles_act =
                type_act(&ed.tables.types, type_id).unwrap_or(level_act(input.level_id));
            let pal = input.pal_act.unwrap_or(tiles_act);
            let off = free_offset(
                &ed.tables.levels,
                4,
                (i64::from(input.width) - 1, i64::from(input.height) - 1),
                Some(row),
            );
            let short: String = name.chars().take(MAX_LEVEL_STRING).collect();
            if name.chars().count() > MAX_LEVEL_STRING {
                warnings.push(format!("The name is cut to {MAX_LEVEL_STRING} characters in LevelName/LevelWarp (the game's limit)."));
            }
            let l = TableName::Levels;
            ed.set(l, row, &label, "Name", &name, None)?;
            ed.set(
                l,
                row,
                &label,
                "Id",
                &id.to_string(),
                Some("its row number: the game reads Levels.txt by row"),
            )?;
            ed.set(
                l,
                row,
                &label,
                "Act",
                "4",
                Some("levels from 109 on are Act 5 in the game"),
            )?;
            ed.set(
                l,
                row,
                &label,
                "Pal",
                &pal.to_string(),
                Some(&format!(
                    "the palette of the act the tiles were drawn for ({type_name})"
                )),
            )?;
            for c in ["QuestFlag", "QuestFlagEx"] {
                ed.set(l, row, &label, c, "", Some("no quest needed to enter"))?;
            }
            ed.set(l, row, &label, "Quest", "", None)?;
            // The automap layer stays the template's (PD2's maps share 98); above 100 crashes.
            if cell_num(&ed.tables.levels, row, "Layer") > MAX_LAYER {
                let layer = safe_layer(&ed.tables.levels).to_string();
                ed.set(
                    l,
                    row,
                    &label,
                    "Layer",
                    &layer,
                    Some("the game's levels use 0-100"),
                )?;
            }
            for s in ["", "(N)", "(H)"] {
                ed.set(
                    l,
                    row,
                    &label,
                    &format!("SizeX{s}"),
                    &size_x,
                    Some("the map is one cell bigger than the level"),
                )?;
                ed.set(
                    l,
                    row,
                    &label,
                    &format!("SizeY{s}"),
                    &size_y,
                    Some("the map is one cell bigger than the level"),
                )?;
            }
            ed.set(l, row, &label, "OffsetX", &off.0.to_string(), Some("a free spot in Act 5, away from the other levels and inside the automap's range"))?;
            ed.set(l, row, &label, "OffsetY", &off.1.to_string(), None)?;
            ed.set(l, row, &label, "Depend", "0", None)?;
            ed.set(
                l,
                row,
                &label,
                "DrlgType",
                "2",
                Some("a preset level: its map comes from LvlPrest"),
            )?;
            if own_type {
                ed.set(
                    l,
                    row,
                    &label,
                    "LevelType",
                    &type_id.to_string(),
                    Some("its own level type (see LvlTypes)"),
                )?;
            }
            for i in 0..8 {
                ed.set(
                    l,
                    row,
                    &label,
                    &format!("Vis{i}"),
                    "0",
                    (i == 0).then_some("no links yet (see Warps)"),
                )?;
                ed.set(l, row, &label, &format!("Warp{i}"), "-1", None)?;
            }
            ed.set(l, row, &label, "Waypoint", "255", Some("no waypoint"))?;
            ed.set(l, row, &label, "LevelName", &short, None)?;
            ed.set(l, row, &label, "LevelWarp", &short, None)?;
            // The loading-screen image: keep an Act 5 template's, else Harrogath's.
            let entry = cell(&ed.tables.levels, row, "EntryFile").trim().to_owned();
            if level_act(input.level_id) != 4 || entry.is_empty() {
                ed.set(
                    l,
                    row,
                    &label,
                    "EntryFile",
                    DEFAULT_ENTRY_FILE,
                    Some("the loading screen (data/local/ui/eng/expansion/A5L1.dc6, Harrogath's)"),
                )?;
            }
            new_level_id = Some(id);
            target = id;
            if pal != 4 {
                warnings.push(format!("The level is in Act 5 but uses the Act {} palette (Pal {pal}), because {type_name}'s tiles were drawn for that act. If the tiles show bright green, blue or red specks in game, convert them to the Act 5 palette.", pal + 1));
            }
        }
        AddMode::Existing => {
            let drlg = cell_num(&ed.tables.levels, level_row, "DrlgType");
            if drlg != 2 {
                return refuse(format!(
                    "{} is built at random (DrlgType {drlg}), not from one map, so it can't use this map. Make a new level instead.",
                    level_name_of(&ed.tables.levels, input.level_id)
                ));
            }
            let cur = (
                cell_num(&ed.tables.levels, level_row, "SizeX"),
                cell_num(&ed.tables.levels, level_row, "SizeY"),
            );
            let lvl_label = format!(
                "Level {} \"{}\"",
                input.level_id,
                level_name_of(&ed.tables.levels, input.level_id)
            );
            if cur != (i64::from(input.width) - 1, i64::from(input.height) - 1) {
                for s in ["", "(N)", "(H)"] {
                    ed.set(
                        TableName::Levels,
                        level_row,
                        &lvl_label,
                        &format!("SizeX{s}"),
                        &size_x,
                        Some("the map is one cell bigger than the level"),
                    )?;
                    ed.set(
                        TableName::Levels,
                        level_row,
                        &lvl_label,
                        &format!("SizeY{s}"),
                        &size_y,
                        None,
                    )?;
                }
                warnings.push(format!("The level's size changes from {}x{} to {size_x}x{size_y}. If it grows, check it doesn't overlap a neighbouring level (OffsetX/OffsetY).", cur.0, cur.1));
            }
            if own_type {
                ed.set(
                    TableName::Levels,
                    level_row,
                    &lvl_label,
                    "LevelType",
                    &type_id.to_string(),
                    Some("its own level type (see LvlTypes)"),
                )?;
            }
        }
    }

    // LvlPrest: the level's claiming row (Existing) or a new one (New). Def = record number.
    let claiming = prest_rows.iter().copied().find(|&r| {
        cell_num(&ed.tables.prest, r, "LevelId") == i64::from(target)
            && !cell(&ed.tables.prest, r, "Def").trim().is_empty()
    });
    let p_row;
    let p_label;
    let p = TableName::LvlPrest;
    if input.mode == AddMode::Existing {
        let Some(row) = claiming else {
            return refuse(format!(
                "No LvlPrest row claims {}.",
                level_name_of(&ed.tables.levels, target)
            ));
        };
        p_row = row;
        p_label = format!(
            "Preset {} \"{}\"",
            cell(&ed.tables.prest, row, "Def"),
            cell(&ed.tables.prest, row, "Name")
        );
        let files: Vec<String> = (1..=6)
            .map(|i| cell(&ed.tables.prest, row, &format!("File{i}")))
            .filter(|f| !f.is_empty() && f != "0")
            .collect();
        let listed = files
            .iter()
            .any(|f| crate::txt_util::rel_key(f) == crate::txt_util::rel_key(&input.map_rel));
        if !listed {
            warnings.push(format!(
                "{} will use this map instead of {} (that's the preset row the game reads for it).",
                level_name_of(&ed.tables.levels, target),
                if files.is_empty() {
                    "its current map".to_owned()
                } else {
                    files.join(", ")
                }
            ));
            ed.set(
                p,
                row,
                &p_label,
                "File1",
                &input.map_rel,
                Some("the level now loads this map"),
            )?;
            for i in 2..=6 {
                ed.set(p, row, &p_label, &format!("File{i}"), "0", None)?;
            }
            ed.set(p, row, &p_label, "Files", "1", None)?;
        }
    } else {
        // Flags from the template level's own preset, else what whole-level presets use.
        let tpl = prest_rows
            .iter()
            .copied()
            .find(|&r| cell_num(&ed.tables.prest, r, "LevelId") == i64::from(input.level_id));
        let flag = |col: &str, fallback: &str| -> String {
            tpl.map_or_else(
                || fallback.to_owned(),
                |r| {
                    let v = cell(&ed.tables.prest, r, col);
                    if v.is_empty() { fallback.to_owned() } else { v }
                },
            )
        };
        let def = prest_rows.len();
        let outdoors = if cell(&ed.tables.levels, level_row, "IsInside") == "1" {
            "0"
        } else {
            "1"
        };
        let values: Vec<(&str, String)> = vec![
            ("Name", name.clone()),
            ("Def", def.to_string()),
            ("LevelId", target.to_string()),
            ("Populate", flag("Populate", "1")),
            ("Logicals", flag("Logicals", "1")),
            ("Outdoors", flag("Outdoors", outdoors)),
            ("Animate", flag("Animate", "0")),
            ("KillEdge", flag("KillEdge", "1")),
            ("FillBlanks", "1".into()),
            ("SizeX", "0".into()),
            ("SizeY", "0".into()),
            ("AutoMap", "0".into()),
            ("Scan", "1".into()),
            ("Pops", "0".into()),
            ("PopPad", "0".into()),
            ("Files", "1".into()),
            ("File1", input.map_rel.clone()),
            ("File2", "0".into()),
            ("File3", "0".into()),
            ("File4", "0".into()),
            ("File5", "0".into()),
            ("File6", "0".into()),
            ("Dt1Mask", "0".into()),
            ("Beta", "0".into()),
            ("Expansion", "1".into()),
        ];
        let owned: Vec<(String, String)> = values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        p_row = append_values(&mut ed.tables.prest, &owned)?;
        p_label = format!("Preset {def} \"{name}\"");
        ed.note(
            p,
            &p_label,
            "(new row)",
            &format!("record {def} for level {target}"),
            None,
        );
        for (k, v) in &values {
            if has(&ed.tables.prest, k) {
                let why = match *k {
                    "Def" => Some("its row number: the game loads presets by Def"),
                    "LevelId" => Some("the level it builds"),
                    "FillBlanks" => Some("as every game preset level"),
                    "SizeX" => Some("0 = fill the level"),
                    "AutoMap" => Some("1 would reveal the whole automap on entry (towns)"),
                    "Scan" => Some("finds warps and special tiles"),
                    "Files" => Some("one map"),
                    "Expansion" => Some("an Act 5 level"),
                    "Populate" => Some(if tpl.is_some() {
                        "as the template level"
                    } else {
                        "spawn monsters"
                    }),
                    _ => None,
                };
                ed.note(p, &p_label, k, v, why);
            }
        }
    }

    // Dt1Mask and hide areas.
    let old_mask = crate::level_tables::preset_mask(&ed.tables.prest, p_row);
    let new_mask = if input.mode == AddMode::New || own_type {
        mask_of(&slots)
    } else {
        mask_for(&ed.tables.types, type_row, &input.used_dt1s, old_mask)
    };
    let mut slot_list: Vec<_> = slots.values().copied().collect();
    slot_list.sort_unstable();
    slot_list.dedup();
    ed.set(
        p,
        p_row,
        &p_label,
        "Dt1Mask",
        &new_mask.to_string(),
        Some(&format!(
            "LvlTypes \"{type_name}\" File slots {}",
            slot_list
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    )?;
    if i64::from(input.pop_count) > cell_num(&ed.tables.prest, p_row, "Pops") {
        ed.set(
            p,
            p_row,
            &p_label,
            "Pops",
            &input.pop_count.to_string(),
            Some("the map's roof hide areas"),
        )?;
        let pad = cell(&ed.tables.prest, p_row, "PopPad");
        if pad.is_empty() || pad == "0" {
            ed.set(
                p,
                p_row,
                &p_label,
                "PopPad",
                "-4",
                Some("as the game's houses"),
            )?;
        }
    }
    let preset_def = cell_num(&ed.tables.prest, p_row, "Def") as u32;
    Ok(AddMapOutcome {
        tables: ed.tables,
        changes: ed.changes,
        warnings,
        level_id: target,
        new_level_id,
        type_id,
        own_type,
        preset_def,
        touched: ed.touched,
    })
}
