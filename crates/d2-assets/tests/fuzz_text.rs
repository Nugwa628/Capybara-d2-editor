//! Fuzz-style tests for the text inputs of the asset layer: legacy DS1Edit
//! `assets/*.ini` files, profile TOML, game paths and listfiles.
//!
//! Each parser gets random strings, random bytes decoded lossily, mutations
//! of a valid sample and (for the ini) the private corpus' real files when
//! present. Nothing may panic; the cheap structural invariants are checked.
//!
//! Knobs: proptest cases follow `PROPTEST_CASES` (default 256); the
//! private-corpus loop runs `CAPY_FUZZ_ITERS` (else `PROPTEST_CASES`, else 64)
//! mutations per file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use d2_assets::{GamePath, KnownNames, Profile, parse_legacy_ini};
use proptest::prelude::*;

const INI_SAMPLE: &str = "\u{feff}# header\r\n\r\n; comment\r\n\
    1 1 assets/tiles/ACT1/TOWN/townE1.ds1\n\
    7 167 assets/tiles/expansion/Siege/snowbarrFD1.ds1\r\n\
    2   3\tassets\\tiles\\ACT2\\Town\\a b.ds1  \r\n\
    5 6 ./assets/tiles/act1/x.ds1\n\
    x 1 assets/tiles/act1/a.ds1\n\
    1 -4 assets/tiles/act1/b.ds1\n\
    1 1 C:\\d2\\data\\global\\tiles\\act1\\c.tg1\n";

const TOML_SAMPLE: &str = r#"
id = "mod-1"
name = "My mod"
game_version = "1.13c"

[[layer]]
kind = "saves"

[[layer]]
kind = "folder"
path = "C:\\mods\\x"

[[layer]]
kind = "mpq"
path = "a.mpq"
optional = true

[[layer]]
kind = "mpq-glob"
dir = "d"
pattern = "x-*.mpq"

[[layer]]
kind = "casc"
path = "d2r"
"#;

/// Fragments that steer mutations towards the interesting corners.
const INI_TOKENS: &[&str] = &[
    "\u{feff}",
    "\t",
    "\r",
    "\n",
    " ",
    "#",
    ";",
    "/",
    "\\",
    "..",
    "./",
    "C:",
    "c:\\",
    "\\d2\\",
    "0",
    "-1",
    "+7",
    "4294967295",
    "4294967296",
    "99999999999999999999999",
    "assets/tiles/",
    ".ds1",
    ".tg1",
    ".DS1",
    "\u{0}",
    "\u{e9}",
    "\u{1F600}",
    "\u{2028}",
];

const TOML_TOKENS: &[&str] = &[
    "[[layer]]",
    "[layer]",
    "kind = \"mpq-glob\"",
    "kind = \"nope\"",
    "=",
    "\"",
    "'",
    "'''",
    "\"\"\"",
    "[",
    "]",
    "{",
    "}",
    "\n",
    "\r\n",
    "#",
    "id = \"\"",
    "id = \"..\"",
    "optional = 1",
    "1e999",
    "-0",
    "\\u0000",
    "\\uD800",
    "\u{0}",
    "\u{feff}",
    "game_version = \"d2r\"",
    "layer = []",
    "layer = 5",
];

fn check_ini(text: &str) {
    let (entries, warnings) = parse_legacy_ini(text);
    assert!(entries.len() + warnings.len() <= text.lines().count());
    for entry in &entries {
        assert_eq!(entry.ds1.extension(), Some("ds1"), "{entry:?}");
        assert!(entry.ds1.as_str().starts_with("data\\"), "{entry:?}");
    }
}

fn check_toml(text: &str) {
    if let Ok(profile) = Profile::from_toml(text) {
        assert!(!profile.id.is_empty());
        let _ = profile.to_toml();
        let _ = profile.saves_dir(std::path::Path::new("saves"));
    }
}

fn check_game_path(raw: &str) {
    let canonical = |p: &GamePath| {
        let s = p.as_str();
        assert!(!s.contains('/'), "{s:?}");
        assert!(!s.starts_with('\\'), "{s:?}");
        assert!(!s.contains("\\\\"), "{s:?}");
        assert!(!s.bytes().any(|b| b.is_ascii_uppercase()), "{s:?}");
        let _ = p.file_name();
        let _ = p.extension();
        let _ = p.to_native(std::path::Path::new("root"));
    };
    let path = GamePath::new(raw);
    canonical(&path);
    assert_eq!(GamePath::new(path.as_str()), path, "not idempotent");
    if let Some(tile) = GamePath::from_tile_ref(raw) {
        canonical(&tile);
        assert!(tile.as_str().starts_with("data\\"), "{tile}");
    }
}

fn check_listfile(text: &str) {
    let mut names = KnownNames::new();
    let added = names.extend_from_listfile(text);
    assert_eq!(added, names.len());
    assert!(names.iter().all(|p| !p.as_str().is_empty()));
}

#[derive(Clone, Debug)]
enum Edit {
    Insert(usize, &'static str),
    Byte(usize, u8),
    Delete(usize, usize),
    Truncate(usize),
    Duplicate(usize, usize),
}

/// Applies `edits` to `sample` byte-wise and decodes the result lossily.
fn mutate(sample: &str, edits: &[Edit]) -> String {
    let mut b = sample.as_bytes().to_vec();
    for edit in edits {
        let len = b.len();
        match *edit {
            Edit::Insert(at, token) => {
                let at = at % (len + 1);
                b.splice(at..at, token.bytes());
            }
            Edit::Byte(at, value) if len > 0 => b[at % len] = value,
            Edit::Byte(..) => {}
            Edit::Delete(at, n) if len > 0 => {
                let at = at % len;
                let end = (at + n % 16).min(len);
                b.drain(at..end);
            }
            Edit::Delete(..) => {}
            Edit::Truncate(keep) => b.truncate(keep % (len + 1)),
            Edit::Duplicate(at, n) if len > 0 => {
                let at = at % len;
                let end = (at + n % 64).min(len);
                let copy = b[at..end].to_vec();
                b.splice(at..at, copy);
            }
            Edit::Duplicate(..) => {}
        }
    }
    String::from_utf8_lossy(&b).into_owned()
}

fn edits(tokens: &'static [&'static str]) -> impl Strategy<Value = Vec<Edit>> {
    let edit = prop_oneof![
        3 => (any::<usize>(), prop::sample::select(tokens)).prop_map(|(at, t)| Edit::Insert(at, t)),
        2 => (any::<usize>(), any::<u8>()).prop_map(|(at, v)| Edit::Byte(at, v)),
        2 => (any::<usize>(), any::<usize>()).prop_map(|(at, n)| Edit::Delete(at, n)),
        1 => any::<usize>().prop_map(Edit::Truncate),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(at, n)| Edit::Duplicate(at, n)),
    ];
    prop::collection::vec(edit, 1..10)
}

/// A line made of ini-shaped pieces: numbers (some huge or signed), paths.
fn ini_line() -> impl Strategy<Value = String> {
    let number = prop_oneof![
        any::<u32>().prop_map(|n| n.to_string()),
        any::<i64>().prop_map(|n| n.to_string()),
        prop::sample::select(INI_TOKENS).prop_map(str::to_owned),
    ];
    let path = prop::collection::vec(
        prop_oneof![
            "[a-zA-Z0-9 _.]{0,8}",
            prop::sample::select(INI_TOKENS).prop_map(str::to_owned),
        ],
        0..6,
    )
    .prop_map(|parts| parts.concat());
    let sep = prop::sample::select(&[" ", "\t", "  ", " \t ", ""][..]);
    (number.clone(), sep.clone(), number, sep, path)
        .prop_map(|(a, s1, b, s2, p)| format!("{a}{s1}{b}{s2}{p}"))
}

proptest! {
    // Case count from `PROPTEST_CASES` (default 256); no regression files.
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn legacy_ini_random_text(text in "\\PC*") {
        check_ini(&text);
    }

    #[test]
    fn legacy_ini_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        check_ini(&String::from_utf8_lossy(&bytes));
    }

    #[test]
    fn legacy_ini_mutated_sample(edits in edits(INI_TOKENS)) {
        check_ini(&mutate(INI_SAMPLE, &edits));
    }

    #[test]
    fn legacy_ini_shaped_lines(
        lines in prop::collection::vec(ini_line(), 0..12),
        newline in prop::sample::select(&["\n", "\r\n", "\r"][..]),
    ) {
        check_ini(&lines.join(newline));
    }

    #[test]
    fn profile_toml_random_text(text in "\\PC*") {
        check_toml(&text);
    }

    #[test]
    fn profile_toml_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        check_toml(&String::from_utf8_lossy(&bytes));
    }

    #[test]
    fn profile_toml_mutated_sample(edits in edits(TOML_TOKENS)) {
        check_toml(&mutate(TOML_SAMPLE, &edits));
    }

    #[test]
    fn profile_toml_mutated_builtin(edits in edits(TOML_TOKENS)) {
        let builtin = Profile::median_xl(std::path::Path::new("C:\\games\\median-xl")).to_toml();
        check_toml(&mutate(&builtin, &edits));
    }

    #[test]
    fn game_paths_random_text(text in "\\PC*") {
        check_game_path(&text);
    }

    #[test]
    fn game_paths_shaped(parts in prop::collection::vec(
        prop_oneof![
            "[a-zA-Z0-9._ ]{0,6}",
            prop::sample::select(INI_TOKENS).prop_map(str::to_owned),
        ],
        0..8,
    )) {
        check_game_path(&parts.concat());
    }

    #[test]
    fn listfile_random_text(text in "\\PC*") {
        check_listfile(&text);
    }

    #[test]
    fn listfile_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        check_listfile(&String::from_utf8_lossy(&bytes));
    }
}

/// Mutations of the real DS1Edit ini files from the private corpus (skips
/// when the corpus is absent).
#[test]
fn legacy_ini_private_corpus_mutations() {
    let dir = std::env::var_os("CAPY_LEGACY_INI_DIR").map_or_else(
        || common::private_dir().join("oracle").join("assets"),
        std::path::PathBuf::from,
    );
    if !common::dir_available(&dir) {
        return;
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ini")))
        .collect();
    files.sort();
    let iters = common::fuzz_iters(64);
    println!(
        "legacy_ini_private_corpus_mutations: {} files x {iters} mutations",
        files.len()
    );
    let mut seed = 0x2545_F491u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 4) as usize
    };
    for file in files {
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        check_ini(&text);
        for _ in 0..iters {
            let edits: Vec<Edit> = (0..1 + next() % 6)
                .map(|_| match next() % 5 {
                    0 => Edit::Insert(next(), INI_TOKENS[next() % INI_TOKENS.len()]),
                    1 => Edit::Byte(next(), next() as u8),
                    2 => Edit::Delete(next(), next()),
                    3 => Edit::Truncate(next()),
                    _ => Edit::Duplicate(next(), next()),
                })
                .collect();
            check_ini(&mutate(&text, &edits));
        }
    }
}
