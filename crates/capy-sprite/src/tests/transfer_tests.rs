use std::collections::BTreeMap;

use d2_formats::IndexedImage;

use crate::error::SpriteError;
use crate::image_io::export_png;
use crate::model::{Sprite, SpriteFormat};
use crate::quantize::{QuantizeOptions, to_rgba};
use crate::testutil::{small_sprite, synthetic_palette};
use crate::transfer::{
    ExportMeta, META_FILE, export_frames, export_sheet, import_frames, import_frames_subset,
    import_sheet, natural_cmp,
};

/// Options that keep every palette colour (no colour keying).
fn exact() -> QuantizeOptions {
    QuantizeOptions {
        transparent_rgb: None,
        ..QuantizeOptions::default()
    }
}

fn on_screen(img: &IndexedImage) -> BTreeMap<(i32, i32), u8> {
    let mut out = BTreeMap::new();
    for y in 0..img.height {
        for x in 0..img.width {
            let i = (y * img.width + x) as usize;
            if img.alpha[i] != 0 && img.pixels[i] != 0 {
                out.insert(
                    (img.offset_x + x as i32, img.offset_y + y as i32),
                    img.pixels[i],
                );
            }
        }
    }
    out
}

#[test]
fn frames_round_trip_with_exact_offsets() {
    let pal = synthetic_palette();
    let sprite = small_sprite(3, 4, 6);
    let dir = tempfile::tempdir().unwrap();
    let meta = export_frames(
        &sprite,
        SpriteFormat::Dcc,
        &pal,
        "act1",
        Some("a/b.dcc"),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        (
            meta.version,
            meta.format,
            meta.directions,
            meta.frames_per_direction
        ),
        (1, SpriteFormat::Dcc, 3, 4)
    );
    assert_eq!(meta.palette, "act1");
    assert_eq!(meta.source.as_deref(), Some("a/b.dcc"));
    assert_eq!(meta.frames.len(), 12);
    assert_eq!(meta.frames[0].file, "d00_f000.png");
    assert_eq!(meta.frames[11].file, "d02_f003.png");
    assert!(dir.path().join("d01_f002.png").is_file());
    assert!(dir.path().join(META_FILE).is_file());
    // the offsets in the json are the frames' offsets
    for fm in &meta.frames {
        let img = &sprite.directions[fm.dir as usize][fm.frame as usize];
        assert_eq!((fm.offset_x, fm.offset_y), (img.offset_x, img.offset_y));
    }

    let (back, got_meta) = import_frames(dir.path(), &pal, &exact()).unwrap();
    assert_eq!(back, sprite);
    assert_eq!(got_meta, Some(meta));
}

#[test]
fn json_offsets_beat_everything_else() {
    // edit the offsets in sprite.json and they come back
    let pal = synthetic_palette();
    let sprite = small_sprite(1, 2, 3);
    let dir = tempfile::tempdir().unwrap();
    let mut meta = export_frames(&sprite, SpriteFormat::Dc6, &pal, "", None, dir.path()).unwrap();
    meta.frames[1].offset_x = 1234;
    meta.frames[1].offset_y = -4321;
    std::fs::write(
        dir.path().join(META_FILE),
        serde_json::to_vec(&meta).unwrap(),
    )
    .unwrap();
    let (back, _) = import_frames(dir.path(), &pal, &exact()).unwrap();
    assert_eq!(
        (
            back.directions[0][1].offset_x,
            back.directions[0][1].offset_y
        ),
        (1234, -4321)
    );
}

#[test]
fn empty_frames_are_exported_as_one_transparent_pixel() {
    let pal = synthetic_palette();
    let mut sprite = small_sprite(1, 2, 3);
    sprite.directions[0][1] = IndexedImage::new(0, 0, 5, -6);
    let dir = tempfile::tempdir().unwrap();
    export_frames(&sprite, SpriteFormat::Dc6, &pal, "", None, dir.path()).unwrap();
    let (back, _) = import_frames(dir.path(), &pal, &exact()).unwrap();
    let f = &back.directions[0][1];
    assert_eq!((f.width, f.height, f.offset_x, f.offset_y), (1, 1, 5, -6));
    assert_eq!(f.alpha, vec![0]);
}

#[test]
fn folder_without_json_imports_in_natural_order_with_centred_anchors() {
    let pal = synthetic_palette();
    let dir = tempfile::tempdir().unwrap();
    let sprite = small_sprite(1, 12, 4);
    for (i, img) in sprite.directions[0].iter().enumerate() {
        // not zero padded: f10 must sort after f2
        let name = format!("frame{}.png", i + 1);
        std::fs::write(
            dir.path().join(name),
            export_png(&to_rgba(img, &pal)).unwrap(),
        )
        .unwrap();
    }
    std::fs::write(dir.path().join("notes.txt"), b"ignored").unwrap();
    let (back, meta) = import_frames(dir.path(), &pal, &exact()).unwrap();
    assert!(meta.is_none());
    assert_eq!(back.directions.len(), 1);
    assert_eq!(back.directions[0].len(), 12);
    for (got, want) in back.directions[0].iter().zip(&sprite.directions[0]) {
        assert_eq!((got.width, got.height), (want.width, want.height));
        assert_eq!(got.pixels, want.pixels);
        assert_eq!(
            got.offset_x,
            -((want.width / 2) as i32),
            "bottom-centre anchor"
        );
        assert_eq!(got.offset_y, -(want.height as i32 - 1));
    }
}

#[test]
fn empty_or_hostile_folders_are_errors() {
    let pal = synthetic_palette();
    let dir = tempfile::tempdir().unwrap();
    assert!(
        import_frames(dir.path(), &pal, &exact()).is_err(),
        "no images"
    );
    assert!(import_frames(&dir.path().join("missing"), &pal, &exact()).is_err());

    let sprite = small_sprite(1, 1, 2);
    let mut meta = export_frames(&sprite, SpriteFormat::Dc6, &pal, "", None, dir.path()).unwrap();
    for evil in [
        "../x.png",
        "sub/x.png",
        "..\\x.png",
        "/etc/passwd",
        "C:\\x.png",
    ] {
        meta.frames[0].file = evil.to_owned();
        std::fs::write(
            dir.path().join(META_FILE),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
        assert!(
            matches!(
                import_frames(dir.path(), &pal, &exact()),
                Err(SpriteError::Invalid(_))
            ),
            "{evil}"
        );
    }
    // inconsistent counts, duplicates, bad json, a missing PNG
    let mut bad = meta.clone();
    bad.frames[0].file = "d00_f000.png".to_owned();
    bad.directions = 5;
    std::fs::write(
        dir.path().join(META_FILE),
        serde_json::to_vec(&bad).unwrap(),
    )
    .unwrap();
    assert!(import_frames(dir.path(), &pal, &exact()).is_err());
    let mut bad = meta.clone();
    bad.frames[0].file = "d00_f000.png".to_owned();
    bad.frames[0].frame = 7;
    std::fs::write(
        dir.path().join(META_FILE),
        serde_json::to_vec(&bad).unwrap(),
    )
    .unwrap();
    assert!(import_frames(dir.path(), &pal, &exact()).is_err());
    std::fs::write(dir.path().join(META_FILE), b"{not json").unwrap();
    assert!(import_frames(dir.path(), &pal, &exact()).is_err());
    let mut gone = meta;
    gone.frames[0].file = "nope.png".to_owned();
    std::fs::write(
        dir.path().join(META_FILE),
        serde_json::to_vec(&gone).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        import_frames(dir.path(), &pal, &exact()),
        Err(SpriteError::Io { .. })
    ));
}

#[test]
fn ragged_sprites_cannot_be_exported() {
    let pal = synthetic_palette();
    let mut sprite = small_sprite(2, 3, 2);
    sprite.directions[1].pop();
    let dir = tempfile::tempdir().unwrap();
    assert!(export_frames(&sprite, SpriteFormat::Dc6, &pal, "", None, dir.path()).is_err());
    assert!(export_sheet(&sprite, &pal).is_err());
}

#[test]
fn sheet_round_trip_keeps_every_pixel_where_it_was() {
    let pal = synthetic_palette();
    let sprite = small_sprite(3, 5, 7);
    let (png, meta) = export_sheet(&sprite, &pal).unwrap();
    let (min_x, min_y, max_x, max_y) = sprite.bounds().unwrap();
    let sheet = crate::image_io::import_image(&png, crate::image_io::ImageKind::Png).unwrap();
    assert_eq!(sheet.width, (max_x - min_x) as u32 * 5);
    assert_eq!(sheet.height, (max_y - min_y) as u32 * 3);
    assert_eq!((meta.directions, meta.frames_per_direction), (3, 5));
    assert_eq!(meta.frames.len(), 15);
    assert_eq!(
        (meta.frames[0].offset_x, meta.frames[0].offset_y),
        (min_x, min_y)
    );

    for with_meta in [true, false] {
        let back = import_sheet(&png, 3, 5, &pal, &exact(), with_meta.then_some(&meta)).unwrap();
        assert_eq!((back.directions.len(), back.frames_per_direction()), (3, 5));
        for (d, (got_dir, want_dir)) in back.directions.iter().zip(&sprite.directions).enumerate() {
            for (f, (got, want)) in got_dir.iter().zip(want_dir).enumerate() {
                if with_meta {
                    // full cells at the original anchor position render the same
                    assert_eq!(
                        (got.width, got.height, got.offset_x, got.offset_y),
                        ((max_x - min_x) as u32, (max_y - min_y) as u32, min_x, min_y)
                    );
                    assert_eq!(on_screen(got), on_screen(want), "dir {d} frame {f}");
                } else {
                    assert_eq!(got.offset_x, -((got.width / 2) as i32));
                }
            }
        }
    }
}

#[test]
fn sheet_errors() {
    let pal = synthetic_palette();
    let sprite = small_sprite(2, 2, 2);
    let (png, _) = export_sheet(&sprite, &pal).unwrap();
    assert!(import_sheet(&png, 0, 2, &pal, &exact(), None).is_err());
    assert!(import_sheet(&png, 2, 0, &pal, &exact(), None).is_err());
    assert!(
        import_sheet(&png, 2, 3, &pal, &exact(), None).is_err(),
        "width not divisible"
    );
    assert!(import_sheet(b"not a png", 2, 2, &pal, &exact(), None).is_err());
    let blank = Sprite {
        directions: vec![vec![IndexedImage::new(0, 0, 0, 0)]],
    };
    assert!(export_sheet(&blank, &pal).is_err());
    assert!(export_sheet(&Sprite::default(), &pal).is_err());
}

#[test]
fn meta_json_shape_is_stable() {
    let pal = synthetic_palette();
    let dir = tempfile::tempdir().unwrap();
    export_frames(
        &small_sprite(1, 1, 2),
        SpriteFormat::Dc6,
        &pal,
        "act2",
        None,
        dir.path(),
    )
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(META_FILE)).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["format"], "Dc6");
    assert_eq!(json["palette"], "act2");
    assert_eq!(json["frames"][0]["file"], "d00_f000.png");
    let parsed: ExportMeta = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.frames.len(), 1);
}

#[test]
fn natural_order() {
    let mut names = vec!["f10", "f2", "f1", "F3", "f02", "a", "f1a"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["a", "f1", "f1a", "f2", "f02", "F3", "f10"]);
}

/// Exported frames carry alpha, so a `sprite.json` import never colour-keys:
/// opaque pure black survives even with the default (black-keying) options.
#[test]
fn sprite_json_import_keeps_opaque_black() {
    let mut pal = crate::testutil::synthetic_palette();
    pal.colors[7] = [0, 0, 0];
    let px = vec![7, 2, 0, 7, 7, 3];
    let sprite = Sprite {
        directions: vec![vec![crate::testutil::image(3, 2, -1, -2, px.clone())]],
    };
    let dir = tempfile::tempdir().unwrap();
    export_frames(&sprite, SpriteFormat::Dc6, &pal, "", None, dir.path()).unwrap();
    let keyed = QuantizeOptions::default();
    assert_eq!(keyed.transparent_rgb, Some([0, 0, 0]));

    let (back, meta) = import_frames(dir.path(), &pal, &keyed).unwrap();
    assert!(meta.is_some());
    assert_eq!(back, sprite, "black index 7 stays opaque");

    // Without sprite.json the options apply as given: black is keyed out.
    std::fs::remove_file(dir.path().join(META_FILE)).unwrap();
    let (loose, meta) = import_frames(dir.path(), &pal, &keyed).unwrap();
    assert!(meta.is_none());
    let alpha = &loose.directions[0][0].alpha;
    let expect: Vec<u8> = px
        .iter()
        .map(|&p| if p == 0 || p == 7 { 0 } else { 255 })
        .collect();
    assert_eq!(alpha, &expect);
}

/// Same rule for sheets: with the export metadata, `import_sheet` does not
/// colour-key, so opaque black survives; without it, black is keyed out.
#[test]
fn sheet_import_with_meta_keeps_opaque_black() {
    let mut pal = crate::testutil::synthetic_palette();
    pal.colors[7] = [0, 0, 0];
    let px = vec![7, 2, 0, 7, 7, 3];
    let sprite = Sprite {
        directions: vec![vec![crate::testutil::image(3, 2, -1, -2, px.clone())]],
    };
    let (png, meta) = export_sheet(&sprite, &pal).unwrap();
    let keyed = QuantizeOptions::default();

    let back = import_sheet(&png, 1, 1, &pal, &keyed, Some(&meta)).unwrap();
    let frame = &back.directions[0][0];
    assert_eq!(frame.pixels, px, "black index 7 stays opaque");
    let opaque: Vec<u8> = px.iter().map(|&p| if p == 0 { 0 } else { 255 }).collect();
    assert_eq!(frame.alpha, opaque);

    let loose = import_sheet(&png, 1, 1, &pal, &keyed, None).unwrap();
    let keyed_alpha: Vec<u8> = px
        .iter()
        .map(|&p| if p == 0 || p == 7 { 0 } else { 255 })
        .collect();
    assert_eq!(loose.directions[0][0].alpha, keyed_alpha);
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn subset_import_keeps_offsets_and_closes_gaps() {
    let pal = synthetic_palette();
    let sprite = small_sprite(3, 4, 6);
    let dir = tempfile::tempdir().unwrap();
    let full = export_frames(&sprite, SpriteFormat::Dcc, &pal, "act1", None, dir.path()).unwrap();

    // Direction 1 is not picked; frames come back in (dir, frame) order
    // whatever the order of the names, and a repeated name counts once.
    let picked = names(&[
        "d02_f003.png",
        "d00_f001.png",
        "D02_F000.PNG",
        "d00_f000.png",
        "d00_f001.png",
    ]);
    let (back, meta) = import_frames_subset(dir.path(), &picked, &pal, &exact()).unwrap();
    let want = Sprite {
        directions: vec![
            vec![
                sprite.directions[0][0].clone(),
                sprite.directions[0][1].clone(),
            ],
            vec![
                sprite.directions[2][0].clone(),
                sprite.directions[2][3].clone(),
            ],
        ],
    };
    assert_eq!(back, want, "pixels and sprite.json offsets");
    let meta = meta.unwrap();
    assert_eq!((meta.directions, meta.frames_per_direction), (3, 4));
    assert_eq!(meta.palette, full.palette);
    let origin: Vec<(u32, u32)> = meta.frames.iter().map(|f| (f.dir, f.frame)).collect();
    assert_eq!(origin, [(0, 0), (0, 1), (2, 0), (2, 3)]);
    assert_eq!(meta.frames[2].file, "d02_f000.png");

    // Picking every file gives what `import_frames` gives.
    let every: Vec<String> = full.frames.iter().map(|f| f.file.clone()).collect();
    let (all, all_meta) = import_frames_subset(dir.path(), &every, &pal, &exact()).unwrap();
    assert_eq!(all, sprite);
    assert_eq!(all_meta, Some(full));

    // A ragged pick is allowed but cannot be encoded as is.
    let ragged = names(&["d00_f000.png", "d01_f000.png", "d01_f001.png"]);
    let (back, _) = import_frames_subset(dir.path(), &ragged, &pal, &exact()).unwrap();
    let lens: Vec<usize> = back.directions.iter().map(Vec::len).collect();
    assert_eq!(lens, [1, 2]);
    assert!(back.encode(SpriteFormat::Dc6).is_err());
}

#[test]
fn subset_import_with_sprite_json_does_not_colour_key() {
    let mut pal = crate::testutil::synthetic_palette();
    pal.colors[7] = [0, 0, 0];
    let px = vec![7, 2, 0, 7, 7, 3];
    let sprite = Sprite {
        directions: vec![vec![
            crate::testutil::image(3, 2, -1, -2, px.clone()),
            crate::testutil::image(3, 2, 4, 5, px.clone()),
        ]],
    };
    let dir = tempfile::tempdir().unwrap();
    export_frames(&sprite, SpriteFormat::Dc6, &pal, "", None, dir.path()).unwrap();
    let keyed = QuantizeOptions::default();
    let (back, meta) =
        import_frames_subset(dir.path(), &names(&["d00_f001.png"]), &pal, &keyed).unwrap();
    assert!(meta.is_some());
    assert_eq!(back.directions, vec![vec![sprite.directions[0][1].clone()]]);

    // Without sprite.json: the given order, centred anchors, opts as given.
    std::fs::remove_file(dir.path().join(META_FILE)).unwrap();
    let (loose, meta) = import_frames_subset(
        dir.path(),
        &names(&["d00_f001.png", "d00_f000.png"]),
        &pal,
        &keyed,
    )
    .unwrap();
    assert!(meta.is_none());
    assert_eq!(loose.directions.len(), 1);
    assert_eq!(loose.directions[0].len(), 2);
    let mut centred = loose.directions[0][0].clone();
    crate::ops::center_offset(&mut centred);
    assert_eq!(loose.directions[0][0], centred);
    let expect: Vec<u8> = px
        .iter()
        .map(|&p| if p == 0 || p == 7 { 0 } else { 255 })
        .collect();
    assert_eq!(loose.directions[0][0].alpha, expect, "black is keyed out");
}

#[test]
fn subset_import_errors() {
    let pal = synthetic_palette();
    let dir = tempfile::tempdir().unwrap();
    export_frames(
        &small_sprite(2, 2, 4),
        SpriteFormat::Dc6,
        &pal,
        "",
        None,
        dir.path(),
    )
    .unwrap();
    let import = |files: &[&str]| import_frames_subset(dir.path(), &names(files), &pal, &exact());
    assert!(matches!(import(&[]), Err(SpriteError::Invalid(_))));
    let unlisted = import(&["d05_f000.png"]).unwrap_err();
    assert!(
        unlisted.to_string().contains("not a frame listed"),
        "{unlisted}"
    );
    for hostile in ["../d00_f000.png", r"sub\d00_f000.png", "a/b.png", ""] {
        let e = import(&[hostile]).unwrap_err();
        assert!(
            e.to_string().contains("plain file name"),
            "{hostile:?}: {e}"
        );
    }
    // Listed but deleted from the folder.
    std::fs::remove_file(dir.path().join("d01_f001.png")).unwrap();
    assert!(matches!(
        import(&["d01_f001.png"]),
        Err(SpriteError::Io { .. })
    ));
    // Without sprite.json the names must still be plain images.
    std::fs::remove_file(dir.path().join(META_FILE)).unwrap();
    assert!(import(&["d00_f000.png"]).is_ok());
    assert!(import(&["sprite.txt"]).is_err());
    assert!(import(&[r"..\d00_f000.png"]).is_err());
}
