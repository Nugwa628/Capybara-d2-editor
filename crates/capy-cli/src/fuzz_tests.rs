//! Fuzz-style robustness tests of the argument parser and the overrides
//! reader: arbitrary argument vectors (including non-UTF-8 ones) and
//! arbitrary or TOML-shaped text. Nothing may panic. Case counts follow
//! `PROPTEST_CASES`.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::ffi::OsString;

use proptest::prelude::*;

use crate::args::{Command, parse, parse_zoom};
use crate::{overrides, run_in};

/// Pieces that steer the parser into its branches.
fn token() -> impl Strategy<Value = OsString> {
    let words = prop::sample::select(vec![
        "render",
        "RENDER",
        "index",
        "maps",
        "profiles",
        "help",
        "--help",
        "-h",
        "/?",
        "status",
        "build",
        "list",
        "--profile",
        "--out",
        "--zoom",
        "--zoom=1:4",
        "--zoom=1:0",
        "--gamma",
        "--gamma=nan",
        "--gamma=1e40",
        "--layers",
        "--layers=,",
        "--layers=all",
        "--variant",
        "--lvltype",
        "--lvltype=-1",
        "--def",
        "--object-frame",
        "--object-frame=256",
        "--filter",
        "--",
        "--=",
        "=",
        "1:16",
        "17",
        "4294967296",
        "",
        " ",
        "x.ds1",
    ]);
    prop_oneof![
        4 => words.prop_map(OsString::from),
        2 => ".{0,12}".prop_map(OsString::from),
        1 => non_utf8(),
    ]
}

/// A string that is not valid Unicode on this platform.
#[cfg(windows)]
fn non_utf8() -> impl Strategy<Value = OsString> {
    use std::os::windows::ffi::OsStringExt;
    prop::collection::vec(any::<u16>(), 0..6).prop_map(|mut units| {
        units.push(0xD800); // a lone surrogate
        OsString::from_wide(&units)
    })
}

#[cfg(not(windows))]
fn non_utf8() -> impl Strategy<Value = OsString> {
    use std::os::unix::ffi::OsStringExt;
    prop::collection::vec(any::<u8>(), 0..6).prop_map(|mut bytes| {
        bytes.push(0xFF);
        OsString::from_vec(bytes)
    })
}

/// TOML-shaped text built from fragments of an overrides file.
fn toml_like() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "[[overrides]]\n",
        "[overrides]\n",
        "overrides = 3\n",
        "overrides = [1, \"a\", {}]\n",
        "profile = \"vanilla\"\n",
        "profile = 7\n",
        "ds1 = \"data\\\\global\\\\tiles\\\\act1\\\\town\\\\towne1.ds1\"\n",
        "ds1 = \"\"\n",
        "lvl_type = 2\n",
        "lvl_type = -1\n",
        "lvl_type = 99999999999\n",
        "def = 1\n",
        "def = \"x\"\n",
        "def = 1.5\n",
        "x = [[[[[[[[1]]]]]]]]\n",
        "\"\n",
        "[",
        "]]\n",
        "= = =\n",
        "# comment\n",
    ]);
    prop::collection::vec(pieces, 0..20).prop_map(|p| p.concat())
}

proptest! {
    #[test]
    fn parse_takes_any_arguments(args in prop::collection::vec(token(), 0..8)) {
        let parsed = parse(&args);
        // Only runs that cannot reach a profile or the file system.
        if matches!(parsed, Err(_) | Ok(None) | Ok(Some(Command::Help(_)))) {
            let root = std::env::temp_dir().join("capy-cli-fuzz-no-such-root");
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_in(&args, root, &mut out, &mut err);
            prop_assert!(code.is_none_or(|c| c <= 2));
        }
    }

    #[test]
    fn parse_zoom_takes_any_text(text in ".{0,16}") {
        if let Some(n) = parse_zoom(&text) {
            prop_assert!((1..=16).contains(&n));
        }
    }

    #[test]
    fn overrides_take_any_text(text in ".{0,200}", id in prop_oneof![Just("vanilla".to_owned()), ".{0,6}"]) {
        let (found, _) = overrides::parse(&text, &id);
        prop_assert!(found.is_empty() || !id.is_empty() || text.contains("profile"));
    }

    #[test]
    fn overrides_take_toml_shaped_text(text in toml_like()) {
        let (found, _) = overrides::parse(&text, "vanilla");
        for o in &found {
            prop_assert!(!o.ds1.as_str().is_empty());
        }
    }
}

/// Deeply nested TOML must be an error (or parse), never a stack overflow.
#[test]
fn deeply_nested_toml_is_not_a_stack_overflow() {
    for depth in [100usize, 1_000, 20_000] {
        let text = format!("x = {}1{}\n", "[".repeat(depth), "]".repeat(depth));
        let _ = overrides::parse(&text, "vanilla");
        let text = format!("x = {}\n", "{a = ".repeat(depth));
        let _ = overrides::parse(&text, "vanilla");
    }
}
