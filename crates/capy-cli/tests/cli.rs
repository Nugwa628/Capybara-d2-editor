//! CLI acceptance tests against the real game installs.
//!
//! * Headless render parity (P5 acceptance): `render` of the 18 oracle maps
//!   through `capy_cli::run_in`, compared with DS1Edit's renders in
//!   `testdata\private\oracle-renders\` exactly like
//!   `crates/capy-render/tests/oracle.rs` does.
//! * `render` of a Median XL map by game path, with no explicit LvlType.
//! * `index status` on a fresh home, `maps list --filter`.
//!
//! The render tests are heavy: they run in release builds or with
//! `CAPY_RENDER_TESTS=1`, and skip quietly when the installs are absent.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use capy_render::{CompareOptions, Coverage, RgbaImage, compare_images_with};
use d2_assets::{GamePath, Profile};
use d2_formats::ds1::Ds1;

// ---- shared helpers (the same rules as crates/capy-render/tests/common) ----

const MAIN_CHECKOUT: &str = r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor";

fn heavy_enabled() -> bool {
    std::env::var("CAPY_RENDER_TESTS").is_ok_and(|v| v == "1") || !cfg!(debug_assertions)
}

fn install(var: &str, default: &str) -> Option<PathBuf> {
    let dir = std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default));
    dir.is_dir().then_some(dir)
}

fn vanilla_dir() -> Option<PathBuf> {
    install("CAPY_VANILLA_DIR", r"C:\Program Files (x86)\Diablo II")
}

fn mxl_dir() -> Option<PathBuf> {
    install("CAPY_MXL_DIR", r"C:\games\median-xl")
}

fn private_dir() -> PathBuf {
    std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(MAIN_CHECKOUT).join("testdata").join("private"))
}

/// Copied from `crates/capy-render/tests/oracle.rs`: box around every DS1
/// object that is excluded from the comparison, in 1:1 pixels relative to the
/// object's anchor `((x - y) * 16, (x + y) * 8)`: `(left, top, right,
/// bottom)`. Scaled by `1/zoom_div` for zoomed renders. (The oracle draws
/// animated object sprites, which this renderer does not.)
const OBJECT_MASK: (i32, i32, i32, i32) = (-96, -224, 96, 48);

/// Zoom divisor of the oracle renders (1:4).
const ZOOM: i32 = 4;

/// Copied from `oracle.rs`: the mask for `ds1`'s objects on a canvas whose
/// pixel (0, 0) is map-space `origin`, at zoom 1:`ZOOM`; `scale` 2 doubles the
/// box (the `wide16` column).
fn object_mask(ds1: &Ds1, origin: (i32, i32), width: u32, height: u32, scale: i32) -> Vec<bool> {
    let div = ZOOM;
    let mut mask = vec![false; width as usize * height as usize];
    for o in &ds1.objects {
        let ax = (o.x - o.y) * 16 / div - origin.0;
        let ay = (o.x + o.y) * 8 / div - origin.1;
        let x0 = (ax + OBJECT_MASK.0 * scale / div).max(0);
        let y0 = (ay + OBJECT_MASK.1 * scale / div).max(0);
        let x1 = (ax + OBJECT_MASK.2 * scale / div).min(width as i32);
        let y1 = (ay + OBJECT_MASK.3 * scale / div).min(height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                mask[y as usize * width as usize + x as usize] = true;
            }
        }
    }
    mask
}

/// A temporary `capybara\` home holding one profile TOML.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn with_profile(profile: &Profile) -> Home {
        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path().join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        std::fs::write(
            profiles.join(format!("{}.toml", profile.id)),
            profile.to_toml(),
        )
        .unwrap();
        Home { dir }
    }

    fn run(&self, args: &[&str]) -> (u8, String, String) {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = capy_cli::run_in(&args, self.dir.path().to_path_buf(), &mut out, &mut err)
            .expect("a CLI verb");
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }
}

struct MapEntry {
    name: String,
    ds1: String,
    lvltypes_id: u32,
    lvlprest_def: u32,
}

fn read_manifest(renders: &Path) -> Option<Vec<MapEntry>> {
    let text = std::fs::read_to_string(renders.join("manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    manifest
        .as_array()?
        .iter()
        .map(|e| {
            Some(MapEntry {
                name: e["name"].as_str()?.to_owned(),
                ds1: e["ds1"].as_str()?.to_owned(),
                lvltypes_id: u32::try_from(e["lvltypes_id"].as_u64()?).ok()?,
                lvlprest_def: u32::try_from(e["lvlprest_def"].as_u64()?).ok()?,
            })
        })
        .collect()
}

/// `origin X,Y` of the render's result line.
fn parse_origin(stdout: &str) -> (i32, i32) {
    let rest = stdout
        .split("origin ")
        .nth(1)
        .unwrap_or_else(|| panic!("no origin in:\n{stdout}"));
    let coords = rest.split(',').take(2).collect::<Vec<_>>();
    let x = coords[0].trim().parse().unwrap();
    let y: i32 = coords[1]
        .split(|c: char| !(c.is_ascii_digit() || c == '-'))
        .next()
        .unwrap()
        .parse()
        .unwrap();
    (x, y)
}

fn parse_missing(stdout: &str) -> usize {
    let rest = stdout
        .split("missing tiles: ")
        .nth(1)
        .unwrap_or_else(|| panic!("no missing-tile count in:\n{stdout}"));
    rest.split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn headless_render_matches_the_oracle() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_RENDER_TESTS=1 or use --release)");
        return;
    }
    let renders = private_dir().join("oracle-renders");
    let (Some(vanilla), Some(entries)) = (vanilla_dir(), read_manifest(&renders)) else {
        eprintln!("skipped: vanilla install or oracle renders not found");
        return;
    };
    let home = Home::with_profile(&Profile::vanilla(&vanilla));
    let tmp = tempfile::tempdir().unwrap();
    let only = std::env::var("CAPY_ORACLE_ONLY").ok();
    let game = Profile::vanilla(&vanilla)
        .open(&home.dir.path().join("saves"))
        .unwrap();

    eprintln!(
        "{:<28} {:>11} {:>11} {:>9} {:>8} {:>8} {:>5}",
        "map", "ours", "oracle", "shift", "tol16", "wide16", "miss"
    );
    let mut failures = Vec::new();
    let mut checked = 0;
    for entry in &entries {
        if only.as_deref().is_some_and(|o| !entry.name.contains(o)) {
            continue;
        }
        let png = tmp.path().join(format!("{}.png", entry.name));
        let (lvltype, def) = (
            entry.lvltypes_id.to_string(),
            entry.lvlprest_def.to_string(),
        );
        let (code, stdout, stderr) = home.run(&[
            "render",
            &entry.ds1,
            "--profile",
            "vanilla",
            "--lvltype",
            &lvltype,
            "--def",
            &def,
            "--zoom",
            "1:4",
            "--out",
            png.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{}: exit code\n{stdout}\n{stderr}", entry.name);

        let ours = RgbaImage::from_png(&std::fs::read(&png).unwrap()).unwrap();
        let oracle = RgbaImage::from_png(
            &std::fs::read(renders.join(format!("{}.png", entry.name))).unwrap(),
        )
        .unwrap();
        let ds1 = Ds1::parse(
            &game
                .read_bytes(&GamePath::new(&entry.ds1))
                .unwrap()
                .expect("ds1 in the install"),
        )
        .unwrap();
        let origin = parse_origin(&stdout);
        let mask = object_mask(&ds1, origin, ours.width, ours.height, 1);
        let report = |tolerance, mask: &[bool]| {
            compare_images_with(
                &ours,
                &oracle,
                &CompareOptions {
                    tolerance,
                    max_shift: 16,
                    coverage: Coverage::Both,
                    mask: Some(mask),
                },
            )
        };
        let r16 = report(16, &mask);
        let wide_mask = object_mask(&ds1, origin, ours.width, ours.height, 2);
        let wide16 = report(16, &wide_mask).diff.within_tolerance;
        let missing = parse_missing(&stdout);
        eprintln!(
            "{:<28} {:>5}x{:<5} {:>5}x{:<5} {:>4},{:<4} {:>8.4} {:>8.4} {:>5}",
            entry.name,
            ours.width,
            ours.height,
            oracle.width,
            oracle.height,
            r16.diff.best_shift.0,
            r16.diff.best_shift.1,
            r16.diff.within_tolerance,
            wide16,
            missing,
        );
        if (ours.width, ours.height) != (oracle.width, oracle.height) {
            failures.push(format!("{}: canvas size differs", entry.name));
        }
        if r16.diff.within_tolerance < 0.95 {
            failures.push(format!(
                "{}: within_tolerance(16) = {:.4}",
                entry.name, r16.diff.within_tolerance
            ));
        }
        checked += 1;
    }
    assert!(failures.is_empty(), "parity failures: {failures:#?}");
    if only.is_none() {
        assert_eq!(checked, entries.len());
    }
}

#[test]
fn median_xl_map_by_game_path_renders_without_missing_tiles() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_RENDER_TESTS=1 or use --release)");
        return;
    }
    let Some(mxl) = mxl_dir() else {
        eprintln!("skipped: Median XL install not found");
        return;
    };
    let home = Home::with_profile(&Profile::median_xl(&mxl));
    let png = home.dir.path().join("talrasha.png");
    let (code, stdout, stderr) = home.run(&[
        "render",
        r"data\global\tiles\mxl\quest\talrasha\talrasha.ds1",
        "--profile",
        "mxl",
        "--zoom",
        "1:4",
        "--out",
        png.to_str().unwrap(),
    ]);
    eprintln!("{stdout}{stderr}");
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(parse_missing(&stdout), 0, "{stdout}");
    let image = RgbaImage::from_png(&std::fs::read(&png).unwrap()).unwrap();
    assert!(image.width > 1 && image.height > 1);
}

#[test]
fn a_ds1_file_outside_the_profile_needs_a_lvltype() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_RENDER_TESTS=1 or use --release)");
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let profile = Profile::vanilla(&vanilla);
    let home = Home::with_profile(&profile);
    let overlay = profile.open(&home.dir.path().join("saves")).unwrap();
    let bytes = overlay
        .read_bytes(&GamePath::new(r"data\global\tiles\act1\town\towne1.ds1"))
        .unwrap()
        .unwrap();
    let file = home.dir.path().join("copy.ds1");
    std::fs::write(&file, bytes).unwrap();
    let png = home.dir.path().join("copy.png");

    let (code, _, stderr) = home.run(&[
        "render",
        file.to_str().unwrap(),
        "--profile",
        "vanilla",
        "--out",
        png.to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("--lvltype"), "{stderr}");
    assert!(!png.exists());

    let (code, stdout, stderr) = home.run(&[
        "render",
        file.to_str().unwrap(),
        "--profile",
        "vanilla",
        "--lvltype",
        "1",
        "--def",
        "1",
        "--zoom",
        "1:8",
        "--out",
        png.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(parse_missing(&stdout), 0, "{stdout}");
    assert!(png.is_file());

    // Unwritable output: exit 1.
    let (code, _, stderr) = home.run(&[
        "render",
        file.to_str().unwrap(),
        "--profile",
        "vanilla",
        "--lvltype",
        "1",
        "--zoom",
        "1:8",
        "--out",
        home.dir
            .path()
            .join("no-such-dir")
            .join("o.png")
            .to_str()
            .unwrap(),
    ]);
    assert_eq!(code, 1, "{stderr}");
}

#[test]
fn index_status_of_a_fresh_home_is_missing() {
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let home = Home::with_profile(&Profile::vanilla(&vanilla));
    let (code, stdout, stderr) = home.run(&["index", "status", "--profile", "vanilla"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.trim(), "missing");
}

#[test]
fn maps_list_filters_case_insensitively() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_RENDER_TESTS=1 or use --release)");
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let home = Home::with_profile(&Profile::vanilla(&vanilla));
    let (code, stdout, stderr) =
        home.run(&["maps", "list", "--profile", "vanilla", "--filter", "TowNE1"]);
    assert_eq!(code, 0, "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.split('\t').next() == Some(r"data\global\tiles\act1\town\towne1.ds1")),
        "{stdout}"
    );
    let paths: Vec<&str> = lines.iter().filter_map(|l| l.split('\t').next()).collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted, "sorted by path");
    assert!(paths.iter().all(|p| p.contains("towne1")));
    // The second run is served from the name cache and gives the same list.
    let (code, again, _) =
        home.run(&["maps", "list", "--profile", "vanilla", "--filter", "towne1"]);
    assert_eq!(code, 0);
    assert_eq!(again, stdout);
}
