#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::ffi::OsString;
use std::path::PathBuf;

use capy_render::{VariantRule, Visibility};

use crate::args::{Command, RenderArgs, parse};
use d2_assets::installs::{Candidate, InstallSearch};

use crate::{root_from, run_in_with};

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn render_ok(args: &[&str]) -> RenderArgs {
    match parse(&os(args)) {
        Ok(Some(Command::Render(a))) => a,
        other => panic!("{args:?}: expected render, got {other:?}"),
    }
}

fn is_usage_error(args: &[&str]) -> bool {
    parse(&os(args)).is_err()
}

#[test]
fn non_verbs_are_not_cli() {
    for args in [
        &[][..],
        &["--version"],
        &["-help"],
        &["open"],
        &["C:\\maps\\a.ds1"],
        &["renderx"],
        &[""],
    ] {
        assert_eq!(parse(&os(args)), Ok(None), "{args:?}");
    }
}

#[test]
fn verbs_parse_case_insensitively() {
    let cases: &[(&[&str], Command)] = &[
        (&["help"], Command::Help(None)),
        (&["HELP", "Render"], Command::Help(Some("render".into()))),
        (&["profiles", "list"], Command::ProfilesList),
        (&["Profiles", "LIST"], Command::ProfilesList),
        (
            &["index", "status", "--profile", "vanilla"],
            Command::IndexStatus {
                profile: "vanilla".into(),
            },
        ),
        (
            &["INDEX", "build", "--profile=mxl"],
            Command::IndexBuild {
                profile: "mxl".into(),
            },
        ),
        (
            &["maps", "list", "--profile", "v"],
            Command::MapsList {
                profile: "v".into(),
                filter: None,
            },
        ),
        (
            &["maps", "list", "--filter", "Towne1", "--profile", "v"],
            Command::MapsList {
                profile: "v".into(),
                filter: Some("Towne1".into()),
            },
        ),
    ];
    for (args, expected) in cases {
        assert_eq!(parse(&os(args)), Ok(Some(expected.clone())), "{args:?}");
    }
}

#[test]
fn render_defaults_and_flags() {
    let a = render_ok(&["render", "a.ds1", "--profile", "v", "--out", "o.png"]);
    assert_eq!(a.zoom, 1);
    assert!((a.gamma - 1.30).abs() < 1e-6);
    assert_eq!(a.lvltype, None);
    assert_eq!(a.def, None);
    assert_eq!(a.visibility, Visibility::default());
    assert_eq!(a.variant, VariantRule::ReferenceQuirk);
    assert_eq!(a.out, PathBuf::from("o.png"));

    let a = render_ok(&[
        "Render",
        "data\\x.ds1",
        "--out=o.png",
        "--profile=p.toml",
        "--lvltype",
        "3",
        "--def",
        "7",
        "--zoom",
        "1:4",
        "--gamma",
        "1.0",
        "--variant",
        "documented",
        "--layers",
        "floors,Shadow,special",
    ]);
    assert_eq!((a.lvltype, a.def, a.zoom), (Some(3), Some(7), 4));
    assert_eq!(a.variant, VariantRule::Documented);
    assert_eq!(a.visibility.floors, [true, true]);
    assert_eq!(a.visibility.walls, [false; 4]);
    assert!(a.visibility.shadow && a.visibility.special);
    assert!(!a.visibility.roofs && !a.visibility.objects);

    // A bare `N` is accepted as the zoom, `all` turns everything on.
    let a = render_ok(&[
        "render",
        "a.ds1",
        "--profile",
        "v",
        "--out",
        "o",
        "--zoom",
        "16",
        "--layers",
        "all",
    ]);
    assert_eq!(a.zoom, 16);
    assert!(a.visibility.special && a.visibility.walls == [true; 4]);
}

#[test]
fn bad_arguments_are_usage_errors() {
    let base = ["render", "a.ds1", "--profile", "v", "--out", "o.png"];
    let with = |extra: &[&'static str]| -> Vec<&'static str> {
        let mut v: Vec<&'static str> = vec!["render", "a.ds1", "--profile", "v", "--out", "o.png"];
        v.extend_from_slice(extra);
        v
    };
    assert!(!is_usage_error(&base));
    let bad: Vec<Vec<&str>> = vec![
        with(&["--zoom", "1:0"]),
        with(&["--zoom", "1:17"]),
        with(&["--zoom", "1:x"]),
        with(&["--zoom", "2:4"]),
        with(&["--zoom"]),
        with(&["--gamma", "0"]),
        with(&["--gamma", "abc"]),
        with(&["--gamma", "nan"]),
        with(&["--lvltype", "x"]),
        with(&["--lvltype", "-1"]),
        with(&["--def", "3"]),
        with(&["--variant", "fast"]),
        with(&["--layers", "floors,bogus"]),
        with(&["--layers", ","]),
        with(&["--frobnicate", "1"]),
        with(&["--out", "again.png"]),
        with(&["--object-frame", "256"]),
        with(&["--object-frame", "-1"]),
        with(&["--object-frame", "x"]),
        with(&["--object-frame"]),
        with(&["extra.ds1"]),
        // missing pieces
        vec!["render", "a.ds1", "--profile", "v"],
        vec!["render", "a.ds1", "--out", "o.png"],
        vec!["render", "--profile", "v", "--out", "o.png"],
        vec!["render"],
        // a flag swallowing the next flag as its value is an error
        vec!["render", "a.ds1", "--out", "--profile", "v"],
        // other verbs
        vec!["index"],
        vec!["index", "status"],
        vec!["index", "rebuild", "--profile", "v"],
        vec!["index", "status", "--profile", "v", "extra"],
        vec!["index", "status", "--profile", "v", "--filter", "x"],
        vec!["maps"],
        vec!["maps", "show", "--profile", "v"],
        vec!["maps", "list"],
        vec!["profiles"],
        vec!["profiles", "add"],
        vec!["profiles", "list", "--profile", "v"],
        vec!["help", "a", "b"],
    ];
    for args in &bad {
        assert!(is_usage_error(args), "{args:?} should be a usage error");
    }
}

fn run_text(args: &[&str]) -> (Option<u8>, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    run_text_in(tmp.path(), args)
}

/// An install search that finds nothing (no registry, no default folders).
fn no_installs() -> InstallSearch {
    let nowhere = std::env::temp_dir().join("capy-cli-tests-no-such-install");
    InstallSearch {
        default_vanilla: nowhere.join("d2"),
        default_mxl: nowhere.join("mxl"),
        registry: Vec::new(),
    }
}

fn run_text_in(root: &std::path::Path, args: &[&str]) -> (Option<u8>, String, String) {
    run_text_with(root, &no_installs, args)
}

fn run_text_with(
    root: &std::path::Path,
    installs: &dyn Fn() -> InstallSearch,
    args: &[&str],
) -> (Option<u8>, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_in_with(&os(args), root.to_path_buf(), installs, &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn a_clean_folder_gets_the_default_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("capybara");
    let installs = tmp.path().join("games");
    // A vanilla install named only by the registry, and the default Median
    // XL folder.
    let d2 = installs.join("Diablo II");
    let mxl = installs.join("median-xl");
    for (dir, files) in [
        (&d2, &["d2data.mpq", "d2exp.mpq"][..]),
        (&mxl, &["medianxl-dGlsZXM.mpq"][..]),
    ] {
        std::fs::create_dir_all(dir).unwrap();
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
    }
    let calls = std::cell::Cell::new(0);
    let search = || {
        calls.set(calls.get() + 1);
        InstallSearch {
            default_vanilla: installs.join("not-here"),
            default_mxl: mxl.clone(),
            registry: vec![Candidate {
                path: d2.clone(),
                source: "registry (HKLM)".into(),
            }],
        }
    };

    // `help` never touches the folder.
    let (code, _, _) = run_text_with(&root, &search, &["help"]);
    assert_eq!(code, Some(0));
    assert!(!root.exists());
    assert_eq!(calls.get(), 0);

    let (code, out, err) = run_text_with(&root, &search, &["profiles", "list"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        out,
        "mxl\t1.13c\tMedian XL\nvanilla\t1.14d\tDiablo II 1.14d\n"
    );
    assert_eq!(
        err.matches("note: wrote default profile").count(),
        2,
        "{err}"
    );
    let profiles = d2_assets::Profile::load_dir(&root.join("profiles")).unwrap();
    assert_eq!(
        profiles,
        vec![
            d2_assets::Profile::median_xl(&mxl),
            d2_assets::Profile::vanilla(&d2)
        ]
    );
    assert_eq!(calls.get(), 1);

    // Once profiles exist the search is not run again.
    let (code, _, err) = run_text_with(&root, &search, &["profiles", "list"]);
    assert_eq!(code, Some(0));
    assert!(err.is_empty(), "{err}");
    assert_eq!(calls.get(), 1);
}

#[test]
fn a_failed_seeding_is_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let d2 = tmp.path().join("d2");
    std::fs::create_dir_all(&d2).unwrap();
    // `profiles` is a file, so the folder cannot be created.
    std::fs::write(tmp.path().join("profiles"), b"").unwrap();
    let search = || InstallSearch {
        default_vanilla: d2.clone(),
        ..no_installs()
    };
    let (code, _, err) = run_text_with(tmp.path(), &search, &["profiles", "list"]);
    assert!(
        err.contains("warning: cannot write the default profiles"),
        "{err}"
    );
    assert_eq!(code, Some(1), "reading the profiles folder fails: {err}");
}

#[test]
fn run_exit_codes() {
    // Not a verb: the GUI starts.
    assert_eq!(run_text(&[]).0, None);
    assert_eq!(run_text(&["--version"]).0, None);
    // Usage errors.
    let (code, _, err) = run_text(&["render", "a.ds1", "--profile", "v"]);
    assert_eq!(code, Some(2));
    assert!(err.contains("--out"), "{err}");
    assert_eq!(run_text(&["render", "a.ds1", "--bogus", "1"]).0, Some(2));
    assert_eq!(run_text(&["help", "nonsense"]).0, Some(2));
    // Runtime error: unknown profile.
    let (code, _, err) = run_text(&["index", "status", "--profile", "nope"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("nope"), "{err}");
}

#[test]
fn help_lists_the_verbs() {
    let (code, out, _) = run_text(&["help"]);
    assert_eq!(code, Some(0));
    for verb in ["render", "index", "maps", "profiles", "help"] {
        assert!(out.contains(verb), "{verb} missing from help:\n{out}");
    }
    let (code, out, _) = run_text(&["help", "render"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("--lvltype") && out.contains("--out"), "{out}");
}

#[test]
fn profiles_list_reads_the_profiles_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, out, _) = run_text_in(tmp.path(), &["profiles", "list"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("no profiles"), "{out}");

    let dir = tmp.path().join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    let profile = d2_assets::Profile::vanilla(std::path::Path::new("C:\\nowhere"));
    std::fs::write(dir.join("vanilla.toml"), profile.to_toml()).unwrap();
    let (code, out, _) = run_text_in(tmp.path(), &["profiles", "list"]);
    assert_eq!(code, Some(0));
    assert!(out.starts_with("vanilla\t"), "{out}");
}

#[test]
fn render_with_an_unopenable_profile_exits_1() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    // The archives named by this profile do not exist.
    let profile = d2_assets::Profile::vanilla(std::path::Path::new("C:\\no\\such\\install"));
    std::fs::write(dir.join("vanilla.toml"), profile.to_toml()).unwrap();
    let out_png = tmp.path().join("o.png");
    let (code, _, err) = run_text_in(
        tmp.path(),
        &[
            "render",
            "data\\x.ds1",
            "--profile",
            "vanilla",
            "--out",
            out_png.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(1), "{err}");
    assert!(!out_png.exists());
}

#[test]
fn root_rule_matches_the_app() {
    let exe = std::path::Path::new("C:\\apps\\capy\\x.exe");
    assert_eq!(
        root_from(Some("D:\\home".into()), exe),
        PathBuf::from("D:\\home")
    );
    assert_eq!(root_from(Some(OsString::new()), exe), root_from(None, exe));
    assert_eq!(
        root_from(None, exe),
        PathBuf::from("C:\\apps\\capy").join("capybara")
    );
}

#[test]
fn render_object_frame_and_terrain_only_layers() {
    let a = render_ok(&["render", "a.ds1", "--profile", "v", "--out", "o.png"]);
    assert_eq!(a.object_frame, 0);
    assert!(a.visibility.objects, "objects are drawn by default");
    let a = render_ok(&[
        "render",
        "a.ds1",
        "--profile",
        "v",
        "--out",
        "o.png",
        "--Object-Frame",
        "255",
    ]);
    assert_eq!(a.object_frame, 255);
    let a = render_ok(&[
        "render",
        "a.ds1",
        "--profile",
        "v",
        "--out",
        "o.png",
        "--object-frame=3",
        "--layers",
        "floors,walls,shadow,roofs",
    ]);
    assert_eq!(a.object_frame, 3);
    assert!(
        !a.visibility.objects,
        "--layers without objects is terrain only"
    );
}

#[test]
fn help_aliases_print_help() {
    for alias in ["help", "HELP", "--help", "--HELP", "-h", "-H", "/?"] {
        assert_eq!(
            parse(&os(&[alias])),
            Ok(Some(Command::Help(None))),
            "{alias}"
        );
        let (code, out, err) = run_text(&[alias]);
        assert_eq!(code, Some(0), "{alias}: {err}");
        assert!(out.contains("verbs:"), "{alias}: {out}");
        assert!(err.is_empty(), "{alias}: {err}");
    }
    // A verb after the alias shows that verb's help.
    for alias in ["--help", "-h", "/?"] {
        assert_eq!(
            parse(&os(&[alias, "Render"])),
            Ok(Some(Command::Help(Some("render".into())))),
            "{alias}"
        );
        let (code, out, _) = run_text(&[alias, "render"]);
        assert_eq!(code, Some(0));
        assert!(out.contains("--object-frame"), "{out}");
    }
    assert_eq!(run_text(&["--help", "bogus"]).0, Some(2));
    assert_eq!(run_text(&["-h", "a", "b"]).0, Some(2));
}

#[test]
fn exit_code_table() {
    use crate::{Fail, exit_code};
    assert_eq!(exit_code(&Ok(())), 0);
    assert_eq!(exit_code(&Err(Fail::Runtime("x".into()))), 1);
    assert_eq!(exit_code(&Err(Fail::Usage("x".into()))), 2);
}

#[test]
fn version_labels_match_the_profile_toml() {
    use d2_assets::{GameVersion, Profile};

    use crate::profiles::version_label;
    for (version, label) in [
        (GameVersion::V113c, "1.13c"),
        (GameVersion::V114d, "1.14d"),
        (GameVersion::D2r, "d2r"),
    ] {
        assert_eq!(version_label(version), label);
        let mut profile = Profile::vanilla(std::path::Path::new("C:\\nowhere"));
        profile.game_version = version;
        assert!(
            profile
                .to_toml()
                .contains(&format!("game_version = \"{label}\"")),
            "{label}"
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    let profile = Profile::vanilla(std::path::Path::new("C:\\nowhere"));
    std::fs::write(dir.join("vanilla.toml"), profile.to_toml()).unwrap();
    let (code, out, _) = run_text_in(tmp.path(), &["profiles", "list"]);
    assert_eq!(code, Some(0));
    assert_eq!(out.trim_end(), format!("vanilla\t1.14d\t{}", profile.name));
}

/// The app's `config.toml` as `Config::save` writes it, with overrides for
/// two profiles.
const APP_CONFIG: &str = r#"
last_profile = "vanilla"
gamma = 1.3
autosave_minutes = 5
mode = "map"

[window]
width = 1600.0

[[recent_maps]]
profile = "vanilla"
path = 'data\global\tiles\act1\town\towne1.ds1'

[[overrides]]
profile = "vanilla"
ds1 = 'data\global\tiles\act1\town\towne1.ds1'
lvl_type = 2
def = 7

[[overrides]]
profile = "mxl"
ds1 = 'data\global\tiles\act1\town\towne1.ds1'
lvl_type = 9

[[overrides]]
profile = "vanilla"
ds1 = 'Data/Global/Tiles/Act2/Town/TownN1.ds1'
lvl_type = 5
"#;

#[test]
fn overrides_are_read_per_profile() {
    use d2_assets::GamePath;

    use crate::overrides::{MapOverride, parse as parse_overrides};
    let (vanilla, warnings) = parse_overrides(APP_CONFIG, "vanilla");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        vanilla,
        vec![
            MapOverride {
                ds1: GamePath::new(r"data\global\tiles\act1\town\towne1.ds1"),
                lvl_type: 2,
                def: Some(7),
            },
            MapOverride {
                // Canonicalised like the app does (`GamePath::new`).
                ds1: GamePath::new(r"data\global\tiles\act2\town\townn1.ds1"),
                lvl_type: 5,
                def: None,
            },
        ]
    );
    let (mxl, _) = parse_overrides(APP_CONFIG, "mxl");
    assert_eq!(mxl.len(), 1);
    assert_eq!((mxl[0].lvl_type, mxl[0].def), (9, None));
    assert!(parse_overrides(APP_CONFIG, "other").0.is_empty());
    // No overrides array at all: nothing, silently.
    assert_eq!(
        parse_overrides("gamma = 1.0\n", "vanilla"),
        (Vec::new(), Vec::new())
    );
}

#[test]
fn malformed_overrides_are_warnings() {
    use crate::overrides::parse as parse_overrides;
    // Not TOML.
    let (o, w) = parse_overrides("[[overrides]\nprofile = ", "vanilla");
    assert!(o.is_empty());
    assert_eq!(w.len(), 1, "{w:?}");
    // Not an array.
    let (o, w) = parse_overrides("overrides = 3\n", "vanilla");
    assert!(o.is_empty());
    assert_eq!(w.len(), 1, "{w:?}");
    // Bad entries are skipped one by one; good ones survive.
    let text = r#"
overrides = [
    1,
    { profile = "vanilla", ds1 = 'data\a.ds1' },
    { profile = "vanilla", ds1 = 'data\b.ds1', lvl_type = -1 },
    { profile = "vanilla", ds1 = 'data\c.ds1', lvl_type = 3, def = "x" },
    { profile = "vanilla", lvl_type = 3 },
    { profile = "mxl", ds1 = 5 },
    { profile = "vanilla", ds1 = 'data\d.ds1', lvl_type = 4 },
    { profile = "vanilla", ds1 = 'data\d.ds1', lvl_type = 6, def = 1 },
]
"#;
    let (o, w) = parse_overrides(text, "vanilla");
    assert_eq!(w.len(), 5, "{w:?}");
    assert_eq!(o.len(), 1, "{o:?}");
    assert_eq!(
        (o[0].lvl_type, o[0].def),
        (6, Some(1)),
        "the later entry wins"
    );
}

/// A `capybara\` home whose `vanilla` profile is one empty folder layer (no
/// game data), so verbs run without an install.
fn folder_home() -> tempfile::TempDir {
    use d2_assets::{GameVersion, LayerSpec, Profile};
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("gamedata");
    std::fs::create_dir_all(&data).unwrap();
    let profile = Profile {
        id: "vanilla".into(),
        name: "Folder only".into(),
        game_version: GameVersion::V114d,
        layers: vec![LayerSpec::Saves, LayerSpec::Folder { path: data }],
    };
    let dir = tmp.path().join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("vanilla.toml"), profile.to_toml()).unwrap();
    tmp
}

#[test]
fn overrides_reach_the_map_catalog() {
    use d2_assets::{GamePath, MapCatalog, Profile};

    use crate::Ctx;
    let home = folder_home();
    std::fs::write(home.path().join("config.toml"), APP_CONFIG).unwrap();
    let text = std::fs::read_to_string(home.path().join("profiles").join("vanilla.toml")).unwrap();
    let profile = Profile::from_toml(&text).unwrap();
    let overlay = profile.open(&home.path().join("saves")).unwrap();
    let (mut catalog, _) = MapCatalog::build(&overlay, profile.game_version, &[]).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut ctx = Ctx {
        root: home.path().to_path_buf(),
        out: &mut out,
        err: &mut err,
    };
    crate::overrides::apply(&mut ctx, &mut catalog, "vanilla");
    let mut applied = catalog.overrides();
    applied.sort();
    assert_eq!(
        applied,
        vec![
            (
                GamePath::new(r"data\global\tiles\act1\town\towne1.ds1"),
                2,
                Some(7)
            ),
            (
                GamePath::new(r"data\global\tiles\act2\town\townn1.ds1"),
                5,
                None
            ),
        ]
    );
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));

    // A malformed config is a warning, and no overrides are applied.
    std::fs::write(home.path().join("config.toml"), "overrides = [ nope").unwrap();
    let (mut catalog, _) = MapCatalog::build(&overlay, profile.game_version, &[]).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut ctx = Ctx {
        root: home.path().to_path_buf(),
        out: &mut out,
        err: &mut err,
    };
    crate::overrides::apply(&mut ctx, &mut catalog, "vanilla");
    assert!(catalog.overrides().is_empty());
    let err = String::from_utf8_lossy(&err);
    assert!(err.starts_with("warning: config.toml"), "{err}");
}

#[test]
fn a_map_with_no_tile_set_is_a_runtime_error() {
    let home = folder_home();
    let ds1 = d2_formats::ds1::Ds1::new(2, 2, 0);
    let file = home.path().join("loose.ds1");
    std::fs::write(&file, ds1.to_bytes().unwrap()).unwrap();
    let png = home.path().join("o.png");
    let (code, _, err) = run_text_in(
        home.path(),
        &[
            "render",
            file.to_str().unwrap(),
            "--profile",
            "vanilla",
            "--out",
            png.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("--lvltype"), "{err}");
    assert!(!png.exists());
    // An LvlType the (empty) tables do not have is a runtime error too.
    let (code, _, err) = run_text_in(
        home.path(),
        &[
            "render",
            file.to_str().unwrap(),
            "--profile",
            "vanilla",
            "--lvltype",
            "1",
            "--out",
            png.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(1), "{err}");
}

#[test]
fn object_counts_and_result_line() {
    use d2_formats::ds1::{Ds1, Object};

    use crate::render_cmd::{ObjectCounts, objects_summary};
    let mut ds1 = Ds1::new(2, 3, 0);
    let at = |x, y| Object {
        kind: 2,
        id: 1,
        x,
        y,
        flags: 0,
    };
    // Sub-tile coordinates: 2x3 cells are 10x15 sub-tiles.
    ds1.objects = vec![
        at(0, 0),
        at(9, 14),
        at(10, 0),
        at(0, 15),
        at(-1, 3),
        at(5, 5),
    ];
    assert_eq!(
        ObjectCounts::of(&ds1, 1),
        ObjectCounts {
            drawn: 2,
            missing: 1,
            outside: 3
        }
    );
    assert_eq!(objects_summary(None), "objects: off");
    assert_eq!(
        objects_summary(Some(ObjectCounts::of(&ds1, 1))),
        "objects: 2 drawn, 1 missing"
    );
}

#[test]
fn help_after_a_verb_is_that_verbs_help() {
    for args in [
        &["render", "--help"][..],
        &["render", "a.ds1", "-h"][..],
        &["maps", "/?"][..],
        &["index", "--HELP"][..],
    ] {
        let (code, out, _) = run_text(args);
        assert_eq!(code, Some(0), "{args:?}");
        assert!(!out.is_empty(), "{args:?} printed help");
    }
}
