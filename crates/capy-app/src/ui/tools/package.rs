//! Map packages (`.zip`): Export a map with the DT1s it needs and the table
//! rows that register it; Import merges a package into the profile's saves
//! (each file through CHANGED.txt and a snapshot) and applies its rows with
//! the level-type package merge; "DS1 with its DT1s" imports a loose map and
//! finds its tile libraries in a folder.

use std::path::{Path, PathBuf};

use bevy_egui::egui::{self, Ui};
use capy_history::{
    ImportOptions, OnConflict, PackEntry, PackageSpec, SavesDir, SnapshotKind, TableRows,
    export_package_to, import_loose_map, import_package,
};
use capy_mod::level_tables::{LevelTables, map_level_type, preset_rows_for};
use capy_mod::type_package::{PackageDt1, PackageRow, TypePackage, export_type_package};
use d2_assets::GamePath;
use d2_formats::ds1::Ds1;

use super::io;
use crate::doc::SourceCategory;
use crate::state::AppState;

#[derive(Default)]
pub struct PackageUi {
    pub out: String,
    pub note: String,
    pub input: String,
    pub overwrite: bool,
    pub include_base: bool,
    pub loose_ds1: String,
    pub loose_path: String,
    pub loose_folder: String,
    pub msg: String,
}

pub fn on_open(state: &mut AppState) {
    let stem = state
        .active_doc()
        .and_then(|d| d.path.as_str().rsplit('\\').next().map(str::to_owned))
        .and_then(|f| f.strip_suffix(".ds1").map(str::to_owned))
        .unwrap_or_else(|| "map".into());
    let dir = state.paths.root.join("exports");
    let p = &mut state.tools.package;
    p.out = dir.join(format!("{stem}.zip")).display().to_string();
}

/// The table rows that register `map_rel`: its level type, level and preset
/// rows, as the type-package merge reads them back.
fn rows_for(tables: &LevelTables, map_rel: &str) -> Vec<TableRows> {
    if preset_rows_for(&tables.prest, map_rel).is_empty() {
        return Vec::new();
    }
    let (_, type_id) = map_level_type(tables, map_rel, None);
    let Ok(pkg) = export_type_package(tables, type_id) else {
        return Vec::new();
    };
    let want = map_rel.replace('\\', "/").to_ascii_lowercase();
    let lists_map = |row: &PackageRow| {
        (1..=6).any(|n| {
            row.value(&format!("File{n}"))
                .replace('\\', "/")
                .to_ascii_lowercase()
                == want
        })
    };
    let presets: Vec<&PackageRow> = pkg.presets.iter().filter(|r| lists_map(r)).collect();
    let level_ids: Vec<String> = presets.iter().map(|r| r.value("LevelId")).collect();
    let levels: Vec<&PackageRow> = pkg
        .levels
        .iter()
        .filter(|l| level_ids.contains(&l.value("Id")))
        .collect();
    let mk = |table: &str, rows: &[&PackageRow]| {
        rows.first().map(|first| TableRows {
            table: table.to_owned(),
            header: first.columns.clone(),
            rows: rows.iter().map(|r| r.row.clone()).collect(),
        })
    };
    [
        mk("LvlTypes.txt", &[&pkg.type_row]),
        mk("Levels.txt", &levels),
        mk("LvlPrest.txt", &presets),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Exports the open map to `out`.
pub fn export_run(state: &mut AppState, out: &str, note: &str) -> Result<String, String> {
    let (spec, n_files, n_rows) = {
        let doc = state.active_doc().ok_or("Open a map first")?;
        let session = state.session.as_ref().ok_or("No profile is open yet")?;
        let map_bytes = doc
            .edit
            .ds1()
            .to_bytes()
            .map_err(|e| format!("cannot write the map: {e}"))?;
        let mut files = Vec::new();
        if let Some(info) = &doc.info {
            for p in &info.dt1_files {
                let Some((layer, bytes)) = session
                    .overlay
                    .read(p)
                    .map_err(|e| format!("cannot read {p}: {e}"))?
                else {
                    continue;
                };
                let base_game = crate::state::source_info(&session.overlay, layer).category
                    == SourceCategory::Base;
                files.push(PackEntry {
                    path: p.clone(),
                    bytes,
                    base_game,
                });
            }
        }
        let rows = match io::level_tables(state) {
            Ok(t) => rows_for(&t, &io::tiles_rel(doc.path.as_str())),
            Err(_) => Vec::new(),
        };
        let (n_files, n_rows) = (files.len(), rows.len());
        (
            PackageSpec {
                map_path: Some(doc.path.clone()),
                map_bytes,
                files,
                rows,
                note: (!note.trim().is_empty()).then(|| note.trim().to_owned()),
            },
            n_files,
            n_rows,
        )
    };
    if let Some(parent) = Path::new(out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    export_package_to(Path::new(out), &spec).map_err(|e| format!("cannot write {out}: {e}"))?;
    Ok(format!(
        "Exported the map with {n_files} tile librar{} and {n_rows} table row group(s) to {out}",
        if n_files == 1 { "y" } else { "ies" }
    ))
}

/// Applies a package's table rows through the type-package merge.
pub fn apply_rows(state: &mut AppState, rows: &[TableRows]) -> Result<String, String> {
    let find = |name: &str| rows.iter().find(|r| r.table.eq_ignore_ascii_case(name));
    let Some(types) = find("LvlTypes.txt") else {
        return Ok("The package has no table rows to merge".into());
    };
    let row = |t: &TableRows, r: &Vec<String>| PackageRow {
        table: t.table.clone(),
        columns: t.header.clone(),
        row: r.clone(),
    };
    let type_row = row(types, types.rows.first().ok_or("empty LvlTypes rows")?);
    let dt1s: Vec<PackageDt1> = (1..=32u8)
        .filter_map(|s| {
            let v = type_row.value(&format!("File {s}"));
            (!v.trim().is_empty() && v.trim() != "0").then_some(PackageDt1 { slot: s, path: v })
        })
        .collect();
    let levels: Vec<PackageRow> = find("Levels.txt")
        .map(|t| t.rows.iter().map(|r| row(t, r)).collect())
        .unwrap_or_default();
    let presets: Vec<PackageRow> = find("LvlPrest.txt")
        .map(|t| t.rows.iter().map(|r| row(t, r)).collect())
        .unwrap_or_default();
    let mut maps = Vec::new();
    for p in &presets {
        for n in 1..=6 {
            let v = p.value(&format!("File{n}"));
            if !v.trim().is_empty() && v.trim() != "0" {
                maps.push(v.replace('\\', "/"));
            }
        }
    }
    let pkg = TypePackage {
        type_id: type_row.value("Id").trim().parse().unwrap_or(0),
        type_name: type_row.value("Name"),
        type_row,
        dt1s,
        maps,
        levels,
        presets,
    };
    super::game::merge_type_package(state, &pkg)
}

fn snapshot_written(state: &mut AppState, written: &[GamePath]) {
    let Some(profile) = state.active_profile_id().map(str::to_owned) else {
        return;
    };
    let saves = SavesDir::new(&state.paths.saves, &profile);
    for p in written {
        if let Ok(path) = saves.path_for(p)
            && let Ok(bytes) = std::fs::read(&path)
        {
            let _ = crate::history::with_store(&state.paths.history, &profile, |s| {
                s.record(p, &bytes, SnapshotKind::Save)
            });
        }
    }
}

fn after_import(state: &mut AppState, map: Option<&GamePath>) {
    if let Some(m) = map {
        state.register_map(m);
    }
    state.refresh_changed();
    state.history.invalidate();
    state.maps_list.source_cache.clear();
    state.start_saves_refresh();
}

/// Imports the package at `path`.
pub fn import_run(
    state: &mut AppState,
    path: &str,
    overwrite: bool,
    include_base: bool,
) -> Result<String, String> {
    let profile = state
        .active_profile_id()
        .ok_or("No profile is open")?
        .to_owned();
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let saves = SavesDir::new(&state.paths.saves, &profile);
    let opts = ImportOptions {
        on_conflict: if overwrite {
            OnConflict::Overwrite
        } else {
            OnConflict::Skip
        },
        include_base_game: include_base,
    };
    let report =
        import_package(&saves, &bytes, &opts).map_err(|e| format!("cannot import: {e}"))?;
    snapshot_written(state, &report.written);
    let rows_msg = if report.rows.is_empty() {
        String::new()
    } else {
        format!("; {}", apply_rows(state, &report.rows)?)
    };
    after_import(state, report.map.as_ref());
    Ok(format!(
        "Imported {}: {} written, {} unchanged, {} kept (differ), {} base-game left out{rows_msg}",
        report
            .map
            .as_ref()
            .map_or("the package".to_owned(), |m| m.as_str().to_owned()),
        report.written.len(),
        report.unchanged.len(),
        report.conflicts.len(),
        report.skipped_base_game.len()
    ))
}

/// Imports a DS1 from disk with the DT1s it lists, found under `folder`.
pub fn import_loose_run(
    state: &mut AppState,
    ds1_file: &str,
    game_path: &str,
    folder: &str,
    overwrite: bool,
) -> Result<String, String> {
    let profile = state
        .active_profile_id()
        .ok_or("No profile is open")?
        .to_owned();
    let bytes = std::fs::read(ds1_file).map_err(|e| format!("cannot read {ds1_file}: {e}"))?;
    let ds1 = Ds1::parse(&bytes).map_err(|e| format!("not a DS1: {e}"))?;
    let needed: Vec<GamePath> = ds1
        .files
        .iter()
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .filter(|f| !f.contains("DS1EDIT_WRKSPC"))
        .filter(|f| {
            let l = f.to_ascii_lowercase();
            l.ends_with(".dt1") || l.ends_with(".tg1")
        })
        .filter_map(|f| GamePath::from_tile_ref(&f))
        .collect();
    let saves = SavesDir::new(&state.paths.saves, &profile);
    let opts = ImportOptions {
        on_conflict: if overwrite {
            OnConflict::Overwrite
        } else {
            OnConflict::Skip
        },
        include_base_game: true,
    };
    let ds1_path = GamePath::new(game_path);
    let loose = import_loose_map(
        &saves,
        Path::new(ds1_file),
        &ds1_path,
        &PathBuf::from(folder),
        &needed,
        &opts,
    )
    .map_err(|e| format!("cannot import: {e}"))?;
    snapshot_written(state, &loose.report.written);
    after_import(state, Some(&ds1_path));
    Ok(format!(
        "Imported {ds1_path}: {} written, {} unchanged; {} of {} tile librar{} not found in the folder",
        loose.report.written.len(),
        loose.report.unchanged.len(),
        loose.missing.len(),
        needed.len(),
        if needed.len() == 1 { "y" } else { "ies" }
    ))
}

fn path_row(
    ui: &mut Ui,
    label: &str,
    text: &mut String,
    filter: Option<(&str, &[&str])>,
    folder: bool,
    save: bool,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(text).desired_width(360.0));
        if ui.button("Browse...").clicked() {
            let mut dlg = rfd::FileDialog::new();
            if let Some((n, e)) = filter {
                dlg = dlg.add_filter(n, e);
            }
            let picked = if folder {
                dlg.pick_folder()
            } else if save {
                dlg.save_file()
            } else {
                dlg.pick_file()
            };
            if let Some(p) = picked {
                *text = p.display().to_string();
            }
        }
    });
}

pub fn export(ui: &mut Ui, state: &mut AppState) {
    let mut p = std::mem::take(&mut state.tools.package);
    ui.label("Packs the open map, the tile libraries it loads and the table rows that register it into one .zip.");
    path_row(
        ui,
        "Save as",
        &mut p.out,
        Some(("zip", &["zip"])),
        false,
        true,
    );
    ui.horizontal(|ui| {
        ui.label("Note");
        ui.add(egui::TextEdit::singleline(&mut p.note).desired_width(360.0));
    });
    if ui.button("Export the package").clicked() {
        p.msg = export_run(state, &p.out.clone(), &p.note.clone()).unwrap_or_else(|e| e);
    }
    super::message(ui, &p.msg);
    state.tools.package = p;
}

pub fn import(ui: &mut Ui, state: &mut AppState) {
    let mut p = std::mem::take(&mut state.tools.package);
    ui.label("Merges a map package into this profile's saves. Files that differ from yours are kept unless you choose to overwrite.");
    path_row(
        ui,
        "Package",
        &mut p.input,
        Some(("zip", &["zip"])),
        false,
        false,
    );
    ui.checkbox(&mut p.overwrite, "Overwrite files that differ");
    ui.checkbox(&mut p.include_base, "Also import files marked as base game");
    if ui.button("Import").clicked() {
        p.msg =
            import_run(state, &p.input.clone(), p.overwrite, p.include_base).unwrap_or_else(|e| e);
    }
    super::message(ui, &p.msg);
    state.tools.package = p;
}

pub fn import_loose(ui: &mut Ui, state: &mut AppState) {
    let mut p = std::mem::take(&mut state.tools.package);
    ui.label("Imports a DS1 and finds the DT1 libraries it lists in a folder.");
    path_row(
        ui,
        "DS1 file",
        &mut p.loose_ds1,
        Some(("ds1", &["ds1"])),
        false,
        false,
    );
    if p.loose_path.is_empty() && !p.loose_ds1.is_empty() {
        let name = Path::new(&p.loose_ds1)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        p.loose_path = format!("data\\global\\tiles\\custom\\{name}");
    }
    ui.horizontal(|ui| {
        ui.label("Game path");
        ui.add(egui::TextEdit::singleline(&mut p.loose_path).desired_width(360.0));
    });
    path_row(ui, "DT1 folder", &mut p.loose_folder, None, true, false);
    ui.checkbox(&mut p.overwrite, "Overwrite files that differ");
    if ui.button("Import").clicked() {
        p.msg = import_loose_run(
            state,
            &p.loose_ds1.clone(),
            &p.loose_path.clone(),
            &p.loose_folder.clone(),
            p.overwrite,
        )
        .unwrap_or_else(|e| e);
    }
    super::message(ui, &p.msg);
    state.tools.package = p;
}
