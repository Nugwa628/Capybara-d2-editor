//! Tile creation over the private corpus (skipped when it is absent): the
//! measured conventions of shipped tiles, and the rebuild oracle.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use capy_mod::tiles_create::{TileClass, TileShape, reshape_tile};
use d2_formats::dt1::{Dt1, Tile};

fn corpus_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let base = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(|p| PathBuf::from(p).join("extracted"))
        .filter(|p| p.is_dir())
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .map(|a| a.join("testdata").join("private").join("extracted"))
                .find(|p| p.is_dir())
        });
    if let Some(b) = base {
        for sub in ["vanilla", "mxl"] {
            let p = b.join(sub);
            if p.is_dir() {
                roots.push(p);
            }
        }
    }
    roots
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dt1")) {
            out.push(p);
        }
    }
}

/// Every parseable DT1 of the corpus (every 9th file unless
/// `CAPY_FULL_CORPUS=1`).
fn dt1s() -> Vec<(PathBuf, Vec<u8>, Dt1)> {
    let full = std::env::var_os("CAPY_FULL_CORPUS").is_some_and(|v| v == "1");
    let mut out = Vec::new();
    for root in corpus_roots() {
        let mut files = Vec::new();
        collect(&root, &mut files);
        let step = if full { 1 } else { 9 };
        for p in files.into_iter().step_by(step) {
            let Ok(bytes) = std::fs::read(&p) else {
                continue;
            };
            if let Ok(dt1) = Dt1::parse(&bytes) {
                out.push((p, bytes, dt1));
            }
        }
    }
    out
}

#[derive(Default)]
struct Counter(BTreeMap<String, usize>);

impl Counter {
    fn add(&mut self, k: impl Into<String>) {
        *self.0.entry(k.into()).or_insert(0) += 1;
    }

    fn top(&self, n: usize) -> String {
        let mut v: Vec<_> = self.0.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v.iter()
            .take(n)
            .map(|(k, c)| format!("{k}:{c}"))
            .collect::<Vec<_>>()
            .join("  ")
    }
}

fn rle_rows(data: &[u8]) -> usize {
    let (mut pos, mut rows) = (0usize, 0usize);
    while let (Some(&s), Some(&c)) = (data.get(pos), data.get(pos + 1)) {
        pos += 2;
        if s == 0 && c == 0 {
            rows += 1;
        } else {
            pos += usize::from(c);
        }
    }
    rows
}

/// Rows of the 5x5 flags (`y * 5 + x`), top row first.
fn flags_str(t: &Tile) -> String {
    t.subtile_flags
        .chunks(5)
        .map(|r| r.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .collect::<Vec<_>>()
        .join("/")
}

/// Reports, per orientation, the conventions written down in
/// `docs/specs/dt1.md` section 14 (block formats and RLE rows, heights,
/// widths, direction, rarity, flags, corner-half keys), and checks the ones
/// the tile builder relies on.
#[test]
fn measure_shipped_conventions() {
    let files = dt1s();
    if files.is_empty() {
        println!("skipped: corpus missing");
        return;
    }
    let mut by_o: BTreeMap<i32, BTreeMap<&str, Counter>> = BTreeMap::new();
    let mut corner_with_partner = (0usize, 0usize);
    for (_, _, dt1) in &files {
        let keys: BTreeSet<(i32, i32, i32)> = dt1
            .tiles
            .iter()
            .map(|t| (t.orientation, t.main_index, t.sub_index))
            .collect();
        for t in &dt1.tiles {
            let m = by_o.entry(t.orientation).or_default();
            m.entry("tiles").or_default().add("n");
            m.entry("height").or_default().add(t.height.to_string());
            m.entry("width").or_default().add(t.width.to_string());
            m.entry("roof_height")
                .or_default()
                .add(t.roof_height.to_string());
            m.entry("direction")
                .or_default()
                .add(t.direction.to_string());
            m.entry("rarity").or_default().add(t.rarity.to_string());
            m.entry("flags").or_default().add(flags_str(t));
            for b in &t.blocks {
                let kind = if b.raw_format == 1 {
                    "iso".to_owned()
                } else {
                    format!("{:#x} rows={}", b.raw_format, rle_rows(&b.data))
                };
                m.entry("format").or_default().add(kind);
                m.entry("grid")
                    .or_default()
                    .add(format!("{}/{}", b.grid_x, b.grid_y));
            }
            if let (Some(lo), Some(hi)) = (
                t.blocks.iter().map(|b| i32::from(b.y)).min(),
                t.blocks.iter().map(|b| i32::from(b.y)).max(),
            ) {
                let rel = match t.orientation {
                    0 | 15 => format!("y {lo}..{hi}"),
                    16..=19 => format!("top {lo}, height == -(span) {}", t.height == lo - hi - 32),
                    _ => format!("height == top - 32 {}", t.height == lo - 32),
                };
                m.entry("blocks y").or_default().add(rel);
                if !matches!(t.orientation, 0 | 15) {
                    let max_x = t.blocks.iter().map(|b| i32::from(b.x)).max().unwrap_or(0);
                    m.entry("width == max x + 32")
                        .or_default()
                        .add((t.width == max_x + 32).to_string());
                }
            }
            if t.orientation == 3 {
                corner_with_partner.0 += 1;
                if keys.contains(&(4, t.main_index, t.sub_index)) {
                    corner_with_partner.1 += 1;
                }
            }
        }
    }
    for (o, m) in &by_o {
        println!("=== orientation {o}");
        for (k, c) in m {
            println!("  {k}: {}", c.top(if *k == "flags" { 4 } else { 6 }));
        }
    }
    println!(
        "orientation 3 tiles with an orientation 4 of the same main/sub: {} of {}",
        corner_with_partner.1, corner_with_partner.0
    );
    // what the builder relies on
    for (o, m) in &by_o {
        for k in m["format"].0.keys() {
            assert!(
                k == "iso" || k == "0x2005 rows=15" || k == "0x1001 rows=32",
                "orientation {o}: {k}"
            );
        }
        if *o != 0 && *o != 15 {
            assert_eq!(
                m["grid"].0.keys().collect::<Vec<_>>(),
                ["0/0"],
                "orientation {o}"
            );
        }
    }
}

/// Every corpus tile rendered, rebuilt from that render with its inferred
/// shape and rendered again: the pixels must come back identical; the share
/// whose blocks (and whole tile) come back byte-identical is reported and
/// held above a floor.
#[test]
fn rebuild_oracle() {
    let files = dt1s();
    if files.is_empty() {
        println!("skipped: corpus missing");
        return;
    }
    #[derive(Default)]
    struct Stat {
        tiles: usize,
        pixels_same: usize,
        blocks_same: usize,
        tile_same: usize,
    }
    let mut stats: BTreeMap<String, Stat> = BTreeMap::new();
    let mut misses: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut files_same, mut files_total) = (0usize, 0usize);
    for (path, bytes, dt1) in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut rebuilt = dt1.clone();
        for (i, t) in dt1.tiles.iter().enumerate() {
            let Some(shape) = TileShape::of_tile(t) else {
                continue;
            };
            let class = format!("{:?}", TileClass::of_orientation(t.orientation).unwrap());
            let s = stats.entry(class.clone()).or_default();
            s.tiles += 1;
            let built = reshape_tile(t, shape).unwrap();
            let before = t.render().unwrap();
            let after = built.tile.render().unwrap();
            let same_pixels = before == after && built.dropped == 0;
            if same_pixels {
                s.pixels_same += 1;
            }
            if built.tile.blocks == t.blocks {
                s.blocks_same += 1;
            }
            if built.tile == *t {
                s.tile_same += 1;
            }
            rebuilt.tiles[i] = built.tile.clone();
            let why = if !same_pixels {
                Some(format!("pixels (dropped {})", built.dropped))
            } else if built.tile.blocks != t.blocks {
                let a: Vec<String> = t
                    .blocks
                    .iter()
                    .map(|b| format!("{:#x}@{},{}", b.raw_format, b.x, b.y))
                    .collect();
                let b: Vec<String> = built
                    .tile
                    .blocks
                    .iter()
                    .map(|b| format!("{:#x}@{},{}", b.raw_format, b.x, b.y))
                    .collect();
                Some(format!("blocks: {} vs {}", a.join(" "), b.join(" ")))
            } else if built.tile != *t {
                let (a, b) = (t, &built.tile);
                Some(format!(
                    "header: h {}/{} w {}/{} dir {}/{}",
                    a.height, b.height, a.width, b.width, a.direction, b.direction
                ))
            } else {
                None
            };
            if let Some(why) = why {
                let m = misses.entry(class).or_default();
                if m.len() < 6 {
                    m.push(format!("{name} #{i} o{}: {why}", t.orientation));
                }
            }
        }
        // the rebuilt file writes and reads back; most come out byte-identical
        let out = rebuilt.to_bytes().unwrap();
        assert_eq!(Dt1::parse(&out).unwrap(), rebuilt, "{name}");
        files_total += 1;
        if out == *bytes {
            files_same += 1;
        }
    }
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    let mut all = Stat::default();
    for (class, s) in &stats {
        println!(
            "{class}: {} tiles, pixels identical {:.2}%, blocks byte-identical {:.2}%, whole tile identical {:.2}%",
            s.tiles,
            pct(s.pixels_same, s.tiles),
            pct(s.blocks_same, s.tiles),
            pct(s.tile_same, s.tiles)
        );
        all.tiles += s.tiles;
        all.pixels_same += s.pixels_same;
        all.blocks_same += s.blocks_same;
        all.tile_same += s.tile_same;
    }
    println!(
        "all: {} tiles, pixels {:.2}%, blocks {:.2}%, tile {:.2}%",
        all.tiles,
        pct(all.pixels_same, all.tiles),
        pct(all.blocks_same, all.tiles),
        pct(all.tile_same, all.tiles)
    );
    println!(
        "files rebuilt byte-identical: {files_same} of {files_total} ({:.2}%)",
        pct(files_same, files_total)
    );
    for (class, m) in &misses {
        for l in m {
            println!("miss {class}: {}", &l[..l.len().min(600)]);
        }
    }
    assert_eq!(
        all.pixels_same, all.tiles,
        "every rebuilt tile renders identically"
    );
    // measured on the full corpus: 100% of blocks, 99.56% of tiles
    assert!(pct(all.blocks_same, all.tiles) >= 99.9, "blocks floor");
    assert!(pct(all.tile_same, all.tiles) >= 99.0, "tiles floor");
}
