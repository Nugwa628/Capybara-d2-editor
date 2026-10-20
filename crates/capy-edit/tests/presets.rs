//! Presets: save, place, undo, builder and suggestions (unit and corpus).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};

use capy_edit::presets::{
    BUILDER_SIZE, Preset, PresetBuilder, SuggestOptions, categories, place_preset, preview_place,
    sort_presets, suggest_presets, suggestion_groups,
};
use capy_edit::{
    Brush, CellRect, CellValue, Document, LayerRef, PasteMode, Rng, SelectOp, Selection,
};
use common::{cv, set, wall};
use d2_formats::ds1::{Ds1, Object};

fn map_with_building() -> Ds1 {
    let mut ds = Ds1::new(12, 12, 0);
    for x in 0..12 {
        for y in 0..12 {
            set(&mut ds, LayerRef::Floor(0), x, y, cv(1, 0));
        }
    }
    for x in 3..6 {
        set(&mut ds, LayerRef::Wall(0), x, 3, wall(2, 0, 1));
    }
    set(&mut ds, LayerRef::Wall(0), 3, 4, wall(2, 1, 2));
    set(&mut ds, LayerRef::Shadow, 4, 4, cv(2, 0));
    ds.objects.push(Object {
        kind: 2,
        id: 9,
        x: 16,
        y: 17,
        flags: 0,
    });
    ds
}

fn all_layers_selection(d: &Document, r: CellRect) -> Selection {
    let mut sel = Selection::new();
    let layers = LayerRef::all_in(d.ds1());
    sel.box_select(d.ds1(), r, &layers, false, SelectOp::Replace);
    sel
}

#[test]
fn save_place_undo_returns_the_original() {
    let ds = map_with_building();
    let mut d = Document::new(ds.clone());
    let sel = all_layers_selection(&d, CellRect::new((3, 3), (5, 4)));
    let preset = Preset::from_selection(&d, &sel, "House", "Mine").unwrap();
    assert_eq!((preset.width, preset.height), (3, 2));
    assert_eq!(
        preset.objects.len(),
        1,
        "the object stands on selected cell (3, 3)"
    );
    assert!(preset.path().starts_with("data/ds1studio/presets/house-"));

    let preview = preview_place(&d, &preset, (7, 7), PasteMode::Overwrite);
    assert!(preview.writes > 0);
    let before_depth = d.can_undo();
    place_preset(&mut d, &preset, (7, 7), PasteMode::Overwrite).unwrap();
    assert!(before_depth || d.can_undo());
    assert_eq!(d.undo_label(), Some("Place preset House"));
    assert_eq!(wall_at(&d, 7, 7), Some((2, 0)));
    assert_eq!(wall_at(&d, 7, 8), Some((2, 1)));
    d.undo();
    assert_eq!(d.ds1(), &ds);
    assert!(!d.can_undo());
    d.redo();
    assert_eq!(wall_at(&d, 7, 7), Some((2, 0)));
    // a second place and one undo only removes the second
    place_preset(&mut d, &preset, (0, 0), PasteMode::Overwrite).unwrap();
    d.undo();
    assert_eq!(wall_at(&d, 0, 0), None);
    assert_eq!(wall_at(&d, 7, 7), Some((2, 0)));
}

fn wall_at(d: &Document, x: u32, y: u32) -> Option<(u8, u8)> {
    let v = LayerRef::Wall(0).get(d.ds1(), x, y)?;
    capy_edit::tile_key(LayerRef::Wall(0), v).map(|k| (k.main, k.sub))
}

#[test]
fn objects_on_selected_cells_travel_with_the_preset() {
    let mut d = Document::new(map_with_building());
    // cell (3, 3) holds the sub-tile (16, 17) object
    let sel = all_layers_selection(&d, CellRect::new((3, 3), (3, 3)));
    let p = Preset::from_selection(&d, &sel, "Post", "Mine").unwrap();
    assert_eq!(p.objects.len(), 1);
    assert_eq!((p.objects[0].x, p.objects[0].y), (1, 2));
    place_preset(&mut d, &p, (8, 8), PasteMode::Overwrite).unwrap();
    assert!(d.ds1().objects.iter().any(|o| (o.x, o.y) == (41, 42)));
    d.undo();
    assert_eq!(d.ds1().objects.len(), 1);
}

#[test]
fn json_roundtrip_and_library_order() {
    let d = Document::new(map_with_building());
    let sel = all_layers_selection(&d, CellRect::new((3, 3), (3, 3)));
    let p = Preset::from_selection(&d, &sel, "Post", "Zed").unwrap();
    let back = Preset::from_json(&p.to_json()).unwrap();
    assert_eq!(back, p);
    assert!(Preset::from_json("{\"format\":\"other\"}").is_none());
    assert!(Preset::from_json("nonsense").is_none());
    let mut q = p.clone();
    q.name = "Arch".into();
    q.category = "Alpha".into();
    let mut lib = vec![p, q];
    sort_presets(&mut lib);
    assert_eq!(lib[0].name, "Arch");
    assert_eq!(categories(&lib), vec!["Alpha", "Zed"]);
}

#[test]
fn empty_selection_makes_no_preset() {
    let d = Document::new(map_with_building());
    assert!(Preset::from_selection(&d, &Selection::new(), "x", "y").is_none());
}

#[test]
fn builder_trims_to_what_was_drawn() {
    let mut b = PresetBuilder::new(0).unwrap();
    assert!(b.is_empty());
    assert!(b.build("x", "y", true).is_none());
    let w = LayerRef::Wall(1);
    b.document_mut()
        .paint_rect(
            w,
            CellRect::new((4, 5), (6, 5)),
            &[Brush {
                main: 2,
                sub: 3,
                orientation: 1,
            }],
            &mut Rng::new(1),
        )
        .unwrap();
    assert!(!b.is_empty());
    let trimmed = b.build("Wall", "Mine", true).unwrap();
    assert_eq!((trimmed.width, trimmed.height), (3, 1));
    let full = b.build("Wall", "Mine", false).unwrap();
    assert_eq!((full.width, full.height), (BUILDER_SIZE, BUILDER_SIZE));
    assert!(full.cells.iter().any(|c| c.1 == 4 && c.2 == 5));
    // placing the full one at (0, 0) draws at the same cells
    let mut d = Document::new(Ds1::new(24, 24, 0));
    place_preset(&mut d, &full, (0, 0), PasteMode::Overwrite).unwrap();
    assert!(
        LayerRef::Wall(1)
            .get(d.ds1(), 4, 5)
            .is_some_and(|v| v != CellValue::EMPTY)
    );
}

#[test]
fn suggestions_find_and_merge_recurring_structures() {
    let a = map_with_building();
    let mut b = map_with_building();
    // same building, different prop1 flags: still the same structure
    set(
        &mut b,
        LayerRef::Wall(0),
        3,
        3,
        CellValue {
            cell: wall(2, 0, 1).cell | 0x40,
            orientation: 1,
        },
    );
    let groups = suggestion_groups(&a);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 5, "4 wall cells and the shadow");
    let maps = vec![("a.ds1".to_string(), &a), ("b.ds1".to_string(), &b)];
    let found = suggest_presets(&maps, &SuggestOptions::default());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].occurrences, Some(2));
    assert_eq!(found[0].found_in.as_deref(), Some("a.ds1"));
    assert_eq!(found[0].name, "Structure 3\u{d7}2");
    // a map that cannot draw its tiles drops the suggestion
    let none = suggest_presets(
        &maps,
        &SuggestOptions {
            drawable: &|_, _, _| false,
            ..SuggestOptions::default()
        },
    );
    assert!(none.is_empty());
    // limit
    let one = suggest_presets(
        &maps,
        &SuggestOptions {
            limit: 0,
            ..SuggestOptions::default()
        },
    );
    assert!(one.is_empty());
}

// ----- corpus -------------------------------------------------------------------

fn corpus_root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CAPY_PRIVATE_DIR") {
        let p = PathBuf::from(p).join("extracted");
        if p.is_dir() {
            return Some(p);
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
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
    for p in entries {
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ds1")) {
            out.push(p);
        }
    }
}

#[test]
fn corpus_preset_save_place_undo_and_roundtrip() {
    let Some(root) = corpus_root() else {
        eprintln!("skipped: no private corpus on disk");
        return;
    };
    let full = std::env::var_os("CAPY_FULL_CORPUS").is_some_and(|v| v == "1");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let (mut placed, mut suggested) = (0, 0);
    let mut sample: Vec<(String, Ds1)> = Vec::new();
    for (n, path) in files.iter().enumerate() {
        if !full && n % 7 != 0 {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        let Ok(ds1) = Ds1::parse(&bytes) else {
            continue;
        };
        if ds1.to_bytes().ok().as_deref() != Some(bytes.as_slice()) {
            continue;
        }
        let name = path.display().to_string();
        let original = ds1.clone();
        let mut d = Document::new(ds1);
        // a block around the first wall cell, else the map's top-left
        let (w, h) = (d.ds1().width, d.ds1().height);
        let seed = d
            .ds1()
            .walls
            .first()
            .and_then(|l| {
                (0..h).find_map(|y| {
                    (0..w)
                        .find(|&x| l.tiles.get(x, y).is_some_and(|c| !c.is_empty()))
                        .map(|x| (x, y))
                })
            })
            .unwrap_or((0, 0));
        let r = CellRect::new(seed, ((seed.0 + 3).min(w - 1), (seed.1 + 3).min(h - 1)));
        let sel = all_layers_selection(&d, r);
        if let Some(p) = Preset::from_selection(&d, &sel, "Block", "Test") {
            assert_eq!(Preset::from_json(&p.to_json()).unwrap(), p, "{name}");
            let at = ((w as i32) / 2, (h as i32) / 2);
            if place_preset(&mut d, &p, at, PasteMode::Overwrite).is_ok() && d.can_undo() {
                let after = d.ds1().clone();
                // the placed map is still a writable DS1
                if let Ok(out) = after.to_bytes() {
                    let mut back = Ds1::parse(&out).unwrap();
                    back.preserved.missing = after.preserved.missing;
                    assert_eq!(back, after, "{name}: placed map");
                }
                assert!(d.undo());
                assert_eq!(d.ds1(), &original, "{name}: undo");
                assert!(d.redo());
                assert_eq!(d.ds1(), &after, "{name}: redo");
                assert!(d.undo());
                assert_eq!(d.ds1(), &original, "{name}: undo again");
                placed += 1;
            }
        }
        // the original still round-trips byte for byte
        assert_eq!(original.to_bytes().unwrap(), bytes, "{name}");
        if sample.len() < 25 {
            sample.push((name, original));
        }
    }
    let maps: Vec<(String, &Ds1)> = sample.iter().map(|(n, d)| (n.clone(), d)).collect();
    let found = suggest_presets(&maps, &SuggestOptions::default());
    for p in &found {
        assert!(p.width <= 16 && p.height <= 16);
        assert!(p.occurrences.unwrap_or(0) >= 1);
        suggested += 1;
    }
    assert!(
        found
            .windows(2)
            .all(|w| w[0].occurrences >= w[1].occurrences)
    );
    eprintln!("placed {placed} presets, {suggested} suggestions");
    assert!(placed > 0);
}
