#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::ffi::OsString;
use std::path::PathBuf;

use capy_render::{VariantRule, Visibility};

use crate::args::{Command, RenderArgs, parse};
use crate::{root_from, run_in};

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
        &["--help"],
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

fn run_text_in(root: &std::path::Path, args: &[&str]) -> (Option<u8>, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_in(&os(args), root.to_path_buf(), &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
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
