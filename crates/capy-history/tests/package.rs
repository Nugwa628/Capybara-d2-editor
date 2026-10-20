//! Map packages: export -> import gives identical files, merge rules, and
//! the safety checks. Uses the private corpus when present.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::path::PathBuf;

use capy_history::{
    HistoryError, ImportOptions, OnConflict, PackEntry, PackageSpec, SavesDir, TableRows,
    export_package, import_loose_map, import_package, read_package,
};
use d2_assets::GamePath;

fn corpus() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\zffit\Capybara-d2-editor\testdata\private"));
    let d = dir.join("extracted").join("vanilla");
    d.is_dir().then_some(d)
}

fn sample() -> (PackageSpec, Vec<(GamePath, Vec<u8>)>) {
    let mut files: Vec<(GamePath, Vec<u8>)> = Vec::new();
    let ds1 = GamePath::new("data\\global\\tiles\\act1\\outdoors\\cott6.ds1");
    let ds1_bytes;
    if let Some(root) = corpus() {
        let read = |g: &str| std::fs::read(root.join(g.replace('\\', "/"))).unwrap();
        ds1_bytes = read(ds1.as_str());
        for n in ["floor", "cottages", "blank"] {
            let p = format!("data\\global\\tiles\\act1\\outdoors\\{n}.dt1");
            if root.join(p.replace('\\', "/")).is_file() {
                files.push((GamePath::new(&p), read(&p)));
            }
        }
    } else {
        ds1_bytes = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        files.push((
            GamePath::new("data\\global\\tiles\\act1\\outdoors\\floor.dt1"),
            vec![9u8; 70_000],
        ));
        files.push((
            GamePath::new("data\\global\\tiles\\act1\\outdoors\\odd.dt1"),
            vec![],
        ));
    }
    let spec = PackageSpec {
        map_path: Some(ds1),
        map_bytes: ds1_bytes,
        files: files
            .iter()
            .map(|(p, b)| PackEntry {
                path: p.clone(),
                bytes: b.clone(),
                base_game: false,
            })
            .collect(),
        rows: vec![TableRows {
            table: "LvlPrest.txt".into(),
            header: vec!["Name".into(), "File1".into()],
            rows: vec![vec![
                "Act 1 Cottage".into(),
                "act1\\outdoors\\cott6.ds1".into(),
            ]],
        }],
        note: Some("test".into()),
    };
    (spec, files)
}

fn read_back(saves: &SavesDir, g: &GamePath) -> Vec<u8> {
    std::fs::read(saves.path_for(g).unwrap()).unwrap()
}

#[test]
fn export_then_import_gives_identical_files_and_changed_txt() {
    let (spec, files) = sample();
    let zip = export_package(&spec).unwrap();
    assert_eq!(zip, export_package(&spec).unwrap(), "deterministic");
    if let Some(p) = std::env::var_os("CAPY_DUMP_ZIP") {
        // Lets a human check the zip with other tools (Explorer, 7-Zip).
        std::fs::write(p, &zip).unwrap();
    }
    let tmp = tempfile::tempdir().unwrap();
    let saves = SavesDir::new(tmp.path(), "p");
    let rep = import_package(&saves, &zip, &ImportOptions::default()).unwrap();
    let map = spec.map_path.clone().unwrap();
    assert_eq!(rep.map.as_ref(), Some(&map));
    assert_eq!(rep.written.len(), files.len() + 1);
    assert_eq!(read_back(&saves, &map), spec.map_bytes);
    for (p, b) in &files {
        assert_eq!(&read_back(&saves, p), b, "{p}");
    }
    assert_eq!(rep.rows, spec.rows);
    assert_eq!(saves.changed().unwrap().len(), files.len() + 1);
    // Importing again changes nothing.
    let again = import_package(&saves, &zip, &ImportOptions::default()).unwrap();
    assert!(again.written.is_empty());
    assert_eq!(again.unchanged.len(), files.len() + 1);
}

#[test]
fn conflicts_are_kept_or_overwritten_and_base_game_files_are_optional() {
    let (mut spec, files) = sample();
    let base = GamePath::new("data\\global\\tiles\\act1\\outdoors\\base.dt1");
    spec.files.push(PackEntry {
        path: base.clone(),
        bytes: vec![1, 2, 3],
        base_game: true,
    });
    let zip = export_package(&spec).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let saves = SavesDir::new(tmp.path(), "p");
    let victim = &files[0].0;
    saves.save(victim, b"mine").unwrap();
    let rep = import_package(&saves, &zip, &ImportOptions::default()).unwrap();
    assert_eq!(rep.conflicts, vec![victim.clone()]);
    assert_eq!(rep.skipped_base_game, vec![base.clone()]);
    assert_eq!(read_back(&saves, victim), b"mine");
    let opts = ImportOptions {
        on_conflict: OnConflict::Overwrite,
        include_base_game: true,
    };
    let rep = import_package(&saves, &zip, &opts).unwrap();
    assert!(rep.conflicts.is_empty() && rep.written.contains(victim));
    assert_eq!(&read_back(&saves, victim), &files[0].1);
    assert_eq!(read_back(&saves, &base), vec![1, 2, 3]);
}

#[test]
fn damaged_and_hostile_packages_are_refused_before_any_write() {
    let (spec, _) = sample();
    let zip = export_package(&spec).unwrap();
    let mut flipped = zip.clone();
    let n = flipped.len() / 3;
    flipped[n] ^= 0xFF;
    assert!(matches!(
        read_package(&flipped),
        Err(HistoryError::Package(_))
    ));
    assert!(matches!(
        read_package(b"not a zip"),
        Err(HistoryError::Package(_))
    ));

    // A path that escapes the saves folder.
    let evil = PackageSpec {
        map_path: Some(GamePath::new("data\\global\\tiles\\ok.ds1")),
        map_bytes: vec![1],
        files: vec![PackEntry {
            path: GamePath::new("data\\..\\..\\evil.dt1"),
            bytes: vec![2],
            base_game: false,
        }],
        ..PackageSpec::default()
    };
    let zip = export_package(&evil).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let saves = SavesDir::new(tmp.path(), "p");
    assert!(import_package(&saves, &zip, &ImportOptions::default()).is_err());
    assert!(saves.changed().unwrap().is_empty());
    let ok = GamePath::new("data\\global\\tiles\\ok.ds1");
    assert!(!saves.path_for(&ok).unwrap().exists());
}

#[test]
fn a_ds1_imports_with_the_dt1s_it_needs_from_a_loose_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let loose = tmp.path().join("loose");
    std::fs::create_dir_all(loose.join("mymod").join("tiles")).unwrap();
    std::fs::write(loose.join("map.ds1"), b"ds1-bytes").unwrap();
    std::fs::write(loose.join("mymod/tiles/Floor.DT1"), b"floor").unwrap();
    std::fs::create_dir_all(loose.join("data/global/tiles/act1")).unwrap();
    std::fs::write(loose.join("data/global/tiles/act1/wall.dt1"), b"wall").unwrap();
    let saves = SavesDir::new(&tmp.path().join("saves"), "p");
    let needed = [
        GamePath::new("data\\global\\tiles\\act1\\floor.dt1"),
        GamePath::new("data\\global\\tiles\\act1\\wall.dt1"),
        GamePath::new("data\\global\\tiles\\act1\\gone.dt1"),
    ];
    let ds1 = GamePath::new("data\\global\\tiles\\act1\\map.ds1");
    let r = import_loose_map(
        &saves,
        &loose.join("map.ds1"),
        &ds1,
        &loose,
        &needed,
        &ImportOptions::default(),
    )
    .unwrap();
    assert_eq!(r.missing, vec![needed[2].clone()]);
    assert_eq!(r.report.written.len(), 3);
    assert_eq!(read_back(&saves, &ds1), b"ds1-bytes");
    assert_eq!(read_back(&saves, &needed[0]), b"floor");
    assert_eq!(read_back(&saves, &needed[1]), b"wall");
    assert_eq!(saves.changed().unwrap().len(), 3);
}
