//! P2 acceptance: files edited with [`Document`] load in the DS1Edit oracle.
//!
//! For vanilla maps from the oracle manifest the test loads the DS1, applies
//! real edits (floor paint with tiles from elsewhere in the map, a wall
//! selection moved Wall(0) to Wall(1), a new object with a 3-point NPC path,
//! a version upgrade on one map below v16, an erased region), serializes it
//! with `to_bytes`, runs the oracle headless on the file and checks:
//!
//! 1. stdout has `headless mode : saved WxH screenshot` and the PNG exists
//!    (the exit code is 0 even on failure, so it proves nothing);
//! 2. the oracle PNG matches `capy_render`'s CPU render of the edited `Ds1`
//!    (same object mask and settings as `capy-render/tests/oracle.rs`):
//!    `within_tolerance(16) >= 0.95`. The oracle therefore reads our edits the
//!    way we wrote them.
//!
//! Heavy: runs in release builds or with `CAPY_ORACLE_TESTS=1`, and skips
//! with a printed reason when the oracle exe, the game install or the oracle
//! manifest are absent. The oracle folder is never modified by this test (the
//! exe itself writes `tmp\undo*.bin` there).
//!
//! Environment knobs:
//! * `CAPY_ORACLE_ONLY=<substring>`: only maps whose name contains it.
//! * `CAPY_ORACLE_MAPS=<n>`: how many manifest maps to run (default 6, `all`
//!   for every map).
//! * `CAPY_PRIVATE_DIR`, `CAPY_VANILLA_DIR`: as in the capy-render tests.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use capy_edit::{CellRect, Document, EditError, LayerRef, MoveMode, SelectOp, Selection, diff};
use capy_render::{
    CompareOptions, Coverage, RenderOptions, RgbaImage, TileSet, compare_images_with, render,
};
use d2_assets::tables::load_excel;
use d2_assets::{GamePath, Overlay, Profile};
use d2_formats::ds1::{Ds1, Object, PathPoint};
use d2_formats::excel::ExcelTableKind;
use d2_formats::palette::{Palette, act_palette_path};
use d2_formats::tables::{lvl_prests, lvl_types};

const MAIN_CHECKOUT: &str = r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor";
const ORACLE_TIMEOUT: Duration = Duration::from_secs(120);

// Copied from crates/capy-render/tests/oracle.rs (P1 acceptance); keep in sync.
/// Box around every DS1 object that is excluded from the comparison, in 1:1
/// pixels relative to the object's anchor `((x - y) * 16, (x + y) * 8)`
/// (object coordinates are sub-tiles, 5 per tile): `(left, top, right,
/// bottom)`. Scaled by `1/zoom_div` for zoomed renders. The oracle draws
/// animated object sprites (and `rand()` frames) that capy-render does not.
const OBJECT_MASK: (i32, i32, i32, i32) = (-96, -224, 96, 48);
const ZOOM: u32 = 4;
const GAMMA: f32 = 1.30;
// end of copy

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

fn heavy_enabled() -> bool {
    std::env::var("CAPY_ORACLE_TESTS").is_ok_and(|v| v == "1") || !cfg!(debug_assertions)
}

/// Same as capy-render's `object_mask`.
fn object_mask(ds1: &Ds1, origin: (i32, i32), width: u32, height: u32) -> Vec<bool> {
    let div = ZOOM as i32;
    let mut mask = vec![false; width as usize * height as usize];
    for o in &ds1.objects {
        let ax = (o.x - o.y) * 16 / div - origin.0;
        let ay = (o.x + o.y) * 8 / div - origin.1;
        let x0 = (ax + OBJECT_MASK.0 / div).max(0);
        let y0 = (ay + OBJECT_MASK.1 / div).max(0);
        let x1 = (ax + OBJECT_MASK.2 / div).min(width as i32);
        let y1 = (ay + OBJECT_MASK.3 / div).min(height as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                mask[y as usize * width as usize + x as usize] = true;
            }
        }
    }
    mask
}

struct MapEntry {
    name: String,
    ds1: String,
    lvltypes_id: u32,
    lvlprest_def: u32,
    /// The oracle's render of the unedited map, when the manifest has one.
    png: Option<PathBuf>,
    /// Edit with `upgrade_version(18)` first (maps below v16 only).
    upgrade: bool,
}

/// Vanilla maps below v16, which the manifest has none of: `(ds1, LvlTypes id,
/// LvlPrest def, upgrade_version(18) first)`. The ids come from the oracle's
/// `assets\*.ini`. The others take the app's route instead: the first edit
/// that needs a newer version gets an upgrade to exactly that version.
const OLD_MAPS: [(&str, u32, u32, bool); 4] = [
    (r"data\global\tiles\act1\town\townne.ds1", 1, 1, true), // v13
    (r"data\global\tiles\act1\court\court1b.ds1", 6, 166, false), // v3
    (r"data\global\tiles\act1\outdoors\corral1.ds1", 2, 31, false), // v15
    (r"data\global\tiles\act1\outdoors\stnclf3.ds1", 2, 17, false), // v12
];

fn old_maps() -> Vec<MapEntry> {
    OLD_MAPS
        .iter()
        .map(|&(ds1, lvltypes_id, lvlprest_def, upgrade)| {
            let stem = ds1
                .rsplit('\\')
                .next()
                .unwrap_or(ds1)
                .trim_end_matches(".ds1");
            MapEntry {
                name: format!("old__{stem}"),
                ds1: ds1.to_owned(),
                lvltypes_id,
                lvlprest_def,
                png: None,
                upgrade,
            }
        })
        .collect()
}

fn read_manifest(renders: &Path) -> Option<Vec<MapEntry>> {
    let text = std::fs::read_to_string(renders.join("manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    manifest
        .as_array()?
        .iter()
        .map(|e| {
            let name = e["name"].as_str()?.to_owned();
            Some(MapEntry {
                png: Some(renders.join(format!("{name}.png"))),
                upgrade: false,
                ds1: e["ds1"].as_str()?.to_owned(),
                lvltypes_id: u32::try_from(e["lvltypes_id"].as_u64()?).ok()?,
                lvlprest_def: u32::try_from(e["lvlprest_def"].as_u64()?).ok()?,
                name,
            })
        })
        .collect()
}

// ----- edits ------------------------------------------------------------------

/// What was applied to one map, for the table.
#[derive(Default)]
struct Applied {
    painted: usize,
    moved_walls: usize,
    layer_added: bool,
    object: bool,
    path_points: usize,
    upgraded: Option<(i32, i32)>,
    erased: bool,
    notes: Vec<String>,
}

impl Applied {
    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some((from, to)) = self.upgraded {
            parts.push(format!("v{from}->v{to}"));
        }
        parts.push(format!("paint{}", self.painted));
        parts.push(format!(
            "wall{}{}",
            self.moved_walls,
            if self.layer_added { "+L" } else { "" }
        ));
        if self.object {
            parts.push(format!("obj+path{}", self.path_points));
        }
        if self.erased {
            parts.push("erase".to_owned());
        }
        parts.extend(self.notes.iter().cloned());
        parts.join(" ")
    }
}

/// Runs `edit`, and when the map needs a newer version, upgrades (what the
/// app offers on `NeedsVersion`) and runs it again.
fn with_upgrade<T>(
    doc: &mut Document,
    applied: &mut Applied,
    mut edit: impl FnMut(&mut Document) -> capy_edit::Result<T>,
) -> capy_edit::Result<T> {
    match edit(doc) {
        Err(EditError::NeedsVersion {
            current,
            mut required,
        }) => {
            // the version an edit asks for can itself need more (a v3 file
            // with tag cells asks for v4 for a wall layer, but v4 has no tag
            // layer, so the upgrade asks for v10): follow the chain
            for _ in 0..4 {
                match doc.upgrade_version(required) {
                    Ok(()) => break,
                    Err(EditError::NeedsVersion { required: more, .. }) if more > required => {
                        required = more;
                    }
                    Err(e) => return Err(e),
                }
            }
            applied.upgraded = Some((applied.upgraded.map_or(current, |u| u.0), required));
            edit(doc)
        }
        other => other,
    }
}

fn edit_map(ds1: Ds1, upgrade: bool) -> (Document, Applied) {
    let mut doc = Document::new(ds1);
    let mut applied = Applied::default();
    let (w, h) = (doc.ds1().width, doc.ds1().height);

    if upgrade && doc.ds1().version < 16 {
        let from = doc.ds1().version;
        doc.upgrade_version(18).unwrap();
        applied.upgraded = Some((from, 18));
    }

    // the region that is erased at the end; the other edits keep out of it
    let erase_rect = CellRect::new(
        (w / 8, h / 2),
        (
            (w / 8 + (w / 4).max(2) - 1).min(w - 1),
            (h / 2 + (h / 4).max(2) - 1).min(h - 1),
        ),
    );
    let in_erase = |x: u32, y: u32| {
        (erase_rect.x0..=erase_rect.x1).contains(&x) && (erase_rect.y0..=erase_rect.y1).contains(&y)
    };

    // 1. floor cells painted with tiles copied from elsewhere in the same map
    let floor = LayerRef::Floor(0);
    let sources: Vec<_> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter_map(|(x, y)| floor.get(doc.ds1(), x, y).map(|v| (x, y, v)))
        .filter(|(_, _, v)| !floor.is_empty_value(*v))
        .collect();
    if !sources.is_empty() {
        let mut cells = Vec::new();
        for k in 0..16u32 {
            let tx = (k * 7 + 3) % w;
            let ty = (k * 5 + 1) % h;
            let (_, _, value) = sources[(k as usize * 13 + 5) % sources.len()];
            if !in_erase(tx, ty) && floor.get(doc.ds1(), tx, ty) != Some(value) {
                cells.push((tx, ty, value));
            }
        }
        applied.painted = cells.len();
        doc.begin_group("Paint floor");
        doc.paint(floor, &cells).unwrap();
        doc.end_group();
    }

    // 2. a wall selection moved Wall(0) -> Wall(1)
    let walls: Vec<(u32, u32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            LayerRef::Wall(0)
                .get(doc.ds1(), x, y)
                .is_some_and(|v| !LayerRef::Wall(0).is_empty_value(v))
                && !in_erase(x, y)
        })
        .collect();
    if walls.is_empty() {
        applied.notes.push("nowalls".to_owned());
    } else {
        let step = (walls.len() / 12).max(1);
        let mut sel = Selection::new();
        for &(x, y) in walls.iter().step_by(step).take(12) {
            sel.insert(LayerRef::Wall(0), x, y);
        }
        let report = with_upgrade(&mut doc, &mut applied, |d| {
            d.move_to_layer(
                &sel,
                LayerRef::Wall(0),
                LayerRef::Wall(1),
                MoveMode::Overwrite,
            )
        })
        .unwrap();
        applied.moved_walls = report.moved;
        applied.layer_added = report.layer_added.is_some();
    }

    // 3. an object, and a 3-point NPC path where the version allows it
    let template = doc
        .ds1()
        .objects
        .iter()
        .find(|o| o.kind == 1)
        .or(doc.ds1().objects.first())
        .copied()
        // a map without objects: a plain NPC-type entry
        .or(Some(Object {
            kind: 1,
            id: 1,
            x: 0,
            y: 0,
            flags: 0,
        }));
    let (cx, cy) = (w as i32 * 5 / 2, h as i32 * 5 / 2);
    let spot = (0..(w.max(h) as i32 * 5))
        .flat_map(|r| [(cx + r, cy), (cx, cy + r), (cx - r, cy), (cx, cy - r)])
        .find(|&(x, y)| {
            !in_erase(x as u32 / 5, y as u32 / 5)
                && x >= 0
                && y >= 0
                && x < w as i32 * 5
                && y < h as i32 * 5
                && doc.ds1().objects.iter().all(|o| (o.x, o.y) != (x, y))
        });
    if let (Some(t), Some((x, y))) = (template, spot) {
        let index = doc.add_object(Object { x, y, ..t }).unwrap();
        applied.object = true;
        if doc.ds1().version >= 14 {
            let version = doc.ds1().version;
            for (i, (dx, dy)) in [(1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
                let action = if version >= 15 && i == 1 { 2 } else { 1 };
                doc.add_path_point(
                    index,
                    PathPoint {
                        x: x + dx,
                        y: y + dy,
                        action: 1,
                    },
                    None,
                )
                .unwrap();
                if action != 1 {
                    doc.set_path_action(index, i, action).unwrap();
                }
            }
            applied.path_points = doc.path_of(index).map_or(0, |p| p.points.len());
            assert_eq!(applied.path_points, 3);
        } else {
            applied.notes.push("nopath(<v14)".to_owned());
        }
    }

    // 4. erase a region on every layer
    let layers = LayerRef::all_in(doc.ds1());
    let mut sel = Selection::new();
    sel.box_select(doc.ds1(), erase_rect, &layers, false, SelectOp::Replace);
    if !sel.is_empty() {
        doc.erase(&sel).unwrap();
        applied.erased = true;
    }
    (doc, applied)
}

// ----- the oracle ---------------------------------------------------------------

struct OracleRun {
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn run_oracle(
    exe: &Path,
    cwd: &Path,
    ds1: &Path,
    lvltypes_id: u32,
    lvlprest_def: u32,
    out_png: &Path,
) -> OracleRun {
    let mut child = Command::new(exe)
        .current_dir(cwd)
        .arg(ds1)
        .arg(lvltypes_id.to_string())
        .arg(lvlprest_def.to_string())
        .arg("--zoom=1:4")
        .arg("--headless")
        .arg(out_png)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = drain(Box::new(child.stdout.take().unwrap()));
    let err = drain(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let mut timed_out = false;
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > ORACLE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    OracleRun {
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
        timed_out,
    }
}

/// `(width, height)` of the line `headless mode : saved W x H screenshot`.
fn saved_size(stdout: &str) -> Option<(u32, u32)> {
    let line = stdout
        .lines()
        .find(|l| l.contains("headless mode : saved"))?;
    let rest = line.split("saved").nth(1)?;
    let mut nums = rest
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().ok());
    Some((nums.next()??, nums.next()??))
}

// ----- shared game plumbing (copied from capy-render's test helpers) ----------------

fn load_bytes(overlay: &Overlay, path: &str) -> Vec<u8> {
    overlay
        .read_bytes(&GamePath::new(path))
        .unwrap()
        .unwrap_or_else(|| panic!("{path} not found"))
}

struct Row {
    name: String,
    edits: String,
    saved: bool,
    size: String,
    tol16: f64,
    vs_original: Option<f64>,
    shift: (i32, i32),
    compared: u64,
}

#[test]
fn edited_files_load_in_the_oracle() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_ORACLE_TESTS=1 or use --release)");
        return;
    }
    let oracle_dir = private_dir().join("oracle");
    let exe = oracle_dir.join("ds1edit.exe");
    let renders = private_dir().join("oracle-renders");
    let (Some(vanilla), true, Some(entries)) =
        (vanilla_dir(), exe.is_file(), read_manifest(&renders))
    else {
        eprintln!(
            "skipped: need the vanilla install, {} and {}",
            exe.display(),
            renders.join("manifest.json").display()
        );
        return;
    };

    let saves = tempfile::tempdir().unwrap();
    let profile = Profile::vanilla(&vanilla);
    let version = profile.game_version;
    let overlay = profile.open(saves.path()).unwrap();
    let types = load_excel(&overlay, ExcelTableKind::LvlTypes, version).unwrap();
    let prests = load_excel(&overlay, ExcelTableKind::LvlPrest, version).unwrap();
    let (types, prests) = (lvl_types(&types).unwrap(), lvl_prests(&prests).unwrap());

    let only = std::env::var("CAPY_ORACLE_ONLY").ok();
    let limit = match std::env::var("CAPY_ORACLE_MAPS").ok().as_deref() {
        Some("all") => usize::MAX,
        Some(n) => n.parse().unwrap_or(8),
        None => 6,
    };
    let work = tempfile::tempdir().unwrap();
    let mut rows = Vec::new();
    // spread the picks over the manifest (towns, outdoors, dungeons, other acts)
    let mut chosen: Vec<&MapEntry> = entries.iter().collect();
    if chosen.len() > limit {
        let all = chosen.clone();
        chosen = (0..limit).map(|i| all[i * all.len() / limit]).collect();
    }
    let old = old_maps();
    chosen.extend(old.iter());
    chosen.retain(|e| only.as_deref().is_none_or(|o| e.name.contains(o)));

    for entry in chosen {
        let name = &entry.name;
        let original = Ds1::parse(&load_bytes(&overlay, &entry.ds1)).unwrap();
        let (doc, applied) = edit_map(original.clone(), entry.upgrade);
        let edited = doc.ds1().clone();
        let changes = diff(&original, &edited);
        assert!(!changes.is_empty(), "{name}: the edits changed nothing");

        let bytes = edited
            .to_bytes()
            .unwrap_or_else(|e| panic!("{name}: edited map does not serialize: {e}"));
        assert_eq!(Ds1::parse(&bytes).unwrap(), edited, "{name}: round trip");
        let file = work.path().join(format!("{name}.ds1"));
        std::fs::write(&file, &bytes).unwrap();
        let png = work.path().join(format!("{name}.png"));

        let run = run_oracle(
            &exe,
            &oracle_dir,
            &file,
            entry.lvltypes_id,
            entry.lvlprest_def,
            &png,
        );
        assert!(
            !run.timed_out,
            "{name}: oracle timed out after {ORACLE_TIMEOUT:?}"
        );
        let size = saved_size(&run.stdout);
        assert!(
            size.is_some(),
            "{name}: no 'headless mode : saved WxH screenshot' line.\nstdout:\n{}\nstderr:\n{}",
            run.stdout,
            run.stderr
        );
        assert!(
            png.is_file() && std::fs::metadata(&png).unwrap().len() > 0,
            "{name}: the oracle saved no PNG"
        );
        let oracle = RgbaImage::from_png(&std::fs::read(&png).unwrap()).unwrap();
        assert_eq!(
            size,
            Some((oracle.width, oracle.height)),
            "{name}: stdout size"
        );

        // our render of the edited model
        let lvl_type = types
            .iter()
            .find(|t| t.id == entry.lvltypes_id)
            .unwrap_or_else(|| panic!("no LvlTypes id {}", entry.lvltypes_id));
        let prest = prests
            .iter()
            .find(|p| p.def == entry.lvlprest_def)
            .unwrap_or_else(|| panic!("no LvlPrest def {}", entry.lvlprest_def));
        let dt1_files: Vec<GamePath> = prest
            .dt1_files(lvl_type)
            .filter_map(GamePath::from_tile_ref)
            .collect();
        let (tiles, _) = TileSet::load(&overlay, &dt1_files).unwrap();
        let act = capy_render::palette_act(Some(lvl_type), edited.effective_act());
        let palette = Palette::parse_dat(&load_bytes(&overlay, act_palette_path(act))).unwrap();
        let opts = RenderOptions {
            zoom_div: ZOOM,
            gamma: GAMMA,
            ..RenderOptions::default()
        };
        let ours = render(&edited, &tiles, &palette, &opts).unwrap();
        // vanilla maps can have keys without a tile; the edits must add none
        let baseline = render(&original, &tiles, &palette, &opts).unwrap();
        assert!(
            ours.missing.len() <= baseline.missing.len(),
            "{name}: {} tiles missing after the edits, {} before",
            ours.missing.len(),
            baseline.missing.len()
        );
        let mask = object_mask(&edited, ours.origin, ours.image.width, ours.image.height);
        let report = compare_images_with(
            &ours.image,
            &oracle,
            &CompareOptions {
                tolerance: 16,
                max_shift: 16,
                coverage: Coverage::Both,
                mask: Some(&mask),
            },
        );

        // the edits must be visible: the oracle's render of the original differs
        let vs_original = entry.png.as_ref().map(|png| {
            let original_png = RgbaImage::from_png(&std::fs::read(png).unwrap()).unwrap();
            if (original_png.width, original_png.height) == (oracle.width, oracle.height) {
                let total = oracle.width as usize * oracle.height as usize;
                let same = (0..total)
                    .filter(|&i| {
                        original_png.pixels.get(i * 4..i * 4 + 4)
                            == oracle.pixels.get(i * 4..i * 4 + 4)
                    })
                    .count();
                same as f64 / total as f64
            } else {
                0.0
            }
        });

        rows.push(Row {
            name: name.clone(),
            edits: applied.summary(),
            saved: true,
            size: format!("{}x{}", oracle.width, oracle.height),
            tol16: report.diff.within_tolerance,
            vs_original,
            shift: report.diff.best_shift,
            compared: report.compared,
        });
    }

    eprintln!(
        "{:<28} {:<34} {:>6} {:>10} {:>7} {:>9} {:>7} {:>9}",
        "map", "edits", "saved", "size", "tol16", "same-orig", "shift", "compared"
    );
    for r in &rows {
        eprintln!(
            "{:<28} {:<34} {:>6} {:>10} {:>7.4} {:>9} {:>3},{:<3} {:>9}",
            r.name,
            r.edits,
            if r.saved { "yes" } else { "NO" },
            r.size,
            r.tol16,
            r.vs_original.map_or("-".to_owned(), |v| format!("{v:.4}")),
            r.shift.0,
            r.shift.1,
            r.compared
        );
    }
    if only.is_none() {
        assert!(rows.len() >= 6.min(entries.len()), "too few maps ran");
    }
    for r in &rows {
        assert!(
            r.tol16 >= 0.95,
            "{}: within_tolerance(16) = {:.4} < 0.95 (edits: {})",
            r.name,
            r.tol16,
            r.edits
        );
        assert!(
            r.vs_original.is_none_or(|v| v < 1.0),
            "{}: the oracle rendered the edited file exactly like the original",
            r.name
        );
    }
}
