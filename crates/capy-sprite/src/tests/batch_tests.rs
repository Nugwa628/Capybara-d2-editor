use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::batch::{BatchDirection, BatchOptions, batch_convert};
use crate::error::SpriteError;
use crate::image_io::export_png;
use crate::model::{Sprite, SpriteFormat};
use crate::quantize::{QuantizeOptions, to_rgba};
use crate::testutil::{small_sprite, synthetic_palette};
use crate::transfer::META_FILE;

fn exact() -> QuantizeOptions {
    QuantizeOptions {
        transparent_rgb: None,
        ..QuantizeOptions::default()
    }
}

fn opts(recursive: bool) -> BatchOptions {
    BatchOptions {
        recursive,
        quantize: exact(),
        split: None,
        overwrite: false,
    }
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

fn write_sprite(path: &Path, sprite: &Sprite, format: SpriteFormat) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, sprite.encode(format).unwrap()).unwrap();
}

fn run(
    input: &Path,
    output: &Path,
    dir: &BatchDirection,
    o: &BatchOptions,
) -> crate::error::Result<crate::batch::BatchReport> {
    batch_convert(
        input,
        output,
        dir,
        &synthetic_palette(),
        o,
        &mut |_, _, _| {},
        &AtomicBool::new(false),
    )
}

/// Builds `input/{a/x.dc6, a/b/y.dcc, z.dc6, notes.txt}`.
fn sprite_tree(root: &Path) -> (Sprite, Sprite, Sprite) {
    let (x, y, z) = (
        small_sprite(2, 3, 3),
        small_sprite(4, 2, 2),
        small_sprite(1, 5, 3),
    );
    write_sprite(&root.join("a/x.dc6"), &x, SpriteFormat::Dc6);
    write_sprite(&root.join("a/b/y.dcc"), &y, SpriteFormat::Dcc);
    write_sprite(&root.join("z.DC6"), &z, SpriteFormat::Dc6);
    std::fs::write(root.join("notes.txt"), b"ignore me").unwrap();
    (x, y, z)
}

#[test]
fn both_directions_round_trip_a_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, pngs, back) = (
        tmp.path().join("in"),
        tmp.path().join("pngs"),
        tmp.path().join("back"),
    );
    let (x, y, z) = sprite_tree(&input);
    let before = files_under(&input);

    let report = run(&input, &pngs, &BatchDirection::ToPng, &opts(true)).unwrap();
    assert_eq!(
        (report.converted, report.skipped, report.failed.len()),
        (3, 0, 0),
        "{report:?}"
    );
    assert_eq!(report.log.len(), 3);
    let out = files_under(&pngs);
    let rel = |s: &str| PathBuf::from(s);
    assert!(out.contains(&rel("a/x/sprite.json")));
    assert!(out.contains(&rel("a/x/d01_f002.png")));
    assert!(out.contains(&rel("a/b/y/d03_f001.png")));
    assert!(out.contains(&rel("z/d00_f004.png")));
    assert_eq!(out.len(), (6 + 8 + 5) + 3);
    assert_eq!(files_under(&input), before, "the input is untouched");

    let report = run(
        &pngs,
        &back,
        &BatchDirection::ToSprite(SpriteFormat::Dc6),
        &opts(true),
    )
    .unwrap();
    assert_eq!(
        (report.converted, report.failed.len()),
        (3, 0),
        "{report:?}"
    );
    let out = files_under(&back);
    assert_eq!(out, vec![rel("a/b/y.dc6"), rel("a/x.dc6"), rel("z.dc6")]);
    let load =
        |p: &str| Sprite::decode(&std::fs::read(back.join(p)).unwrap(), SpriteFormat::Dc6).unwrap();
    assert_eq!(load("a/x.dc6"), x);
    assert_eq!(load("a/b/y.dc6"), y, "DCC became DC6 with the same frames");
    assert_eq!(load("z.dc6"), z);

    // and into DCC
    let dcc_out = tmp.path().join("dcc");
    run(
        &pngs,
        &dcc_out,
        &BatchDirection::ToSprite(SpriteFormat::Dcc),
        &opts(true),
    )
    .unwrap();
    let got = Sprite::decode(
        &std::fs::read(dcc_out.join("a/x.dcc")).unwrap(),
        SpriteFormat::Dcc,
    )
    .unwrap();
    assert_eq!(got, x);
}

#[test]
fn lone_images_become_sprites_and_can_be_split() {
    let tmp = tempfile::tempdir().unwrap();
    let pal = synthetic_palette();
    let input = tmp.path().join("in");
    let img = small_sprite(1, 1, 5).directions[0][0].clone();
    std::fs::create_dir_all(input.join("sub")).unwrap();
    let png = export_png(&to_rgba(&img, &pal)).unwrap();
    std::fs::write(input.join("one.png"), &png).unwrap();
    std::fs::write(input.join("sub/two.PNG"), &png).unwrap();
    std::fs::write(input.join("broken.png"), b"not an image").unwrap();

    let out = tmp.path().join("out");
    let report = run(
        &input,
        &out,
        &BatchDirection::ToSprite(SpriteFormat::Dc6),
        &opts(true),
    )
    .unwrap();
    assert_eq!(
        (report.converted, report.failed.len()),
        (2, 1),
        "{report:?}"
    );
    assert_eq!(
        report.failed[0].0,
        input.canonicalize().unwrap().join("broken.png")
    );
    assert_eq!(
        files_under(&out),
        vec![PathBuf::from("one.dc6"), PathBuf::from("sub/two.dc6")]
    );
    let s = Sprite::decode(
        &std::fs::read(out.join("one.dc6")).unwrap(),
        SpriteFormat::Dc6,
    )
    .unwrap();
    assert_eq!((s.directions.len(), s.frames_per_direction()), (1, 1));
    let f = &s.directions[0][0];
    assert_eq!(f.pixels, img.pixels);
    assert_eq!(
        f.offset_x,
        -((img.width / 2) as i32),
        "bottom-centre anchor"
    );

    // split into 4x4 tiles
    let out2 = tmp.path().join("out2");
    let o = BatchOptions {
        split: Some((4, 4)),
        ..opts(false)
    };
    let report = run(
        &input,
        &out2,
        &BatchDirection::ToSprite(SpriteFormat::Dcc),
        &o,
    )
    .unwrap();
    assert_eq!(report.converted, 1, "non-recursive: sub/ is not visited");
    let s = Sprite::decode(
        &std::fs::read(out2.join("one.dcc")).unwrap(),
        SpriteFormat::Dcc,
    )
    .unwrap();
    let tiles = (img.width.div_ceil(4) * img.height.div_ceil(4)) as usize;
    assert_eq!(s.frames_per_direction(), tiles);
    assert!(tiles > 1);
}

#[test]
fn existing_targets_are_skipped_unless_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, out) = (tmp.path().join("in"), tmp.path().join("out"));
    sprite_tree(&input);
    run(&input, &out, &BatchDirection::ToPng, &opts(true)).unwrap();
    let marker = out.join("z/marker.txt");
    std::fs::write(&marker, b"x").unwrap();
    let report = run(&input, &out, &BatchDirection::ToPng, &opts(true)).unwrap();
    assert_eq!((report.converted, report.skipped), (0, 3));
    let o = BatchOptions {
        overwrite: true,
        ..opts(true)
    };
    let report = run(&input, &out, &BatchDirection::ToPng, &o).unwrap();
    assert_eq!((report.converted, report.skipped), (3, 0));
}

#[test]
fn same_stem_in_one_folder_does_not_collide() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, out) = (tmp.path().join("in"), tmp.path().join("out"));
    let s = small_sprite(1, 2, 2);
    write_sprite(&input.join("m.dc6"), &s, SpriteFormat::Dc6);
    write_sprite(&input.join("m.dcc"), &s, SpriteFormat::Dcc);
    let report = run(&input, &out, &BatchDirection::ToPng, &opts(false)).unwrap();
    assert_eq!(
        (report.converted, report.failed.len()),
        (2, 0),
        "{report:?}"
    );
    assert!(out.join("m").join(META_FILE).is_file());
    assert!(out.join("m.dcc").join(META_FILE).is_file());
}

#[test]
fn output_inside_input_is_refused_and_nothing_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("in");
    sprite_tree(&input);
    let before = files_under(&input);
    for out in [input.clone(), input.join("deeper"), input.join("a/../q")] {
        let err = run(&input, &out, &BatchDirection::ToPng, &opts(true)).unwrap_err();
        assert!(matches!(err, SpriteError::Invalid(_)), "{err}");
    }
    assert_eq!(files_under(&input), before);
    // an input folder that is not a folder
    assert!(
        run(
            &input.join("z.DC6"),
            &tmp.path().join("o"),
            &BatchDirection::ToPng,
            &opts(true)
        )
        .is_err()
    );
    assert!(
        run(
            &tmp.path().join("nope"),
            &tmp.path().join("o"),
            &BatchDirection::ToPng,
            &opts(true)
        )
        .is_err()
    );
}

#[test]
fn an_input_inside_the_output_never_gets_written() {
    // output is the parent of the input: the mirrored target of in/a/x.dc6 is
    // out/in/a/x, but the folder out/a does not exist; a target that would
    // fall inside the input is reported as a failure
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().to_path_buf();
    let input = out.join("a");
    write_sprite(
        &input.join("a/s.dc6"),
        &small_sprite(1, 1, 2),
        SpriteFormat::Dc6,
    );
    let before = files_under(&input);
    let report = run(&input, &out, &BatchDirection::ToPng, &opts(true)).unwrap();
    // mirrored path is out/a/s: inside the input folder `out/a`
    assert_eq!(report.converted, 0);
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(files_under(&input), before);
}

#[test]
fn bad_files_are_reported_and_the_rest_continue() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, out) = (tmp.path().join("in"), tmp.path().join("out"));
    write_sprite(
        &input.join("good.dc6"),
        &small_sprite(1, 1, 2),
        SpriteFormat::Dc6,
    );
    std::fs::write(input.join("bad.dc6"), b"garbage").unwrap();
    let report = run(&input, &out, &BatchDirection::ToPng, &opts(false)).unwrap();
    assert_eq!((report.converted, report.failed.len()), (1, 1));
    assert!(report.failed[0].0.ends_with("bad.dc6"));
    assert!(report.log.iter().any(|l| l.contains("FAILED")));
}

#[test]
fn progress_is_reported_per_item() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, out) = (tmp.path().join("in"), tmp.path().join("out"));
    sprite_tree(&input);
    let mut calls: Vec<(usize, usize, String)> = Vec::new();
    batch_convert(
        &input,
        &out,
        &BatchDirection::ToPng,
        &synthetic_palette(),
        &opts(true),
        &mut |done, total, name| calls.push((done, total, name.to_owned())),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        calls.len(),
        4,
        "once at the start, then per item: {calls:?}"
    );
    assert_eq!(calls[0], (0, 3, String::new()));
    assert_eq!(calls[3].0, 3);
    assert!(calls.iter().all(|c| c.1 == 3));
    assert!(calls[1..].iter().all(|c| !c.2.is_empty()));
}

#[test]
fn cancel_stops_before_the_next_item_and_returns_the_partial_report() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, out) = (tmp.path().join("in"), tmp.path().join("out"));
    sprite_tree(&input);

    // already cancelled: nothing is converted
    let cancel = AtomicBool::new(true);
    let report = batch_convert(
        &input,
        &out,
        &BatchDirection::ToPng,
        &synthetic_palette(),
        &opts(true),
        &mut |_, _, _| {},
        &cancel,
    )
    .unwrap();
    assert!(report.cancelled);
    assert_eq!((report.converted, report.skipped), (0, 0));
    assert!(report.failed.is_empty());
    assert_eq!(report.log, ["cancelled after 0 of 3 items"]);
    assert!(!out.exists() || files_under(&out).is_empty());

    // cancelled from the progress callback after the first item
    let cancel = AtomicBool::new(false);
    let report = batch_convert(
        &input,
        &out,
        &BatchDirection::ToPng,
        &synthetic_palette(),
        &opts(true),
        &mut |done, _, _| {
            if done == 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(report.converted, 1, "the partial report counts the done item");
    assert_eq!(report.log.len(), 2);
    assert_eq!(
        report.log.last().map(String::as_str),
        Some("cancelled after 1 of 3 items")
    );
    let metas = files_under(&out)
        .iter()
        .filter(|p| p.ends_with(META_FILE))
        .count();
    assert_eq!(metas, 1, "exactly one item was converted before the cancel");

    // an uncancelled run reports cancelled == false
    let out2 = tmp.path().join("out2");
    let report = run(&input, &out2, &BatchDirection::ToPng, &opts(true)).unwrap();
    assert!(!report.cancelled);
}

#[test]
fn nonrecursive_reads_only_the_top_level() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, out) = (tmp.path().join("in"), tmp.path().join("out"));
    sprite_tree(&input);
    let report = run(&input, &out, &BatchDirection::ToPng, &opts(false)).unwrap();
    assert_eq!(report.converted, 1, "only z.DC6");
    assert!(out.join("z").join(META_FILE).is_file());
}

#[test]
fn empty_input_converts_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("in");
    std::fs::create_dir_all(&input).unwrap();
    let report = run(
        &input,
        &tmp.path().join("out"),
        &BatchDirection::ToPng,
        &opts(true),
    )
    .unwrap();
    assert_eq!(report, crate::batch::BatchReport::default());
}
