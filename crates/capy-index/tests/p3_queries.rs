//! P3 acceptance: about twenty queries over the real Median XL index, each
//! must return its expected asset (by key) in the top 5.
//!
//! The expected keys were picked from the data, not from the search results:
//! each row says which fact about the game files makes the asset the right
//! answer. Skipped (`skipped: <dir> missing`) when the install is absent;
//! `CAPY_MXL_DIR` overrides the default location. The Median XL index is shared with
//! `real_data.rs` (see `common`): the first run builds it (about 90 s in debug),
//! later runs reuse it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;

use std::collections::BTreeSet;
use std::time::Instant;

use capy_index::{Filter, UserDb, parse_query};
use d2_assets::{GamePath, Profile};
use d2_formats::ds1::Ds1;

const WHIMSY: &str = "data\\global\\tiles\\mxl\\zone\\whimsy\\";
const SHIRE_DS1: &str = "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.ds1";
const MYTAG_KEY: &str = "data\\global\\tiles\\mxl\\quest\\yshari\\trees.dt1#0/4/57/0";
const ALIAS_KEY: &str = "data\\global\\tiles\\mxl\\zone\\whimsy\\whimsyfloor.dt1#0/23/6/0";

/// A query, the key that must be in its top 5, and why it is the answer.
struct Row {
    query: String,
    expect: String,
    why: &'static str,
}

fn row(query: &str, expect: &str, why: &'static str) -> Row {
    Row {
        query: query.to_owned(),
        expect: expect.to_owned(),
        why,
    }
}

fn rows() -> Vec<Row> {
    let baal = "data\\global\\tiles\\mxl\\zone\\scosglen\\baalfloor.dt1";
    let trees = "data\\global\\tiles\\mxl\\quest\\yshari\\trees.dt1";
    let floor = format!("{WHIMSY}whimsyfloor.dt1");
    let clouds = format!("{WHIMSY}whimsyclouds.dt1");
    let magma = "data\\global\\tiles\\expansion\\magma\\walls.dt1";
    vec![
        // ---- by index
        row(
            "idx:0:5/2 file:baalfloor.dt1",
            &format!("{baal}#0/5/2/0"),
            "baalfloor.dt1 holds exactly one floor (orientation 0) tile 5/2",
        ),
        row(
            "idx:5/2 file:baalfloor.dt1",
            &format!("{baal}#0/5/2/0"),
            "without the orientation, the floor and the orientation-12 wall 5/2 both match",
        ),
        row(
            "type:wall file:magma\\walls.dt1 idx:30/2",
            &format!("{magma}#1/30/2/0"),
            "magma walls.dt1 has wall tiles 30/2 for orientations 1 to 4; orientation 1 sorts first",
        ),
        // ---- by generated name words
        row(
            "yshari trees floor 04/55",
            &format!("{trees}#0/4/55/0"),
            "the name is `Mxl Quest Yshari Trees . Floor . dark . 04/55`: every word hits it",
        ),
        row(
            "whimsyfloor green 23/05",
            &format!("{floor}#0/23/5/0"),
            "colour word and main/sub from the name; whimsyfloor 23/5 is a green floor",
        ),
        row(
            "whimsyflor 23/05",
            &format!("{floor}#0/23/5/0"),
            "a typo in the place word still matches fuzzily, and 23/05 pins the tile",
        ),
        row(
            "magma walls 30/02",
            &format!("{magma}#1/30/2/0"),
            "place words plus index; the four orientations tie and the first one in order wins",
        ),
        // ---- by colour
        row(
            "color:green type:floor file:whimsyfloor.dt1",
            &format!("{floor}#0/23/0/0"),
            "whimsyfloor.dt1 is a green meadow; its first floor tile 23/0 is green",
        ),
        row(
            "color:green type:floor idx:23/3 file:whimsyfloor.dt1",
            &format!("{floor}#0/23/3/0"),
            "colour filter combined with an index",
        ),
        // ---- by user tag and alias (set in a temp UserDb before searching)
        row(
            "tag:mytag",
            MYTAG_KEY,
            "the only asset the test tagged `mytag`",
        ),
        row(
            "tag:MYTAG trees",
            MYTAG_KEY,
            "tags are case-insensitive and combine with free text",
        ),
        row(
            "meadow mosaic",
            ALIAS_KEY,
            "the alias the test gave this tile is searched like a name",
        ),
        // ---- by source file
        row(
            "file:shire.ds1 type:ds1",
            SHIRE_DS1,
            "the only DS1 file named shire.ds1",
        ),
        row(
            "file:whimsyclouds.dt1 idx:7:25/9",
            &format!("{clouds}#7/25/9/0"),
            "a tile of whimsyclouds.dt1 by orientation 7, main 25, sub 9",
        ),
        // ---- used-in (the test checks the DS1's cells itself, see `oracle`)
        row(
            "used-in:whimsy\\shire.ds1 type:floor",
            &format!("{floor}#0/23/0/0"),
            "shire.ds1 has floor cells 23/0 (checked by parsing the DS1)",
        ),
        row(
            "used-in:whimsy\\shire.ds1 type:floor idx:23/3",
            &format!("{floor}#0/23/3/0"),
            "shire.ds1 has floor cells 23/3 (checked by parsing the DS1)",
        ),
        // ---- similar and mirrored
        row(
            &format!("similar:{baal}#0/5/2/0"),
            &format!("{baal}#0/5/2/0"),
            "the tile itself is always its own nearest neighbour",
        ),
        row(
            &format!("similar:{baal}#0/5/2/0"),
            "data\\global\\tiles\\mxl\\quest\\void\\wsfloor.dt1#0/7/22/0",
            "a brown floor of the Void quest tileset looks like the Baal floor",
        ),
        row(
            &format!("similar:{trees}#0/4/55/0"),
            "data\\global\\tiles\\mxl\\zone\\toraja\\trees.dt1#0/4/55/0",
            "the Toraja trees.dt1 carries a copy of the same tile",
        ),
        row(
            "mirrored:yes file:magma\\walls.dt1 idx:1:30/0",
            &format!("{magma}#1/30/0/0"),
            "the left wall 30/0 is the mirror of the right wall 30/0",
        ),
        row(
            "mirrored:yes file:magma\\walls.dt1 idx:2:30/0",
            &format!("{magma}#2/30/0/0"),
            "and the right wall is the mirror of the left",
        ),
        // ---- by source layer
        row(
            "src:mod sanctum yshari",
            "data\\global\\tiles\\mxl\\quest\\yshari\\stage6.ds1",
            "the Median XL map whose level is called `Yshari Sanctum`",
        ),
        row(
            "src:base type:ds1 barnstheme1",
            "data\\global\\tiles\\act1\\barracks\\barnstheme1.ds1",
            "a vanilla map that Median XL keeps in its base archives",
        ),
        // ---- DS1 plus a name word
        row(
            "type:ds1 specter hatred",
            "data\\global\\tiles\\mxl\\quest\\yshari\\stage1.ds1",
            "the level name `Specter of Hatred` is part of the DS1's name",
        ),
        row(
            "act:1 type:ds1 barracks bare",
            "data\\global\\tiles\\act1\\barracks\\bare.ds1",
            "act filter plus path words; bare.ds1 beats bare2.ds1 and bareforge.ds1",
        ),
        // ---- sprites
        row(
            "type:dcc aihdbhma11hs",
            "data\\global\\chars\\ai\\hd\\aihdbhma11hs.dcc",
            "a DCC found by its file name",
        ),
        row(
            "type:cof aia11hs",
            "data\\global\\chars\\ai\\cof\\aia11hs.cof",
            "a COF found by its file name",
        ),
    ]
}

/// Parse `shire.ds1` ourselves and check it really contains floor cells
/// 23/0 and 23/3, so the used-in rows do not rest on the index alone.
fn oracle(overlay: &d2_assets::Overlay) {
    let bytes = overlay
        .read_bytes(&GamePath::new(SHIRE_DS1))
        .unwrap()
        .expect("shire.ds1 is in Median XL");
    let ds1 = Ds1::parse(&bytes).unwrap();
    let has = |main: u8, sub: u8| {
        ds1.floors.iter().any(|f| {
            f.as_slice()
                .iter()
                .any(|c| !c.is_empty() && c.main_index() == main && c.sub_index() == sub)
        })
    };
    assert!(has(23, 0), "shire.ds1 should use floor 23/0");
    assert!(has(23, 3), "shire.ds1 should use floor 23/3");
}

/// The Median XL install, or `None` (test skipped) when it is absent.
fn mxl_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_MXL_DIR")
        .map_or_else(|| PathBuf::from("C:\\games\\median-xl"), PathBuf::from);
    if dir.is_dir() {
        Some(dir)
    } else {
        println!("skipped: {} missing", dir.display());
        None
    }
}

#[test]
fn p3_queries_over_median_xl() {
    let Some(dir) = mxl_dir() else {
        return;
    };
    let profile = Profile::median_xl(&dir);
    let user_dir = tempfile::tempdir().unwrap();
    let common::Shared { idx, overlay, .. } = common::shared_mxl(&profile);
    oracle(&overlay);

    let user = UserDb::open(&user_dir.path().join("user.db")).unwrap();
    user.add_tag("mxl", MYTAG_KEY, "mytag").unwrap();
    user.set_alias("mxl", ALIAS_KEY, Some("Meadow mosaic"))
        .unwrap();

    let mut failures = Vec::new();
    println!("{:<62} {:>4}  expected", "query", "rank");
    for r in rows() {
        assert!(
            idx.asset("mxl", &r.expect).unwrap().is_some(),
            "expected asset {} is not in the index ({})",
            r.expect,
            r.why
        );
        let hits = idx
            .search("mxl", &parse_query(&r.query).unwrap(), &user, 5)
            .unwrap();
        let rank = hits.iter().position(|h| h.asset.key == r.expect);
        println!(
            "{:<62} {:>4}  {}",
            r.query,
            rank.map_or("-".to_owned(), |p| (p + 1).to_string()),
            r.expect.rsplit('\\').next().unwrap_or(&r.expect)
        );
        if rank.is_none() {
            let got: Vec<&str> = hits.iter().map(|h| h.asset.key.as_str()).collect();
            failures.push(format!(
                "{:?} should return {} ({}), got {got:?}",
                r.query, r.expect, r.why
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    // The alias is shown on the hit.
    let alias_hit = idx
        .search("mxl", &parse_query("meadow mosaic").unwrap(), &user, 1)
        .unwrap();
    assert_eq!(alias_hit[0].alias.as_deref(), Some("Meadow mosaic"));

    // Fuzzy floor: `shire` is only a loose subsequence of the Yshari trees
    // names, so `shire floor` must not return them; the shire DS1 is still
    // found by `type:ds1 shire`.
    let shire_floor = idx
        .search("mxl", &parse_query("shire floor").unwrap(), &user, 100_000)
        .unwrap();
    println!("shire floor: {} hits", shire_floor.len());
    assert!(
        shire_floor
            .iter()
            .all(|h| !h.asset.path.contains("yshari\\trees.dt1")),
        "yshari trees floors came back for `shire floor`"
    );
    let shire_ds1 = idx
        .search("mxl", &parse_query("type:ds1 shire").unwrap(), &user, 5)
        .unwrap();
    assert!(
        shire_ds1.iter().any(|h| h.asset.key == SHIRE_DS1),
        "type:ds1 shire should find {SHIRE_DS1}"
    );

    // Ranking property: with several words, tiles whose name has all of them
    // come first.
    let both = idx
        .search("mxl", &parse_query("whimsy floor").unwrap(), &user, 5)
        .unwrap();
    assert_eq!(both.len(), 5);
    for h in &both {
        let name = h.asset.gen_name.to_lowercase();
        assert!(
            name.contains("whimsy") && name.contains("floor"),
            "{}",
            h.asset.gen_name
        );
    }
    // Used-in is symmetrical with the index: the tile lists the DS1.
    let tile = idx
        .asset("mxl", &format!("{WHIMSY}whimsyfloor.dt1#0/23/3/0"))
        .unwrap()
        .unwrap();
    assert!(
        idx.used_in(tile.id)
            .unwrap()
            .iter()
            .any(|(ds1, cells)| ds1 == SHIRE_DS1 && *cells > 0)
    );
}

/// `Filter::AnyFile` over every whimsy DT1 returns exactly the union of one
/// `file:` query per DT1 (the browser's "this map's tile set" scope).
#[test]
fn any_file_over_whimsy_dt1s() {
    let Some(dir) = mxl_dir() else {
        return;
    };
    let profile = Profile::median_xl(&dir);
    let user_dir = tempfile::tempdir().unwrap();
    let common::Shared { idx, .. } = common::shared_mxl(&profile);
    let user = UserDb::open(&user_dir.path().join("user.db")).unwrap();
    let all = 1_000_000;

    let dt1s: Vec<String> = idx
        .search(
            "mxl",
            &parse_query(&format!("type:dt1 file:{WHIMSY}")).unwrap(),
            &user,
            all,
        )
        .unwrap()
        .into_iter()
        .map(|h| h.asset.path)
        .collect();
    println!("whimsy DT1s: {dt1s:?}");
    assert!(dt1s.len() >= 2, "Median XL has several whimsy DT1s");
    assert!(dt1s.contains(&format!("{WHIMSY}whimsyfloor.dt1")));

    let started = Instant::now();
    let mut separate = BTreeSet::new();
    for dt1 in &dt1s {
        let q = parse_query(&format!("type:tile file:\"{dt1}\"")).unwrap();
        for h in idx.search("mxl", &q, &user, all).unwrap() {
            separate.insert(h.asset.key);
        }
    }
    let per_file = started.elapsed();

    let started = Instant::now();
    let mut q = parse_query("type:tile").unwrap();
    q.filters.push(Filter::AnyFile(dt1s.clone()));
    let hits = idx.search("mxl", &q, &user, all).unwrap();
    let one = started.elapsed();
    let combined: BTreeSet<String> = hits.iter().map(|h| h.asset.key.clone()).collect();
    println!(
        "{} tiles: {} file: queries in {per_file:?}, one AnyFile query in {one:?}",
        combined.len(),
        dt1s.len()
    );
    assert_eq!(combined.len(), hits.len(), "each tile once");
    assert!(!combined.is_empty());
    assert_eq!(combined, separate);

    // Combined with filters and free text, as the browser does.
    let mut q = parse_query("color:green type:floor").unwrap();
    q.filters.push(Filter::AnyFile(dt1s.clone()));
    let green = idx.search("mxl", &q, &user, all).unwrap();
    assert!(
        green
            .iter()
            .any(|h| h.asset.key == format!("{WHIMSY}whimsyfloor.dt1#0/23/0/0"))
    );
    assert!(green.iter().all(|h| h.asset.path.starts_with(WHIMSY)));
    let mut q = parse_query("whimsyfloor 23/05").unwrap();
    q.filters.push(Filter::AnyFile(dt1s));
    let top = idx.search("mxl", &q, &user, 5).unwrap();
    assert!(
        top.iter()
            .any(|h| h.asset.key == format!("{WHIMSY}whimsyfloor.dt1#0/23/5/0"))
    );
}
