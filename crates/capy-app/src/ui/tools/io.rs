//! Reading and writing game files for the tool windows. Reads go through the
//! profile's overlay (the edited copy in `capybara\saves` wins over the game).
//! Every write is `capybara\saves\<profile>\<game path>` + CHANGED.txt + a
//! history snapshot (`save_game_file`); nothing touches the game folder.

use std::collections::BTreeSet;
use std::path::PathBuf;

use capy_mod::level_tables::{LevelTables, TableName};
use d2_assets::GamePath;
use d2_formats::tbl::TblDoc;
use d2_formats::txt::TxtDoc;

use crate::state::AppState;

/// `data\global\excel\<name>`.
pub fn excel_path(name: &str) -> GamePath {
    GamePath::new(&format!("data\\global\\excel\\{name}"))
}

/// The open map's tile set: the one the renderer loaded, or (before it has)
/// loaded here from the map's DT1 list.
pub fn doc_tiles(state: &AppState) -> Option<std::sync::Arc<capy_render::TileSet>> {
    let doc = state.active_doc()?;
    if let Some(t) = &doc.tiles {
        return Some(t.clone());
    }
    let info = doc.info.as_ref()?;
    let overlay = &state.session.as_ref()?.overlay;
    capy_render::TileSet::load(overlay, &info.dt1_files)
        .ok()
        .map(|(t, _)| std::sync::Arc::new(t))
}

/// The bytes of a game file as the profile sees them.
pub fn read_game(state: &AppState, path: &GamePath) -> Result<Option<Vec<u8>>, String> {
    let session = state.session.as_ref().ok_or("No profile is open yet")?;
    session
        .overlay
        .read_bytes(path)
        .map_err(|e| format!("cannot read {path}: {e}"))
}

pub fn read_txt(state: &AppState, name: &str) -> Result<TxtDoc, String> {
    let path = excel_path(name);
    match read_game(state, &path)? {
        Some(bytes) => Ok(TxtDoc::parse(&bytes)),
        None => Err(format!(
            "{name} is not in this profile as a text table (the game may hold only the compiled .bin)"
        )),
    }
}

pub fn read_tbl(state: &AppState, path: &str) -> Result<TblDoc, String> {
    let game = GamePath::new(path);
    match read_game(state, &game)? {
        Some(bytes) => TblDoc::parse(&bytes).map_err(|e| format!("cannot read {path}: {e}")),
        None => Ok(TblDoc::new()),
    }
}

pub fn level_tables(state: &AppState) -> Result<LevelTables, String> {
    Ok(LevelTables {
        prest: read_txt(state, "lvlprest.txt")?,
        levels: read_txt(state, "levels.txt")?,
        types: read_txt(state, "lvltypes.txt")?,
        lvl_sub: read_txt(state, "lvlsub.txt").ok(),
    })
}

/// Writes `bytes` to the profile's saves, snapshots it and refreshes what
/// shows saves (CHANGED.txt count, history, the asset index).
pub fn save_game(state: &mut AppState, path: &GamePath, bytes: &[u8]) -> Result<(), String> {
    let profile = state
        .session
        .as_ref()
        .map(|s| s.profile.id.clone())
        .ok_or("No profile is open yet")?;
    let report = crate::edit::save::save_game_file(&state.paths, &profile, path, bytes)?;
    for w in &report.warnings {
        state.warn(format!("{path}: {w}"));
    }
    state.edit.changed = report.changed_total;
    state.history.invalidate();
    state.maps_list.source_cache.clear();
    state.start_saves_refresh();
    state.info(format!(
        "Saved {path} ({} bytes) to the profile's saves; CHANGED.txt lists {} file(s)",
        report.bytes, report.changed_total
    ));
    Ok(())
}

pub fn save_txt(state: &mut AppState, name: &str, doc: &TxtDoc) -> Result<(), String> {
    save_game(state, &excel_path(name), &doc.to_bytes())
}

pub fn save_tbl(state: &mut AppState, path: &str, doc: &TblDoc) -> Result<(), String> {
    let bytes = doc
        .to_bytes()
        .map_err(|e| format!("cannot write {path}: {e}"))?;
    save_game(state, &GamePath::new(path), &bytes)
}

/// Saves the level tables `touched` names, then rebuilds the map catalog.
pub fn save_level_tables(
    state: &mut AppState,
    tables: &LevelTables,
    touched: &BTreeSet<TableName>,
) -> Result<(), String> {
    for t in touched {
        let doc = match t {
            TableName::Levels => &tables.levels,
            TableName::LvlPrest => &tables.prest,
            TableName::LvlTypes => &tables.types,
        };
        save_txt(state, &t.file_name().to_ascii_lowercase(), doc)?;
    }
    state.rebuild_catalog();
    Ok(())
}

/// CHANGED.txt stops listing `gone` (files removed from the saves).
pub fn forget_changed(state: &mut AppState, gone: &[GamePath]) {
    let Some(id) = state.active_profile_id().map(str::to_owned) else {
        return;
    };
    let saves = capy_history::SavesDir::new(&state.paths.saves, &id);
    if let Ok(list) = saves.changed() {
        let keep: String = list
            .into_iter()
            .filter(|p| !gone.contains(p))
            .map(|p| format!("{}\r\n", p.as_str()))
            .collect();
        let _ = capy_history::atomic_write(&saves.root().join("CHANGED.txt"), keep.as_bytes());
    }
    state.refresh_changed();
}

/// The profile's saves folder on disk.
pub fn saves_root(state: &AppState) -> Option<PathBuf> {
    let id = state.active_profile_id()?;
    Some(state.paths.saves.join(id))
}

/// Every file below `dir` (relative game paths in `\` form, lower case).
pub fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}\\{name}")
        };
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path(), &rel, out),
            Ok(_) => out.push(rel),
            Err(_) => {}
        }
    }
}

/// The saved files of the profile (game paths).
pub fn saved_files(state: &AppState) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(root) = saves_root(state) {
        walk(&root, "", &mut out);
    }
    out.sort();
    out
}

/// `data\global\tiles\a\b.dt1` -> `a/b.dt1` (what the tables hold).
pub fn tiles_rel(path: &str) -> String {
    let p = path.replace('\\', "/");
    p.strip_prefix("data/global/tiles/")
        .unwrap_or(&p)
        .to_owned()
}

/// `a/b.dt1` -> `data\global\tiles\a\b.dt1`.
pub fn tiles_game(rel: &str) -> GamePath {
    GamePath::new(&format!("data\\global\\tiles\\{}", rel.replace('/', "\\")))
}
