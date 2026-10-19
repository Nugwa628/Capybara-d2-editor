//! P1 acceptance prep: render Median XL maps (town, zone, quest) at 1:4 and
//! require that no tile is missing. The reference editor cannot render these
//! (it reads `.txt` tables only), so the pictures are for a human to check:
//! they land in `testdata\private\mxl-renders\<name>.png`.
//!
//! Each map's LvlTypes id and LvlPrest def come from Median XL's own tables
//! (`.bin`, 1.13c layout): the LvlPrest row whose `File` entries name the DS1,
//! its `LevelId`, and `Levels[LevelId].LevelType`.
//!
//! The palette is `Levels.Pal` (what the game uses). LvlTypes' act, which
//! DS1Edit uses, is wrong for some Median XL levels.
//!
//! Heavy: runs in release builds or with `CAPY_RENDER_TESTS=1`; skips when
//! the Median XL install is absent.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod common;

use capy_render::{
    ObjectOptions, ObjectSprites, RenderOptions, TileSet, render, render_with_objects,
};
use common::*;
use d2_assets::objects::ObjectCatalog;
use d2_assets::tables::load_excel;
use d2_assets::{GamePath, GameVersion, Profile};
use d2_formats::excel::ExcelTableKind;
use d2_formats::tables::levels;

/// Maps rendered, relative to `data\global\tiles\`: towns, zones and quests.
const MAPS: &[&str] = &[
    "mxl/zone/scosglen/town.ds1",
    "mxl/quest/uldy/tri_town4.ds1",
    "mxl/zone/teganze/freebird.ds1",
    "mxl/zone/heaven/charismatic_base.ds1",
    "mxl/zone/dunc/bonehexerX.ds1",
    "mxl/quest/talrasha/talrasha.ds1",
    "mxl/quest/kab/kabraxis2.ds1",
    "mxl/quest/ureh/downtown.ds1",
];

fn norm(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

#[test]
fn median_xl_maps_render_without_missing_tiles() {
    if !heavy_enabled() {
        eprintln!("skipped: debug build (set CAPY_RENDER_TESTS=1 or use --release)");
        return;
    }
    let Some(mxl) = mxl_dir() else {
        eprintln!("skipped: Median XL install not found");
        return;
    };
    let out_dir = private_dir().join("mxl-renders");
    std::fs::create_dir_all(&out_dir).unwrap();
    let object_dir = out_dir.join("objects");
    std::fs::create_dir_all(&object_dir).unwrap();

    let game = open_profile(Profile::median_xl(&mxl));
    assert_eq!(game.version, GameVersion::V113c);
    let tables = load_tables(&game);
    let (catalog, catalog_warnings) = ObjectCatalog::build(&game.overlay, game.version).unwrap();
    for w in &catalog_warnings {
        eprintln!("object catalog warning: {w}");
    }
    let sprites = ObjectSprites::new(&game.overlay, &catalog);
    let mut object_totals = (0usize, 0usize);
    let lv =
        levels(&load_excel(&game.overlay, ExcelTableKind::Levels, game.version).unwrap()).unwrap();

    eprintln!(
        "{:<34} {:>8} {:>6} {:>5} {:>11} {:>8} {:>5}",
        "map", "lvltype", "def", "dt1s", "size", "items", "miss"
    );
    let mut failures = Vec::new();
    let mut rendered = 0;
    for rel in MAPS {
        let Some(prest) = tables
            .lvl_prests
            .iter()
            .find(|p| p.files.iter().flatten().any(|f| norm(f) == norm(rel)))
        else {
            failures.push(format!("{rel}: no LvlPrest row names it"));
            continue;
        };
        let Some(level) = lv
            .iter()
            .find(|l| i64::from(l.id) == i64::from(prest.level_id))
        else {
            failures.push(format!("{rel}: LevelId {} not in Levels", prest.level_id));
            continue;
        };
        let ds1_path = GamePath::from_tile_ref(rel).unwrap();
        let ds1 = load_ds1(&game, ds1_path.as_str());
        let ctx = resolve_explicit(
            &tables,
            ds1.effective_act(),
            level.level_type,
            prest.def,
            Some(level.pal),
        );
        let (tiles, warnings) = TileSet::load(&game.overlay, &ctx.dt1_files).unwrap();
        if !warnings.is_empty() {
            failures.push(format!("{rel}: DT1 warnings {warnings:?}"));
        }
        let palette = load_palette(&game, ctx.palette_act);
        let opts = RenderOptions {
            zoom_div: 4,
            gamma: 1.30,
            ..RenderOptions::default()
        };
        let out = render(&ds1, &tiles, &palette, &opts).unwrap();
        let (items, _) =
            capy_render::draw_list(&ds1, &tiles, &capy_render::Visibility::default()).unwrap();
        let name = norm(rel).trim_end_matches(".ds1").replace('/', "_");
        std::fs::write(
            out_dir.join(format!("{name}.png")),
            out.image.to_png().unwrap(),
        )
        .unwrap();
        eprintln!(
            "{:<34} {:>8} {:>6} {:>5} {:>5}x{:<5} {:>8} {:>5}",
            name,
            level.level_type,
            prest.def,
            ctx.dt1_files.len(),
            out.image.width,
            out.image.height,
            items.len(),
            out.missing.len()
        );
        if !out.missing.is_empty() {
            let mut counts: std::collections::BTreeMap<(char, u8, u8, u8), usize> =
                std::collections::BTreeMap::new();
            for m in &out.missing {
                *counts
                    .entry((m.layer.0, m.key.orientation, m.key.main, m.key.sub))
                    .or_default() += 1;
            }
            let top: Vec<String> = counts
                .iter()
                .take(8)
                .map(|((l, o, m, s), n)| format!("{l}:o{o}/m{m}/s{s}x{n}"))
                .collect();
            failures.push(format!(
                "{rel}: {} missing tiles: {}",
                out.missing.len(),
                top.join(" ")
            ));
        }
        for w in &out.warnings {
            failures.push(format!("{rel}: render warning {w}"));
        }
        // Objects and monsters. Median XL's monster tables may be only
        // partly decodable, so unresolved objects are reported, not failed.
        let with_objects =
            render_with_objects(&ds1, &tiles, Some(&sprites), &palette, &opts).unwrap();
        std::fs::write(
            object_dir.join(format!("{name}.png")),
            with_objects.image.to_png().unwrap(),
        )
        .unwrap();
        let (placed, missing) = sprites.place(&ds1, &ObjectOptions::default());
        object_totals.0 += placed.len();
        object_totals.1 += missing.len();
        eprintln!(
            "  objects: {} in the DS1, {} drawn, {} missing",
            ds1.objects.len(),
            placed.len(),
            missing.len()
        );
        let mut reasons: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for m in &missing {
            let why = match &m.reason {
                capy_render::MissingReason::CofNotFound(p) => format!("cof not found {p}"),
                other => other.to_string(),
            };
            *reasons
                .entry(format!("t{}/id{} {why}", m.kind, m.id))
                .or_default() += 1;
        }
        let mut reasons: Vec<_> = reasons.into_iter().collect();
        reasons.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (why, n) in reasons.iter().take(8) {
            eprintln!("    missing x{n}: {why}");
        }
        for w in &with_objects.warnings {
            failures.push(format!("{rel}: object render warning {w}"));
        }
        rendered += 1;
    }
    eprintln!(
        "objects over all maps: {} drawn, {} missing",
        object_totals.0, object_totals.1
    );
    assert!(object_totals.0 > 0, "no object drew on any Median XL map");
    assert!(rendered >= 5, "only {rendered} maps rendered: {failures:?}");
    assert!(failures.is_empty(), "{failures:#?}");
}
