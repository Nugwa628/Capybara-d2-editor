//! P1 acceptance test: render the 18 vanilla oracle maps and compare them with
//! the DS1Edit 2.1.1 headless renders in
//! `testdata\private\oracle-renders\` (zoom 1:4, gamma 1.30, vanilla MPQs).
//!
//! Each map is rendered twice, once per [`VariantRule`]: the documented game
//! rule (highest rarity, first on ties) and DS1Edit's own choice, which
//! never compares the group head's rarity (`misc.c:156-187`). The oracle is
//! DS1Edit's output, so only the second can match it everywhere; the table
//! shows both and the assertions pin both.
//!
//! Object pixels are excluded with [`OBJECT_MASK`] (the oracle draws animated
//! object sprites, `rand()` frames included, which this crate does not), and
//! so is every pixel that is background in either image. The `wide16` column
//! repeats the tolerance-16 figure with the mask box doubled, to show that
//! what remains next to the masked areas is object overhang.
//!
//! Heavy: runs in release builds or with `CAPY_RENDER_TESTS=1`, and skips
//! quietly when the game install or the oracle renders are absent.
//!
//! Environment knobs for investigating mismatches:
//! * `CAPY_ORACLE_ONLY=<substring>`: only maps whose name contains it.
//!
//! Diff images land in `testdata\private\render-diffs\<rule>\`: `*_ours.png`,
//! `*_oracle.png` (shifted onto our canvas) and `*_diff.png` (a heatmap:
//! black = equal, red = difference, blue = masked, yellow = only ours,
//! magenta = only the oracle).
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use capy_render::{
    CompareOptions, Coverage, RenderOptions, RgbaImage, TileRef, TileSet, VariantRule, Visibility,
    compare_images_with, draw_list, render,
};
use common::*;
use d2_assets::Profile;

/// Box around every DS1 object that is excluded from the comparison, in 1:1
/// pixels relative to the object's anchor `((x - y) * 16, (x + y) * 8)`
/// (object coordinates are sub-tiles, 5 per tile): `(left, top, right,
/// bottom)`. Scaled by `1/zoom_div` for zoomed renders.
pub const OBJECT_MASK: (i32, i32, i32, i32) = (-96, -224, 96, 48);

const ZOOM: u32 = 4;
const GAMMA: f32 = 1.30;

struct Row {
    name: String,
    ours: (u32, u32),
    oracle: (u32, u32),
    shift: (i32, i32),
    tol0: f64,
    tol8: f64,
    tol16: f64,
    wide16: f64,
    mean_abs: f64,
    compared: u64,
    only_ours: u64,
    only_oracle: u64,
    missing: usize,
    /// Draw items whose tile differs from the other rule's pick.
    rule_diff: usize,
}

fn object_mask(
    ds1: &d2_formats::ds1::Ds1,
    origin: (i32, i32),
    width: u32,
    height: u32,
    scale: i32,
) -> Vec<bool> {
    let div = ZOOM as i32;
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

fn oracle_pixel(oracle: &RgbaImage, x: u32, y: u32, shift: (i32, i32)) -> Option<[u8; 4]> {
    let ox = u32::try_from(i64::from(x) + i64::from(shift.0)).ok()?;
    let oy = u32::try_from(i64::from(y) + i64::from(shift.1)).ok()?;
    oracle.get(ox, oy)
}

/// Heatmap: black = equal, towards red = larger difference; blue = masked,
/// yellow = only ours, magenta = only the oracle.
fn heatmap(ours: &RgbaImage, oracle: &RgbaImage, shift: (i32, i32), mask: &[bool]) -> RgbaImage {
    let mut out = RgbaImage::filled(ours.width, ours.height, [0, 0, 0, 255]);
    for y in 0..ours.height {
        for x in 0..ours.width {
            let a = ours.get(x, y).filter(|p| p[3] != 0);
            let b = oracle_pixel(oracle, x, y, shift).filter(|p| p[3] != 0);
            let masked = mask[y as usize * ours.width as usize + x as usize];
            let px = match (a, b) {
                (Some(a), Some(b)) => {
                    let d = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0);
                    let v = (u16::from(d) * 8).min(255) as u8;
                    if masked {
                        [0, 0, 80 + v / 2, 255]
                    } else {
                        [v, 0, 0, 255]
                    }
                }
                (Some(_), None) => [255, 255, 0, 255],
                (None, Some(_)) => [255, 0, 255, 255],
                (None, None) => [0, 0, 0, 255],
            };
            let i = (y as usize * ours.width as usize + x as usize) * 4;
            out.pixels[i..i + 4].copy_from_slice(&px);
        }
    }
    out
}

fn shifted_oracle(oracle: &RgbaImage, ours: &RgbaImage, shift: (i32, i32)) -> RgbaImage {
    let mut out = RgbaImage::filled(ours.width, ours.height, [0, 0, 0, 0]);
    for y in 0..ours.height {
        for x in 0..ours.width {
            if let Some(p) = oracle_pixel(oracle, x, y, shift) {
                let i = (y as usize * ours.width as usize + x as usize) * 4;
                out.pixels[i..i + 4].copy_from_slice(&p);
            }
        }
    }
    out
}

fn rule_dir(rule: VariantRule) -> &'static str {
    match rule {
        VariantRule::Documented => "documented",
        VariantRule::ReferenceQuirk => "reference-quirk",
    }
}

fn print_table(rule: VariantRule, rows: &[Row]) {
    eprintln!("variant rule: {rule:?}, zoom 1:{ZOOM}, gamma {GAMMA}");
    eprintln!(
        "{:<28} {:>11} {:>11} {:>5} {:>9} {:>7} {:>7} {:>7} {:>7} {:>8} {:>9} {:>8} {:>8} {:>5} {:>8}",
        "map",
        "ours",
        "oracle",
        "size",
        "shift",
        "tol0",
        "tol8",
        "tol16",
        "wide16",
        "mean",
        "compared",
        "onlyOurs",
        "onlyOrc",
        "miss",
        "ruleDiff"
    );
    for r in rows {
        eprintln!(
            "{:<28} {:>5}x{:<5} {:>5}x{:<5} {:>5} {:>4},{:<4} {:>7.4} {:>7.4} {:>7.4} {:>7.4} {:>8.3} {:>9} {:>8} {:>8} {:>5} {:>8}",
            r.name,
            r.ours.0,
            r.ours.1,
            r.oracle.0,
            r.oracle.1,
            if r.ours == r.oracle { "ok" } else { "DIFF" },
            r.shift.0,
            r.shift.1,
            r.tol0,
            r.tol8,
            r.tol16,
            r.wide16,
            r.mean_abs,
            r.compared,
            r.only_ours,
            r.only_oracle,
            r.missing,
            r.rule_diff,
        );
    }
    let passing = rows.iter().filter(|r| r.tol16 >= 0.95).count();
    eprintln!(
        "{passing} of {} maps reach within_tolerance(16) >= 0.95 ({rule:?})\n",
        rows.len()
    );
}

struct MapEntry {
    name: String,
    ds1: String,
    lvltypes_id: u32,
    lvlprest_def: u32,
    png: PathBuf,
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
                png: renders.join(format!("{name}.png")),
                ds1: e["ds1"].as_str()?.to_owned(),
                lvltypes_id: u32::try_from(e["lvltypes_id"].as_u64()?).ok()?,
                lvlprest_def: u32::try_from(e["lvlprest_def"].as_u64()?).ok()?,
                name,
            })
        })
        .collect()
}

fn draw_tiles(ds1: &d2_formats::ds1::Ds1, tiles: &TileSet) -> Vec<TileRef> {
    let (items, _) = draw_list(ds1, tiles, &Visibility::default()).unwrap();
    items.into_iter().map(|i| i.tile).collect()
}

fn run(
    rule: VariantRule,
    entries: &[MapEntry],
    game: &Game,
    tables: &Tables,
    only: Option<&str>,
    diffs: &Path,
) -> Vec<Row> {
    let out_dir = diffs.join(rule_dir(rule));
    std::fs::create_dir_all(&out_dir).unwrap();
    let other_rule = match rule {
        VariantRule::Documented => VariantRule::ReferenceQuirk,
        VariantRule::ReferenceQuirk => VariantRule::Documented,
    };
    let mut rows = Vec::new();
    for entry in entries {
        let name = &entry.name;
        if only.is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let ds1 = load_ds1(game, &entry.ds1);
        let ctx = resolve_explicit(
            tables,
            ds1.effective_act(),
            entry.lvltypes_id,
            entry.lvlprest_def,
            None,
        );
        let (tiles, warnings) = TileSet::load(&game.overlay, &ctx.dt1_files).unwrap();
        for w in &warnings {
            eprintln!("{name}: dt1 warning: {w}");
        }
        let (other, _) = TileSet::load(&game.overlay, &ctx.dt1_files).unwrap();
        let tiles = tiles.with_variant_rule(rule);
        let other = other.with_variant_rule(other_rule);
        let rule_diff = {
            let a = draw_tiles(&ds1, &tiles);
            let b = draw_tiles(&ds1, &other);
            assert_eq!(a.len(), b.len(), "{name}: rules changed the item count");
            a.iter().zip(&b).filter(|(x, y)| x != y).count()
        };
        let palette = load_palette(game, ctx.palette_act);
        let opts = RenderOptions {
            zoom_div: ZOOM,
            gamma: GAMMA,
            ..RenderOptions::default()
        };
        let out = render(&ds1, &tiles, &palette, &opts).unwrap();
        for w in &out.warnings {
            eprintln!("{name}: render warning: {w}");
        }
        if !out.missing.is_empty() {
            let mut counts: std::collections::BTreeMap<(char, u8, u8, u8), usize> =
                std::collections::BTreeMap::new();
            for m in &out.missing {
                *counts
                    .entry((m.layer.0, m.key.orientation, m.key.main, m.key.sub))
                    .or_default() += 1;
            }
            let mut v: Vec<_> = counts.into_iter().collect();
            v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            let top: Vec<String> = v
                .iter()
                .take(6)
                .map(|((l, o, m, s), n)| format!("{l}:o{o}/m{m}/s{s}x{n}"))
                .collect();
            eprintln!("{name}: missing tiles: {}", top.join(" "));
        }
        let oracle = RgbaImage::from_png(&std::fs::read(&entry.png).unwrap()).unwrap();
        let mask = object_mask(&ds1, out.origin, out.image.width, out.image.height, 1);
        let wide_mask = object_mask(&ds1, out.origin, out.image.width, out.image.height, 2);
        let cmp = |tolerance, mask: &[bool]| {
            compare_images_with(
                &out.image,
                &oracle,
                &CompareOptions {
                    tolerance,
                    max_shift: 16,
                    coverage: Coverage::Both,
                    mask: Some(mask),
                },
            )
        };
        let r16 = cmp(16, &mask);
        let shift = r16.diff.best_shift;
        std::fs::write(
            out_dir.join(format!("{name}_ours.png")),
            out.image.to_png().unwrap(),
        )
        .unwrap();
        std::fs::write(
            out_dir.join(format!("{name}_oracle.png")),
            shifted_oracle(&oracle, &out.image, shift).to_png().unwrap(),
        )
        .unwrap();
        std::fs::write(
            out_dir.join(format!("{name}_diff.png")),
            heatmap(&out.image, &oracle, shift, &mask).to_png().unwrap(),
        )
        .unwrap();
        rows.push(Row {
            name: name.clone(),
            ours: (out.image.width, out.image.height),
            oracle: (oracle.width, oracle.height),
            shift,
            tol0: cmp(0, &mask).diff.within_tolerance,
            tol8: cmp(8, &mask).diff.within_tolerance,
            tol16: r16.diff.within_tolerance,
            wide16: cmp(16, &wide_mask).diff.within_tolerance,
            mean_abs: r16.diff.mean_abs,
            compared: r16.compared,
            only_ours: r16.only_ours,
            only_oracle: r16.only_oracle,
            missing: out.missing.len(),
            rule_diff,
        });
    }
    rows
}

#[test]
fn vanilla_maps_match_the_oracle() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_RENDER_TESTS=1 or use --release)");
        return;
    }
    let renders = private_dir().join("oracle-renders");
    let (Some(vanilla), Some(entries)) = (vanilla_dir(), read_manifest(&renders)) else {
        eprintln!("skipped: vanilla install or oracle renders not found");
        return;
    };
    let only = std::env::var("CAPY_ORACLE_ONLY").ok();
    let diffs = private_dir().join("render-diffs");
    let game = open_profile(Profile::vanilla(&vanilla));
    let tables = load_tables(&game);

    let documented = run(
        VariantRule::Documented,
        &entries,
        &game,
        &tables,
        only.as_deref(),
        &diffs,
    );
    let quirk = run(
        VariantRule::ReferenceQuirk,
        &entries,
        &game,
        &tables,
        only.as_deref(),
        &diffs,
    );
    print_table(VariantRule::Documented, &documented);
    print_table(VariantRule::ReferenceQuirk, &quirk);

    if only.is_none() {
        // Geometry, palette, pass order and shadow blending do not depend on
        // the variant rule: sizes and shifts must match under both.
        for r in documented.iter().chain(&quirk) {
            assert_eq!(r.ours, r.oracle, "{}: canvas size", r.name);
            assert_eq!(r.shift, (0, 0), "{}: shift", r.name);
        }
        // P1 acceptance (>= 5 maps) holds with the documented rule...
        let passing = documented.iter().filter(|r| r.tol16 >= 0.95).count();
        assert!(
            passing >= 5,
            "P1 acceptance: only {passing} maps reach 0.95 at tolerance 16 (documented rule)"
        );
        // ...and DS1Edit's own variant choice reproduces every map.
        for r in &quirk {
            assert!(
                r.tol16 >= 0.95,
                "{}: tolerance-16 fraction {:.4} (reference variant rule)",
                r.name,
                r.tol16
            );
        }
        // Sprites aside, nothing else may differ: with the object mask
        // doubled the match is complete.
        for r in &quirk {
            assert!(r.wide16 >= 0.999, "{}: wide16 {:.4}", r.name, r.wide16);
        }
    }
}
