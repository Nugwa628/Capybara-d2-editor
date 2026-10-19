//! End-to-end asset browser (P3): the real exe runs a `CAPY_SCRIPT` on the
//! Median XL profile that searches the index, makes a tile the brush, paints
//! with it, favorites and aliases it, finds it again by the alias and opens
//! the Ctrl+K palette, taking screenshots on the way. The test then reads
//! the saved map and `user.db` back.
//!
//! Needs a window (GPU) and the Median XL install, so it only runs with
//! `CAPY_GPU_TESTS=1`; it is skipped when the install is missing.
//! `CAPY_MXL_DIR` overrides the install folder. The index is seeded from
//! the copy `capy-index`'s tests keep in `target\tmp\capy-index-mxl` when
//! there is one (the app checks it and rebuilds it when stale). Screenshots
//! go to `target\tmp\browser_e2e\`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_index::UserDb;
use d2_formats::ds1::{Cell, Ds1};

const MAP: &str = r"data\global\tiles\mxl\zone\whimsy\shire.ds1";
/// A green floor of whimsyfloor.dt1 (orientation 0, main 23, sub 0), used by
/// shire.ds1; checked against the real index.
const KEY: &str = r"data\global\tiles\mxl\zone\whimsy\whimsyfloor.dt1#0/23/0/0";
/// The brush that tile makes on Floor 1: prop1 194, sub 0, main 23.
const CELL: u32 = 0x0170_00C2;
const ALIAS: &str = "capymeadow";
const GROUP: &str = "Meadow";

fn mxl_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_MXL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\games\median-xl"));
    dir.join("d2data.mpq").is_file().then_some(dir)
}

fn gpu_tests_enabled() -> bool {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return false;
    }
    true
}

/// Copies the shared Median XL index into the scratch cache, if it exists.
fn seed_index(home: &Path) {
    let shared = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("capy-index-mxl")
        .join("index.db");
    if !shared.is_file() {
        eprintln!(
            "no shared index at {}; the app builds one",
            shared.display()
        );
        return;
    }
    let cache = home.join("capybara").join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::copy(&shared, cache.join("index.db")).unwrap();
}

#[test]
fn search_brush_paint_favorite_alias_and_ctrl_k() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(mxl) = mxl_dir() else {
        eprintln!("skipped: Median XL install not found");
        return;
    };
    let shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("browser_e2e");
    std::fs::create_dir_all(&shots).unwrap();
    let shot = |name: &str| shots.join(name).display().to_string();

    let home = tempfile::tempdir().unwrap();
    seed_index(home.path());
    let script = format!(
        "open {MAP}\n\
         tiles-tab\n\
         search color:green type:floor file:whimsyfloor.dt1\n\
         assert-search-top 5 {KEY}\n\
         thumb-size 64\n\
         screenshot {grid64}\n\
         thumb-size 160\n\
         screenshot {grid160}\n\
         thumb-size 36\n\
         screenshot {list}\n\
         thumb-size 64\n\
         brush-from {KEY}\n\
         assert-brush floor0 {CELL:#010x}\n\
         paint-brush 3 3\n\
         assert-recent {KEY}\n\
         favorite-add {GROUP} {KEY}\n\
         alias {KEY} {ALIAS}\n\
         search {ALIAS}\n\
         assert-search-top 1 {KEY}\n\
         screenshot {favorites}\n\
         tile-menu {KEY}\n\
         screenshot {menu}\n\
         ctrl-k type:floor color:green whimsy\n\
         screenshot {ctrl_k}\n\
         ctrl-k type:floor act:9\n\
         screenshot {ctrl_k_error}\n\
         ctrl-k-close\n\
         save\n",
        grid64 = shot("grid-64.png"),
        grid160 = shot("grid-160.png"),
        list = shot("list-36.png"),
        favorites = shot("favorites.png"),
        menu = shot("context-menu.png"),
        ctrl_k = shot("ctrl-k.png"),
        ctrl_k_error = shot("ctrl-k-error.png"),
    );
    let file = home.path().join("script.txt");
    std::fs::write(&file, &script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.path().join("capybara"))
        .env("CAPY_PROFILE", "mxl")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "script failed:\n{stderr}");

    // The painted cell is in the saved map.
    let mut saved = home.path().join("capybara").join("saves").join("mxl");
    for part in MAP.split('\\') {
        saved = saved.join(part);
    }
    let ds1 = Ds1::parse(&std::fs::read(&saved).expect("the saved map")).expect("parses");
    assert_eq!(ds1.floors[0].get(3, 3), Some(Cell(CELL)));

    // user.db holds the use, the favorite and the alias.
    let db = UserDb::open(&home.path().join("capybara").join("user.db")).unwrap();
    assert_eq!(
        db.recents("mxl", 10).unwrap().first().map(String::as_str),
        Some(KEY)
    );
    assert_eq!(db.alias("mxl", KEY).unwrap().as_deref(), Some(ALIAS));
    let group = db
        .favorite_groups()
        .unwrap()
        .into_iter()
        .find(|g| g.name == GROUP)
        .expect("the group");
    assert_eq!(
        db.favorites(group.id).unwrap(),
        vec![("mxl".to_owned(), KEY.to_owned())]
    );
    for name in [
        "grid-64.png",
        "grid-160.png",
        "list-36.png",
        "favorites.png",
        "context-menu.png",
        "ctrl-k.png",
        "ctrl-k-error.png",
    ] {
        assert!(shots.join(name).is_file(), "{name}");
    }
    eprintln!(
        "screenshots in {} (install {})",
        shots.display(),
        mxl.display()
    );
}

/// A key that is not in the index fails the script with exit code 1.
#[test]
fn an_unknown_key_fails_the_script() {
    if !gpu_tests_enabled() {
        return;
    }
    if mxl_dir().is_none() {
        eprintln!("skipped: Median XL install not found");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    seed_index(home.path());
    let file = home.path().join("script.txt");
    std::fs::write(&file, "brush-from data\\global\\tiles\\nope.dt1#0/1/2/0\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.path().join("capybara"))
        .env("CAPY_PROFILE", "mxl")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not in the index"), "{stderr}");
}
