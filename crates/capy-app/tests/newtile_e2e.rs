//! End-to-end checks of New tile: the real exe runs `CAPY_SCRIPT` lines
//! against the vanilla install, creates a floor, an upper wall, a north
//! corner pair and a tile imported from a PNG in a town map, adds each to the
//! map's own DT1 ("Add to this map"), paints it, saves, and the test reads
//! the profile's saves back. Also: a new empty DT1 with a tile made into it
//! and re-shaped in Tile libraries, a tile painted in the sprite editor, and
//! a render check (the created tiles draw where shipped tiles of their kind
//! do, and the app's map view draws what the CPU compositor draws).
//!
//! Needs a window and the install, so it only runs with `CAPY_GPU_TESTS=1`
//! (on Linux under a virtual X server, e.g. `xvfb-run -a`);
//! `CAPY_VANILLA_DIR` overrides the install folder. The test writes its own
//! `profiles/vanilla.toml`, so it does not depend on where the app looks
//! for installs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_render::{RenderOptions, RgbaImage, TileSet, compare_images, render_with_objects};
use d2_assets::objects::ObjectCatalog;
use d2_assets::{GamePath, MapCatalog, Profile};
use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::{Dt1, Tile};
use d2_formats::palette::{Palette, act_palette_path};
use d2_formats::txt::TxtDoc;

const MAP: &str = "data\\global\\tiles\\act1\\town\\townn1.ds1";
const GAMMA: f32 = 1.30;

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
    let dir = vanilla_dir();
    if dir.is_none() {
        eprintln!("skipped: no vanilla install (set CAPY_VANILLA_DIR)");
    }
    dir
}

/// A fresh `capybara` home with a vanilla profile pointing at `install`.
fn home_with_profile(install: &Path) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let profiles = home.path().join("capybara").join("profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::write(
        profiles.join("vanilla.toml"),
        Profile::vanilla(install).to_toml(),
    )
    .unwrap();
    home
}

fn run_app(home: &Path, envs: &[(&str, &std::ffi::OsStr)]) -> (Option<i32>, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"));
    cmd.env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_SCALE_FACTOR", "1")
        .env("CAPY_NO_INDEX", "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("start the app");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run_script(home: &Path, script: &str) -> (Option<i32>, String) {
    let file = home.join("script.txt");
    std::fs::write(&file, script).unwrap();
    run_app(home, &[("CAPY_SCRIPT", file.as_os_str())])
}

fn saves(home: &Path) -> PathBuf {
    home.join("capybara").join("saves").join("vanilla")
}

/// A game path (`a\b\c`) as a file under the saves (stored lower case).
fn saved_file(home: &Path, game: &str) -> PathBuf {
    GamePath::new(game).to_native(&saves(home))
}

fn read_saved(home: &Path, game: &str) -> Vec<u8> {
    std::fs::read(saved_file(home, game)).unwrap_or_else(|e| panic!("{game}: {e}"))
}

fn txt_cell(bytes: &[u8], row_name: &str, col: &str) -> String {
    let doc = TxtDoc::parse(bytes);
    let c = doc.column(col).unwrap_or_else(|| panic!("no column {col}"));
    let nc = doc.column("Name").unwrap();
    (0..doc.row_count())
        .find(|&r| doc.cell(r, nc).as_deref() == Some(row_name))
        .and_then(|r| doc.cell(r, c))
        .unwrap_or_default()
}

/// The own-tiles DT1 LvlTypes lists: (File slot, game path).
fn own_dt1(home: &Path) -> (u32, String) {
    let types = read_saved(home, "data\\global\\excel\\lvltypes.txt");
    let doc = TxtDoc::parse(&types);
    let (slot_col, own_rel) = (0..doc.row_count())
        .flat_map(|r| (0..doc.headers().len()).map(move |c| (r, c)))
        .filter_map(|(r, c)| doc.cell(r, c).map(|v| (c, v)))
        .find(|(_, v)| v.to_ascii_lowercase().ends_with("_custom.dt1"))
        .expect("a File slot lists the own-tiles DT1");
    let slot: u32 = doc.headers()[slot_col]
        .trim_start_matches("File")
        .trim()
        .parse()
        .unwrap();
    (
        slot,
        format!("data\\global\\tiles\\{}", own_rel.replace('/', "\\")),
    )
}

/// The act 1 palette index closest to pure red (a colour no town floor has
/// much of).
fn red_index(install: &Path) -> u8 {
    let saves = tempfile::tempdir().unwrap();
    let overlay = Profile::vanilla(install).open(saves.path()).unwrap();
    let pal = Palette::parse_dat(
        &overlay
            .read_bytes(&GamePath::new(act_palette_path(0)))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    (1..=224u8)
        .max_by_key(|&i| {
            let [r, g, b] = pal.colors[usize::from(i)];
            i32::from(r) * 2 - i32::from(g) - i32::from(b)
        })
        .unwrap()
}

/// The wall layer and tile of a painted wall cell.
fn wall_at(ds1: &Ds1, x: u32, y: u32) -> Option<(usize, u8, Cell)> {
    (0..ds1.walls.len()).find_map(|w| {
        let c = ds1.walls[w].tiles.get(x, y)?;
        (!c.is_empty()).then(|| (w, ds1.orientation(w, x, y).unwrap_or(0), c))
    })
}

fn tile_of(dt1: &Dt1, orientation: i32, cell: Cell) -> &Tile {
    dt1.tiles
        .iter()
        .find(|t| {
            (t.orientation, t.main_index, t.sub_index)
                == (
                    orientation,
                    i32::from(cell.main_index()),
                    i32::from(cell.sub_index()),
                )
        })
        .unwrap_or_else(|| panic!("no tile {orientation}:{cell:?} in the own DT1"))
}

#[test]
fn new_tiles_are_created_registered_placed_and_undone() {
    let Some(install) = enabled() else { return };
    let home = home_with_profile(&install);
    let red = red_index(&install);
    // a 40x30 picture: red left half, white right half
    let mut png = RgbaImage::filled(40, 30, [250, 250, 250, 255]);
    for y in 0..30usize {
        for x in 0..20usize {
            let i = (y * 40 + x) * 4;
            png.pixels[i..i + 4].copy_from_slice(&[230, 20, 20, 255]);
        }
    }
    let png_path = home.path().join("art.png");
    std::fs::write(&png_path, png.to_png().unwrap()).unwrap();

    // 1. a floor, a wall, a corner pair and a PNG wall; each painted
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool newtile floor solid={red}\n\
         tool newtile-assert-warning none\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 3 3\n\
         tool newtile wall o=1 h=100 solid={red}\n\
         tool newtile-flags set 01 12\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 5 5\n\
         tool newtile corner h=64 solid={red}\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 7 7\n\
         tool newtile wall o=2 h=96 image={} place=as-is\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 9 9\n\
         save\n",
        png_path.display()
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "create failed:\n{stderr}");

    // the own DT1: parses, writes back byte for byte, holds the tiles
    let (slot, own_game) = own_dt1(home.path());
    let own_bytes = read_saved(home.path(), &own_game);
    let own = Dt1::parse(&own_bytes).unwrap();
    assert_eq!(own.to_bytes().unwrap(), own_bytes, "write-back differs");
    assert_eq!(
        own.tiles.len(),
        5,
        "floor + wall + 2 corner halves + PNG wall"
    );

    // registered: LvlPrest Dt1Mask of the town has the slot's bit
    let prest = read_saved(home.path(), "data\\global\\excel\\lvlprest.txt");
    assert!(
        txt_cell(&prest, "Act 1 - Town 1", "Dt1Mask")
            .parse::<u32>()
            .is_ok_and(|m| m & (1 << (slot - 1)) != 0),
        "Dt1Mask"
    );

    // the DS1 lists the DT1 and its cells hold the new numbers
    let ds1 = Ds1::parse(&read_saved(home.path(), MAP)).unwrap();
    assert!(
        ds1.files.iter().any(|f| String::from_utf8_lossy(f)
            .to_ascii_lowercase()
            .contains("_custom.tg1")),
        "DS1 file list"
    );
    let floor = tile_of(&own, 0, ds1.floors[0].get(3, 3).unwrap());
    assert_eq!(floor.blocks.len(), 25, "a full floor diamond");
    assert_eq!(floor.height, -128);

    let (_, o, cell) = wall_at(&ds1, 5, 5).expect("the wall is painted");
    assert_eq!(o, 1);
    let wall = tile_of(&own, 1, cell);
    assert_eq!(wall.height, -(128 + 32), "100 px rounds up to 128");
    assert_eq!(
        wall.subtile_flags[12], 0x01,
        "the flags painted in the dialog"
    );
    assert_eq!(wall.subtile_flags[0], 0x07, "the default flags elsewhere");

    let (_, o, cell) = wall_at(&ds1, 7, 7).expect("the corner is painted");
    assert_eq!(o, 3);
    let right = tile_of(&own, 3, cell);
    let left = tile_of(&own, 4, cell);
    assert_eq!((right.height, left.height), (-96, -96));
    assert!(right.blocks.iter().all(|b| b.x >= 64));
    assert!(left.blocks.iter().all(|b| b.x < 96));

    let (_, o, cell) = wall_at(&ds1, 9, 9).expect("the PNG wall is painted");
    assert_eq!(o, 2);
    let art = tile_of(&own, 2, cell).render().unwrap();
    let opaque = art.alpha.iter().filter(|&&a| a != 0).count();
    assert_eq!(opaque, 40 * 30, "every pixel of the picture is stored");
    // its top-left is the drawable area's top-left (as-is placement)
    assert_eq!((art.offset_x, art.offset_y), (0, -96));

    // 2. reopen: the map loads the DT1 and is clean
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool addmap-assert-loaded {own_game} true\n\
         assert-dirty false\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "reopen failed:\n{stderr}");

    // 3. a fresh profile: create, add, paint, undo both; the map is as it was
    let fresh = home_with_profile(&install);
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool newtile floor solid={red}\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 3 3\n\
         assert-dirty true\n\
         undo\n\
         undo\n\
         assert-dirty false\n"
    );
    let (code, stderr) = run_script(fresh.path(), &script);
    assert_eq!(code, Some(0), "undo failed:\n{stderr}");
}

#[test]
fn a_new_empty_dt1_gets_a_tile_and_a_new_shape_with_undo() {
    let Some(install) = enabled() else { return };
    let home = home_with_profile(&install);
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool dt1-new act1/town mytiles register\n\
         tool dt1-assert-tiles 0\n\
         tool dt1-newtile\n\
         tool newtile wall o=1 h=64 solid=40 into=lib\n\
         tool newtile-add\n\
         tool dt1-assert-tiles 1\n\
         tool dt1-shape 0 wall o=1 h=128\n\
         tool dt1-undo\n\
         tool dt1-redo\n\
         tool dt1-save\n\
         tool wait-catalog\n\
         tool addmap-assert-loaded data\\global\\tiles\\act1\\town\\mytiles.dt1 true\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let path = "data\\global\\tiles\\act1\\town\\mytiles.dt1";
    let bytes = read_saved(home.path(), path);
    let dt1 = Dt1::parse(&bytes).unwrap();
    assert_eq!(dt1.to_bytes().unwrap(), bytes);
    assert_eq!(dt1.tiles.len(), 1);
    let t = &dt1.tiles[0];
    assert_eq!(t.orientation, 1);
    assert_eq!(t.height, -(128 + 32), "the new shape");
    // the 64-pixel picture kept its place: the bottom two block rows
    let img = t.render().unwrap();
    assert_eq!((img.offset_y, img.height), (-64, 64));
    // registered for the town's level type
    let types = read_saved(home.path(), "data\\global\\excel\\lvltypes.txt");
    assert!(
        String::from_utf8_lossy(&types)
            .to_ascii_lowercase()
            .contains("act1/town/mytiles.dt1"),
        "LvlTypes lists the new DT1"
    );
    // a bad name is refused before anything is written
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool dt1-new act1/town bad-name\n"
    );
    let (code, _) = run_script(home.path(), &script);
    assert_ne!(code, Some(0), "a name with a dash must be refused");
}

#[test]
fn a_blank_tile_painted_in_the_editor_is_added_to_the_map() {
    let Some(install) = enabled() else { return };
    let home = home_with_profile(&install);
    // a blank floor: the centre of the diamond is free to paint (no block
    // exists yet); a corner pixel is locked
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool newtile floor\n\
         tool newtile-edit\n\
         paint-pencil 80 39 120\n\
         paint-pencil 81 39 120\n\
         tool editor-shape roof roof=80\n\
         paint-pencil 82 39 121\n\
         tool editor-add-new\n\
         tool addmap-confirm\n\
         mode map\n\
         tool wait-catalog\n\
         save\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let (_, own_game) = own_dt1(home.path());
    let own = Dt1::parse(&read_saved(home.path(), &own_game)).unwrap();
    assert_eq!(own.tiles.len(), 1);
    let t = &own.tiles[0];
    assert_eq!(
        (t.orientation, t.roof_height),
        (15, 80),
        "reshaped to a roof"
    );
    let img = t.render().unwrap();
    let painted: Vec<u8> = img
        .pixels
        .iter()
        .zip(&img.alpha)
        .filter(|(_, a)| **a != 0)
        .map(|(p, _)| *p)
        .collect();
    assert_eq!(painted, [120, 120, 121]);
}

/// The opaque bounding box of `img`: (x0, y0, x1, y1), exclusive ends.
fn bbox(img: &RgbaImage) -> Option<(i32, i32, i32, i32)> {
    let mut b: Option<(i32, i32, i32, i32)> = None;
    for y in 0..img.height as i32 {
        for x in 0..img.width as i32 {
            let i = (y as usize * img.width as usize + x as usize) * 4;
            if img.pixels[i + 3] != 0 {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x + 1), e.max(y + 1)),
                });
            }
        }
    }
    b
}

/// `ds1` with nothing on it but what `keep` puts back.
fn bare(ds1: &Ds1) -> Ds1 {
    let mut d = ds1.clone();
    for f in &mut d.floors {
        *f = d2_formats::grid::Grid::new(f.width(), f.height());
    }
    for w in &mut d.walls {
        w.tiles = d2_formats::grid::Grid::new(w.tiles.width(), w.tiles.height());
    }
    d.shadow = d2_formats::grid::Grid::new(d.shadow.width(), d.shadow.height());
    d.objects.clear();
    d
}

fn black_to_transparent(mut img: RgbaImage) -> RgbaImage {
    for p in img.pixels.as_chunks_mut::<4>().0 {
        let background = p[0] == 0 && p[1] == 0 && p[2] == 0;
        p[3] = if background { 0 } else { 255 };
    }
    img
}

#[test]
fn created_tiles_draw_where_the_map_view_puts_their_kind() {
    let Some(install) = enabled() else { return };
    let home = home_with_profile(&install);
    let red = red_index(&install);
    let (fx, fy, wx, wy) = (12u32, 10u32, 14u32, 10u32);
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool newtile floor solid={red}\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush {fx} {fy}\n\
         tool newtile wall o=1 h=96 solid={red}\n\
         tool newtile-add\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush {wx} {wy}\n\
         save\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "create failed:\n{stderr}");

    // the CPU compositor on the profile's saves, as the app resolves the map
    let profile = Profile::vanilla(&install);
    let overlay = profile
        .open(&home.path().join("capybara").join("saves"))
        .unwrap();
    let path = GamePath::new(MAP);
    let (catalog, _) =
        MapCatalog::build(&overlay, profile.game_version, std::slice::from_ref(&path)).unwrap();
    let info = catalog.best(&path).expect("the map resolves");
    let (tiles, _) = TileSet::load(&overlay, &info.dt1_files).unwrap();
    let palette = Palette::parse_dat(
        &overlay
            .read_bytes(&GamePath::new(act_palette_path(info.palette_act)))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let ds1 = Ds1::parse(&overlay.read_bytes(&path).unwrap().unwrap()).unwrap();
    let one = |opts: &RenderOptions, d: &Ds1| render_with_objects(d, &tiles, None, &palette, opts);
    let at1 = RenderOptions {
        zoom_div: 1,
        gamma: GAMMA,
        ..RenderOptions::default()
    };

    // the new floor alone covers exactly what the shipped floor of a cell
    // covers there: the 160x80 box at the cell's origin
    let new_floor = ds1.floors[0].get(fx, fy).unwrap();
    let mut only = bare(&ds1);
    only.floors[0].set(fx, fy, new_floor);
    let out = one(&at1, &only).unwrap();
    assert!(out.missing.is_empty(), "the new floor is found");
    let cell_x = (fx as i32 - fy as i32) * 80 - out.origin.0;
    let cell_y = (fx as i32 + fy as i32) * 40 - out.origin.1;
    let b = bbox(&out.image).expect("the floor draws");
    assert_eq!(b, (cell_x, cell_y, cell_x + 160, cell_y + 79), "floor box");
    // ... in its colour
    let shown = palette.with_gamma(GAMMA);
    let want = shown.colors[usize::from(red)];
    let centre = ((cell_y + 39) as usize * out.image.width as usize + (cell_x + 80) as usize) * 4;
    assert_eq!(
        &out.image.pixels[centre..centre + 3],
        &want[..],
        "floor colour"
    );

    // the new wall alone stands on its cell: 160 wide from the cell's
    // left edge, its base on the bottom corner of the cell's diamond
    let (w, o, cell) = wall_at(&ds1, wx, wy).unwrap();
    let mut only = bare(&ds1);
    only.walls[w].tiles.set(wx, wy, cell);
    only.walls[w]
        .orientations
        .set(wx, wy, ds1.walls[w].orientations.get(wx, wy).unwrap());
    assert_eq!(o, 1);
    let out = one(&at1, &only).unwrap();
    let cell_x = (wx as i32 - wy as i32) * 80 - out.origin.0;
    let cell_y = (wx as i32 + wy as i32) * 40 - out.origin.1;
    let b = bbox(&out.image).expect("the wall draws");
    assert_eq!(
        b,
        (cell_x, cell_y + 80 - 96, cell_x + 160, cell_y + 80),
        "wall box"
    );

    // the app's map view draws the saved map as the CPU compositor does
    let shot = home.path().join("map.png");
    let zoom = 4u32;
    let (code, stderr) = run_app(
        home.path(),
        &[
            ("CAPY_OPEN_MAP", std::ffi::OsStr::new(MAP)),
            (
                "CAPY_LVLTYPE",
                info.lvl_type.unwrap_or(0).to_string().as_ref(),
            ),
            (
                "CAPY_DEF",
                info.lvlprest_def.unwrap_or(0).to_string().as_ref(),
            ),
            ("CAPY_ZOOM", zoom.to_string().as_ref()),
            ("CAPY_SCREENSHOT_MAP_ONLY", "1".as_ref()),
            ("CAPY_SCREENSHOT", shot.as_os_str()),
        ],
    );
    assert_eq!(code, Some(0), "screenshot run failed:\n{stderr}");
    let gpu = black_to_transparent(RgbaImage::from_png(&std::fs::read(&shot).unwrap()).unwrap());
    let objects = ObjectCatalog::build(&overlay, profile.game_version)
        .unwrap()
        .0;
    let sprites = capy_render::ObjectSprites::new(&overlay, &objects);
    let cpu = render_with_objects(
        &ds1,
        &tiles,
        Some(&sprites),
        &palette,
        &RenderOptions {
            zoom_div: zoom,
            gamma: GAMMA,
            ..RenderOptions::default()
        },
    )
    .unwrap();
    assert_eq!((gpu.width, gpu.height), (cpu.image.width, cpu.image.height));
    let d = compare_images(&gpu, &cpu.image, 16, 4);
    assert!(
        d.within_tolerance >= 0.98,
        "GPU and CPU differ: {} within tolerance 16",
        d.within_tolerance
    );
    // and in the new floor's box the GPU draws what the CPU draws
    let z = zoom as i32;
    let bx = (fx as i32 - fy as i32) * 80 / z - cpu.origin.0;
    let by = (fx as i32 + fy as i32) * 40 / z - cpu.origin.1;
    let (mut same, mut total, mut red_px) = (0, 0, 0);
    for y in by + 2..by + 80 / z - 2 {
        for x in bx + 40 / z - 4..bx + 120 / z + 4 {
            let i = (y as usize * gpu.width as usize + x as usize) * 4;
            let (g, c) = (&gpu.pixels[i..i + 3], &cpu.image.pixels[i..i + 3]);
            total += 1;
            if g.iter().zip(c).all(|(a, b)| a.abs_diff(*b) <= 16) {
                same += 1;
            }
            if c.iter().zip(&want).all(|(a, b)| a.abs_diff(*b) <= 2) {
                red_px += 1;
            }
        }
    }
    assert!(
        red_px > 0,
        "the new floor shows in the CPU render of the map"
    );
    assert!(
        same * 100 >= total * 95,
        "GPU and CPU differ at the new floor: {same} of {total}"
    );
}
