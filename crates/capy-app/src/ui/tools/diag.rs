//! The Diagnostics tab: unused DT1s/DS1s with archiving inside `capybara\`,
//! restoring archived assets, the compatibility check with one-click fixes,
//! and the crash-log reader.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;

use bevy_egui::egui::{self, Color32, Ui};
use capy_history::SavesDir;
use capy_mod::compat::{
    Dt1Candidate, Env, Finding, FixAction, LoadedDt1, MapInput, Severity, TableName as CTable,
    Tables, check_map, placed_tiles,
};
use capy_mod::crash_log::{explain_crash, parse_crash_log};
use capy_mod::tiles_review::TileId;
use capy_mod::tiles_usage::{
    ARCHIVE_DIR, ArchiveManifest, Ds1Asset, MapRefs, PlannedMove, ReferenceGraph, UnusedReport,
    build_dt1_graph, plan_archive, plan_restore, unused_report,
};
use d2_assets::GamePath;
use d2_formats::dt1::Dt1;

use super::io;
use crate::state::AppState;

#[derive(Default)]
pub struct DiagUi {
    pub unused: Option<UnusedReport>,
    pub chosen: BTreeSet<String>,
    pub msg: String,
    pub archives: Vec<(String, ArchiveManifest)>,
    pub findings: Vec<Finding>,
    pub kept: HashSet<String>,
    pub ran: bool,
    pub crash_path: String,
    pub crash_text: String,
    pub crash_lines: Vec<String>,
}

/// `capybara\archive\<profile>`.
fn archive_root(state: &AppState) -> Option<PathBuf> {
    Some(
        state
            .paths
            .root
            .join("archive")
            .join(state.active_profile_id()?),
    )
}

// ───────────────────────── unused assets ─────────────────────────

/// Scans which saved DT1s and DS1s nothing refers to.
pub fn scan_unused(state: &AppState) -> Result<UnusedReport, String> {
    let session = state.session.as_ref().ok_or("No profile is open yet")?;
    let catalog = state
        .catalog
        .ready()
        .ok_or("The map catalog is still loading; try again in a moment")?;
    let saved = io::saved_files(state);
    let mut maps: Vec<MapRefs> = Vec::new();
    for p in &session.maps {
        if let Some(info) = catalog.best(p) {
            maps.push(MapRefs {
                path: p.as_str().to_owned(),
                dt1s: info
                    .dt1_files
                    .iter()
                    .map(|d| d.as_str().to_owned())
                    .collect(),
                used: BTreeSet::new(),
            });
        }
    }
    let dt1_tiles: BTreeMap<String, Vec<TileId>> = saved
        .iter()
        .filter(|f| f.ends_with(".dt1"))
        .map(|f| (f.replace('\\', "/"), Vec::new()))
        .collect();
    let mut graph = ReferenceGraph {
        dt1s: build_dt1_graph(&maps, &dt1_tiles, &BTreeSet::new()),
        ..ReferenceGraph::default()
    };
    // A saved DT1 is also used when LvlTypes lists it.
    let types = io::read_txt(state, "lvltypes.txt").ok();
    let listed: HashSet<String> = types
        .iter()
        .flat_map(|t| {
            (0..t.row_count()).flat_map(move |r| {
                (0..t.headers().len()).filter_map(move |c| {
                    t.headers()[c]
                        .to_ascii_lowercase()
                        .starts_with("file")
                        .then(|| t.cell(r, c))
                        .flatten()
                })
            })
        })
        .filter_map(|v| GamePath::from_tile_ref(&v))
        .map(|p| p.as_str().replace('\\', "/"))
        .collect();
    for d in &mut graph.dt1s {
        if listed.contains(&d.path) && d.maps.is_empty() {
            d.maps.push("(listed in LvlTypes.txt)".into());
        }
    }
    let prest = io::read_txt(state, "lvlprest.txt").ok();
    for f in saved.iter().filter(|f| f.ends_with(".ds1")) {
        let rel = io::tiles_rel(f).to_ascii_lowercase();
        let mut by = Vec::new();
        if let Some(p) = &prest {
            for r in 0..p.row_count() {
                for n in 1..=6 {
                    if let Some(c) = p.column(&format!("File{n}"))
                        && let Some(v) = p.cell(r, c)
                        && v.replace('\\', "/").to_ascii_lowercase() == rel
                    {
                        by.push(format!("LvlPrest row {r}"));
                    }
                }
            }
        }
        graph.ds1s.push(Ds1Asset {
            path: f.replace('\\', "/"),
            referenced_by: by,
            protected: false,
        });
    }
    if prest.is_none() {
        graph.errors.push("LvlPrest.txt could not be read".into());
    }
    Ok(unused_report(&graph))
}

/// Archives `paths` (saved files, `/` form) under `capybara\archive\`.
pub fn archive(state: &mut AppState, paths: &[String]) -> Result<String, String> {
    let profile = state
        .active_profile_id()
        .ok_or("No profile is open")?
        .to_owned();
    let root = archive_root(state).ok_or("No profile is open")?;
    let saves = SavesDir::new(&state.paths.saves, &profile);
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let plan = plan_archive(paths, &stamp);
    let dir = root.join(&stamp);
    let mut moved = Vec::new();
    for m in &plan.moves {
        let from = saves.root().join(m.from.replace('/', "\\"));
        let to = dir.join(m.from.replace('/', "\\"));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::rename(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).and_then(|_| std::fs::remove_file(&from)))
            .map_err(|e| format!("cannot move {}: {e}", from.display()))?;
        moved.push(m.clone());
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), plan.manifest.to_json())
        .map_err(|e| format!("cannot write the manifest: {e}"))?;
    // CHANGED.txt no longer lists what was moved away.
    let gone: HashSet<GamePath> = moved.iter().map(|m| GamePath::new(&m.from)).collect();
    if let Ok(list) = saves.changed() {
        let keep: Vec<String> = list
            .into_iter()
            .filter(|p| !gone.contains(p))
            .map(|p| format!("{}\r\n", p.as_str()))
            .collect();
        let _ =
            capy_history::atomic_write(&saves.root().join("CHANGED.txt"), keep.concat().as_bytes());
    }
    state.refresh_changed();
    state.start_saves_refresh();
    let mut msg = format!("Archived {} file(s) to {}", moved.len(), dir.display());
    if !plan.refused.is_empty() {
        msg.push_str(&format!("; refused {}", plan.refused.len()));
    }
    Ok(msg)
}

fn read_archives(state: &AppState) -> Vec<(String, ArchiveManifest)> {
    let Some(root) = archive_root(state) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            if let Ok(text) = std::fs::read_to_string(e.path().join("manifest.json"))
                && let Some(m) = ArchiveManifest::from_json(&text)
            {
                out.push((e.file_name().to_string_lossy().into_owned(), m));
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// Moves an archive's files back into the saves (not over files that exist).
pub fn restore_archive(state: &mut AppState, stamp: &str) -> Result<String, String> {
    let root = archive_root(state).ok_or("No profile is open")?;
    let saves_root = io::saves_root(state).ok_or("No profile is open")?;
    let dir = root.join(stamp);
    let manifest = ArchiveManifest::from_json(
        &std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?,
    )
    .ok_or("the manifest is unreadable")?;
    let prefix = format!("{ARCHIVE_DIR}/{}/", manifest.stamp);
    let native = |p: &str| -> PathBuf {
        match p.strip_prefix(&prefix) {
            Some(rest) => dir.join(rest.replace('/', "\\")),
            None => saves_root.join(p.replace('/', "\\")),
        }
    };
    let exists = |p: &str| native(p).exists();
    let plan = plan_restore(&manifest, &exists);
    let mut restored = 0;
    for PlannedMove { from, to } in &plan.moves {
        let src = native(from);
        let bytes =
            std::fs::read(&src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
        io::save_game(state, &GamePath::new(to), &bytes)?;
        let _ = std::fs::remove_file(&src);
        restored += 1;
    }
    Ok(format!(
        "Restored {restored} file(s); {} already exist again, {} missing",
        plan.conflicts.len(),
        plan.missing.len()
    ))
}

pub fn unused(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.diag);
    ui.label("Saved DT1s and DS1s that no map loads and nothing lists. Archiving moves them to capybara\\archive (game files are never touched).");
    if ui.button("Scan").clicked() || d.unused.is_none() {
        match scan_unused(state) {
            Ok(r) => {
                d.unused = Some(r);
                d.chosen.clear();
                d.msg.clear();
            }
            Err(e) => {
                d.unused = None;
                d.msg = e;
            }
        }
    }
    if let Some(r) = d.unused.clone() {
        if !r.blocked.is_empty() {
            ui.colored_label(
                Color32::from_rgb(230, 150, 90),
                "The scan found problems, so nothing is offered:",
            );
            for b in &r.blocked {
                ui.label(b.as_str());
            }
        }
        ui.separator();
        ui.strong(format!("{} unused DT1(s)", r.unused_dt1s.len()));
        for p in &r.unused_dt1s {
            let mut on = d.chosen.contains(p);
            if ui.checkbox(&mut on, p.as_str()).changed() {
                if on {
                    d.chosen.insert(p.clone());
                } else {
                    d.chosen.remove(p);
                }
            }
        }
        ui.strong(format!("{} unreferenced DS1(s)", r.unused_ds1s.len()));
        for p in &r.unused_ds1s {
            let mut on = d.chosen.contains(p);
            if ui.checkbox(&mut on, p.as_str()).changed() {
                if on {
                    d.chosen.insert(p.clone());
                } else {
                    d.chosen.remove(p);
                }
            }
        }
        if ui
            .add_enabled(
                !d.chosen.is_empty(),
                egui::Button::new(format!("Archive {} selected", d.chosen.len())),
            )
            .clicked()
        {
            let list: Vec<String> = d.chosen.iter().cloned().collect();
            d.msg = archive(state, &list).unwrap_or_else(|e| e);
            d.unused = None;
            d.chosen.clear();
        }
    }
    super::message(ui, &d.msg);
    state.tools.diag = d;
}

pub fn restore(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.diag);
    if ui.button("Refresh").clicked() || d.archives.is_empty() {
        d.archives = read_archives(state);
    }
    if d.archives.is_empty() {
        ui.label("Nothing has been archived for this profile.");
    }
    let mut pick = None;
    for (stamp, m) in &d.archives {
        ui.horizontal(|ui| {
            ui.label(format!("{stamp}: {} file(s)", m.entries.len()));
            if ui.button("Restore").clicked() {
                pick = Some(stamp.clone());
            }
        });
    }
    if let Some(stamp) = pick {
        d.msg = restore_archive(state, &stamp).unwrap_or_else(|e| e);
        d.archives = read_archives(state);
    }
    super::message(ui, &d.msg);
    state.tools.diag = d;
}

// ───────────────────────── compatibility ─────────────────────────

fn load_tables(state: &AppState) -> Result<Tables, String> {
    Ok(Tables {
        prest: Some(io::read_txt(state, "lvlprest.txt")?),
        levels: Some(io::read_txt(state, "levels.txt")?),
        types: Some(io::read_txt(state, "lvltypes.txt")?),
        automap: io::read_txt(state, "automap.txt").ok(),
    })
}

/// Runs the checker on the open map.
pub fn run_compat(state: &AppState, kept: &HashSet<String>) -> Result<Vec<Finding>, String> {
    let doc = state.active_doc().ok_or("No map is open")?;
    let session = state.session.as_ref().ok_or("No profile is open yet")?;
    let tables = load_tables(state)?;
    let rel = io::tiles_rel(doc.path.as_str());
    let ds1 = doc.edit.ds1();
    let mut loaded: Vec<LoadedDt1> = Vec::new();
    if let Some(info) = &doc.info {
        for p in &info.dt1_files {
            let tiles = io::read_game(state, p)?
                .and_then(|b| Dt1::parse(&b).ok())
                .map(|d| d.tiles.iter().filter_map(|t| t.key()).collect())
                .unwrap_or_default();
            let found = session.overlay.resolve(p).is_some();
            loaded.push(LoadedDt1 {
                path: io::tiles_rel(p.as_str()),
                found,
                tiles,
            });
        }
        for p in &info.missing_dt1 {
            loaded.push(LoadedDt1 {
                path: io::tiles_rel(p.as_str()),
                found: false,
                tiles: HashSet::new(),
            });
        }
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, key) in placed_tiles(ds1) {
        if let Some(l) = loaded.iter().find(|l| l.tiles.contains(&key)) {
            *counts.entry(l.path.clone()).or_insert(0) += 1;
        }
    }
    let used: Vec<(String, usize)> = counts.into_iter().collect();
    let candidates: Vec<Dt1Candidate> = Vec::new();
    let input = MapInput {
        path: &rel,
        ds1,
        loaded: &loaded,
        candidates: &candidates,
        used_dt1s: &used,
    };
    let overlay = session.overlay.clone();
    let dt1_exists = |p: &str| overlay.resolve(&io::tiles_game(p)).is_some();
    let entry_exists = |n: &str| {
        overlay
            .resolve(&GamePath::new(&format!(
                "data/local/ui/eng/expansion/{n}.dc6"
            )))
            .is_some()
    };
    let is_kept = |k: &str| kept.contains(k);
    let mut compiled = Vec::new();
    if session.profile.game_version == d2_assets::GameVersion::V113c {
        for t in ["levels", "lvlprest", "lvltypes", "automap"] {
            if overlay
                .resolve(&io::excel_path(&format!("{t}.bin")))
                .is_some()
            {
                compiled.push(format!("{t}.txt"));
            }
        }
    }
    let env = Env {
        dt1_exists: Some(&dt1_exists),
        entry_image_exists: Some(&entry_exists),
        kept: Some(&is_kept),
        compiled_bins: compiled,
        ..Env::default()
    };
    Ok(check_map(&input, &tables, &env))
}

/// Applies the edits of fix `fix_index` of finding `finding_index`: tables
/// are saved to the saves, map edits are one undoable step. Returns whether
/// anything changed.
pub fn apply_fix(
    state: &mut AppState,
    finding: &Finding,
    fix_index: usize,
) -> Result<bool, String> {
    let fix = finding.fixes.get(fix_index).ok_or("No such fix")?.clone();
    let mut tables = load_tables(state)?;
    let before = tables.clone();
    let mut ds1 = state
        .active_doc()
        .ok_or("No map is open")?
        .edit
        .ds1()
        .clone();
    let orig_ds1 = ds1.clone();
    let changed = fix
        .apply(&mut tables, Some(&mut ds1))
        .map_err(|e| e.to_string())?;
    if !changed {
        return Ok(false);
    }
    for (t, name) in [
        (CTable::LvlPrest, "lvlprest.txt"),
        (CTable::Levels, "levels.txt"),
        (CTable::LvlTypes, "lvltypes.txt"),
        (CTable::AutoMap, "automap.txt"),
    ] {
        if let (Some(a), Some(b)) = (tables.doc(t), before.doc(t))
            && a != b
        {
            io::save_txt(state, name, a)?;
        }
    }
    if ds1 != orig_ds1 {
        super::game::replace_map(state, &format!("Fix: {}", fix.label), ds1)?;
    }
    state.rebuild_catalog();
    Ok(true)
}

fn colour(s: Severity) -> Color32 {
    match s {
        Severity::Error => Color32::from_rgb(235, 100, 90),
        Severity::Warning => Color32::from_rgb(230, 185, 80),
        Severity::Info => Color32::from_rgb(130, 170, 230),
        Severity::Ok => Color32::from_rgb(110, 200, 120),
    }
}

pub fn compat(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.diag);
    ui.horizontal(|ui| {
        if ui.button("Check the open map").clicked() || !d.ran {
            d.ran = true;
            match run_compat(state, &d.kept) {
                Ok(f) => {
                    d.findings = f;
                    d.msg.clear();
                }
                Err(e) => d.msg = e,
            }
        }
        let n = |s| d.findings.iter().filter(|f| f.severity == s).count();
        ui.label(format!(
            "{} error(s), {} warning(s), {} note(s), {} passed",
            n(Severity::Error),
            n(Severity::Warning),
            n(Severity::Info),
            n(Severity::Ok)
        ));
    });
    super::message(ui, &d.msg);
    ui.separator();
    let mut apply: Option<(usize, usize)> = None;
    let mut open: Option<&'static str> = None;
    let mut keep: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("compat_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, f) in d.findings.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        colour(f.severity),
                        match f.severity {
                            Severity::Error => "ERROR",
                            Severity::Warning => "WARN",
                            Severity::Info => "NOTE",
                            Severity::Ok => "OK",
                        },
                    );
                    ui.label(f.title.as_str());
                });
                if let Some(det) = &f.detail {
                    ui.add(egui::Label::new(egui::RichText::new(det.as_str()).weak()).wrap());
                }
                ui.horizontal_wrapped(|ui| {
                    for (j, fix) in f.fixes.iter().enumerate() {
                        if ui.small_button(fix.label.as_str()).clicked() {
                            match &fix.action {
                                FixAction::Edits(_) => apply = Some((i, j)),
                                FixAction::OpenTable { table, .. } => {
                                    open = Some("game.tables");
                                    let _ = table;
                                }
                                FixAction::OpenAutomapEditor => open = Some("game.automap"),
                                FixAction::Register => open = Some("game.addToGame"),
                                FixAction::Keep { key } => keep = Some(key.clone()),
                                FixAction::ShortenPaths { .. } => {}
                            }
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });
    if let Some((i, j)) = apply {
        let f = d.findings[i].clone();
        d.msg = match apply_fix(state, &f, j) {
            Ok(true) => "Fixed.".to_owned(),
            Ok(false) => "Already fixed; nothing changed.".to_owned(),
            Err(e) => e,
        };
        if let Ok(f) = run_compat(state, &d.kept) {
            d.findings = f;
        }
    }
    if let Some(k) = keep {
        d.kept.insert(k);
        if let Ok(f) = run_compat(state, &d.kept) {
            d.findings = f;
        }
    }
    state.tools.diag = d;
    if let Some(w) = open {
        state.tools.open.insert(w);
    }
}

// ───────────────────────── crash log ─────────────────────────

/// The crash log as readable lines.
pub fn explain_log(text: &str) -> Vec<String> {
    let crashes = parse_crash_log(text);
    if crashes.is_empty() {
        return vec!["No crash found in this text.".to_owned()];
    }
    let mut out = Vec::new();
    for c in &crashes {
        let e = explain_crash(c, None);
        out.push(format!("{} {}", c.time, e.title));
        out.push(format!("  {}", e.detail));
        if !c.failed_files.is_empty() {
            out.push(format!(
                "  Files it could not open: {}",
                c.failed_files.join(", ")
            ));
        }
    }
    out
}

pub fn crash_log(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.diag);
    ui.label("Paste the game's crash log (d2debug / Crashdump text) or open the file.");
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut d.crash_path)
                .hint_text("path of the log")
                .desired_width(360.0),
        );
        if ui.button("Browse...").clicked()
            && let Some(p) = rfd::FileDialog::new().pick_file()
        {
            d.crash_path = p.display().to_string();
        }
        if ui.button("Read").clicked() {
            match std::fs::read(&d.crash_path) {
                Ok(b) => {
                    d.crash_text = b.iter().map(|&c| c as char).collect();
                    d.crash_lines = explain_log(&d.crash_text);
                }
                Err(e) => d.msg = format!("Cannot read {}: {e}", d.crash_path),
            }
        }
    });
    ui.add(
        egui::TextEdit::multiline(&mut d.crash_text)
            .desired_rows(6)
            .desired_width(f32::INFINITY),
    );
    if ui.button("Explain").clicked() {
        d.crash_lines = explain_log(&d.crash_text);
    }
    super::message(ui, &d.msg);
    ui.separator();
    for l in &d.crash_lines {
        ui.add(egui::Label::new(l.as_str()).wrap());
    }
    state.tools.diag = d;
}
