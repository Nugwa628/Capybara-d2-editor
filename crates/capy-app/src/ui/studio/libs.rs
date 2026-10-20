//! DT1 libraries made for one map (ds1-studio `floorLibrary.ts`,
//! `ownTiles.ts`): `<map>_custom.dt1` holds floors copied from other
//! libraries (Re-roll floors), `<map>_walk.dt1` the blockers and tile copies
//! of sub-tile walk painting. Tiles are only ever added, so undoing the
//! map's edits never leaves a cell on a tile that is gone. Both files are
//! written to the profile's saves and listed in the DS1's file list.

use std::collections::BTreeSet;

use d2_assets::GamePath;
use d2_assets::tables::load_excel;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::{Dt1, Tile};
use d2_formats::excel::ExcelTableKind;

use super::walkplan::tileset_keys;
use crate::state::AppState;

/// `<map>_custom.dt1`, next to the map.
pub fn custom_path(map: &GamePath) -> GamePath {
    sibling(map, "_custom")
}

/// `<map>_walk.dt1`, next to the map.
pub fn walk_path(map: &GamePath) -> GamePath {
    sibling(map, "_walk")
}

fn sibling(map: &GamePath, suffix: &str) -> GamePath {
    let s = map.as_str();
    let stem = s
        .strip_suffix(".ds1")
        .or_else(|| s.strip_suffix(".DS1"))
        .unwrap_or(s);
    GamePath::new(&format!("{stem}{suffix}.dt1"))
}

/// The DS1 file-list entry of a DT1: `\d2\data\global\tiles\...\x.tg1`.
pub fn file_entry(dt1: &GamePath) -> Vec<u8> {
    let s = dt1.as_str().replace('/', "\\");
    let stem = s.strip_suffix(".dt1").unwrap_or(&s);
    format!("\\d2\\{stem}.tg1").into_bytes()
}

/// Whether `dt1` is in the DS1's file list.
pub fn lists(ds1: &Ds1, dt1: &GamePath) -> bool {
    ds1.tileset_refs()
        .filter_map(|r| GamePath::from_tile_ref(&String::from_utf8_lossy(r)))
        .any(|p| p == *dt1)
}

/// `ds1` with `dt1` in its file list (as is when it already is).
pub fn with_file(ds1: &Ds1, dt1: &GamePath) -> Ds1 {
    let mut out = ds1.clone();
    if !lists(&out, dt1) {
        out.files.push(file_entry(dt1));
    }
    out
}

/// Makes the map's tile set load `dt1` too (the DS1 list is changed by the
/// caller's edit).
pub fn attach_to_info(state: &mut AppState, index: usize, dt1: &GamePath) {
    if let Some(info) = state.docs.get_mut(index).and_then(|d| d.info.as_mut())
        && !info.dt1_files.contains(dt1)
    {
        info.dt1_files.push(dt1.clone());
    }
}

/// The library as stored in the saves or the game (`None`: not there yet).
pub fn read_dt1(state: &AppState, path: &GamePath) -> Result<Option<Dt1>, String> {
    match super::super::tools::io::read_game(state, path)? {
        Some(bytes) => Dt1::parse(&bytes)
            .map(Some)
            .map_err(|e| format!("{path}: {e}")),
        None => Ok(None),
    }
}

/// Writes `dt1` to the profile's saves.
pub fn save_dt1(state: &mut AppState, path: &GamePath, dt1: &Dt1) -> Result<(), String> {
    let bytes = dt1.to_bytes().map_err(|e| format!("{path}: {e}"))?;
    super::super::tools::io::save_game(state, path, &bytes)
}

/// The DT1s of the map's level type that are not in its tile set (or, with
/// `all`, of every level type): the libraries Re-roll floors can take floors
/// from besides the map's own.
pub fn level_type_libs(state: &AppState, index: usize, all: bool) -> Vec<String> {
    let (Some(session), Some(doc)) = (state.session.as_ref(), state.docs.get(index)) else {
        return Vec::new();
    };
    let Ok(table) = load_excel(
        &session.overlay,
        ExcelTableKind::LvlTypes,
        session.profile.game_version,
    ) else {
        return Vec::new();
    };
    let own: Vec<String> = doc
        .info
        .as_ref()
        .map(|i| {
            i.dt1_files
                .iter()
                .map(|p| p.as_str().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let wanted = doc.info.as_ref().and_then(|i| i.lvl_type);
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (n, row) in table.rows().enumerate() {
        let id = row
            .get_i64("Id")
            .or_else(|| row.get_i64("*Id"))
            .unwrap_or(n as i64);
        if !all && Some(id as u32) != wanted {
            continue;
        }
        for (h, cell) in table.headers().iter().zip(row.cells()) {
            if h.to_ascii_lowercase().starts_with("file")
                && let Some(p) = GamePath::from_tile_ref(cell)
                && p.as_str().to_ascii_lowercase().ends_with(".dt1")
                && !own.contains(&p.as_str().to_ascii_lowercase())
            {
                out.insert(p.as_str().to_owned());
            }
        }
    }
    out.into_iter().collect()
}

/// What copying floors from another library produced.
#[derive(Debug, Default)]
pub struct FloorCopies {
    /// The new tiles, to append to `<map>_custom.dt1`.
    pub tiles: Vec<Tile>,
    /// `(main, sub, rarity, walk signature)` of each new floor number.
    pub choices: Vec<(u8, u8, u32, u32)>,
}

fn walk_sig(t: &Tile) -> u32 {
    super::dialogs::walk_signature(&t.subtile_flags)
}

/// Copies the picked floors of `source` under free numbers. A picked frame
/// of an animation takes the whole animation along; a static floor gets
/// rarity 1 so it can be chosen next to weighted tiles. `taken` holds the
/// numbers in use and gains the new ones.
pub fn copy_floors(
    source: &Dt1,
    picks: &BTreeSet<usize>,
    taken: &mut BTreeSet<(i32, i32, i32)>,
    out: &mut FloorCopies,
) -> Result<(), String> {
    let mut done: BTreeSet<(i32, i32)> = BTreeSet::new();
    for &i in picks {
        let tile = source
            .tiles
            .get(i)
            .filter(|t| t.orientation == 0)
            .ok_or_else(|| format!("tile #{i} is not a floor"))?;
        let group: Vec<&Tile> = source
            .tiles
            .iter()
            .filter(|t| {
                t.orientation == 0
                    && t.main_index == tile.main_index
                    && t.sub_index == tile.sub_index
            })
            .collect();
        let animated = group.len() > 1 && group.iter().all(|t| t.animated != 0);
        if animated && !done.insert((tile.main_index, tile.sub_index)) {
            continue;
        }
        let (main, sub) = (0..64)
            .flat_map(|m| (0..256).map(move |s| (m, s)))
            .find(|&(m, s)| !taken.contains(&(0, m, s)))
            .ok_or("no free floor number is left in the map's tile set")?;
        taken.insert((0, main, sub));
        let frames: Vec<&Tile> = if animated { group } else { vec![tile] };
        for f in frames {
            let mut t = f.clone();
            t.main_index = main;
            t.sub_index = sub;
            if !animated {
                t.rarity = 1;
            }
            out.choices
                .push((main as u8, sub as u8, t.rarity.max(0) as u32, walk_sig(&t)));
            out.tiles.push(t);
        }
    }
    Ok(())
}

/// The keys of the map's tile set (plus a library's own tiles).
pub fn taken_keys(
    state: &AppState,
    index: usize,
    extra: Option<&Dt1>,
) -> BTreeSet<(i32, i32, i32)> {
    let mut taken = state
        .docs
        .get(index)
        .and_then(|d| d.tiles.as_ref())
        .map(|t| tileset_keys(t))
        .unwrap_or_default();
    if let Some(d) = extra {
        taken.extend(
            d.tiles
                .iter()
                .map(|t| (t.orientation, t.main_index, t.sub_index)),
        );
    }
    taken
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn floor(main: i32, sub: i32, animated: u8) -> Tile {
        Tile {
            main_index: main,
            sub_index: sub,
            animated,
            rarity: 0,
            ..Tile::default()
        }
    }

    #[test]
    fn paths_sit_next_to_the_map() {
        let m = GamePath::new("data\\global\\tiles\\act1\\town\\townn1.ds1");
        assert_eq!(
            walk_path(&m).as_str(),
            "data\\global\\tiles\\act1\\town\\townn1_walk.dt1"
        );
        assert_eq!(
            custom_path(&m).as_str(),
            "data\\global\\tiles\\act1\\town\\townn1_custom.dt1"
        );
        assert_eq!(
            GamePath::from_tile_ref(&String::from_utf8(file_entry(&walk_path(&m))).unwrap()),
            Some(walk_path(&m))
        );
    }

    #[test]
    fn copies_get_free_numbers_and_animations_move_whole() {
        let src = Dt1 {
            version: (7, 6),
            tiles: vec![floor(1, 0, 0), floor(2, 0, 1), floor(2, 0, 1)],
        };
        let mut taken: BTreeSet<(i32, i32, i32)> = BTreeSet::from([(0, 0, 0), (0, 0, 1)]);
        let mut out = FloorCopies::default();
        copy_floors(&src, &BTreeSet::from([0, 1]), &mut taken, &mut out).unwrap();
        assert_eq!(
            out.tiles.len(),
            3,
            "one static floor and a two frame animation"
        );
        assert_eq!((out.tiles[0].main_index, out.tiles[0].sub_index), (0, 2));
        assert_eq!(out.tiles[0].rarity, 1);
        assert_eq!(out.tiles[1].sub_index, out.tiles[2].sub_index);
        assert_ne!(out.tiles[0].sub_index, out.tiles[1].sub_index);
    }
}
