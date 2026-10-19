//! Optional smoke test against the private corpus of real maps (gitignored).
//! Prints `skipped` when the corpus is not on disk.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use capy_edit::{CellRect, CellValue, Document, LayerRef, MoveMode, SelectOp, Selection};
use d2_formats::ds1::Ds1;

const MAX_FILES: usize = 50;

fn corpus_root() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|a| a.join("testdata").join("private").join("extracted"))
        .find(|p| p.is_dir())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ds1"))
        {
            out.push(path);
        }
    }
}

#[test]
fn edit_and_undo_restores_real_maps_byte_for_byte() {
    let Some(root) = corpus_root() else {
        eprintln!("skipped: no private corpus on disk");
        return;
    };
    let mut files = Vec::new();
    for set in ["vanilla", "mxl"] {
        let mut found = Vec::new();
        collect(
            &root.join(set).join("data").join("global").join("tiles"),
            &mut found,
        );
        // spread the sample over the whole tree
        let step = (found.len() / (MAX_FILES / 2)).max(1);
        files.extend(found.into_iter().step_by(step).take(MAX_FILES / 2));
    }
    if files.is_empty() {
        eprintln!("skipped: corpus has no DS1 files");
        return;
    }

    let mut checked = 0;
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let Ok(ds1) = Ds1::parse(&bytes) else {
            continue;
        };
        if ds1.to_bytes().ok().as_deref() != Some(bytes.as_slice()) {
            continue; // d2-formats' own round-trip guarantee is tested there
        }
        let name = path.display();
        let mut doc = Document::new(ds1);
        assert!(!doc.is_dirty());

        // paint + undo
        let layer = *LayerRef::all_in(doc.ds1())
            .iter()
            .find(|l| matches!(l, LayerRef::Floor(_) | LayerRef::Shadow))
            .unwrap_or(&LayerRef::Shadow);
        let value = CellValue {
            cell: 0x0010_0101,
            orientation: 0,
        };
        let w = doc.ds1().width;
        let h = doc.ds1().height;
        if w > 0 && h > 0 {
            doc.paint(layer, &[(w - 1, h - 1, value), (0, 0, value)])
                .unwrap();
        }
        // move everything on the first wall to a layer that may be new, resize, upgrade
        let mut sel = Selection::new();
        if w > 0 && h > 0 {
            sel.box_select(
                doc.ds1(),
                CellRect::new((0, 0), (w - 1, h - 1)),
                &LayerRef::all_in(doc.ds1()),
                false,
                SelectOp::Replace,
            );
        }
        let _ = doc.cut(&sel);
        let _ = doc.paste(&doc.copy(&sel), (1, 1), capy_edit::PasteMode::Overwrite);
        if !doc.ds1().walls.is_empty() {
            let _ = doc.move_to_layer(
                &sel,
                LayerRef::Wall(0),
                LayerRef::Wall(3),
                MoveMode::Overwrite,
            );
        }
        let _ = doc.resize(w + 3, h.saturating_sub(1).max(1));
        let _ = doc.upgrade_version(doc.ds1().version + 1);
        if let Ok(bytes) = doc.ds1().to_bytes() {
            assert_eq!(
                Ds1::parse(&bytes).unwrap(),
                *doc.ds1(),
                "{name}: edited map re-parses"
            );
        }

        while doc.undo() {}
        assert!(!doc.is_dirty(), "{name}");
        let back = doc.ds1().to_bytes().unwrap();
        assert!(
            back == bytes,
            "{name}: undo-all must restore the file bytes"
        );
        checked += 1;
    }
    eprintln!("checked {checked} of {} sampled maps", files.len());
    assert!(checked > 0);
}
