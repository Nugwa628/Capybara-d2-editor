//! GPU-vs-CPU parity: the app's textured renderer against `capy-render`'s CPU
//! compositor, on the oracle maps at 1:4.
//!
//! For each map the test starts the real app with the map-only screenshot hook
//! (`CAPY_SCREENSHOT_MAP_ONLY=1`: no UI, black background, an off-screen target
//! of the reference canvas size, objects at animation frame 0), renders the
//! same map in-process with `capy_render::render_with_objects` (the profile's
//! `ObjectCatalog`, frame 0) using the same LvlType, Def and palette, and
//! compares them with `compare_images` at tolerance 16.
//!
//! * `CAPY_GPU_TERRAIN=1`     compare without objects on the CPU side (the
//!   app still draws them: only useful on object-free maps)
//!
//! Needs a GPU, the game install and `testdata\private\oracle-renders` (for the
//! manifest of LvlType/Def values), so it only runs with `CAPY_GPU_TESTS=1`.
//!
//! * `CAPY_GPU_MAPS=<n>|all`  how many manifest maps (default 6, spread evenly)
//! * `CAPY_GPU_ONLY=<text>`   only maps whose name contains it
//! * `CAPY_GPU_ZOOM=<n>`      zoom divisor (default 4)
//! * `CAPY_PRIVATE_DIR`, `CAPY_VANILLA_DIR`  as for the `capy-render` tests
//!
//! Output images land in `<CAPY_HOME of the run>\gpu-parity\`; set
//! `CAPY_GPU_KEEP=<dir>` to keep them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_render::{
    ObjectSprites, RenderOptions, RgbaImage, TileSet, compare_images, render_with_objects,
};
use d2_assets::objects::ObjectCatalog;
use d2_assets::{GamePath, MapCatalog, Profile};
use d2_formats::ds1::Ds1;
use d2_formats::palette::{Palette, act_palette_path};

const MAIN_CHECKOUT: &str = r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor";
const GAMMA: f32 = 1.30;
const MIN_FRACTION: f64 = 0.98;

struct Entry {
    name: String,
    ds1: String,
    lvltypes_id: u32,
    lvlprest_def: u32,
}

fn private_dir() -> PathBuf {
    std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(MAIN_CHECKOUT).join("testdata").join("private"))
}

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn read_manifest(dir: &Path) -> Option<Vec<Entry>> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    manifest
        .as_array()?
        .iter()
        .map(|e| {
            Some(Entry {
                name: e["name"].as_str()?.to_owned(),
                ds1: e["ds1"].as_str()?.to_owned(),
                lvltypes_id: u32::try_from(e["lvltypes_id"].as_u64()?).ok()?,
                lvlprest_def: u32::try_from(e["lvlprest_def"].as_u64()?).ok()?,
            })
        })
        .collect()
}

/// Evenly spread `n` picks from `len` entries.
fn spread(len: usize, n: usize) -> Vec<usize> {
    if n >= len {
        return (0..len).collect();
    }
    (0..n).map(|i| i * len / n).collect()
}

/// The CPU reference for one map, with the context the app is told to use.
fn cpu_render(
    overlay: &d2_assets::Overlay,
    version: d2_assets::GameVersion,
    objects: Option<&ObjectCatalog>,
    entry: &Entry,
    zoom: u32,
) -> (RgbaImage, usize, usize) {
    let path = GamePath::new(&entry.ds1);
    let (catalog, _) = MapCatalog::build(overlay, version, std::slice::from_ref(&path)).unwrap();
    let info = catalog
        .resolve_explicit(&path, entry.lvltypes_id, Some(entry.lvlprest_def))
        .unwrap();
    let ds1 = Ds1::parse(&overlay.read_bytes(&path).unwrap().unwrap()).unwrap();
    let (tiles, _) = TileSet::load(overlay, &info.dt1_files).unwrap();
    let palette = Palette::parse_dat(
        &overlay
            .read_bytes(&GamePath::new(act_palette_path(info.palette_act)))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let opts = RenderOptions {
        zoom_div: zoom,
        gamma: GAMMA,
        ..RenderOptions::default()
    };
    let sprites = objects.map(|c| ObjectSprites::new(overlay, c));
    let out = render_with_objects(&ds1, &tiles, sprites.as_ref(), &palette, &opts).unwrap();
    let drawn = ds1.objects.len() - out.missing_objects.len();
    (
        out.image,
        out.missing.len(),
        if objects.is_some() { drawn } else { 0 },
    )
}

/// Black pixels of the GPU image are background (the map-only shot has no
/// outlines for missing tiles): make them transparent so the comparison skips
/// them like it skips the CPU image's transparent ones.
fn black_to_transparent(mut img: RgbaImage) -> RgbaImage {
    for p in img.pixels.as_chunks_mut::<4>().0 {
        let background = p[0] == 0 && p[1] == 0 && p[2] == 0;
        p[3] = if background { 0 } else { 255 };
    }
    img
}

#[test]
fn gpu_render_matches_the_cpu_compositor() {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return;
    }
    let renders = private_dir().join("oracle-renders");
    let (Some(vanilla), Some(entries)) = (vanilla_dir(), read_manifest(&renders)) else {
        eprintln!("skipped: vanilla install or oracle manifest not found");
        return;
    };
    let zoom: u32 = std::env::var("CAPY_GPU_ZOOM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let count = match std::env::var("CAPY_GPU_MAPS").ok().as_deref() {
        Some("all") => entries.len(),
        Some(n) => n.parse().unwrap_or(6),
        None => 6,
    };

    let home = tempfile::tempdir().unwrap();
    let out_dir = std::env::var_os("CAPY_GPU_KEEP")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.path().join("gpu-parity"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let saves = tempfile::tempdir().unwrap();
    let profile = Profile::vanilla(&vanilla);
    let version = profile.game_version;
    let overlay = profile.open(saves.path()).unwrap();
    let terrain_only = std::env::var("CAPY_GPU_TERRAIN").ok().as_deref() == Some("1");
    let catalog = (!terrain_only).then(|| ObjectCatalog::build(&overlay, version).unwrap().0);

    eprintln!(
        "{:<28} {:>11} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "map", "size", "tol0", "tol16", "shift", "missing", "objects"
    );
    let mut failures = Vec::new();
    let only = std::env::var("CAPY_GPU_ONLY").ok();
    for i in spread(entries.len(), count) {
        let entry = &entries[i];
        if only.as_deref().is_some_and(|o| !entry.name.contains(o)) {
            continue;
        }
        let shot = out_dir.join(format!("{}_gpu.png", entry.name));
        let _ = std::fs::remove_file(&shot);
        let status = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
            .env("CAPY_HOME", home.path().join("capybara"))
            .env("CAPY_PROFILE", "vanilla")
            .env("CAPY_NO_INDEX", "1")
            .env("CAPY_OPEN_MAP", &entry.ds1)
            .env("CAPY_LVLTYPE", entry.lvltypes_id.to_string())
            .env("CAPY_DEF", entry.lvlprest_def.to_string())
            .env("CAPY_ZOOM", zoom.to_string())
            .env("CAPY_SCREENSHOT_MAP_ONLY", "1")
            .env("CAPY_SCREENSHOT", &shot)
            .status()
            .expect("start the app");
        assert!(
            status.success(),
            "{}: the app exited with {status}",
            entry.name
        );
        let gpu = black_to_transparent(
            RgbaImage::from_png(&std::fs::read(&shot).expect("screenshot")).unwrap(),
        );
        let (cpu, missing, objects) = cpu_render(&overlay, version, catalog.as_ref(), entry, zoom);
        std::fs::write(
            out_dir.join(format!("{}_cpu.png", entry.name)),
            cpu.to_png().unwrap(),
        )
        .unwrap();
        let size_ok = (gpu.width, gpu.height) == (cpu.width, cpu.height);
        let d16 = compare_images(&gpu, &cpu, 16, 4);
        let d0 = compare_images(&gpu, &cpu, 0, 0);
        eprintln!(
            "{:<28} {:>5}x{:<5} {:>8.4} {:>8.4} {:>3},{:<4} {:>8} {:>8} {}",
            entry.name,
            gpu.width,
            gpu.height,
            d0.within_tolerance,
            d16.within_tolerance,
            d16.best_shift.0,
            d16.best_shift.1,
            missing,
            objects,
            if size_ok { "" } else { "SIZE DIFFERS" }
        );
        if d16.within_tolerance < MIN_FRACTION || !size_ok {
            failures.push(entry.name.clone());
        }
    }
    assert!(
        failures.is_empty(),
        "GPU render differs from the CPU compositor (tolerance 16, need {MIN_FRACTION}): {failures:?}"
    );
}
