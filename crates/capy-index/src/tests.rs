//! Index tests over a synthetic profile (loose folders in a temp dir) with a
//! fake tile source standing in for DT1 decoding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use d2_assets::{AssetSource, FolderSource, GamePath, GameVersion, Overlay, Profile};
use d2_formats::ds1::{Cell, Ds1};

use crate::error::Result;
use crate::index::{BuildPhase, Index, IndexStatus};
use crate::query::{AssetKind, Filter, parse_query};
use crate::tiles::{
    Progress, Sink, TileBatch, TileCtx, TileKeys, TileOrigin, UsedInRow, record_from_rgba,
    resolve_used_in,
};
use crate::user::UserDb;

mod robustness;

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = GamePath::new(rel).to_native(root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

const LVLTYPES: &str = "Name\tId\tAct\tFile 1\tFile 2\nAct1 Town\t1\t1\tact1\\town\\floor.dt1\t0\n";
const LVLPREST: &str = "Def\tLevelId\tDt1Mask\tFile1\n1\t1\t1\tact1\\town\\towne1.ds1\n";
const LEVELS: &str = "Id\tLevelName\tAct\tLevelType\tDrlgType\n1\tTown\t0\t1\t2\n";

fn ds1_bytes(cells: &[(u32, u32, u8, u8)], walls: &[(u32, u32, u8, u8, u32)]) -> Vec<u8> {
    let mut d = Ds1::new(8, 8, 0);
    for (x, y, main, sub) in cells {
        d.floors[0].set(*x, *y, Cell(1).with_indices(*main, *sub));
    }
    for (x, y, main, sub, orientation) in walls {
        d.walls[0]
            .tiles
            .set(*x, *y, Cell(1).with_indices(*main, *sub));
        d.walls[0].orientations.set(*x, *y, *orientation);
    }
    d.to_bytes().unwrap()
}

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    id: String,
}

impl Fx {
    fn new(id: &str) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let base = root.join(id).join("base");
        let modl = root.join(id).join("mod");
        write(
            &base,
            "data\\global\\excel\\lvltypes.txt",
            LVLTYPES.as_bytes(),
        );
        write(
            &base,
            "data\\global\\excel\\lvlprest.txt",
            LVLPREST.as_bytes(),
        );
        write(&base, "data\\global\\excel\\levels.txt", LEVELS.as_bytes());
        write(
            &base,
            "data\\global\\tiles\\act1\\town\\floor.dt1",
            b"not really a dt1",
        );
        write(
            &base,
            "data\\global\\tiles\\act1\\town\\towne1.ds1",
            &ds1_bytes(
                &[(1, 1, 5, 2), (2, 1, 5, 2), (3, 1, 5, 2), (4, 4, 9, 0)],
                &[(2, 2, 3, 0, 1)],
            ),
        );
        write(
            &modl,
            "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1",
            b"also not a dt1",
        );
        write(
            &modl,
            "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.ds1",
            &ds1_bytes(&[(0, 0, 5, 2), (1, 0, 5, 2)], &[]),
        );
        Fx {
            _dir: dir,
            root,
            id: id.to_owned(),
        }
    }

    fn profile(&self) -> Profile {
        Profile {
            id: self.id.clone(),
            name: self.id.clone(),
            game_version: GameVersion::V114d,
            layers: Vec::new(),
        }
    }

    fn overlay(&self) -> Overlay {
        let layers: Vec<Box<dyn AssetSource>> = vec![
            Box::new(FolderSource::new(
                &self.root.join(&self.id).join("saves"),
                format!("saves\\{}", self.id),
            )),
            Box::new(FolderSource::new(
                &self.root.join(&self.id).join("mod"),
                "mod-pack.mpq",
            )),
            Box::new(FolderSource::new(
                &self.root.join(&self.id).join("base"),
                "d2data.mpq",
            )),
        ];
        Overlay::new(layers)
    }

    fn base(&self) -> PathBuf {
        self.root.join(&self.id).join("base")
    }
}

// ---- fake tiles ----

fn noise(w: u32, h: u32, base: [u8; 3], amp: i32, seed: u32) -> Vec<u8> {
    let mut s = seed;
    let mut out = Vec::new();
    for _ in 0..w * h {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = ((s >> 24) as i32 % (amp * 2 + 1)) - amp;
        for c in base {
            out.push((i32::from(c) + n).clamp(0, 255) as u8);
        }
        out.push(255);
    }
    out
}

fn gradient(w: u32, h: u32, flip: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let xx = if flip { w - 1 - x } else { x };
            let v = (xx * 255 / (w - 1)) as u8;
            out.extend([v, (u32::from(v) * (y + 1) / h) as u8, 255 - v, 255]);
        }
    }
    out
}

fn fake_run(ctx: &TileCtx<'_>, progress: Progress<'_>, sink: Sink<'_>) -> Result<Vec<UsedInRow>> {
    let mut by_dt1: TileKeys = HashMap::new();
    let total = ctx.dt1s.len() as u64;
    for (i, input) in ctx.dt1s.iter().enumerate() {
        progress(i as u64, total, input.path.as_str())?;
        let name = input.path.as_str();
        // (orientation, main, sub, pixels)
        let specs: Vec<(u8, u8, u8, Vec<u8>)> = vec![
            (0, 5, 2, noise(64, 32, [50, 150, 50], 14, 1)),
            (0, 5, 2, noise(64, 32, [50, 150, 50], 14, 2)),
            (13, 0, 0, noise(64, 32, [5, 5, 5], 3, 3)),
            (1, 3, 0, gradient(48, 64, false)),
            (2, 3, 0, gradient(48, 64, true)),
        ];
        let mut variants: HashMap<(u8, u8, u8), u32> = HashMap::new();
        let mut batch = TileBatch {
            dt1: input.path.clone(),
            tiles: Vec::new(),
            warnings: Vec::new(),
        };
        for (o, m, s, px) in specs {
            let v = variants.entry((o, m, s)).or_insert(0);
            let (w, h) = if (1..=2).contains(&o) {
                (48, 64)
            } else {
                (64, 32)
            };
            let origin = TileOrigin {
                dt1_path: name,
                place: &input.place,
                orientation: o,
                main: m,
                sub: s,
                variant: *v,
                rarity: 1,
            };
            *v += 1;
            batch.tiles.push(record_from_rgba(&origin, &px, w, h));
        }
        if name.ends_with("floor.dt1") {
            batch.warnings.push(format!("{name}: fake warning"));
        }
        let keys = by_dt1.entry(batch.dt1.clone()).or_default();
        for t in &batch.tiles {
            keys.entry((t.orientation, t.main, t.sub))
                .or_default()
                .push(t.key.clone());
        }
        sink(batch)?;
    }
    Ok(resolve_used_in(ctx.ds1s, &by_dt1))
}

struct Built {
    fx: Fx,
    idx: Index,
    _cache: tempfile::TempDir,
    user: UserDb,
    _user_dir: tempfile::TempDir,
}

fn built() -> Built {
    let fx = Fx::new("test");
    let cache = tempfile::tempdir().unwrap();
    let mut idx = Index::open(cache.path()).unwrap();
    let overlay = fx.overlay();
    let cancel = AtomicBool::new(false);
    idx.build_with(&fx.profile(), &overlay, &mut |_| {}, &cancel, fake_run)
        .unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let user = UserDb::open(&user_dir.path().join("user.db")).unwrap();
    Built {
        fx,
        idx,
        _cache: cache,
        user,
        _user_dir: user_dir,
    }
}

fn names(b: &Built, q: &str, limit: usize) -> Vec<String> {
    let query = parse_query(q).unwrap();
    b.idx
        .search("test", &query, &b.user, limit)
        .unwrap()
        .into_iter()
        .map(|h| h.asset.key)
        .collect()
}

#[test]
fn build_report_counts_and_rows() {
    let mut b = built();
    let overlay = b.fx.overlay();
    let cancel = AtomicBool::new(false);
    let mut phases = Vec::new();
    let report = b
        .idx
        .build_with(
            &b.fx.profile(),
            &overlay,
            &mut |p| phases.push(p.phase),
            &cancel,
            fake_run,
        )
        .unwrap();
    assert_eq!(report.tiles, 10);
    assert_eq!(b.idx.count("test", Some(AssetKind::Tile)).unwrap(), 10);
    assert_eq!(b.idx.count("test", Some(AssetKind::Ds1)).unwrap(), 2);
    assert_eq!(b.idx.count("test", Some(AssetKind::Dt1)).unwrap(), 2);
    assert_eq!(b.idx.count("test", None).unwrap(), 14);
    assert_eq!(b.idx.count("other", None).unwrap(), 0);
    assert_eq!(report.assets, 14);
    assert!(report.files >= 7, "tables, dt1s and ds1s: {}", report.files);
    assert!(
        report.warnings.iter().any(|w| w.contains("fake warning")),
        "{:?}",
        report.warnings
    );
    for phase in [
        BuildPhase::Listing,
        BuildPhase::Tables,
        BuildPhase::Maps,
        BuildPhase::Tiles,
        BuildPhase::Writing,
    ] {
        assert!(
            phases.contains(&phase),
            "no progress for {phase:?}: {phases:?}"
        );
    }

    // Rebuilding replaces, never duplicates.
    assert_eq!(b.idx.count("test", None).unwrap(), 14);

    let floor = b
        .idx
        .asset("test", "data\\global\\tiles\\act1\\town\\floor.dt1")
        .unwrap()
        .unwrap();
    assert_eq!(floor.kind, AssetKind::Dt1);
    assert_eq!(floor.act, Some(0));
    assert_eq!(floor.src, "d2data.mpq");
    assert_eq!(floor.src_tag, "base");
    assert_eq!(floor.gen_name, "Act1 Town Floor · DT1");
    assert_eq!(b.idx.lvltypes_of(floor.id).unwrap(), vec![(1, 0)]);

    let shire = b
        .idx
        .asset("test", "DATA\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1")
        .unwrap()
        .unwrap();
    assert_eq!(shire.src_tag, "mod");
    assert_eq!(shire.act, None);
    assert_eq!(shire.gen_name, "Mxl Zone Whimsy Shire · DT1");
    assert!(b.idx.lvltypes_of(shire.id).unwrap().is_empty());

    let ds1 = b
        .idx
        .asset("test", "data\\global\\tiles\\act1\\town\\towne1.ds1")
        .unwrap()
        .unwrap();
    assert_eq!(
        (ds1.width, ds1.height, ds1.act),
        (Some(8), Some(8), Some(0))
    );
    assert_eq!(ds1.gen_name, "Act1 Town Towne1 · DS1 · Town");
    assert_eq!(b.idx.asset("test", "nothing").unwrap(), None);

    // Tiles: names use the LvlType name as the place.
    let tile = b
        .idx
        .asset("test", "data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/1")
        .unwrap()
        .unwrap();
    assert_eq!(tile.kind, AssetKind::Tile);
    assert_eq!(
        (tile.orientation, tile.main, tile.sub, tile.variant),
        (Some(0), Some(5), Some(2), Some(1))
    );
    assert!(
        tile.gen_name.starts_with("Act1 Town · Floor · "),
        "{}",
        tile.gen_name
    );
    assert!(tile.gen_name.ends_with(" · 05/02"), "{}", tile.gen_name);
    assert_eq!(tile.class.as_deref(), Some("Floor"));
    assert_eq!(tile.src_tag, "base");
    let thumb = b.idx.thumbnail(tile.id).unwrap().unwrap();
    assert_eq!(&thumb[..4], b"\x89PNG");
    assert_eq!(b.idx.thumbnail(floor.id).unwrap(), None);
    assert_eq!(b.idx.thumbnail(-5).unwrap(), None);

    // The mod DT1 has no LvlType: its tiles are named from the path.
    let mod_tile = b
        .idx
        .asset(
            "test",
            "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1#13/0/0/0",
        )
        .unwrap()
        .unwrap();
    assert!(
        mod_tile
            .gen_name
            .starts_with("Mxl Zone Whimsy Shire · Shadow · dark · "),
        "{}",
        mod_tile.gen_name
    );

    let twins = b.idx.tiles_with_key("test", 0, 5, 2).unwrap();
    assert_eq!(twins.len(), 4);
    assert_eq!(twins[0].variant, Some(0));
    assert!(twins[0].path < twins[2].path);
    assert!(b.idx.tiles_with_key("test", 9, 9, 9).unwrap().is_empty());

    // used_in: towne1 resolves through LvlPrest to floor.dt1 only; shire.ds1
    // has no list, so every DT1 with the key counts.
    let used = b.idx.used_in(tile.id).unwrap();
    assert_eq!(
        used,
        vec![
            ("data\\global\\tiles\\act1\\town\\towne1.ds1".to_owned(), 3),
            (
                "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.ds1".to_owned(),
                2
            ),
        ]
    );
    let mod_floor = b
        .idx
        .asset(
            "test",
            "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1#0/5/2/0",
        )
        .unwrap()
        .unwrap();
    assert_eq!(b.idx.used_in(mod_floor.id).unwrap().len(), 1);
}

#[test]
fn files_table_has_size_hash_and_layer() {
    let b = built();
    let (size, hash, layer, source, kind): (i64, Vec<u8>, i64, String, String) = b
        .idx
        .conn
        .query_row(
            "SELECT size, hash, layer, source, kind FROM files WHERE profile = 'test' AND path = ?1",
            ["data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(size, "also not a dt1".len() as i64);
    assert_eq!(hash, blake3::hash(b"also not a dt1").as_bytes().to_vec());
    assert_eq!(
        (layer, source.as_str(), kind.as_str()),
        (1, "mod-pack.mpq", "dt1")
    );
}

#[test]
fn saves_layer_wins_and_is_tagged() {
    let mut b = built();
    write(
        &b.fx.root.join("test").join("saves"),
        "data\\global\\tiles\\act1\\town\\towne1.ds1",
        &ds1_bytes(&[(0, 0, 1, 1)], &[]),
    );
    let overlay = b.fx.overlay();
    b.idx
        .build_with(
            &b.fx.profile(),
            &overlay,
            &mut |_| {},
            &AtomicBool::new(false),
            fake_run,
        )
        .unwrap();
    let ds1 = b
        .idx
        .asset("test", "data\\global\\tiles\\act1\\town\\towne1.ds1")
        .unwrap()
        .unwrap();
    assert_eq!(
        (ds1.src.as_str(), ds1.src_tag.as_str()),
        ("saves\\test", "saves")
    );
    assert_eq!(names(&b, "src:saves", 10), vec![ds1.key]);
}

#[test]
fn fts_is_populated_and_survives_rebuilds() {
    let mut b = built();
    let hits = |b: &Built, q: &str| -> i64 {
        b.idx
            .conn
            .query_row(
                "SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH ?1",
                [q],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(hits(&b, "gen_name:shadow"), 2);
    assert_eq!(hits(&b, "class:floor"), 4);
    let overlay = b.fx.overlay();
    for _ in 0..2 {
        b.idx
            .build_with(
                &b.fx.profile(),
                &overlay,
                &mut |_| {},
                &AtomicBool::new(false),
                fake_run,
            )
            .unwrap();
    }
    assert_eq!(
        hits(&b, "gen_name:shadow"),
        2,
        "old rows must leave the FTS index"
    );
    b.idx
        .conn
        .execute_batch("INSERT INTO assets_fts(assets_fts) VALUES('integrity-check')")
        .unwrap();
}

#[test]
fn filters() {
    let b = built();
    let all = |q: &str| names(&b, q, 100);
    assert_eq!(all("type:ds1").len(), 2);
    assert_eq!(all("type:dt1").len(), 2);
    assert_eq!(all("type:tile").len(), 10);
    assert_eq!(all("type:floor").len(), 4);
    assert_eq!(all("type:shadow").len(), 2);
    assert_eq!(all("type:wall").len(), 4);
    assert_eq!(all("type:roof").len(), 0);
    assert_eq!(all("-type:floor type:tile").len(), 6);
    assert_eq!(all("").len(), 14);
    // Both maps are v18 files whose header says Act 1.
    assert_eq!(all("type:ds1 act:1").len(), 2);
    assert_eq!(all("act:2").len(), 0);
    assert_eq!(
        all("src:mod").len(),
        7,
        "shire dt1, its 5 tiles and its ds1"
    );
    assert_eq!(all("src:base").len(), 7);
    assert_eq!(all("src:mod-pack").len(), 7);
    assert_eq!(all("src:saves").len(), 0);
    assert_eq!(all("-src:mod").len(), 7);
    assert_eq!(all("file:shire").len(), 1 + 5 + 1);
    assert_eq!(all("file:MXL/zone").len(), 7);
    assert_eq!(all("file:floor.dt1 type:tile").len(), 5);
    assert_eq!(all("idx:5/2").len(), 4);
    assert_eq!(all("idx:5").len(), 4);
    assert_eq!(all("idx:0:5/2").len(), 4);
    assert_eq!(all("idx:13:0/0").len(), 2);
    assert_eq!(all("idx:1:5/2").len(), 0);
    assert_eq!(all("color:green").len(), 4);
    assert_eq!(all("color:dark").len(), 2);
    assert_eq!(
        all("used-in:towne1").len(),
        3,
        "variants 0 and 1 of 0/5/2 and the wall 1/3/0, all in floor.dt1"
    );
    assert_eq!(all("used-in:shire").len(), 4);
    assert_eq!(all("used-in:nothing").len(), 0);
    assert_eq!(
        all("type:ds1 limit-not-a-token").len(),
        0,
        "free text must match"
    );
}

/// Keys of a search with one programmatic filter added to a parsed query.
fn keys_with(b: &Built, q: &str, extra: Filter) -> Vec<String> {
    let mut query = parse_query(q).unwrap();
    query.filters.push(extra);
    b.idx
        .search("test", &query, &b.user, 100)
        .unwrap()
        .into_iter()
        .map(|h| h.asset.key)
        .collect()
}

#[test]
fn any_file_is_the_union_of_file_filters() {
    let b = built();
    let floor = r"data\global\tiles\act1\town\floor.dt1".to_owned();
    let shire = r"data\global\tiles\mxl\zone\whimsy\shire.dt1".to_owned();
    let any = |q: &str, files: &[&String]| {
        keys_with(
            &b,
            q,
            Filter::AnyFile(files.iter().map(|f| (*f).clone()).collect()),
        )
    };
    let both = any("type:tile", &[&floor, &shire]);
    assert_eq!(both.len(), 10, "5 tiles in each DT1");
    // Same set (and order) as the two `file:` queries merged by path.
    let mut separate = names(&b, &format!("type:tile file:{floor}"), 100);
    separate.extend(names(&b, &format!("type:tile file:{shire}"), 100));
    separate.sort();
    let mut sorted = both.clone();
    sorted.sort();
    assert_eq!(sorted, separate);
    assert_eq!(any("type:tile", &[&floor]).len(), 5);
    // Case-insensitive, substrings, duplicates harmless.
    let upper = floor.to_ascii_uppercase();
    assert_eq!(any("type:tile", &[&upper, &upper]).len(), 5);
    assert_eq!(any("", &[&"shire".to_owned()]).len(), 1 + 5 + 1);
    // Combines with other filters and with free text.
    assert_eq!(any("type:floor", &[&floor, &shire]).len(), 4);
    assert_eq!(any("idx:0:5/2", &[&shire]).len(), 2);
    assert_eq!(any("shire", &[&floor]).len(), 0);
    assert!(!any("whimsy", &[&floor, &shire]).is_empty());
    // An empty list matches nothing; its negation matches everything.
    assert!(any("", &[]).is_empty());
    assert_eq!(
        keys_with(&b, "", Filter::Not(Box::new(Filter::AnyFile(Vec::new())))).len(),
        14
    );
    assert_eq!(
        keys_with(
            &b,
            "type:tile",
            Filter::Not(Box::new(Filter::AnyFile(vec![floor.clone()])))
        )
        .len(),
        5
    );
}

#[test]
fn limit_and_order() {
    let b = built();
    assert_eq!(names(&b, "", 0).len(), 0);
    assert_eq!(names(&b, "", 3).len(), 3);
    let all = names(&b, "type:tile", 100);
    let mut sorted = all.clone();
    sorted.sort();
    assert_eq!(all, {
        // path, orientation, main, sub, variant order
        let mut v = b
            .idx
            .search("test", &parse_query("type:tile").unwrap(), &b.user, 100)
            .unwrap();
        v.sort_by(|a, c| {
            (
                &a.asset.path,
                a.asset.orientation,
                a.asset.main,
                a.asset.sub,
                a.asset.variant,
            )
                .cmp(&(
                    &c.asset.path,
                    c.asset.orientation,
                    c.asset.main,
                    c.asset.sub,
                    c.asset.variant,
                ))
        });
        v.into_iter().map(|h| h.asset.key).collect::<Vec<_>>()
    });
}

#[test]
fn free_text_ranks_and_fuzzes() {
    let b = built();
    let top = |q: &str| names(&b, q, 5);
    // The exact name part wins.
    assert!(top("shadow dark 00/00")[0].contains("#13/0/0/0"));
    assert_eq!(top("shadow").len(), 2);
    // Fuzzy: skipped letters still match.
    assert!(!top("shdw").is_empty());
    // Every word must match: "shire floor" ranks tiles named with both first.
    let both = b
        .idx
        .search("test", &parse_query("shire floor").unwrap(), &b.user, 4)
        .unwrap();
    assert!(both.len() >= 2);
    for hit in &both[..2] {
        let name = hit.asset.gen_name.to_lowercase();
        assert!(name.contains("shire") && name.contains("floor"), "{name}");
    }
    assert!(both[0].score >= both[1].score);
    // A word that matches nothing (not even fuzzily) rules the asset out.
    assert!(top("shire floor zzzzqq").is_empty());
    // The path is searched too.
    assert!(top("whimsy shire ds1")[0].ends_with("shire.ds1"));
    // Text combines with filters.
    let hits = names(&b, "grass type:floor act:1", 10);
    assert!(
        hits.iter()
            .all(|k| k.contains("act1\\town\\floor.dt1#0/5/2/")),
        "{hits:?}"
    );
    // Nothing matches nonsense.
    assert!(top("zzzzqqqq").is_empty());
    // Scores are positive and sorted best first.
    let scored = b
        .idx
        .search("test", &parse_query("town").unwrap(), &b.user, 50)
        .unwrap();
    assert!(!scored.is_empty());
    assert!(scored.windows(2).all(|w| w[0].score >= w[1].score));
}

#[test]
fn aliases_tags_and_negation() {
    let b = built();
    let key = "data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/0";
    let other = "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1#0/5/2/0";
    b.user.add_tag("test", key, "favourite grass").unwrap();
    b.user.add_tag("test", other, "meh").unwrap();
    b.user
        .add_tag("elsewhere", other, "favourite grass")
        .unwrap();
    assert_eq!(names(&b, "tag:\"favourite grass\"", 10), vec![key]);
    assert_eq!(names(&b, "tag:MEH", 10), vec![other]);
    assert_eq!(names(&b, "type:tile -tag:meh idx:5/2", 10).len(), 3);
    assert_eq!(names(&b, "tag:nothing", 10).len(), 0);

    // An alias is found by text and reported on the hit.
    b.user.set_alias("test", key, Some("Mossy meadow")).unwrap();
    let hits = b
        .idx
        .search("test", &parse_query("meadow").unwrap(), &b.user, 5)
        .unwrap();
    assert_eq!(hits[0].asset.key, key, "{hits:?}");
    assert_eq!(hits[0].alias.as_deref(), Some("Mossy meadow"));
    // Other searches still report it.
    let hits = b
        .idx
        .search(
            "test",
            &parse_query("tag:\"favourite grass\"").unwrap(),
            &b.user,
            5,
        )
        .unwrap();
    assert_eq!(hits[0].alias.as_deref(), Some("Mossy meadow"));
    // Another profile's alias is invisible.
    b.user.set_alias("elsewhere", other, Some("nope")).unwrap();
    assert!(
        b.idx
            .search("test", &parse_query("nope").unwrap(), &b.user, 5)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn similar_and_mirrored() {
    let b = built();
    // Tiles 1/3/0 and 2/3/0 are gradients mirrored against each other.
    let left = "data\\global\\tiles\\act1\\town\\floor.dt1#1/3/0/0";
    let right = "data\\global\\tiles\\act1\\town\\floor.dt1#2/3/0/0";
    let mirrored = names(&b, "mirrored:yes", 50);
    for k in [left, right] {
        assert!(mirrored.contains(&k.to_owned()), "{k} in {mirrored:?}");
    }
    assert!(
        mirrored
            .iter()
            .all(|k| k.contains("#1/3/0") || k.contains("#2/3/0")),
        "{mirrored:?}"
    );
    assert_eq!(mirrored.len(), 4, "two DT1s, two gradients each");
    let not_mirrored = names(&b, "type:tile mirrored:no", 50);
    assert_eq!(not_mirrored.len(), 6);

    // Similar: the tile itself first, then nearest first; the identical copy
    // in the other DT1 comes right after it.
    let hits = b
        .idx
        .search(
            "test",
            &parse_query(&format!("similar:{left}")).unwrap(),
            &b.user,
            10,
        )
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].asset.key, left);
    assert_eq!(hits[0].score, 1.0);
    assert_eq!(
        hits[1].asset.key,
        "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1#1/3/0/0"
    );
    assert!(hits[1].score >= hits[hits.len() - 1].score);
    assert!(hits[0].score >= hits[hits.len() - 1].score);
    assert!(hits.iter().all(|h| h.score >= 1.0 - 10.0 / 64.0 - 1e-6));
    assert!(names(&b, "similar:no-such-key", 10).is_empty());
}

#[test]
fn status_is_per_profile_and_survives_reopen() {
    let a = Fx::new("pa");
    let c = Fx::new("pb");
    let cache = tempfile::tempdir().unwrap();
    let mut idx = Index::open(cache.path()).unwrap();
    let (oa, oc) = (a.overlay(), c.overlay());
    assert_eq!(idx.status("pa", &oa).unwrap(), IndexStatus::Missing);
    assert_eq!(idx.status("pb", &oc).unwrap(), IndexStatus::Missing);
    let cancel = AtomicBool::new(false);
    idx.build_with(&a.profile(), &oa, &mut |_| {}, &cancel, fake_run)
        .unwrap();
    idx.build_with(&c.profile(), &oc, &mut |_| {}, &cancel, fake_run)
        .unwrap();
    assert_eq!(idx.status("pa", &oa).unwrap(), IndexStatus::UpToDate);
    assert_eq!(idx.status("pb", &oc).unwrap(), IndexStatus::UpToDate);
    drop(idx);

    // Second launch: a fresh Index on the same cache dir needs no rebuild.
    let idx = Index::open(cache.path()).unwrap();
    assert_eq!(
        idx.status("pa", &a.overlay()).unwrap(),
        IndexStatus::UpToDate
    );
    assert_eq!(
        idx.status("pb", &c.overlay()).unwrap(),
        IndexStatus::UpToDate
    );
    assert_eq!(idx.count("pa", Some(AssetKind::Ds1)).unwrap(), 2);
    drop(idx);

    // Change the size of one archive (a layer) of profile pa only.
    let target = a
        .base()
        .join("data\\global\\tiles\\act1\\town\\floor.dt1".replace('\\', "/"));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&target)
            .unwrap();
        f.write_all(b"more bytes").unwrap();
    }
    let idx = Index::open(cache.path()).unwrap();
    match idx.status("pa", &a.overlay()).unwrap() {
        IndexStatus::Stale { reason } => {
            assert!(
                reason.contains("d2data.mpq") || reason.contains("base"),
                "{reason}"
            );
            assert!(reason.contains("size"), "{reason}");
        }
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(
        idx.status("pb", &c.overlay()).unwrap(),
        IndexStatus::UpToDate
    );
    drop(idx);

    // Modified time alone also counts.
    let target_b = c.base().join("data/global/tiles/act1/town/floor.dt1");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&target_b)
        .unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .unwrap();
    drop(f);
    let idx = Index::open(cache.path()).unwrap();
    match idx.status("pb", &c.overlay()).unwrap() {
        IndexStatus::Stale { reason } => assert!(reason.contains("modified"), "{reason}"),
        other => panic!("expected Stale, got {other:?}"),
    }

    // A new file in a layer (entry count) is stale too, a rebuild fixes it.
    let mut idx = idx;
    let cancel = AtomicBool::new(false);
    idx.build_with(&a.profile(), &a.overlay(), &mut |_| {}, &cancel, fake_run)
        .unwrap();
    assert_eq!(
        idx.status("pa", &a.overlay()).unwrap(),
        IndexStatus::UpToDate
    );
    // Saving a map into the saves layer never makes the profile stale ...
    write(
        &a.root.join("pa").join("saves"),
        "data\\global\\x.txt",
        b"hi",
    );
    assert_eq!(
        idx.status("pa", &a.overlay()).unwrap(),
        IndexStatus::UpToDate
    );
    // ... but a new file in any other layer does.
    write(&a.root.join("pa").join("mod"), "data\\global\\y.txt", b"hi");
    assert!(matches!(
        idx.status("pa", &a.overlay()).unwrap(),
        IndexStatus::Stale { .. }
    ));
}

fn towne1_cells(idx: &Index, tile: i64) -> Option<u32> {
    idx.used_in(tile)
        .unwrap()
        .into_iter()
        .find(|(p, _)| p == TOWNE1)
        .map(|(_, n)| n)
}

const TOWNE1: &str = "data\\global\\tiles\\act1\\town\\towne1.ds1";

#[test]
fn saving_keeps_the_index_fresh_and_refresh_saves_picks_the_map_up() {
    let mut b = built();
    let overlay = b.fx.overlay();
    let profile = b.fx.profile();
    let floor_tile = b
        .idx
        .asset("test", "data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/0")
        .unwrap()
        .unwrap();
    let before = b.idx.asset("test", TOWNE1).unwrap().unwrap();
    assert_eq!(before.src_tag, "base");
    assert_eq!(towne1_cells(&b.idx, floor_tile.id), Some(3));

    // Nothing to do yet.
    let nothing = b.idx.refresh_saves(&profile, &overlay).unwrap();
    assert_eq!((nothing.files, nothing.assets), (0, 0));

    // Save an edited towne1 into the saves layer.
    write(
        &b.fx.root.join("test").join("saves"),
        TOWNE1,
        &ds1_bytes(&[(0, 0, 5, 2), (7, 7, 9, 0)], &[]),
    );
    assert_eq!(
        b.idx.status("test", &overlay).unwrap(),
        IndexStatus::UpToDate
    );
    assert_eq!(
        b.idx.asset("test", TOWNE1).unwrap().unwrap().src_tag,
        "base"
    );

    let report = b.idx.refresh_saves(&profile, &overlay).unwrap();
    assert_eq!((report.files, report.assets), (1, 1), "{report:?}");
    let after = b.idx.asset("test", TOWNE1).unwrap().unwrap();
    assert_eq!(after.id, before.id, "the row is updated, not replaced");
    assert_eq!(
        (after.src.as_str(), after.src_tag.as_str()),
        ("saves\\test", "saves")
    );
    assert_eq!(
        b.idx.status("test", &overlay).unwrap(),
        IndexStatus::UpToDate
    );
    assert_eq!(b.idx.count("test", None).unwrap(), 14, "no duplicates");
    assert_eq!(names(&b, "src:saves", 10), vec![TOWNE1.to_owned()]);
    // used_in follows the new cells: one use of 0/5/2, none of the wall key.
    assert_eq!(towne1_cells(&b.idx, floor_tile.id), Some(1));
    let wall = b
        .idx
        .asset("test", "data\\global\\tiles\\act1\\town\\floor.dt1#1/3/0/0")
        .unwrap()
        .unwrap();
    assert!(
        b.idx
            .used_in(wall.id)
            .unwrap()
            .iter()
            .all(|(p, _)| p != TOWNE1),
        "the wall tile is no longer used by towne1"
    );
    let files_size: i64 = b
        .idx
        .conn
        .query_row(
            "SELECT size FROM files WHERE profile = 'test' AND path = ?1",
            [TOWNE1],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        files_size,
        ds1_bytes(&[(0, 0, 5, 2), (7, 7, 9, 0)], &[]).len() as i64
    );
    // The FTS row follows too, and the other maps are untouched.
    let fts: i64 = b
        .idx
        .conn
        .query_row(
            "SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH 'gen_name:towne1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fts, 1);

    // Removing the saved map reverts the row to the game's version.
    std::fs::remove_file(GamePath::new(TOWNE1).to_native(&b.fx.root.join("test").join("saves")))
        .unwrap();
    let report = b.idx.refresh_saves(&profile, &overlay).unwrap();
    assert_eq!(report.assets, 1);
    let reverted = b.idx.asset("test", TOWNE1).unwrap().unwrap();
    assert_eq!(
        (reverted.src.as_str(), reverted.src_tag.as_str()),
        ("d2data.mpq", "base")
    );
    assert_eq!(towne1_cells(&b.idx, floor_tile.id), Some(3));

    // A new map saved under a new name becomes an asset.
    let fresh = "data\\global\\tiles\\mxl\\mine\\new.ds1";
    write(
        &b.fx.root.join("test").join("saves"),
        fresh,
        &ds1_bytes(&[(1, 1, 5, 2)], &[]),
    );
    b.idx.refresh_saves(&profile, &overlay).unwrap();
    assert_eq!(
        b.idx.asset("test", fresh).unwrap().unwrap().src_tag,
        "saves"
    );
    assert_eq!(b.idx.count("test", Some(AssetKind::Ds1)).unwrap(), 3);
    // ... and disappears again when the file does.
    std::fs::remove_file(GamePath::new(fresh).to_native(&b.fx.root.join("test").join("saves")))
        .unwrap();
    b.idx.refresh_saves(&profile, &overlay).unwrap();
    assert_eq!(b.idx.asset("test", fresh).unwrap(), None);
    assert_eq!(b.idx.count("test", Some(AssetKind::Ds1)).unwrap(), 2);

    // An unbuilt profile cannot be refreshed.
    let mut other = Index::open(tempfile::tempdir().unwrap().path()).unwrap();
    assert!(other.refresh_saves(&profile, &overlay).is_err());
}

fn dc6_bytes() -> Vec<u8> {
    // 2 directions x 1 frame, each frame 4x2 at (-2, 1), plain pixel rows.
    let mut out = Vec::new();
    for v in [6u32, 1, 0, 0xEEEE_EEEE, 2, 1] {
        out.extend(v.to_le_bytes());
    }
    let frame_len = 32 + 12 + 3;
    out.extend(((24 + 8) as u32).to_le_bytes());
    out.extend(((24 + 8 + frame_len) as u32).to_le_bytes());
    for _ in 0..2 {
        for v in [0i32, 4, 2, -2, 1, 0, 0] {
            out.extend(v.to_le_bytes());
        }
        out.extend(12u32.to_le_bytes());
        out.extend([4, 1, 2, 3, 4, 0x80, 4, 5, 6, 7, 8, 0x80, 0, 0, 0]);
    }
    out
}

fn cof_bytes() -> Vec<u8> {
    // 2 layers, 3 frames, 4 directions.
    let mut out = vec![0u8; 28];
    out[0] = 2;
    out[1] = 3;
    out[2] = 4;
    out.extend([0u8; 18]);
    out.extend(vec![0u8; 3 + 4 * 3 * 2]);
    out
}

#[test]
fn sprites_get_sizes_counts_and_thumbnails() {
    let fx = Fx::new("spr");
    let modl = fx.root.join("spr").join("mod");
    write(&modl, "data\\global\\items\\inv\\sword.dc6", &dc6_bytes());
    write(&modl, "data\\global\\monsters\\zz\\zz.cof", &cof_bytes());
    write(&modl, "data\\global\\items\\inv\\broken.dc6", b"nope");
    write(&modl, "data\\global\\monsters\\zz\\a1.dcc", b"nope");
    let cache = tempfile::tempdir().unwrap();
    let mut idx = Index::open(cache.path()).unwrap();
    let mut phases = Vec::new();
    let report = idx
        .build_with(
            &fx.profile(),
            &fx.overlay(),
            &mut |p| phases.push(p.phase),
            &AtomicBool::new(false),
            fake_run,
        )
        .unwrap();
    assert!(phases.contains(&BuildPhase::Sprites));
    assert_eq!(idx.count("spr", Some(AssetKind::Dc6)).unwrap(), 2);
    assert_eq!(idx.count("spr", Some(AssetKind::Cof)).unwrap(), 1);
    assert_eq!(idx.count("spr", Some(AssetKind::Dcc)).unwrap(), 1);
    assert!(
        report.warnings.iter().any(|w| w.contains("broken.dc6")),
        "{:?}",
        report.warnings
    );
    assert!(
        report.warnings.iter().any(|w| w.contains("a1.dcc")),
        "{:?}",
        report.warnings
    );

    let dc6 = idx
        .asset("spr", "data\\global\\items\\inv\\sword.dc6")
        .unwrap()
        .unwrap();
    assert_eq!((dc6.width, dc6.height), (Some(4), Some(2)));
    assert_eq!(
        (dc6.directions, dc6.frames),
        (Some(2), Some(1)),
        "2 directions, 1 frame"
    );
    assert_eq!(dc6.gen_name, "Items Inv Sword · DC6");
    assert_eq!(dc6.src_tag, "mod");
    let thumb = idx.thumbnail(dc6.id).unwrap().unwrap();
    let png = image::load_from_memory(&thumb).unwrap();
    assert_eq!((png.width(), png.height()), (4, 2));

    let cof = idx
        .asset("spr", "data\\global\\monsters\\zz\\zz.cof")
        .unwrap()
        .unwrap();
    assert_eq!(
        (cof.directions, cof.frames),
        (Some(4), Some(3)),
        "4 directions, 3 frames"
    );
    assert_eq!(cof.class.as_deref(), Some("2 layers"));
    assert_eq!(idx.thumbnail(cof.id).unwrap(), None);

    // The unreadable ones are still assets, just without details.
    let broken = idx
        .asset("spr", "data\\global\\items\\inv\\broken.dc6")
        .unwrap()
        .unwrap();
    assert_eq!((broken.width, broken.directions), (None, None));
}

#[test]
fn cancel_before_and_during_a_build_keeps_the_old_index() {
    let mut b = built();
    let overlay = b.fx.overlay();
    let profile = b.fx.profile();

    // Cancelled from the start.
    let cancel = AtomicBool::new(true);
    let mut calls = 0;
    let r = b
        .idx
        .build_with(&profile, &overlay, &mut |_| calls += 1, &cancel, fake_run);
    assert!(matches!(r, Err(crate::IndexError::Cancelled)), "{r:?}");

    // Cancelled once tiles start: the transaction rolls back.
    let cancel = AtomicBool::new(false);
    let r = b.idx.build_with(
        &profile,
        &overlay,
        &mut |p| {
            if p.phase == BuildPhase::Tiles {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
        fake_run,
    );
    assert!(matches!(r, Err(crate::IndexError::Cancelled)), "{r:?}");
    assert_eq!(
        b.idx.status("test", &overlay).unwrap(),
        IndexStatus::UpToDate
    );
    assert_eq!(b.idx.count("test", None).unwrap(), 14);
    assert_eq!(
        b.idx
            .thumbnail(
                b.idx
                    .asset("test", "data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/0")
                    .unwrap()
                    .unwrap()
                    .id
            )
            .unwrap()
            .map(|t| t.len() > 8),
        Some(true)
    );
}

#[test]
fn real_pipeline_reports_bad_dt1_as_warnings() {
    // The fixture's DT1s are not DT1s: the real pipeline must warn, not fail.
    let fx = Fx::new("real");
    let cache = tempfile::tempdir().unwrap();
    let mut idx = Index::open(cache.path()).unwrap();
    let report = idx
        .build(
            &fx.profile(),
            &fx.overlay(),
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(report.tiles, 0);
    assert_eq!(idx.count("real", Some(AssetKind::Dt1)).unwrap(), 2);
    assert!(
        report.warnings.iter().any(|w| w.contains("floor.dt1")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn schema_mismatch_and_garbage_are_recreated() {
    let cache = tempfile::tempdir().unwrap();
    {
        let idx = Index::open(cache.path()).unwrap();
        idx.conn
            .execute(
                "UPDATE meta SET value = '1' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        idx.conn
            .execute("INSERT INTO profiles VALUES('x', 'f', 0)", [])
            .unwrap();
    }
    {
        let idx = Index::open(cache.path()).unwrap();
        let n: i64 = idx
            .conn
            .query_row("SELECT COUNT(*) FROM profiles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "a schema mismatch drops the old data");
        let v: String = idx
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "2");
    }
    std::fs::write(
        cache.path().join("index.db"),
        b"this is not a database at all",
    )
    .unwrap();
    let idx = Index::open(cache.path()).unwrap();
    assert_eq!(idx.count("x", None).unwrap(), 0);
}

#[test]
fn source_tags_and_act_from_path() {
    use crate::build::{act_from_path, src_tag};
    assert_eq!(src_tag("saves\\mxl"), "saves");
    assert_eq!(src_tag("D2Data.MPQ"), "base");
    assert_eq!(src_tag("Patch_D2.mpq"), "base");
    assert_eq!(src_tag("medianxl-dGlsZXM.mpq"), "mod");
    assert_eq!(src_tag("C:\\mods\\folder"), "mod");
    assert_eq!(
        act_from_path("data\\global\\tiles\\act3\\jungle\\x.ds1"),
        Some(2)
    );
    assert_eq!(
        act_from_path("data\\global\\tiles\\expansion\\siege\\x.ds1"),
        Some(4)
    );
    assert_eq!(act_from_path("data\\global\\tiles\\mxl\\zone\\x.ds1"), None);
    assert_eq!(act_from_path("data\\global\\tiles\\act9\\x.ds1"), None);
    assert_eq!(act_from_path("data\\global\\tiles\\actually\\x.ds1"), None);
}
