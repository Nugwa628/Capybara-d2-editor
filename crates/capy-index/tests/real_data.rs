//! Index builds over the real installs (read-only). Each test prints
//! `skipped: <dir> missing` and passes when its install is absent.
//!
//! `CAPY_VANILLA_DIR` / `CAPY_MXL_DIR` override the default locations. The
//! index, user db and saves layer live in temp dirs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use capy_index::{AssetKind, BuildPhase, Index, IndexStatus, UserDb, parse_query};
use d2_assets::Profile;

fn dir(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

struct Built {
    idx: Index,
    user: UserDb,
    /// `None` when the index was reused from an earlier run.
    report: Option<capy_index::BuildReport>,
    overlay: d2_assets::Overlay,
    _dirs: Vec<tempfile::TempDir>,
}

fn build(profile: &Profile) -> Built {
    let saves = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let overlay = profile.open(saves.path()).unwrap();
    let mut idx = Index::open(cache.path()).unwrap();
    assert_eq!(
        idx.status(&profile.id, &overlay).unwrap(),
        IndexStatus::Missing
    );
    let started = Instant::now();
    let mut last_phase = None;
    let mut phase_start = Instant::now();
    let report = idx
        .build(
            profile,
            &overlay,
            &mut |p| {
                if last_phase != Some(p.phase) {
                    if let Some(prev) = last_phase {
                        println!("  {prev:?} took {:?}", phase_start.elapsed());
                    }
                    last_phase = Some(p.phase);
                    phase_start = Instant::now();
                }
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    println!(
        "{}: built in {:?}: {} files, {} assets ({} tiles), {} warnings",
        profile.id,
        started.elapsed(),
        report.files,
        report.assets,
        report.tiles,
        report.warnings.len()
    );
    for w in report.warnings.iter().take(12) {
        println!("  warning: {w}");
    }
    assert_eq!(
        idx.status(&profile.id, &overlay).unwrap(),
        IndexStatus::UpToDate
    );
    let user = UserDb::open(&user_dir.path().join("user.db")).unwrap();
    Built {
        idx,
        user,
        report: Some(report),
        overlay,
        _dirs: vec![saves, cache, user_dir],
    }
}

/// The shared Median XL index (see `common`), with a throw-away user db.
fn shared_mxl(profile: &Profile) -> Built {
    let shared = common::shared_mxl(profile);
    let user_dir = tempfile::tempdir().unwrap();
    let user = UserDb::open(&user_dir.path().join("user.db")).unwrap();
    Built {
        idx: shared.idx,
        user,
        report: shared.report,
        overlay: shared.overlay,
        _dirs: vec![user_dir],
    }
}

fn search(b: &Built, profile: &str, q: &str, limit: usize) -> Vec<capy_index::Hit> {
    let started = Instant::now();
    let hits = b
        .idx
        .search(profile, &parse_query(q).unwrap(), &b.user, limit)
        .unwrap();
    println!(
        "  search {q:?}: {} hits in {:?}{}",
        hits.len(),
        started.elapsed(),
        hits.first()
            .map(|h| format!(", first {}", h.asset.gen_name))
            .unwrap_or_default()
    );
    hits
}

#[test]
fn median_xl_index() {
    let mxl = dir("CAPY_MXL_DIR", "C:\\games\\median-xl");
    if !mxl.is_dir() {
        println!("skipped: {} missing", mxl.display());
        return;
    }
    let b = shared_mxl(&Profile::median_xl(&mxl));
    let ds1 = b.idx.count("mxl", Some(AssetKind::Ds1)).unwrap();
    let dt1 = b.idx.count("mxl", Some(AssetKind::Dt1)).unwrap();
    let tiles = b.idx.count("mxl", Some(AssetKind::Tile)).unwrap();
    println!(
        "mxl: ds1 {ds1}, dt1 {dt1}, tiles {tiles}, dc6 {}, dcc {}, cof {}",
        b.idx.count("mxl", Some(AssetKind::Dc6)).unwrap(),
        b.idx.count("mxl", Some(AssetKind::Dcc)).unwrap(),
        b.idx.count("mxl", Some(AssetKind::Cof)).unwrap(),
    );
    assert!(ds1 >= 574, "ds1 files: {ds1}");
    assert!(dt1 >= 129, "dt1 files: {dt1}");
    assert!(tiles >= 5_000, "tiles: {tiles}");
    if let Some(report) = &b.report {
        assert_eq!(report.tiles, tiles);
    }

    let shire = search(&b, "mxl", "file:shire", 50);
    assert!(!shire.is_empty());
    assert!(shire.iter().all(|h| h.asset.path.contains("shire")));
    let act1 = search(&b, "mxl", "type:ds1 act:1", 5000);
    assert!(!act1.is_empty());
    assert!(
        act1.iter()
            .all(|h| h.asset.kind == AssetKind::Ds1 && h.asset.act == Some(0))
    );
    let modded = search(&b, "mxl", "src:mod", 100_000);
    assert!(modded.len() > 574, "mod assets: {}", modded.len());
    assert!(modded.iter().all(|h| h.asset.src_tag == "mod"));
    let base = search(&b, "mxl", "src:base type:ds1", 100_000);
    assert!(!base.is_empty());
    let floors = search(&b, "mxl", "type:floor color:green", 20);
    assert!(
        floors
            .iter()
            .all(|h| h.asset.class.as_deref() == Some("Floor"))
    );
    search(&b, "mxl", "grass", 10);
    search(&b, "mxl", "shire floor", 10);
    search(&b, "mxl", "type:tile idx:5/2 act:1", 10);
    search(&b, "mxl", "mirrored:yes", 10);
    // Distribution of the generated names, to eyeball the heuristics.
    let mut seen = std::collections::BTreeMap::new();
    for h in search(&b, "mxl", "type:tile", 100_000) {
        let name = &h.asset.gen_name;
        let descriptor = name.split(" · ").nth(2).unwrap_or("?").to_owned();
        *seen.entry(descriptor).or_insert(0u32) += 1;
    }
    println!("  descriptors: {seen:?}");
    let mut colors = std::collections::BTreeMap::new();
    let mut textures = std::collections::BTreeMap::new();
    for h in search(&b, "mxl", "type:tile", 100_000) {
        *colors
            .entry(h.asset.color.clone().unwrap_or_default())
            .or_insert(0u32) += 1;
        *textures
            .entry(h.asset.texture.clone().unwrap_or_default())
            .or_insert(0u32) += 1;
    }
    println!("  colours: {colors:?}");
    println!("  textures: {textures:?}");
    for kind in ["dc6", "dcc", "cof"] {
        let all = search(&b, "mxl", &format!("type:{kind}"), 1_000_000);
        let sized = all.iter().filter(|h| h.asset.width.is_some()).count();
        let counted = all.iter().filter(|h| h.asset.directions.is_some()).count();
        let thumbs = all
            .iter()
            .filter(|h| b.idx.thumbnail(h.asset.id).unwrap().is_some())
            .count();
        println!(
            "  {kind}: {} assets, {sized} with a size, {counted} with counts, {thumbs} with a thumbnail",
            all.len()
        );
        assert!(counted * 100 >= all.len() * 95, "{kind}: counts missing");
        if kind != "cof" {
            assert!(thumbs * 100 >= all.len() * 90, "{kind}: thumbnails missing");
        }
    }
    println!(
        "  mirrored tiles: {}",
        search(&b, "mxl", "mirrored:yes", 100_000).len()
    );

    // A tile has a thumbnail, used-in links when some map uses it, and is
    // found again by key.
    let tile = search(&b, "mxl", "type:tile", 1).pop().unwrap().asset;
    let thumb = b.idx.thumbnail(tile.id).unwrap().unwrap();
    assert_eq!(&thumb[..4], b"\x89PNG");
    assert_eq!(b.idx.asset("mxl", &tile.key).unwrap().unwrap(), tile);
    let similar = search(&b, "mxl", &format!("similar:{}", tile.key), 10);
    assert_eq!(
        similar[0].asset.key, tile.key,
        "the tile itself comes first"
    );

    // Second launch: a new Index on the same cache dir is up to date.
    let cache = b.idx.db_path().parent().unwrap().to_path_buf();
    drop(b.idx);
    let again = Index::open(&cache).unwrap();
    assert_eq!(
        again.status("mxl", &b.overlay).unwrap(),
        IndexStatus::UpToDate
    );
    assert_eq!(again.count("mxl", Some(AssetKind::Tile)).unwrap(), tiles);
    let _ = BuildPhase::Listing;
}

#[test]
#[ignore = "about 30 s in debug; run with --ignored"]
fn vanilla_index() {
    let vanilla = dir("CAPY_VANILLA_DIR", "C:\\Program Files (x86)\\Diablo II");
    if !vanilla.is_dir() {
        println!("skipped: {} missing", vanilla.display());
        return;
    }
    let b = build(&Profile::vanilla(&vanilla));
    let ds1 = b.idx.count("vanilla", Some(AssetKind::Ds1)).unwrap();
    let dt1 = b.idx.count("vanilla", Some(AssetKind::Dt1)).unwrap();
    let tiles = b.idx.count("vanilla", Some(AssetKind::Tile)).unwrap();
    println!("vanilla: ds1 {ds1}, dt1 {dt1}, tiles {tiles}");
    assert!(ds1 >= 1_000, "ds1 files: {ds1}");
    assert!(dt1 >= 150, "dt1 files: {dt1}");
    assert!(tiles >= 5_000, "tiles: {tiles}");
    let old: Vec<&String> = b
        .report
        .iter()
        .flat_map(|r| &r.warnings)
        .filter(|w| w.contains("unsupported DT1 version"))
        .collect();
    println!("old-format DT1 warnings: {old:?}");
    assert!(
        old.len() >= 6,
        "the six version-4 act 1 DT1s are warnings: {old:?}"
    );
    // Vanilla LvlTypes carry names, so tiles use them as the place.
    let town = search(&b, "vanilla", "file:act1\\town\\floor.dt1 type:tile", 3);
    assert!(!town.is_empty());
    println!("  town floor tile: {}", town[0].asset.gen_name);
}
