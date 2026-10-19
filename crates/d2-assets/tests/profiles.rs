//! Profiles, overlay priority and thread-safety guarantees.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use common::{Builder, FileSpec, dir_available, mpqs_in, mxl_dir, text, vanilla_dir, write_temp};
use d2_assets::{
    AssetError, AssetSource, GamePath, GameVersion, LayerId, LayerSpec, MpqArchive, MpqSource,
    Overlay, Profile,
};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn overlay_and_archive_are_send_and_sync() {
    assert_send_sync::<Overlay>();
    assert_send_sync::<MpqArchive>();
    assert_send_sync::<MpqSource>();
    assert_send_sync::<d2_assets::FolderSource>();
    assert_send_sync::<Box<dyn AssetSource>>();
}

#[test]
fn toml_round_trip_of_builtins() {
    for profile in [
        Profile::vanilla(std::path::Path::new("C:\\Program Files (x86)\\Diablo II")),
        Profile::median_xl(std::path::Path::new("C:\\games\\median-xl")),
    ] {
        let text = profile.to_toml();
        assert_eq!(Profile::from_toml(&text).unwrap(), profile);
    }
    let v = Profile::vanilla(std::path::Path::new("x"));
    assert_eq!(
        (v.id.as_str(), v.game_version),
        ("vanilla", GameVersion::V114d)
    );
    assert!(matches!(v.layers[0], LayerSpec::Saves));
    let m = Profile::median_xl(std::path::Path::new("x"));
    assert_eq!((m.id.as_str(), m.game_version), ("mxl", GameVersion::V113c));
    // Median XL never points at the vanilla install.
    for layer in &m.layers {
        match layer {
            LayerSpec::Mpq { path, .. } => assert!(path.starts_with("x")),
            LayerSpec::MpqGlob { dir, .. } => assert!(dir.starts_with("x")),
            _ => {}
        }
    }
}

#[test]
fn load_dir_sorts_and_names_the_bad_file() {
    let dir = tempfile::tempdir().unwrap();
    let mk = |id: &str| {
        Profile {
            id: id.into(),
            name: id.to_uppercase(),
            game_version: GameVersion::V114d,
            layers: vec![LayerSpec::Saves],
        }
        .to_toml()
    };
    std::fs::write(dir.path().join("zeta.toml"), mk("zeta")).unwrap();
    std::fs::write(dir.path().join("alpha.toml"), mk("alpha")).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
    let ids: Vec<String> = Profile::load_dir(dir.path())
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ["alpha", "zeta"]);

    std::fs::write(dir.path().join("broken.toml"), "id = \"x\"\nname = [").unwrap();
    let err = Profile::load_dir(dir.path()).unwrap_err();
    assert!(matches!(err, AssetError::Profile { .. }), "{err:?}");
    assert!(err.to_string().contains("broken.toml"), "{err}");

    // Valid TOML, invalid id.
    std::fs::remove_file(dir.path().join("broken.toml")).unwrap();
    std::fs::write(
        dir.path().join("badid.toml"),
        "id = \"Bad Id\"\nname = \"x\"\ngame_version = \"1.14d\"\nlayer = []\n",
    )
    .unwrap();
    let err = Profile::load_dir(dir.path()).unwrap_err();
    assert!(err.to_string().contains("badid.toml"), "{err}");

    // Duplicate ids.
    std::fs::remove_file(dir.path().join("badid.toml")).unwrap();
    std::fs::write(dir.path().join("alpha2.toml"), mk("alpha")).unwrap();
    let err = Profile::load_dir(dir.path()).unwrap_err();
    assert!(
        err.to_string().contains("alpha2.toml") || err.to_string().contains("alpha.toml"),
        "{err}"
    );

    // A missing directory is simply "no profiles".
    assert!(
        Profile::load_dir(&dir.path().join("nope"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn synthetic_profile_layers_glob_optional_and_priority() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    std::fs::create_dir_all(&game).unwrap();
    let mk = |name: &str, files: &[(&str, &[u8])]| {
        let mut b = Builder::new();
        for (n, d) in files {
            b = b.file(FileSpec::new(n, d));
        }
        write_temp(&game, name, &b.build());
    };
    mk(
        "mod-b.mpq",
        &[("data\\x.txt", b"from b"), ("data\\only_b.txt", b"b")],
    );
    mk("MOD-A.mpq", &[("data\\x.txt", b"from a")]);
    mk(
        "base.mpq",
        &[
            ("data\\x.txt", b"from base"),
            ("data\\only_base.txt", b"base"),
        ],
    );
    std::fs::write(game.join("mod-c.txt"), "not an archive").unwrap();

    let profile = Profile {
        id: "t".into(),
        name: "T".into(),
        game_version: GameVersion::V113c,
        layers: vec![
            LayerSpec::Saves,
            LayerSpec::MpqGlob {
                dir: game.clone(),
                pattern: "mod-*.mpq".into(),
            },
            LayerSpec::Mpq {
                path: game.join("absent.mpq"),
                optional: true,
            },
            LayerSpec::Mpq {
                path: game.join("base.mpq"),
                optional: false,
            },
        ],
    };
    let saves_root = dir.path().join("saves");
    let overlay = profile.open(&saves_root).unwrap();
    // saves, MOD-A, mod-b (sorted case-insensitively), base; optional one skipped.
    let names: Vec<&str> = overlay.layers().iter().map(|l| l.name()).collect();
    assert_eq!(names, ["saves\\t", "MOD-A.mpq", "mod-b.mpq", "base.mpq"]);
    let x = GamePath::new("data\\x.txt");
    assert_eq!(
        overlay.read(&x).unwrap(),
        Some((LayerId(1), b"from a".to_vec()))
    );
    assert_eq!(
        overlay.resolve_all(&x),
        [LayerId(1), LayerId(2), LayerId(3)]
    );
    assert_eq!(
        overlay.read_from(LayerId(3), &x).unwrap().unwrap(),
        b"from base"
    );
    assert_eq!(overlay.fingerprints().unwrap().len(), 4);

    // A file in the saves folder wins over everything.
    let saved = GamePath::new("data\\x.txt").to_native(&profile.saves_dir(&saves_root));
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, b"edited").unwrap();
    let overlay = profile.open(&saves_root).unwrap();
    assert_eq!(
        overlay.read(&x).unwrap(),
        Some((LayerId(0), b"edited".to_vec()))
    );
    assert_eq!(overlay.resolve_all(&x).len(), 4);

    // Failure modes.
    let mut bad = profile.clone();
    bad.layers = vec![LayerSpec::MpqGlob {
        dir: game.clone(),
        pattern: "nothing-*.mpq".into(),
    }];
    let err = bad.open(&saves_root).err().unwrap();
    assert!(matches!(err, AssetError::Profile { .. }), "{err:?}");
    bad.layers = vec![LayerSpec::Mpq {
        path: game.join("absent.mpq"),
        optional: false,
    }];
    assert!(matches!(
        bad.open(&saves_root).err().unwrap(),
        AssetError::Io { .. }
    ));
    bad.layers = vec![LayerSpec::Mpq {
        path: game.join("mod-c.txt"),
        optional: false,
    }];
    assert!(matches!(
        bad.open(&saves_root).err().unwrap(),
        AssetError::Mpq { .. }
    ));
    bad.layers = vec![LayerSpec::Casc { path: game.clone() }];
    assert!(matches!(
        bad.open(&saves_root).err().unwrap(),
        AssetError::Unsupported(_)
    ));
    bad.layers = vec![LayerSpec::Saves];
    bad.id = "../evil".into();
    assert!(bad.open(&saves_root).is_err());
}

#[test]
fn mpq_source_basics() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = Builder::new()
        .file(FileSpec::new(
            "(listfile)",
            b"data\\a.txt\r\nData/B.txt\r\ndata\\ghost.txt\r\ndata\\a.txt\r\n",
        ))
        .file(FileSpec::new("data\\a.txt", &text(100)))
        .file(FileSpec::new("data\\b.txt", b"b"))
        .build();
    let path = write_temp(dir.path(), "src.mpq", &bytes);
    let src = MpqSource::open(&path).unwrap();
    assert_eq!(src.name(), "src.mpq");
    assert_eq!(src.kind(), d2_assets::SourceKind::Mpq);
    assert!(src.contains(&GamePath::new("DATA\\A.TXT")));
    assert_eq!(
        src.read(&GamePath::new("data\\b.txt")).unwrap().unwrap(),
        b"b"
    );
    assert_eq!(src.read(&GamePath::new("data\\ghost.txt")).unwrap(), None);
    // list(): existing listed names, deduplicated, no "(listfile)".
    let listed: BTreeSet<String> = src
        .list()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        listed,
        BTreeSet::from(["data\\a.txt".to_owned(), "data\\b.txt".to_owned()])
    );
    let fp = src.fingerprint().unwrap();
    assert_eq!((fp.size, fp.entries), (bytes.len() as u64, 3));
    assert!(fp.modified_unix_ms > 0);
    assert_eq!(fp.path, path);
}

// ---------------------------------------------------------------- real data

fn vanilla_overlay(saves: &std::path::Path) -> Option<Overlay> {
    let dir = vanilla_dir();
    if !dir_available(&dir) {
        return None;
    }
    Some(
        Profile::vanilla(&dir)
            .open(saves)
            .expect("open vanilla profile"),
    )
}

#[test]
fn vanilla_profile_opens_and_patch_d2_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let saves_root = tmp.path().join("no-such-saves");
    let Some(overlay) = vanilla_overlay(&saves_root) else {
        return;
    };
    let layer_names: Vec<&str> = overlay.layers().iter().map(|l| l.name()).collect();
    println!("vanilla layers: {layer_names:?}");
    assert_eq!(
        layer_names,
        [
            "saves\\vanilla",
            "Patch_D2.mpq",
            "d2exp.mpq",
            "d2data.mpq",
            "d2char.mpq"
        ]
    );
    assert!(
        !saves_root.exists(),
        "opening a profile must not create the saves folder"
    );

    // Names from the lower layers' listfiles that Patch_D2 also holds.
    let mut shared: Vec<GamePath> = Vec::new();
    for layer in &overlay.layers()[2..4] {
        for path in layer.list().unwrap() {
            if overlay.layers()[1].contains(&path) {
                shared.push(path);
            }
        }
    }
    shared.sort();
    shared.dedup();
    println!("names in Patch_D2 and d2data/d2exp: {}", shared.len());
    assert!(!shared.is_empty(), "expected at least one overridden name");
    for path in shared.iter().take(25) {
        let all = overlay.resolve_all(path);
        assert_eq!(overlay.resolve(path), Some(LayerId(1)), "{path}");
        assert_eq!(all[0], LayerId(1), "{path}");
        assert!(
            all.len() >= 2 && all.iter().any(|l| l.0 >= 2),
            "{path}: {all:?}"
        );
        let (layer, bytes) = overlay.read(path).unwrap().unwrap();
        assert_eq!(layer, LayerId(1));
        assert_eq!(overlay.read_from(LayerId(1), path).unwrap().unwrap(), bytes);
    }
    println!(
        "example: {} -> {:?}",
        shared[0],
        overlay.resolve_all(&shared[0])
    );

    // A loose file in the saves folder overrides the archives.
    let target = shared[0].clone();
    let archive_bytes = overlay.read_bytes(&target).unwrap().unwrap();
    let profile = Profile::vanilla(&vanilla_dir());
    let file = target.to_native(&profile.saves_dir(&saves_root));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"edited by the user").unwrap();
    let overlay = profile.open(&saves_root).unwrap();
    assert_eq!(overlay.resolve(&target), Some(LayerId(0)));
    assert_eq!(
        overlay.read(&target).unwrap(),
        Some((LayerId(0), b"edited by the user".to_vec()))
    );
    assert_eq!(
        overlay.read_from(LayerId(1), &target).unwrap().unwrap(),
        archive_bytes
    );
    assert!(overlay.resolve_all(&target).len() >= 3);
    assert_eq!(overlay.fingerprints().unwrap().len(), 5);
}

#[test]
fn median_xl_profile_opens_and_mods_win() {
    let dir = mxl_dir();
    if !dir_available(&dir) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let overlay = Profile::median_xl(&dir)
        .open(&tmp.path().join("saves"))
        .expect("open mxl profile");
    let layers = overlay.layers();
    let mods = mpqs_in(&dir)
        .iter()
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("medianxl-")
        })
        .count();
    println!(
        "mxl layers: {} (saves + {mods} medianxl-* + 4 base + patch_d2 as listed)",
        layers.len()
    );
    assert_eq!(mods, 56);
    assert_eq!(layers.len(), 1 + mods + 4);
    let tail: Vec<&str> = layers[layers.len() - 4..]
        .iter()
        .map(|l| l.name())
        .collect();
    assert_eq!(
        tail,
        ["patch_d2.mpq", "d2exp.mpq", "d2data.mpq", "d2char.mpq"]
    );
    // Everything comes from the Median XL folder, never from vanilla's.
    for l in layers {
        assert_eq!(
            l.kind(),
            if l.name().starts_with("saves") {
                d2_assets::SourceKind::Folder
            } else {
                d2_assets::SourceKind::Mpq
            }
        );
    }
    let base = layers.len() - 2; // d2data.mpq
    let mut overridden = 0usize;
    for (i, layer) in layers.iter().enumerate().take(1 + mods).skip(1) {
        for path in layer.list().unwrap() {
            if layers[base].contains(&path) {
                overridden += 1;
                assert_eq!(overlay.resolve(&path), Some(LayerId(i)), "{path}");
                assert!(overlay.resolve_all(&path).contains(&LayerId(base)));
            }
        }
    }
    println!("medianxl-* files overriding d2data.mpq: {overridden}");
    assert!(overridden > 0);
}

#[test]
fn profile_open_never_touches_game_dir_contents() {
    // Fingerprints are stable across two opens (nothing is written).
    let tmp = tempfile::tempdir().unwrap();
    let Some(a) = vanilla_overlay(&tmp.path().join("s")) else {
        return;
    };
    let before = a.fingerprints().unwrap();
    drop(a);
    let b = vanilla_overlay(&tmp.path().join("s")).unwrap();
    assert_eq!(before, b.fingerprints().unwrap());
    let sample = &before[1];
    assert!(sample.path.ends_with("Patch_D2.mpq"));
    assert!(sample.size > 0 && sample.entries > 0 && sample.modified_unix_ms > 0);
}
