//! P11 tile tools over the private corpus (skipped when it is absent).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use capy_mod::tiles_actsafe::{ActSafeOptions, count_unsafe_pixels, make_act_safe, tile_histogram};
use capy_mod::tiles_dt1::{
    HueOptions, copy_tiles, duplicate_dt1, edit_block_pixels, hue_remap, identity_remap,
    recolor_tiles,
};
use capy_mod::tiles_flags::{FlagOp, edit_subtile_flags};
use capy_mod::tiles_settings::{apply_ini, dt1_to_ini, parse_dt1_ini, read_settings};
use d2_formats::dt1::Dt1;
use d2_formats::palette::Palette;
use d2_formats::remap::{SpriteKind, remap_indices};

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

/// Every parseable, writable DT1 of the corpus (a sample unless
/// `CAPY_FULL_CORPUS=1`), with its bytes and model.
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
            let Ok(dt1) = Dt1::parse(&bytes) else {
                continue;
            };
            if dt1.to_bytes().ok().as_deref() != Some(&bytes[..]) {
                continue;
            }
            out.push((p, bytes, dt1));
        }
    }
    out
}

fn first_palette() -> Option<Palette> {
    for root in corpus_roots() {
        let p = root.join("data/global/palette/act1/pal.dat");
        if let Ok(b) = std::fs::read(p) {
            return Palette::parse_dat(&b).ok();
        }
    }
    None
}

fn diff_offsets(a: &[u8], b: &[u8]) -> Vec<usize> {
    assert_eq!(a.len(), b.len(), "size changed");
    (0..a.len()).filter(|&i| a[i] != b[i]).collect()
}

#[test]
fn flag_edit_changes_only_the_flag_bytes() {
    let files = dt1s();
    if files.is_empty() {
        return;
    }
    let mut checked = 0;
    for (path, bytes, dt1) in &files {
        if dt1.tiles.is_empty() {
            continue;
        }
        let tile = dt1.tiles.len() / 2;
        let mut edited = dt1.clone();
        let changes =
            edit_subtile_flags(&mut edited, &[tile], &[0, 6, 24], FlagOp::Add(0x05)).unwrap();
        let out = edited.to_bytes().unwrap();
        let diff = diff_offsets(bytes, &out);
        assert_eq!(diff.len(), changes.len(), "{}", path.display());
        let header = 276 + tile * 96;
        assert!(
            diff.iter()
                .all(|&o| (header + 40..header + 65).contains(&o)),
            "{}: bytes outside the flag block changed",
            path.display()
        );
        // and it parses back to the edited model
        assert_eq!(Dt1::parse(&out).unwrap(), edited);
        // undo
        let mut back = edited.clone();
        edit_subtile_flags(&mut back, &[tile], &[0, 6, 24], FlagOp::Set(0)).ok();
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn duplicate_equals_source_tile_for_tile() {
    for (path, bytes, dt1) in dt1s() {
        let copy = duplicate_dt1(&dt1);
        assert_eq!(copy, dt1, "{}", path.display());
        assert_eq!(copy.to_bytes().unwrap(), bytes);
        let all: Vec<usize> = (0..dt1.tiles.len()).collect();
        let picked = copy_tiles(&dt1, &all).unwrap();
        assert_eq!(picked.tiles, dt1.tiles);
    }
}

#[test]
fn recolour_matches_the_byte_patcher_and_only_touches_selected_tiles() {
    let mut table = identity_remap();
    for (i, v) in table.iter_mut().enumerate().skip(1) {
        *v = ((i * 37 + 11) % 255 + 1) as u8;
    }
    for (path, bytes, dt1) in dt1s() {
        // all tiles: identical to the whole-file patcher
        let mut all = dt1.clone();
        recolor_tiles(&mut all, None, &table).unwrap();
        let expect = remap_indices(SpriteKind::Dt1, &bytes, &table).unwrap();
        assert_eq!(all.to_bytes().unwrap(), expect, "{}", path.display());

        // identity changes nothing
        let mut same = dt1.clone();
        assert!(
            recolor_tiles(&mut same, None, &identity_remap())
                .unwrap()
                .is_empty()
        );
        assert_eq!(same, dt1);

        // a selection leaves every other tile untouched
        if dt1.tiles.len() >= 3 {
            let mut some = dt1.clone();
            recolor_tiles(&mut some, Some(&[1]), &table).unwrap();
            for (i, (a, b)) in some.tiles.iter().zip(&dt1.tiles).enumerate() {
                if i != 1 {
                    assert_eq!(a, b, "{} tile {i}", path.display());
                }
            }
            let out = some.to_bytes().unwrap();
            assert_eq!(out.len(), bytes.len());
        }
    }
}

#[test]
fn hue_recolour_runs_on_real_tiles() {
    let (Some(pal), files) = (first_palette(), dt1s()) else {
        return;
    };
    let table = hue_remap(
        &pal,
        &HueOptions {
            hue: 40.0,
            brightness: 0.8,
            ..HueOptions::default()
        },
    );
    assert_eq!(table[0], 0);
    assert!(table[1..].iter().all(|&v| v != 0));
    for (_, bytes, dt1) in files.into_iter().take(10) {
        let mut d = dt1.clone();
        recolor_tiles(&mut d, None, &table).unwrap();
        let out = d.to_bytes().unwrap();
        assert_eq!(out.len(), bytes.len());
        assert_eq!(Dt1::parse(&out).unwrap(), d);
    }
}

#[test]
fn act_safe_leaves_no_act_specific_colours() {
    let (Some(pal), files) = (first_palette(), dt1s()) else {
        return;
    };
    let opts = ActSafeOptions::default();
    let mut converted = 0;
    for (path, bytes, dt1) in files {
        let before = count_unsafe_pixels(&dt1, None, &opts.safe);
        let mut d = dt1.clone();
        let report = make_act_safe(&mut d, &pal, None, &opts).unwrap();
        assert_eq!(report.unsafe_pixels, before);
        let h = tile_histogram(&d, None);
        assert!(
            h[225..=254].iter().all(|&n| n == 0),
            "{}: indices 225-254 remain",
            path.display()
        );
        if before == 0 {
            assert!(report.tiles.is_empty());
            assert_eq!(d.to_bytes().unwrap(), bytes);
        } else {
            converted += 1;
            // geometry and transparency are unchanged
            for (a, b) in d.tiles.iter().zip(&dt1.tiles) {
                let (ia, ib) = (a.render().unwrap(), b.render().unwrap());
                assert_eq!(ia.alpha, ib.alpha, "{}", path.display());
            }
            assert_eq!(Dt1::parse(&d.to_bytes().unwrap()).unwrap(), d);
        }
    }
    eprintln!("act-safe converted {converted} DT1s");
}

#[test]
fn pixel_edit_changes_only_that_pixel() {
    for (path, _, dt1) in dt1s().into_iter().take(12) {
        let Some((t, b)) = dt1.tiles.iter().enumerate().find_map(|(t, tile)| {
            tile.blocks
                .iter()
                .position(|b| b.render().is_ok_and(|i| i.alpha.contains(&255)))
                .map(|b| (t, b))
        }) else {
            continue;
        };
        let mut d = dt1.clone();
        let original = dt1.tiles[t].blocks[b].render().unwrap();
        let at = original.alpha.iter().position(|&a| a != 0).unwrap();
        let ch = edit_block_pixels(&mut d, t, b, |img| {
            img.pixels[at] = if img.pixels[at] == 77 { 78 } else { 77 };
        })
        .unwrap();
        assert_eq!(ch.pixels, 1, "{}", path.display());
        let edited = d.tiles[t].blocks[b].render().unwrap();
        for i in 0..original.pixels.len() {
            if i != at {
                assert_eq!(
                    (edited.alpha[i], edited.pixels[i]),
                    (original.alpha[i], original.pixels[i])
                );
            }
        }
        for (i, (x, y)) in d.tiles.iter().zip(&dt1.tiles).enumerate() {
            if i != t {
                assert_eq!(x, y);
            }
        }
        assert_eq!(Dt1::parse(&d.to_bytes().unwrap()).unwrap(), d);
    }
}

#[test]
fn ini_export_import_is_lossless_for_settings() {
    for (path, _, dt1) in dt1s() {
        let text = dt1_to_ini(&dt1);
        let (blocks, count) = parse_dt1_ini(&text);
        assert_eq!(count, Some(dt1.tiles.len()));
        // apply onto a copy with scrambled settings: must restore them
        let mut scrambled = dt1.clone();
        for t in &mut scrambled.tiles {
            t.sound = t.sound.wrapping_add(1);
            t.subtile_flags = [0xAA; 25];
            t.rarity += 1;
        }
        let (_, missing) = apply_ini(&mut scrambled, &blocks);
        assert!(missing.is_empty());
        for (a, b) in scrambled.tiles.iter().zip(&dt1.tiles) {
            assert_eq!(read_settings(a), read_settings(b), "{}", path.display());
        }
        // importing the unchanged ini is a no-op
        let mut same = dt1.clone();
        let (changed, _) = apply_ini(&mut same, &blocks);
        assert!(changed.is_empty());
        assert_eq!(same, dt1);
    }
}
