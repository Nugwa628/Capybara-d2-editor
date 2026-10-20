//! "Add to this map" (tiles_register) over the private corpus: barset.dt1
//! tiles into a vanilla map and into Median XL's shire.ds1.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use capy_mod::level_tables::LevelTables;
use capy_mod::level_tables::{filled_slots, slot_key};
use capy_mod::tiles_register::{AddTilesInput, add_tiles_to_map};
use capy_mod::tiles_review::{TileId, TilePick, tile_ids};
use common::{TestResult, best, catalog_with, corpus, load_tables, read_game_file};
use d2_assets::tables::load_excel;
use d2_assets::{FolderSource, GameVersion, Overlay};
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::excel::ExcelTableKind;
use d2_formats::txt::TxtDoc;

fn type_row(types: &TxtDoc, id: u32) -> usize {
    let c = types.column("Id").unwrap();
    (0..types.row_count())
        .find(|&r| types.cell(r, c).unwrap_or_default().trim() == id.to_string())
        .unwrap()
}

fn changed_cells(a: &TxtDoc, b: &TxtDoc) -> Vec<(usize, usize)> {
    assert_eq!(a.row_count(), b.row_count());
    let mut out = Vec::new();
    for r in 0..a.row_count() {
        for c in 0..a.headers().len() {
            if a.cell(r, c) != b.cell(r, c) {
                out.push((r, c));
            }
        }
    }
    out
}

/// The tables a 1.13c game really reads: the compiled `.bin` forms, as text.
fn tables_from_bins(root: &std::path::Path) -> Result<LevelTables, Box<dyn std::error::Error>> {
    let overlay = Overlay::new(vec![Box::new(FolderSource::new(root, "corpus"))]);
    let doc = |kind| -> Result<TxtDoc, Box<dyn std::error::Error>> {
        let t = load_excel(&overlay, kind, GameVersion::V113c)?;
        let mut text = t.headers().join("\t");
        text.push_str("\r\n");
        for r in t.rows() {
            text.push_str(&r.cells().join("\t"));
            text.push_str("\r\n");
        }
        Ok(TxtDoc::parse(text.as_bytes()))
    };
    Ok(LevelTables {
        prest: doc(ExcelTableKind::LvlPrest)?,
        levels: doc(ExcelTableKind::Levels)?,
        types: doc(ExcelTableKind::LvlTypes)?,
        lvl_sub: None,
    })
}

fn run(
    profile: &str,
    map: &str,
    source: &str,
    only: Option<usize>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let Some(root) = corpus(profile) else {
        eprintln!("skipped: no {profile} corpus");
        return Ok(usize::MAX);
    };
    let t = if profile == "mxl" {
        tables_from_bins(&root)?
    } else {
        load_tables(&root)?
    };
    let base = catalog_with(&root, &t, &[], &[map])?;
    let info = best(&base, map).expect("map resolves");
    let type_id = info.lvl_type.unwrap();
    let ds1 = Ds1::parse(&read_game_file(&root, map)?).unwrap();
    let src = Dt1::parse(&read_game_file(&root, source)?).unwrap();
    let trow = type_row(&t.types, type_id);
    let mut type_dt1s = Vec::new();
    for i in filled_slots(&t.types, trow) {
        let rel = slot_key(&t.types, trow, i).unwrap();
        let path = format!(r"data\global\tiles\{}", rel.replace('/', "\\"));
        if let Ok(b) = read_game_file(&root, &path)
            && let Ok(d) = Dt1::parse(&b)
        {
            type_dt1s.push((rel, d));
        }
    }
    let n = only.map_or(src.tiles.len(), |n| n.min(src.tiles.len()));
    let picks: Vec<TilePick> = (0..n).map(|index| TilePick { source: 0, index }).collect();
    let sources = vec![(source.to_owned(), &src)];
    let td: Vec<(String, &Dt1)> = type_dt1s.iter().map(|(p, d)| (p.clone(), d)).collect();
    let out = add_tiles_to_map(&AddTilesInput {
        map_path: map,
        ds1: &ds1,
        types: &t.types,
        prest: &t.prest,
        type_id,
        prest_def: info.lvlprest_def,
        picks: &picks,
        sources: &sources,
        type_dt1s: &td,
        existing_own: None,
        existing_own_path: None,
    })?;
    assert!(out.added > 0);

    // only intended cells change: one File cell of the type, Dt1Mask of the map's row
    let tcells = changed_cells(&t.types, &out.types);
    assert_eq!(tcells.len(), 1, "LvlTypes: {tcells:?}");
    assert!(
        t.types.headers()[tcells[0].1]
            .to_ascii_lowercase()
            .starts_with("file")
    );
    assert_eq!(tcells[0].0, trow);
    let pcells = changed_cells(&t.prest, &out.prest);
    assert!(!pcells.is_empty(), "no Dt1Mask changed");
    for (_, c) in &pcells {
        assert_eq!(t.prest.headers()[*c], "Dt1Mask");
    }
    // DS1: only the file list grew
    assert_eq!(out.ds1.files.len(), ds1.files.len() + 1);

    // renumbering avoids every taken key
    let mut taken: BTreeSet<TileId> = BTreeSet::new();
    for (_, d) in &td {
        taken.extend(tile_ids(d));
    }
    let own_ids = tile_ids(&out.dt1);
    for to in out.placed.values() {
        assert!(own_ids.contains(to));
    }
    for (from, to) in &out.remap {
        assert!(!taken.contains(to), "{from:?} -> {to:?} is taken");
        assert_eq!(from.orientation, to.orientation);
    }
    let renumbered_to: BTreeSet<_> = out.remap.values().collect();
    assert_eq!(
        renumbered_to.len(),
        out.remap.len(),
        "two tiles share a number"
    );

    // the catalog now loads the own DT1 for the map
    let own_game = format!(r"data\global\tiles\{}", out.own_path.replace('/', "\\"));
    let bytes = out.dt1.to_bytes().unwrap();
    let cat = catalog_with(
        &root,
        &common_tables(&t, &out),
        &[(&own_game, bytes)],
        &[map],
    )?;
    let after = best(&cat, map).expect("map resolves after");
    assert!(
        after
            .dt1_files
            .iter()
            .any(|p| p.as_str().eq_ignore_ascii_case(&own_game)),
        "{:?} lacks {own_game}",
        after.dt1_files
    );
    assert!(after.missing_dt1.is_empty(), "{:?}", after.missing_dt1);
    assert_eq!(after.lvl_type, info.lvl_type);
    Ok(out.remap.len())
}

fn common_tables(
    t: &capy_mod::level_tables::LevelTables,
    out: &capy_mod::tiles_register::AddTilesOutcome,
) -> capy_mod::level_tables::LevelTables {
    let mut n = t.clone();
    n.types = out.types.clone();
    n.prest = out.prest.clone();
    n
}

#[test]
fn barset_tiles_into_a_vanilla_map() -> TestResult {
    run(
        "vanilla",
        r"data\global\tiles\act1\town\townn1.ds1",
        r"data\global\tiles\act1\barracks\barset.dt1",
        None,
    )?;
    Ok(())
}

#[test]
fn barset_tiles_into_median_xl_shire() -> TestResult {
    run(
        "mxl",
        r"data\global\tiles\mxl\zone\whimsy\shire.ds1",
        r"data\global\tiles\act1\barracks\barset.dt1",
        None,
    )?;
    Ok(())
}

#[test]
fn tiles_of_the_types_own_library_are_renumbered() -> TestResult {
    // every number of the town's floor.dt1 is taken by itself
    let n = run(
        "vanilla",
        r"data\global\tiles\act1\town\townn1.ds1",
        r"data\global\tiles\act1\town\floor.dt1",
        Some(6),
    )?;
    assert!(n > 0, "nothing was renumbered");
    Ok(())
}
