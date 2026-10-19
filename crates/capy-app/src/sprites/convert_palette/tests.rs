use super::*;
use crate::sprites::palette::grey_palette;

fn state() -> (tempfile::TempDir, AppState) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = crate::paths::Paths::at(tmp.path().join("capybara"));
    paths.ensure().expect("ensure");
    (
        tmp,
        AppState::new(paths, crate::config::Config::default(), Vec::new()),
    )
}

fn dc6_file(dir: &Path, name: &str, indices: &[u8]) -> PathBuf {
    let mut img = IndexedImage::new(indices.len() as u32, 1, 0, -1);
    img.pixels = indices.to_vec();
    img.alpha = vec![255; indices.len()];
    let bytes = Sprite {
        directions: vec![vec![img.clone(), img]],
    }
    .encode(SpriteFormat::Dc6)
    .expect("encode");
    let p = dir.join(name);
    std::fs::write(&p, bytes).expect("write");
    p
}

#[test]
fn targets_parse_and_label() {
    assert_eq!(TargetChoice::parse("all-acts"), Some(TargetChoice::AllActs));
    assert_eq!(
        TargetChoice::parse("act1"),
        Some(TargetChoice::Palette(PaletteChoice::Act(0)))
    );
    assert_eq!(TargetChoice::AllActs.palette(), PaletteChoice::Act(0));
    assert!(TargetChoice::AllActs.shared_only());
    assert!(TargetChoice::AllActs.label().contains("shared"));
}

#[test]
fn converted_paths_are_checked() {
    assert!(check_converted_path("data\\global\\tiles\\act1\\a.dt1", SpriteKind::Dt1).is_ok());
    assert!(check_converted_path("data\\global\\tiles\\act1\\a.dt1", SpriteKind::Dc6).is_err());
    assert!(check_converted_path("C:\\a.dt1", SpriteKind::Dt1).is_err());
    assert!(check_converted_path("data\\..\\a.dt1", SpriteKind::Dt1).is_err());
    assert!(check_converted_path("global\\a.dt1", SpriteKind::Dt1).is_err());
}

#[test]
fn install_folders_are_never_written() {
    let tmp = tempfile::tempdir().expect("tmp");
    let game = tmp.path().join("Diablo II");
    std::fs::create_dir_all(&game).expect("mkdir");
    let installs = install_dirs(&[d2_assets::Profile::vanilla(&game)]);
    assert!(installs.contains(&game), "{installs:?}");
    assert!(check_not_install(&game.join("data").join("a.dt1"), &installs).is_err());
    assert!(check_not_install(&game.join("a.dt1"), &installs).is_err());
    assert!(check_not_install(&tmp.path().join("out").join("a.dt1"), &installs).is_ok());
    // A sibling folder whose name starts the same is not inside.
    assert!(check_not_install(&tmp.path().join("Diablo II x").join("a.dt1"), &installs).is_ok());
}

#[test]
fn a_loose_dc6_converts_with_an_override_and_saves_as() {
    let (tmp, mut s) = state();
    // Without a profile the palettes are the grey ramp; make the source and
    // target differ by picking Grey vs Grey with an override.
    let file = dc6_file(tmp.path(), "icon.dc6", &[10, 20, 230, 230]);
    s.convert_open_file(&file).expect("open");
    let c = &s.sprites.convert;
    assert!(c.open);
    let input = c.input.as_ref().expect("input");
    assert_eq!(
        (input.directions, input.frames, input.items.len()),
        (1, 2, 2)
    );
    assert_eq!(input.histogram[230], 4);
    s.sprites.convert.source = PaletteChoice::Grey;
    s.sprites.convert.target = TargetChoice::Palette(PaletteChoice::Grey);
    let plan = s.convert_plan().expect("plan");
    assert!(
        plan.entries.iter().all(|e| e.target == e.source),
        "same palette: kept"
    );
    s.sprites.convert.settings.set_override(230, Some(40));
    let plan = s.convert_plan().expect("plan");
    let rows = table::rows(&plan, |_| true);
    assert_eq!(rows[0].entry.source, 230, "the override is the worst match");
    assert_eq!(rows[0].entry.target, 40);
    s.convert_apply().expect("apply");
    let r = s.sprites.convert.result.as_ref().expect("result");
    assert_eq!(r.summary.changed, 1);
    let out = Sprite::decode(&r.bytes, SpriteFormat::Dc6).expect("decode");
    assert_eq!(out.directions[0][0].pixels, vec![10, 20, 40, 40]);
    // The Viewer shows the converted file, unsaved.
    assert!(matches!(
        s.sprites.viewer.source,
        Some(SpriteSource::Converted { .. })
    ));
    // The original file is untouched; Save As writes a new one.
    let saved = s
        .convert_save_as(&tmp.path().join("out").join("icon2"))
        .expect("save as");
    assert_eq!(saved.extension().and_then(|e| e.to_str()), Some("dc6"));
    assert_eq!(std::fs::read(&saved).expect("read"), *r_bytes(&s));
    let orig = Sprite::decode(&std::fs::read(&file).expect("read"), SpriteFormat::Dc6).expect("d");
    assert_eq!(orig.directions[0][0].pixels, vec![10, 20, 230, 230]);
    // Saving to the game path needs a profile.
    assert!(s.convert_save_game("data\\global\\ui\\icon.dc6").is_err());
    // The saved file remembers the target palette.
    assert_eq!(
        s.remembered_palette(&saved.display().to_string())
            .as_deref(),
        Some("grey")
    );
}

fn r_bytes(s: &AppState) -> Arc<Vec<u8>> {
    s.sprites
        .convert
        .result
        .as_ref()
        .map(|r| r.bytes.clone())
        .unwrap_or_default()
}

#[test]
fn opening_defaults_to_the_remembered_source_palette() {
    let (tmp, mut s) = state();
    let file = dc6_file(tmp.path(), "a.dc6", &[5]);
    let key = file.display().to_string();
    s.remember_palette(&key, &PaletteChoice::Act(2));
    s.convert_open_file(&file).expect("open");
    assert_eq!(s.sprites.convert.source, PaletteChoice::Act(2));
    // A sibling falls back to the folder's palette.
    let b = dc6_file(tmp.path(), "b.dc6", &[5]);
    s.convert_open_file(&b).expect("open");
    assert_eq!(s.sprites.convert.source, PaletteChoice::Act(2));
    // An explicit source pick is remembered for the file.
    s.convert_set_source(PaletteChoice::Act(4));
    assert_eq!(
        s.remembered_palette(&b.display().to_string()).as_deref(),
        Some("act5")
    );
    // Junk is an error, not a panic.
    let junk = tmp.path().join("junk.dt1");
    std::fs::write(&junk, b"junk").expect("write");
    assert!(s.convert_open_file(&junk).is_err());
    assert!(s.convert_open_file(&tmp.path().join("x.png")).is_err());
    let _ = grey_palette();
}

#[test]
fn a_save_into_a_file_of_another_palette_asks_first() {
    let (_tmp, mut s) = state();
    let key = "data\\global\\ui\\x\\a.dc6";
    s.remember_palette(key, &PaletteChoice::Act(4));
    s.sprites.palette.choice = PaletteChoice::Act(0);
    let write = crate::sprites::DccWrite::Game(key.to_owned());
    assert!(!s.sprite_write_checked(crate::sprites::Target::Viewer, write));
    let w = s.sprites.palette_warning.clone().expect("asked");
    assert_eq!(w.remembered, PaletteChoice::Act(4));
    assert!(s.sprites.modal_open());
    s.answer_palette_warning(crate::sprites::PaletteAnswer::Cancel);
    assert!(s.sprites.palette_warning.is_none());
    assert_eq!(s.sprites.palette.choice, PaletteChoice::Act(0));
    let write = crate::sprites::DccWrite::Game(key.to_owned());
    assert!(!s.sprite_write_checked(crate::sprites::Target::Viewer, write));
    s.answer_palette_warning(crate::sprites::PaletteAnswer::UseRemembered);
    assert_eq!(s.sprites.palette.choice, PaletteChoice::Act(4), "switched");
}
