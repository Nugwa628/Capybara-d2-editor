//! End-to-end checks of the editable view modes and the stacking, walk and
//! library tools (ds1-studio parity gaps): Alt stack painting with undo,
//! Automap painting saved to AutoMap.txt, Level light saved to Levels.txt,
//! whole-cell and sub-tile walk painting with undo and save, the no-spawn
//! area planner and Re-roll floors with floors from another DT1.
//!
//! Needs a window (GPU) and the vanilla install: runs only with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_render::TileSet;
use capy_render::rooms::{is_node_floor, preset_rooms, room_regions};
use capy_render::walk::{BLOCK_WALK, Collision, sub_tile_flags};
use d2_assets::{GamePath, MapCatalog, Profile};
use d2_formats::ds1::Ds1;

const MAP: &str = "data\\global\\tiles\\act1\\town\\townn1.ds1";

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn enabled() -> Option<PathBuf> {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return None;
    }
    let v = vanilla_dir();
    if v.is_none() {
        eprintln!("skipped: vanilla install not found");
    }
    v
}

fn run_script(home: &Path, script: &str) -> (Option<i32>, String) {
    let file = home.join("script.txt");
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(&file, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_SCALE_FACTOR", "1")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn saves(home: &Path) -> PathBuf {
    home.join("capybara").join("saves").join("vanilla")
}

fn changed(home: &Path) -> String {
    std::fs::read_to_string(saves(home).join("CHANGED.txt"))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

struct Facts {
    overlay: d2_assets::Overlay,
    ds1: Ds1,
    tiles: TileSet,
    dt1s: Vec<String>,
}

fn townn1(vanilla: &Path) -> Facts {
    let saves = tempfile::tempdir().unwrap();
    let profile = Profile::vanilla(vanilla);
    let version = profile.game_version;
    let overlay = profile.open(saves.path()).unwrap();
    let path = GamePath::new(MAP);
    let (catalog, _) = MapCatalog::build(&overlay, version, std::slice::from_ref(&path)).unwrap();
    let info = catalog.best(&path).expect("townn1 map info").clone();
    let (tiles, _) = TileSet::load(&overlay, &info.dt1_files).unwrap();
    let ds1 = Ds1::parse(&overlay.read_bytes(&path).unwrap().unwrap()).unwrap();
    Facts {
        overlay,
        ds1,
        tiles,
        dt1s: info
            .dt1_files
            .iter()
            .map(|p| p.as_str().to_ascii_lowercase())
            .collect(),
    }
}

#[test]
fn stack_painting_takes_the_next_free_layer_and_one_undo_removes_it() {
    let Some(_) = enabled() else { return };
    let home = tempfile::tempdir().unwrap();
    let script = "new-map data\\global\\tiles\\act1\\town\\capystack.ds1 20 16 1\n\
         set-brush floor0 0x017000c2\n\
         paint-rect floor0 2 2 2 2\n\
         assert-layers 1 1\n\
         set-brush floor0 0x018000c2\n\
         stack-paint floor0 2 2 3 3\n\
         assert-layers 2 1\n\
         assert-tile floor0 2 2 23 0\n\
         assert-tile floor1 2 2 24 0\n\
         assert-tile floor0 3 3 24 0\n\
         assert-tile floor1 3 3 empty\n\
         stack-paint floor0 2 2\n\
         assert-tile floor1 2 2 24 0\n\
         undo\n\
         undo\n\
         assert-layers 1 1\n\
         assert-tile floor0 2 2 23 0\n\
         assert-tile floor0 3 3 empty\n";
    let (code, stderr) = run_script(home.path(), script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
}

#[test]
fn automap_painting_is_saved_to_automap_txt() {
    let Some(vanilla) = enabled() else { return };
    let f = townn1(&vanilla);
    // A cell with a floor tile.
    let (x, y) = (0..f.ds1.height)
        .flat_map(|y| (0..f.ds1.width).map(move |x| (x, y)))
        .find(|&(x, y)| f.ds1.floors[0].get(x, y).is_some_and(|c| !c.is_empty()))
        .expect("townn1 has floors");
    let cell = f.ds1.floors[0].get(x, y).unwrap();
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         view automap\n\
         automap-open\n\
         automap-cel {x} {y} 37\n\
         automap-save\n\
         tool assert-saved data\\global\\excel\\automap.txt\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let bytes = std::fs::read(saves(home.path()).join("data\\global\\excel\\automap.txt")).unwrap();
    let table = capy_mod::automap::AutomapTable::parse(&d2_formats::txt::TxtDoc::parse(&bytes));
    let (main, sub) = (i32::from(cell.main_index()), i32::from(cell.sub_index()));
    let hit = table.rules.iter().any(|r| {
        r.code == "fl"
            && r.cels.iter().any(|c| c.cel == 37)
            && (r.style == main || r.style == -1)
            && (r.start == -1 || (r.start..=r.end.max(r.start)).contains(&sub))
    });
    assert!(hit, "no floor rule with picture 37 for {main}/{sub}");
    assert!(changed(home.path()).contains("automap.txt"));
}

fn mean_brightness(png: &Path) -> f64 {
    let img = image::open(png).unwrap().to_rgba8();
    let sum: u64 = img
        .pixels()
        .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
        .sum();
    sum as f64 / (img.width() as f64 * img.height() as f64 * 3.0)
}

#[test]
fn level_light_is_saved_to_levels_txt_and_darkens_the_map() {
    let Some(_) = enabled() else { return };
    let home = tempfile::tempdir().unwrap();
    let out = home.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let (a, b) = (out.join("tiles.png"), out.join("light.png"));
    let script = format!(
        "open {MAP}\n\
         wait 30\n\
         screenshot {}\n\
         light-set 90 128 128 128\n\
         assert-light 90 128 128 128\n\
         view light\n\
         wait 30\n\
         screenshot {}\n",
        a.display(),
        b.display()
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let bytes = std::fs::read(saves(home.path()).join("data\\global\\excel\\levels.txt")).unwrap();
    let doc = d2_formats::txt::TxtDoc::parse(&bytes);
    let (id, int) = (doc.column("Id").unwrap(), doc.column("Intensity").unwrap());
    let row = (0..doc.row_count())
        .find(|&r| doc.cell(r, id).as_deref() == Some("1"))
        .expect("level 1");
    assert_eq!(doc.cell(row, int).as_deref(), Some("90"));
    assert!(changed(home.path()).contains("levels.txt"));
    let (bright, dark) = (mean_brightness(&a), mean_brightness(&b));
    assert!(
        dark < bright * 0.85,
        "Level light should multiply the map darker: {bright:.1} -> {dark:.1}"
    );
}

#[test]
fn walk_painting_works_by_cell_and_by_subtile_with_undo_and_save() {
    let Some(vanilla) = enabled() else { return };
    let f = townn1(&vanilla);
    let grid = sub_tile_flags(&f.ds1, &f.tiles, Collision::Editor);
    let free_floor = |x: u32, y: u32| {
        f.ds1
            .floors
            .get(1)
            .is_none_or(|l| l.get(x, y).is_none_or(|c| c.is_empty()))
            && f.ds1.floors[0].get(x, y).is_some_and(|c| !c.is_empty())
    };
    let (x, y) = (0..f.ds1.height)
        .flat_map(|y| (0..f.ds1.width).map(move |x| (x, y)))
        .find(|&(x, y)| {
            free_floor(x, y)
                && (0..25)
                    .all(|k| grid.flags[(y * f.ds1.width + x) as usize * 25 + k] & BLOCK_WALK == 0)
        })
        .expect("an open cell with room for a blocker");
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         assert-subtile {x} {y} 2 2 open\n\
         view walk\n\
         # whole cell (DS1 flag)\n\
         walk-cell {x} {y}\n\
         assert-flag floor0 {x} {y} unwalkable on\n\
         undo\n\
         assert-flag floor0 {x} {y} unwalkable off\n\
         redo\n\
         assert-flag floor0 {x} {y} unwalkable on\n\
         walk-cell {x} {y} clear\n\
         assert-flag floor0 {x} {y} unwalkable off\n\
         undo\n\
         undo\n\
         assert-dirty false\n\
         # one sub-tile (blocker in the map's _walk.dt1)\n\
         walk-sub {x} {y} 2 2\n\
         assert-subtile {x} {y} 2 2 blocked\n\
         assert-subtile {x} {y} 1 1 open\n\
         assert-dirty true\n\
         undo\n\
         assert-dirty false\n\
         assert-subtile {x} {y} 2 2 open\n\
         redo\n\
         assert-subtile {x} {y} 2 2 blocked\n\
         save\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let list = changed(home.path());
    assert!(list.contains("townn1_walk.dt1"), "{list}");
    assert!(list.contains("townn1.ds1"), "{list}");
    let walk = std::fs::read(saves(home.path()).join(MAP.replace(".ds1", "_walk.dt1"))).unwrap();
    let dt1 = d2_formats::dt1::Dt1::parse(&walk).unwrap();
    assert!(
        dt1.tiles
            .iter()
            .any(|t| t.blocks.is_empty() && t.subtile_flags[12] & 1 != 0),
        "a blocker for the middle sub-tile"
    );
    let saved = Ds1::parse(&std::fs::read(saves(home.path()).join(MAP)).unwrap()).unwrap();
    assert!(
        saved.tileset_refs().any(|r| String::from_utf8_lossy(r)
            .to_ascii_lowercase()
            .contains("townn1_walk")),
        "the saved map lists its walk library"
    );
}

#[test]
fn the_no_spawn_planner_hides_region_seeds_and_undoes() {
    let Some(vanilla) = enabled() else { return };
    let f = townn1(&vanilla);
    let seed = preset_rooms(f.ds1.width, f.ds1.height)
        .into_iter()
        .flat_map(|r| room_regions(&f.ds1, r).regions)
        .map(|g| g.seed)
        .find(|&(x, y)| {
            let c = f.ds1.floors[0].get(x, y).unwrap_or_default();
            !c.is_empty()
                && !is_node_floor(c)
                && f.ds1
                    .floors
                    .get(1)
                    .is_none_or(|l| l.get(x, y).is_none_or(|c| c.is_empty()))
        })
        .expect("a floored region seed");
    let (x, y) = seed;
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         view walk\n\
         assert-flag floor0 {x} {y} hidden off\n\
         no-spawn\n\
         assert-flag floor0 {x} {y} hidden on\n\
         assert-dirty true\n\
         undo\n\
         assert-flag floor0 {x} {y} hidden off\n\
         assert-dirty false\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
}

#[test]
fn re_roll_takes_floors_from_another_library_of_the_level_type() {
    let Some(vanilla) = enabled() else { return };
    let f = townn1(&vanilla);
    let other = [
        "data\\global\\tiles\\act1\\outdoor\\floor.dt1",
        "data\\global\\tiles\\act1\\barracks\\floor.dt1",
        "data\\global\\tiles\\act1\\crypt\\floor.dt1",
        "data\\global\\tiles\\act1\\cave\\floor.dt1",
        "data\\global\\tiles\\act2\\desert\\floor.dt1",
        "data\\global\\tiles\\act1\\outdoors\\floor.dt1",
    ]
    .into_iter()
    .find(|p| {
        !f.dt1s.contains(&p.to_ascii_lowercase())
            && f.overlay
                .read_bytes(&GamePath::new(p))
                .ok()
                .flatten()
                .and_then(|b| d2_formats::dt1::Dt1::parse(&b).ok())
                .is_some_and(|d| d.tiles.iter().any(|t| t.orientation == 0))
    })
    .expect("a floor library outside townn1's tile set");
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         assert-walkable 0 100000\n\
         reroll-lib 0 {other}\n\
         assert-dirty true\n\
         save\n\
         undo\n\
         assert-dirty true\n\
         redo\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let custom = saves(home.path()).join(MAP.replace(".ds1", "_custom.dt1"));
    let dt1 = d2_formats::dt1::Dt1::parse(&std::fs::read(&custom).unwrap()).unwrap();
    assert!(dt1.tiles.iter().all(|t| t.orientation == 0));
    assert!(!dt1.tiles.is_empty());
    let saved = Ds1::parse(&std::fs::read(saves(home.path()).join(MAP)).unwrap()).unwrap();
    assert!(
        saved.tileset_refs().any(|r| String::from_utf8_lossy(r)
            .to_ascii_lowercase()
            .contains("townn1_custom")),
        "the saved map lists the custom library"
    );
    let used = |m: i32, s: i32| {
        saved.floors[0].as_slice().iter().any(|c| {
            !c.is_empty() && i32::from(c.main_index()) == m && i32::from(c.sub_index()) == s
        })
    };
    assert!(
        dt1.tiles.iter().any(|t| used(t.main_index, t.sub_index)),
        "the re-rolled floor uses one of the copied tiles"
    );
    assert!(changed(home.path()).contains("townn1_custom.dt1"));
}
