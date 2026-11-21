//! Created tiles stand where shipped tiles of the same orientation stand in
//! the map view. Every shipped tile of the private corpus (every 9th file
//! unless `CAPY_FULL_CORPUS=1`; skipped when the corpus is absent) is
//! rendered, cut to its template as a plain picture (what a user paints for
//! "As is" or sends as `create_tile` pixels), built again with
//! `build_tile` / `build_corner_pair`, and must come back with the same
//! pixels in anchor space and the same `capy_render::TileGeometry` (bitmap
//! size, `y_add`, zero line and so the bitmap's row on its cell), which is
//! all the compositor uses to place a tile on its cell.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use capy_mod::tiles_create::{BuildOptions, TileShape, build_corner_pair, build_tile, template};
use capy_render::TileGeometry;
use d2_formats::IndexedImage;
use d2_formats::dt1::{Dt1, Tile};

fn corpus_roots() -> Vec<PathBuf> {
    let base = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(|p| PathBuf::from(p).join("extracted"))
        .filter(|p| p.is_dir())
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .map(|a| a.join("testdata").join("private").join("extracted"))
                .find(|p| p.is_dir())
        });
    let Some(b) = base else { return Vec::new() };
    ["vanilla", "mxl"]
        .iter()
        .map(|s| b.join(s))
        .filter(|p| p.is_dir())
        .collect()
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

fn dt1s() -> Vec<(PathBuf, Dt1)> {
    let full = std::env::var_os("CAPY_FULL_CORPUS").is_some_and(|v| v == "1");
    let mut out = Vec::new();
    for root in corpus_roots() {
        let mut files = Vec::new();
        collect(&root, &mut files);
        for p in files.into_iter().step_by(if full { 1 } else { 9 }) {
            if let Some(d) = std::fs::read(&p).ok().and_then(|b| Dt1::parse(&b).ok()) {
                out.push((p, d));
            }
        }
    }
    out
}

/// What the compositor uses to place a tile on its cell.
fn placement(t: &Tile) -> (TileGeometry, i32) {
    let g = TileGeometry::of(t);
    (g, g.bitmap_top(1))
}

fn opts(t: &Tile) -> BuildOptions {
    BuildOptions {
        flags: Some(t.subtile_flags),
        rarity: t.rarity,
        sound: t.sound,
        animated: t.animated,
    }
}

/// `t` as the plain picture of its template (pixel (0,0) on the template's
/// top-left), placed back in anchor space as "As is" does.
fn as_is_picture(t: &Tile, shape: TileShape) -> IndexedImage {
    let tpl = template(shape).unwrap();
    tpl.to_anchor(&tpl.to_plain(&t.render().unwrap()))
}

/// Opaque pixels of `img` as anchor-space `(x, y, index)`.
fn opaque(img: &IndexedImage) -> Vec<(i32, i32, u8)> {
    let mut v = Vec::new();
    for row in 0..img.height as i32 {
        for col in 0..img.width as i32 {
            let i = (row * img.width as i32 + col) as usize;
            if img.alpha[i] != 0 {
                v.push((col + img.offset_x, row + img.offset_y, img.pixels[i]));
            }
        }
    }
    v
}

#[test]
fn created_tiles_stand_where_shipped_tiles_stand() {
    let files = dt1s();
    if files.is_empty() {
        println!("skipped: corpus missing");
        return;
    }
    // orientation -> (tiles, same pixels, same placement)
    let mut stats: BTreeMap<i32, (usize, usize, usize)> = BTreeMap::new();
    let mut misses: Vec<String> = Vec::new();
    let mut check = |o: i32, name: &str, shipped: &Tile, built: &Tile| {
        let s = stats.entry(o).or_default();
        s.0 += 1;
        let same_pixels = opaque(&shipped.render().unwrap()) == opaque(&built.render().unwrap());
        if same_pixels {
            s.1 += 1;
        }
        let (a, b) = (placement(shipped), placement(built));
        if a == b {
            s.2 += 1;
        } else if misses.len() < 40 {
            misses.push(format!(
                "{name} o{o} {}/{}: shipped h {} top {} / built h {} top {}",
                shipped.main_index, shipped.sub_index, shipped.height, a.1, built.height, b.1
            ));
        }
        assert!(same_pixels, "{name} o{o}: pixels differ");
    };
    for (path, dt1) in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        for t in &dt1.tiles {
            let Some(shape) = TileShape::of_tile(t) else {
                continue;
            };
            if t.orientation == 3 || t.orientation == 4 {
                // corner halves: one 160-wide picture for both (when the
                // pair is complete), as the dialog's "North corner (both
                // halves)" and create_tile corner_pair build them
                let partner = dt1.tiles.iter().find(|p| {
                    p.orientation == 7 - t.orientation
                        && p.main_index == t.main_index
                        && p.sub_index == t.sub_index
                        && TileShape::of_tile(p).is_some_and(|ps| ps.height == shape.height)
                });
                if let (Some(p), 3) = (partner, t.orientation) {
                    let mut pic = as_is_picture(t, shape);
                    let left = as_is_picture(p, TileShape::wall(4, shape.height));
                    // merge the left half into the picture (same rect origin
                    // y, x 0..80)
                    let mut full = IndexedImage::new(160, pic.height, 0, pic.offset_y);
                    for img in [&left, &pic] {
                        for (x, y, v) in opaque(img) {
                            let i = ((y - full.offset_y) * 160 + x) as usize;
                            full.pixels[i] = v;
                            full.alpha[i] = 255;
                        }
                    }
                    pic = full;
                    let (r, l) =
                        build_corner_pair(&pic, shape.height, t.main_index, t.sub_index, &opts(t))
                            .unwrap();
                    check(3, &name, t, &r.tile);
                    check(4, &name, p, &l.tile);
                    continue;
                }
                if partner.is_some() {
                    continue; // checked with its orientation-3 half
                }
            }
            let built = build_tile(
                &as_is_picture(t, shape),
                shape,
                t.main_index,
                t.sub_index,
                &opts(t),
            )
            .unwrap();
            check(t.orientation, &name, t, &built.tile);
        }
    }
    let mut total = (0, 0, 0);
    for (o, (n, px, pl)) in &stats {
        println!("o{o:2}: {n:5} tiles, same pixels {px}, same placement {pl}");
        total = (total.0 + n, total.1 + px, total.2 + pl);
    }
    for m in &misses {
        println!("placement differs: {m}");
    }
    // The only placement differences allowed are the shipped lower walls
    // whose blocks reach below their declared height and the empty walls
    // storing height 0 (docs/specs/dt1.md section 14, "Rebuild misses"):
    // their stored height is not the one their blocks need.
    for m in &misses {
        assert!(
            m.contains(" o16 ")
                || m.contains(" o17 ")
                || m.contains(" o18 ")
                || m.contains(" o19 ")
                || m.contains("shipped h 0 "),
            "unexpected placement difference: {m}"
        );
    }
    println!(
        "all: {} tiles, same pixels {}, same placement {}",
        total.0, total.1, total.2
    );
}

/// A plain rectangle painted over each orientation's whole template: where
/// it lands relative to the cell's floor diamond (cell space: diamond
/// (0,0)-(160,80), the bottom corner at y = 80), next to where the lowest
/// opaque pixel of shipped tiles of that orientation is. Report only.
#[test]
fn a_full_template_rectangle_against_shipped_walls() {
    let files = dt1s();
    let mut lowest: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for (_, dt1) in &files {
        for t in &dt1.tiles {
            let Ok(img) = t.render() else { continue };
            if let Some(max_y) = opaque(&img).iter().map(|p| p.1).max() {
                // anchor space -> cell space: walls hang off the bottom
                // corner (y + 80), floors are at (0,0), roofs lifted
                let shift = match t.orientation {
                    0 => 0,
                    15 => -i32::from(t.roof_height),
                    _ => 80,
                };
                lowest.entry(t.orientation).or_default().push(max_y + shift);
            }
        }
    }
    for o in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19] {
        let shape = match o {
            0 => TileShape::floor(),
            15 => TileShape::roof(80),
            _ => TileShape::wall(o as u8, 128),
        };
        let tpl = template(shape).unwrap();
        let shift = match o {
            0 => 0,
            15 => -80,
            _ => 80,
        };
        let (x0, x1) = (tpl.rect.x, tpl.rect.x + tpl.rect.width as i32 - 1);
        let (y0, y1) = (
            tpl.rect.y + shift,
            tpl.rect.y + tpl.rect.height as i32 - 1 + shift,
        );
        let ship = lowest.get(&o).map(|v| {
            let mut v = v.clone();
            v.sort_unstable();
            (v[v.len() / 2], v[v.len() * 95 / 100], v.len())
        });
        println!(
            "o{o:2}: rectangle x {x0}..{x1} y {y0}..{y1} (cell space); shipped lowest pixel: {}",
            ship.map_or("-".to_string(), |(m, p95, n)| format!(
                "median y {m}, 95% y {p95} ({n} tiles)"
            ))
        );
    }
}
