//! Reading and writing game files for the tool windows. Reads go through the
//! profile's overlay (the edited copy in `capybara\saves` wins over the game).
//! Every write is `capybara\saves\<profile>\<game path>` + CHANGED.txt + a
//! history snapshot (`save_game_file`); nothing touches the game folder.

use std::collections::BTreeSet;
use std::path::PathBuf;

use capy_mod::level_tables::{LevelTables, TableName};
use d2_assets::tables::load_excel;
use d2_assets::{GamePath, GameVersion};
use d2_formats::excel::ExcelTableKind;
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

/// The three level tables from their `.txt` files. On 1.13c, where the game
/// reads compiled `.bin` tables, a `.txt` with a different number of rows
/// than the `.bin` is stale: it is refused, because a tool that saved it
/// would write tables missing the mod's rows. (Tables are never synthesized
/// from `.bin`.)
pub fn level_tables(state: &AppState) -> Result<LevelTables, String> {
    let t = LevelTables {
        prest: read_txt(state, "lvlprest.txt")?,
        levels: read_txt(state, "levels.txt")?,
        types: read_txt(state, "lvltypes.txt")?,
        lvl_sub: read_txt(state, "lvlsub.txt").ok(),
    };
    if reads_bin_tables(state) {
        for (name, doc, kind) in [
            ("lvlprest.txt", &t.prest, ExcelTableKind::LvlPrest),
            ("levels.txt", &t.levels, ExcelTableKind::Levels),
            ("lvltypes.txt", &t.types, ExcelTableKind::LvlTypes),
        ] {
            if let Some(c) = compiled_table(state, kind)
                && let Some(msg) = capy_mod::tiles_register::stale_against(name, doc, c.len())
            {
                return Err(msg);
            }
        }
    }
    Ok(t)
}

/// Whether the profile reads compiled `.bin` tables (1.13c, e.g. Median XL).
pub fn reads_bin_tables(state: &AppState) -> bool {
    state
        .session
        .as_ref()
        .is_some_and(|s| s.profile.game_version == GameVersion::V113c)
}

fn compiled_table(state: &AppState, kind: ExcelTableKind) -> Option<d2_formats::txt::TxtTable> {
    let session = state.session.as_ref()?;
    load_excel(&session.overlay, kind, GameVersion::V113c).ok()
}

/// A decoded compiled table as text, for reading only (the decoder exposes a
/// subset of the columns, so it must never be written back).
fn compiled_doc(t: &d2_formats::txt::TxtTable) -> TxtDoc {
    let mut text = t.headers().join("\t");
    text.push_str("\r\n");
    for r in t.rows() {
        text.push_str(&r.cells().join("\t"));
        text.push_str("\r\n");
    }
    TxtDoc::parse(text.as_bytes())
}

/// LvlTypes and LvlPrest as the game reads them, for working out a DT1
/// registration: the decoded `.bin` on 1.13c (true), else the `.txt`.
pub fn tables_for_tiles(state: &AppState) -> Result<(TxtDoc, TxtDoc, bool), String> {
    if reads_bin_tables(state)
        && let (Some(t), Some(p)) = (
            compiled_table(state, ExcelTableKind::LvlTypes),
            compiled_table(state, ExcelTableKind::LvlPrest),
        )
    {
        return Ok((compiled_doc(&t), compiled_doc(&p), true));
    }
    let t = level_tables(state)?;
    Ok((t.types, t.prest, false))
}

/// What registering a DT1 for the open map did.
pub struct Registered {
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
}

/// Saves a registration made on `types`/`prest` (from [`tables_for_tiles`]).
/// `.txt` profiles: the edited tables are written. `compiled` (1.13c): only
/// the File slot and Dt1Mask fields of `lvltypes.bin` / `lvlprest.bin` are
/// patched in place, and the `.txt` forms are updated to match when they
/// exist and hold the rows, so no recompiling is needed.
pub fn save_registration(
    state: &mut AppState,
    reg: &capy_mod::tiles_register::Registration,
    type_id: u32,
    def: Option<u32>,
    map: &str,
    edited: (&TxtDoc, &TxtDoc),
    compiled: bool,
) -> Result<(), String> {
    if !compiled {
        if read_txt(state, "lvltypes.txt").is_ok_and(|d| d != *edited.0) {
            save_txt(state, "lvltypes.txt", edited.0)?;
        }
        if read_txt(state, "lvlprest.txt").is_ok_and(|d| d != *edited.1) {
            save_txt(state, "lvlprest.txt", edited.1)?;
        }
        return Ok(());
    }
    let patch = |state: &mut AppState,
                 file: &str,
                 kind: ExcelTableKind,
                 edits: Vec<(usize, String, String)>|
     -> Result<(), String> {
        if edits.is_empty() {
            return Ok(());
        }
        let path = excel_path(file);
        let mut bytes = read_game(state, &path)?.ok_or_else(|| format!("{file} is missing"))?;
        for (rec, col, value) in edits {
            bytes = d2_formats::excel::patch_bin_field(kind, &bytes, rec, &col, &value)
                .map_err(|e| format!("{file}: {e}"))?;
        }
        save_game(state, &path, &bytes)
    };
    let mut type_edits = Vec::new();
    if reg.slot_added {
        type_edits.push((
            type_id as usize,
            format!("File {}", reg.slot),
            reg.slot_value.clone(),
        ));
    }
    patch(state, "lvltypes.bin", ExcelTableKind::LvlTypes, type_edits)?;
    let prest_edits = reg
        .masks
        .iter()
        .map(|(row, mask)| (*row, "Dt1Mask".to_owned(), mask.to_string()))
        .collect();
    patch(state, "lvlprest.bin", ExcelTableKind::LvlPrest, prest_edits)?;
    // keep the .txt forms in step when they have the rows
    if let (Ok(mut t), Ok(mut p)) = (
        read_txt(state, "lvltypes.txt"),
        read_txt(state, "lvlprest.txt"),
    ) {
        let (t0, p0) = (t.clone(), p.clone());
        capy_mod::tiles_register::mirror_registration(&mut t, &mut p, type_id, map, def, reg);
        if t != t0 {
            save_txt(state, "lvltypes.txt", &t)?;
        }
        if p != p0 {
            save_txt(state, "lvlprest.txt", &p)?;
        }
    }
    Ok(())
}

/// Makes the game load `rel` (a DT1 relative to data/global/tiles) for the
/// open map: LvlTypes File slot of its level type and the map's LvlPrest
/// Dt1Mask, saved to the profile's saves. Used by every feature that creates
/// a DT1 for a map.
pub fn register_for_open_map(state: &mut AppState, rel: &str) -> Result<Registered, String> {
    let (type_id, def, map) = {
        let doc = state.active_doc().ok_or("Open a map first")?;
        let info = doc
            .info
            .as_ref()
            .ok_or("The map's level type is not known yet")?;
        (
            info.lvl_type.ok_or("The map has no level type")?,
            info.lvlprest_def,
            tiles_rel(doc.path.as_str()),
        )
    };
    let (mut types, mut prest, compiled) = tables_for_tiles(state)?;
    let rows = capy_mod::tiles_register::map_prest_rows(&prest, &map, def);
    let reg = capy_mod::tiles_register::register_dt1(&mut types, &mut prest, type_id, &rows, rel)
        .map_err(|e| e.to_string())?;
    save_registration(state, &reg, type_id, def, &map, (&types, &prest), compiled)?;
    Ok(Registered {
        changes: reg.changes,
        warnings: reg.warnings,
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
