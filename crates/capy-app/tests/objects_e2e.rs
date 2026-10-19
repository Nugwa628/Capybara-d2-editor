//! End-to-end objects and NPC paths: the real exe runs a `CAPY_SCRIPT` on
//! vanilla `towne1.ds1` (v18): import a DS1Edit ini mapping, place a torch
//! (type 2) and an NPC (type 1) on free sub-tiles, give the NPC a 3-point
//! path with actions, save, with screenshots of the town, the animation,
//! the placement picker and path editing. The test then parses the saved
//! DS1 (objects, path anchor, points, actions), checks the ini override in
//! `config.toml`, and renders the saved map a second time with the map-only
//! screenshot hook (objects at frame 0) to compare it with
//! `capy_render::render_with_objects` at tolerance 16.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_render::{
    ObjectOptions, ObjectSprites, RenderOptions, RgbaImage, TileSet, compare_images,
    render_with_objects,
};
use d2_assets::objects::{ObjectCatalog, ObjectClass, ObjectInfo};
use d2_assets::{GamePath, MapCatalog, Profile};
use d2_formats::ds1::{Ds1, PathPoint};
use d2_formats::palette::{Palette, act_palette_path};

const MAP: &str = "data\\global\\tiles\\act1\\town\\towne1.ds1";
const ZOOM: u32 = 2;
const MIN_FRACTION: f64 = 0.98;

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn gpu_tests_enabled() -> bool {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return false;
    }
    true
}

fn exe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
}

/// Runs the exe on `script` with a scratch home; returns (exit code, stderr).
fn run_script(home: &Path, script: &str) -> (Option<i32>, String) {
    let file = home.join("script.txt");
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(&file, script).unwrap();
    let out = exe()
        .env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Every file directly in `dir` with its size and modification time.
fn listing(dir: &Path) -> Vec<(String, u64, Option<std::time::SystemTime>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            let m = e.metadata().unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                m.len(),
                m.modified().ok(),
            )
        })
        .collect();
    out.sort();
    out
}

/// A placeable entry of act 1 that draws a sprite.
fn pick(
    list: &[ObjectInfo],
    sprites: &ObjectSprites<'_>,
    want: impl Fn(&ObjectInfo) -> bool,
) -> ObjectInfo {
    list.iter()
        .filter(|i| want(i) && i.token.is_some())
        .find(|i| {
            sprites
                .frame_of_info(i, &ObjectOptions::default(), None, i.direction.unwrap_or(0))
                .is_ok()
        })
        .cloned()
        .expect("a placeable entry with a sprite")
}

/// A sub-tile in the middle of a cell with a floor and no object nearby.
fn free_subtile(ds1: &Ds1, near: (u32, u32), taken: &[(i32, i32)]) -> (i32, i32) {
    for r in 0..20u32 {
        for dy in 0..=r {
            for dx in 0..=r {
                let (x, y) = (near.0 + dx, near.1 + dy);
                if x + 2 >= ds1.width || y + 2 >= ds1.height {
                    continue;
                }
                if ds1.floors[0].get(x, y).is_none_or(|c| c.is_empty()) {
                    continue;
                }
                let s = (5 * x as i32 + 2, 5 * y as i32 + 2);
                let crowded = ds1
                    .objects
                    .iter()
                    .map(|o| (o.x, o.y))
                    .chain(taken.iter().copied())
                    .any(|(ox, oy)| (ox - s.0).abs() <= 12 && (oy - s.1).abs() <= 12);
                if !crowded {
                    return s;
                }
            }
        }
    }
    panic!("no free sub-tile near {near:?}");
}

fn black_to_transparent(mut img: RgbaImage) -> RgbaImage {
    for p in img.pixels.as_chunks_mut::<4>().0 {
        let background = p[0] == 0 && p[1] == 0 && p[2] == 0;
        p[3] = if background { 0 } else { 255 };
    }
    img
}

#[test]
fn place_objects_edit_an_npc_path_save_and_match_the_cpu_render() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let install_before = listing(&vanilla);
    let path = GamePath::new(MAP);
    let profile = Profile::vanilla(&vanilla);
    let version = profile.game_version;
    let empty_saves = tempfile::tempdir().unwrap();
    let base = profile.open(empty_saves.path()).unwrap();
    let orig = Ds1::parse(&base.read_bytes(&path).unwrap().unwrap()).unwrap();
    assert_eq!(orig.version, 18);
    let (catalog, _) = ObjectCatalog::build(&base, version).unwrap();
    let (maps, _) = MapCatalog::build(&base, version, std::slice::from_ref(&path)).unwrap();
    let info = maps.best(&path).expect("towne1 context");
    let (lvl_type, def) = (info.lvl_type.unwrap(), info.lvlprest_def.unwrap());

    // What to place: a torch and an NPC that draw sprites.
    let placeable = catalog.placeable(orig.effective_act());
    let sprites = ObjectSprites::new(&base, &catalog);
    let torch = pick(&placeable, &sprites, |i| {
        i.ds1_kind == 2 && i.name.to_ascii_lowercase().contains("torch")
    });
    let npc = pick(&placeable, &sprites, |i| {
        i.ds1_kind == 1 && i.class == ObjectClass::Npc
    });
    let centre = (orig.width / 2, orig.height / 2);
    let torch_at = free_subtile(&orig, (centre.0 - 4, centre.1 - 2), &[]);
    let npc_at = free_subtile(&orig, (centre.0 + 2, centre.1 + 1), &[torch_at]);
    let n = orig.objects.len();
    let (torch_index, npc_index) = (n, n + 1);
    let points = [
        PathPoint {
            x: npc_at.0 + 6,
            y: npc_at.1,
            action: 1,
        },
        PathPoint {
            x: npc_at.0 + 6,
            y: npc_at.1 + 7,
            action: 2,
        },
        PathPoint {
            x: npc_at.0,
            y: npc_at.1 + 7,
            action: 3,
        },
    ];
    eprintln!(
        "torch {} (2, {}) at {torch_at:?}, npc {} (1, {}) at {npc_at:?}",
        torch.name, torch.ds1_id, npc.name, npc.ds1_id
    );

    let home = tempfile::tempdir().unwrap();
    let ini = home.path().join("act1.ini");
    std::fs::write(
        &ini,
        format!("; DS1Edit\n{lvl_type} {def} assets/tiles/ACT1/TOWN/townE1.ds1\nnot a line\n"),
    )
    .unwrap();
    let shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("objects_e2e");
    std::fs::create_dir_all(&shots).unwrap();
    let shot = |name: &str| shots.join(name).display().to_string();
    let cell = |s: (i32, i32)| format!("{} {}", s.0 / 5, s.1 / 5);
    let path_cmds: String = points
        .iter()
        .map(|p| format!("path-add {} {} {}\n", p.x, p.y, p.action))
        .collect();
    let script = format!(
        "open {MAP}\n\
         import-ini {ini}\n\
         screenshot {ini_shot}\n\
         assert-objects {n}\n\
         place-object 2 {torch_id} {tx} {ty}\n\
         place-object 1 {npc_id} {nx} {ny}\n\
         assert-objects {n2}\n\
         select-object {npc_index}\n\
         {path_cmds}\
         assert-path {npc_index} 3\n\
         path-action 2 3\n\
         assert-dirty true\n\
         save\n\
         assert-dirty false\n\
         zoom 2\n\
         center-cell {npc_cell}\n\
         animate off\n\
         screenshot {town}\n\
         zoom 1\n\
         center-cell {torch_cell}\n\
         object-frame-tick 0\n\
         screenshot {anim0}\n\
         object-frame-tick 9\n\
         screenshot {anim1}\n\
         object-frame-tick off\n\
         animate on\n\
         objects-mode add\n\
         picker-search torch\n\
         picker-choose 2 {torch_id}\n\
         screenshot {picker}\n\
         center-cell {npc_cell}\n\
         select-object {npc_index}\n\
         objects-mode path\n\
         screenshot {path_shot}\n",
        ini = ini.display(),
        ini_shot = shot("ini_import.png"),
        torch_id = torch.ds1_id,
        npc_id = npc.ds1_id,
        tx = torch_at.0,
        ty = torch_at.1,
        nx = npc_at.0,
        ny = npc_at.1,
        n2 = n + 2,
        npc_cell = cell(npc_at),
        torch_cell = cell(torch_at),
        town = shot("town_objects_1to2.png"),
        anim0 = shot("animation_tick0.png"),
        anim1 = shot("animation_tick9.png"),
        picker = shot("placement_picker.png"),
        path_shot = shot("npc_path_editing.png"),
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");

    // The saved DS1: both objects, the NPC's path anchored on its sub-tile.
    let capy = home.path().join("capybara");
    let saves = capy.join("saves").join("vanilla");
    let saved_file = saves.join("data/global/tiles/act1/town/towne1.ds1");
    let saved = Ds1::parse(&std::fs::read(&saved_file).expect("saved map")).unwrap();
    assert_eq!(saved.objects.len(), n + 2);
    assert_eq!(saved.objects[..n], orig.objects[..]);
    let t = saved.objects[torch_index];
    assert_eq!(
        (t.kind, t.id, t.x, t.y),
        (2, torch.ds1_id, torch_at.0, torch_at.1)
    );
    let p = saved.objects[npc_index];
    assert_eq!(
        (p.kind, p.id, p.x, p.y),
        (1, npc.ds1_id, npc_at.0, npc_at.1)
    );
    let new_paths: Vec<_> = saved
        .npc_paths
        .iter()
        .filter(|q| !orig.npc_paths.contains(q))
        .collect();
    assert_eq!(new_paths.len(), 1, "{:?}", saved.npc_paths);
    let route = new_paths[0];
    assert_eq!((route.x, route.y), npc_at, "anchored on the NPC's sub-tile");
    assert_eq!(route.points, points.to_vec());

    // The ini mapping is an override of the profile in config.toml.
    let config = std::fs::read_to_string(capy.join("config.toml")).unwrap();
    assert!(
        config.contains("towne1.ds1") && config.contains(&format!("lvl_type = {lvl_type}")),
        "{config}"
    );

    // Map-only render of the saved map (frame 0) against the CPU compositor.
    let gpu_file = shots.join("saved_map_only_gpu.png");
    let _ = std::fs::remove_file(&gpu_file);
    let status = exe()
        .env("CAPY_HOME", &capy)
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_OPEN_MAP", MAP)
        .env("CAPY_LVLTYPE", lvl_type.to_string())
        .env("CAPY_DEF", def.to_string())
        .env("CAPY_ZOOM", ZOOM.to_string())
        .env("CAPY_SCREENSHOT_MAP_ONLY", "1")
        .env("CAPY_SCREENSHOT", &gpu_file)
        .status()
        .expect("start the app");
    assert!(status.success(), "map-only run exited with {status}");
    let gpu =
        black_to_transparent(RgbaImage::from_png(&std::fs::read(&gpu_file).unwrap()).unwrap());

    let overlay = profile.open(&capy.join("saves")).unwrap();
    let saved_bytes = overlay.read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        saved_bytes,
        std::fs::read(&saved_file).unwrap(),
        "the overlay reads the save"
    );
    let (maps, _) = MapCatalog::build(&overlay, version, std::slice::from_ref(&path)).unwrap();
    let info = maps.resolve_explicit(&path, lvl_type, Some(def)).unwrap();
    let (tiles, _) = TileSet::load(&overlay, &info.dt1_files).unwrap();
    let palette = Palette::parse_dat(
        &overlay
            .read_bytes(&GamePath::new(act_palette_path(info.palette_act)))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let (catalog, _) = ObjectCatalog::build(&overlay, version).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let opts = RenderOptions {
        zoom_div: ZOOM,
        gamma: 1.30,
        ..RenderOptions::default()
    };
    let cpu = render_with_objects(&saved, &tiles, Some(&sprites), &palette, &opts).unwrap();
    for i in [torch_index, npc_index] {
        assert!(
            !cpu.missing_objects.iter().any(|m| m.index == i),
            "object {i} draws on the CPU: {:?}",
            cpu.missing_objects
        );
    }
    std::fs::write(
        shots.join("saved_map_only_cpu.png"),
        cpu.image.to_png().unwrap(),
    )
    .unwrap();
    let size_ok = (gpu.width, gpu.height) == (cpu.image.width, cpu.image.height);
    let d16 = compare_images(&gpu, &cpu.image, 16, 4);
    let d0 = compare_images(&gpu, &cpu.image, 0, 0);
    eprintln!(
        "saved towne1 at 1:{ZOOM}: {}x{} tol0 {:.4} tol16 {:.4} shift {:?}",
        gpu.width, gpu.height, d0.within_tolerance, d16.within_tolerance, d16.best_shift
    );
    assert!(
        size_ok,
        "canvas {}x{} vs {}x{}",
        gpu.width, gpu.height, cpu.image.width, cpu.image.height
    );
    assert!(
        d16.within_tolerance >= MIN_FRACTION,
        "GPU vs CPU within_tolerance(16) = {}",
        d16.within_tolerance
    );

    for name in [
        "ini_import.png",
        "town_objects_1to2.png",
        "animation_tick0.png",
        "animation_tick9.png",
        "placement_picker.png",
        "npc_path_editing.png",
    ] {
        assert!(shots.join(name).is_file(), "{name}");
    }
    eprintln!("screenshots in {}", shots.display());
    assert_eq!(listing(&vanilla), install_before, "the install changed");
}
