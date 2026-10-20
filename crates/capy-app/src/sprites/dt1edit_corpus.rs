//! Corpus check of the DT1 editor document: on real tile sets, open then
//! save is byte-identical and one painted pixel changes exactly that pixel.
//! Skipped without the private corpus (`CAPY_PRIVATE_DIR`).
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::path::{Path, PathBuf};

use d2_formats::dt1::Dt1;

use super::dt1edit::Dt1Doc;
use super::dt1view::Dt1View;

fn dt1_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            dt1_files(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dt1")) {
            out.push(p);
        }
    }
}

#[test]
fn real_tile_sets_round_trip_and_take_one_edit() {
    let root = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .map(|a| a.join("testdata").join("private"))
                .find(|p| p.is_dir())
        });
    let Some(root) = root else {
        eprintln!("skipped: no private corpus");
        return;
    };
    let mut files = Vec::new();
    dt1_files(&root.join("extracted").join("vanilla"), &mut files);
    if files.is_empty() {
        eprintln!("skipped: no vanilla DT1 files");
        return;
    }
    let step = (files.len() / 12).max(1);
    let mut checked = 0;
    for path in files.iter().step_by(step) {
        let bytes = std::fs::read(path).unwrap();
        let Ok(view) = Dt1View::load(&bytes) else {
            continue; // an undocumented version
        };
        let mut d = Dt1Doc::from_view(&view);
        let (out, changed) = d.save(&d.frames()).unwrap();
        assert!(changed == 0 && out == bytes, "{}", path.display());
        // Paint the first editable pixel of the first tile that has one.
        let masks = d.editable();
        let Some((t, i)) = masks
            .iter()
            .enumerate()
            .find_map(|(t, m)| m.bits.iter().position(|&b| b).map(|i| (t, i)))
        else {
            continue;
        };
        let mut frames = d.frames();
        let img = &mut frames[0][t];
        let was = img.pixels[i];
        img.pixels[i] = if was == 200 { 201 } else { 200 };
        img.alpha[i] = 255;
        let painted = img.pixels[i];
        let (out, changed) = d.save(&frames).unwrap();
        assert_eq!(changed, 1, "{}", path.display());
        let (before, after) = (Dt1::parse(&bytes).unwrap(), Dt1::parse(&out).unwrap());
        for (n, (a, b)) in before.tiles.iter().zip(&after.tiles).enumerate() {
            if n == t {
                let (ra, rb) = (a.render().unwrap(), b.render().unwrap());
                let diff = (0..ra.pixels.len())
                    .filter(|&k| (ra.pixels[k], ra.alpha[k]) != (rb.pixels[k], rb.alpha[k]))
                    .count();
                assert_eq!(diff, 1, "{} tile {n}", path.display());
                assert_eq!(rb.pixels[i], painted);
            } else {
                assert_eq!(a, b, "{} tile {n}", path.display());
            }
        }
        checked += 1;
    }
    eprintln!("round-tripped {checked} real tile sets");
}
