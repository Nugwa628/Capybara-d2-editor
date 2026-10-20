//! Levels.txt rows for the warp, light and spawn views, read once per
//! profile session (`.txt` or `.bin`, as the profile's game version reads
//! them), and the maps each level is built from (for "open the target map").

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use capy_render::links::{LevelRef, LevelRow, MapWarp, WarpLink, level_links, map_warps};
use d2_assets::GamePath;
use d2_formats::excel::ExcelTableKind;

use crate::state::AppState;

#[derive(Default)]
pub struct Levels {
    /// Session epoch the rows were read for.
    epoch: Option<u64>,
    rows: Option<Arc<BTreeMap<i32, LevelRow>>>,
    /// Level id -> the DS1s the catalog places in it (built on first use).
    maps: Option<(u64, Arc<HashMap<i32, Vec<GamePath>>>)>,
    /// Warps of one map revision.
    warps: Option<((u64, u64, u64), Arc<Vec<MapWarp>>)>,
}

fn parse_rows(table: &d2_formats::txt::TxtTable) -> BTreeMap<i32, LevelRow> {
    let int = |r: &d2_formats::txt::Row<'_>, c: &str| r.get_i64(c).unwrap_or(0) as i32;
    let mut out = BTreeMap::new();
    for r in table.rows() {
        let Some(id) = r.get_i64("Id") else { continue };
        let id = id as i32;
        let name = r
            .get("LevelName")
            .or_else(|| r.get("Name"))
            .unwrap_or("")
            .to_owned();
        let mut row = LevelRow {
            id,
            name,
            intensity: int(&r, "Intensity"),
            rgb: [int(&r, "Red"), int(&r, "Green"), int(&r, "Blue")],
            ..LevelRow::default()
        };
        for k in 0..8 {
            row.vis[k] = int(&r, &format!("Vis{k}"));
            row.warp[k] = r.get_i64(&format!("Warp{k}")).map_or(-1, |v| v as i32);
        }
        out.insert(id, row);
    }
    out
}

/// Every Levels.txt row of the open profile (`None` when it cannot be read).
pub fn rows(state: &mut AppState) -> Option<Arc<BTreeMap<i32, LevelRow>>> {
    let epoch = state.session_epoch;
    if state.studio.levels.epoch == Some(epoch) {
        return state.studio.levels.rows.clone();
    }
    state.studio.levels.epoch = Some(epoch);
    state.studio.levels.rows = None;
    state.studio.levels.maps = None;
    state.studio.levels.warps = None;
    let session = state.session.as_ref()?;
    let version = session.profile.game_version;
    match d2_assets::tables::load_excel(&session.overlay, ExcelTableKind::Levels, version) {
        Ok(table) => {
            let rows = Arc::new(parse_rows(&table));
            state.studio.levels.rows = Some(rows.clone());
            Some(rows)
        }
        Err(e) => {
            state.warn(format!("Levels table: {e}"));
            None
        }
    }
}

/// The Levels.txt row of map `index` (by the catalog's level id).
pub fn row_of(state: &mut AppState, index: usize) -> Option<LevelRow> {
    let level = state.docs.get(index)?.info.as_ref()?.level_id?;
    let rows = rows(state)?;
    rows.get(&(level as i32)).cloned()
}

/// The warp tiles of map `index` and where each leads (cached per revision).
pub fn warps(state: &mut AppState, index: usize) -> Arc<Vec<MapWarp>> {
    let Some(doc) = state.docs.get(index) else {
        return Arc::new(Vec::new());
    };
    let key = (doc.id, doc.rev, state.session_epoch);
    if let Some((k, w)) = &state.studio.levels.warps
        && *k == key
    {
        return w.clone();
    }
    let ds1 = doc.ds1.clone();
    let links: [Option<WarpLink>; 8] = match (row_of(state, index), rows(state)) {
        (Some(row), Some(all)) => level_links(
            &row,
            &|id| LevelRef {
                id,
                name: all
                    .get(&id)
                    .map(|r| r.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Level {id}")),
                maps: Vec::new(),
            },
            &|_| None,
        ),
        _ => Default::default(),
    };
    let w = Arc::new(map_warps(&ds1, &links));
    state.studio.levels.warps = Some((key, w.clone()));
    w
}

/// The DS1s the catalog places in `level` (scans the profile's maps once).
pub fn maps_of_level(state: &mut AppState, level: i32) -> Vec<GamePath> {
    let epoch = state.session_epoch;
    let fresh = state
        .studio
        .levels
        .maps
        .as_ref()
        .is_some_and(|(e, _)| *e == epoch);
    if !fresh {
        let mut by: HashMap<i32, Vec<GamePath>> = HashMap::new();
        if let (Some(session), Some(catalog)) = (state.session.as_ref(), state.catalog.ready()) {
            for path in &session.maps {
                if let Some(id) = catalog.best(path).and_then(|i| i.level_id) {
                    by.entry(id as i32).or_default().push(path.clone());
                }
            }
        }
        state.studio.levels.maps = Some((epoch, Arc::new(by)));
    }
    state
        .studio
        .levels
        .maps
        .as_ref()
        .and_then(|(_, m)| m.get(&level).cloned())
        .unwrap_or_default()
}

/// Open the first map of the level a warp leads to.
pub fn open_target(state: &mut AppState, level: i32, name: &str) {
    let maps = maps_of_level(state, level);
    match maps.first() {
        Some(path) => {
            let path = path.clone();
            state.open_map(&path);
            if maps.len() > 1 {
                state.notify(format!(
                    "{name} is built from {} maps; opened {}",
                    maps.len(),
                    path.file_name()
                ));
            }
        }
        None => state.notify(format!(
            "{name} (level {level}) has no preset map: it is a random level"
        )),
    }
}
