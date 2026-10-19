//! Shared helpers of the heavy tests: game installs, map context, gating.
#![allow(dead_code, clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::path::{Path, PathBuf};

use d2_assets::tables::load_excel;
use d2_assets::{GamePath, GameVersion, Overlay, Profile};
use d2_formats::ds1::Ds1;
use d2_formats::excel::ExcelTableKind;
use d2_formats::palette::{Palette, act_palette_path};
use d2_formats::tables::{LvlPrest, LvlType, lvl_prests, lvl_types};

pub const MAIN_CHECKOUT: &str = r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor";

/// Heavy tests run in release builds, or when `CAPY_RENDER_TESTS=1`.
pub fn heavy_enabled() -> bool {
    std::env::var("CAPY_RENDER_TESTS").is_ok_and(|v| v == "1") || !cfg!(debug_assertions)
}

pub fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

pub fn mxl_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_MXL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\games\median-xl"));
    dir.is_dir().then_some(dir)
}

/// `testdata\private` of the main checkout (gitignored, so absent in worktrees).
pub fn private_dir() -> PathBuf {
    std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(MAIN_CHECKOUT).join("testdata").join("private"))
}

pub struct Game {
    pub overlay: Overlay,
    pub version: GameVersion,
    _saves: tempfile::TempDir,
}

pub fn open_profile(profile: Profile) -> Game {
    let saves = tempfile::tempdir().unwrap();
    let version = profile.game_version;
    let overlay = profile.open(saves.path()).unwrap();
    Game {
        overlay,
        version,
        _saves: saves,
    }
}

pub struct Tables {
    pub lvl_types: Vec<LvlType>,
    pub lvl_prests: Vec<LvlPrest>,
}

pub fn load_tables(game: &Game) -> Tables {
    let types = load_excel(&game.overlay, ExcelTableKind::LvlTypes, game.version).unwrap();
    let prests = load_excel(&game.overlay, ExcelTableKind::LvlPrest, game.version).unwrap();
    Tables {
        lvl_types: lvl_types(&types).unwrap(),
        lvl_prests: lvl_prests(&prests).unwrap(),
    }
}

/// Everything needed to render one map.
pub struct MapContext {
    pub lvl_type: LvlType,
    pub lvl_prest: LvlPrest,
    pub dt1_files: Vec<GamePath>,
    pub palette_act: u8,
}

// TODO(presets): replace with MapCatalog::resolve_explicit
///
/// `level_pal` is `Levels.Pal` (0-based) when the caller knows the level: the
/// game takes the palette from there. DS1Edit takes it from LvlTypes (see
/// `capy_render::palette_act`), which agrees on every vanilla map but not on
/// Median XL's (e.g. level 241 is LvlTypes act 5 but `Pal` 3).
pub fn resolve_explicit(
    tables: &Tables,
    ds1_act: u8,
    lvltypes_id: u32,
    lvlprest_def: u32,
    level_pal: Option<u8>,
) -> MapContext {
    let lvl_type = tables
        .lvl_types
        .iter()
        .find(|t| t.id == lvltypes_id)
        .unwrap_or_else(|| panic!("no LvlTypes id {lvltypes_id}"))
        .clone();
    let lvl_prest = tables
        .lvl_prests
        .iter()
        .find(|p| p.def == lvlprest_def)
        .unwrap_or_else(|| panic!("no LvlPrest def {lvlprest_def}"))
        .clone();
    let dt1_files = lvl_prest
        .dt1_files(&lvl_type)
        .filter_map(GamePath::from_tile_ref)
        .collect();
    let palette_act = level_pal.map_or_else(
        || capy_render::palette_act(Some(&lvl_type), ds1_act),
        |p| p.min(4),
    );
    MapContext {
        lvl_type,
        lvl_prest,
        dt1_files,
        palette_act,
    }
}

pub fn load_palette(game: &Game, act: u8) -> Palette {
    let bytes = game
        .overlay
        .read_bytes(&GamePath::new(act_palette_path(act)))
        .unwrap()
        .unwrap_or_else(|| panic!("palette for act {act} missing"));
    Palette::parse_dat(&bytes).unwrap()
}

pub fn load_ds1(game: &Game, path: &str) -> Ds1 {
    let bytes = game
        .overlay
        .read_bytes(&GamePath::new(path))
        .unwrap()
        .unwrap_or_else(|| panic!("{path} not found"));
    Ds1::parse(&bytes).unwrap()
}
