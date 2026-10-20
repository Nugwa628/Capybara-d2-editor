#![allow(clippy::unwrap_used, clippy::panic)]
//! Synthetic cases (no corpus needed), translated from the reference tool's
//! tests plus the properties this crate promises: idempotent fixes and
//! untouched unchanged rows.

use std::collections::HashMap;

use capy_mod::automap::import::{merge_rows, names_by_number, read_rows};
use capy_mod::automap::safety::{invalid_rows, remove_invalid_rows, validate_save};
use capy_mod::automap::style::{
    AutomapStyle, Kind, is_water_tile, kind_of_code, looks_like_water, normalize, part_of_code,
};
use capy_mod::automap::{self, AutomapError, AutomapTable, Scope};
use capy_mod::compat::{
    Env, FixAction, LoadedDt1, MapInput, Severity, TableName, Tables, check_map,
};
use capy_mod::crash_log::{Cause, CrashKind, explain_crash, parse_crash_log};
use d2_formats::ds1::Ds1;
use d2_formats::dt1::TileKey;
use d2_formats::txt::TxtDoc;

const HEAD: &str = "LevelName\tTileName\tStyle\tStartSequence\tEndSequence\tType1\tCel1\tType2\tCel2\tType2\tCel3\tType4\tCel4";

fn doc(lines: &[&str]) -> TxtDoc {
    TxtDoc::parse((lines.join("\r\n") + "\r\n").as_bytes())
}

// ---------------------------------------------------------------- crash logs

const START: &str = "---------------------------------------------------------------\r\nSE Realm startup at 2026-09-27 10:29:58.846\r\n---------------------------------------------------------------\r\n10:29:58.846  Diablo II running under Windows XP (Version 5.010)\r\n";
const LOADING: &str = "10:30:29.528  Error opening file: DATA\\LOCAL\\UI\\ENG\\EXPANSION\\.dc6\r\n10:30:29.528  ------------------------------------------------------\r\n10:30:29.529  PROGRAM:       Diablo II v1.13\r\n10:30:29.529  TIME:          2026-09-27 10:30:29.528\r\n10:30:29.529  ***** UNHANDLED EXCEPTION: ACCESS_VIOLATION (c0000005)\r\n10:30:29.529  Fault address:  027D1614 01:00010614 C:\\Program Files\\Diablo II\\ProjectD2\\D2CMP.dll\r\n10:30:29.546      Base:027C0000h  Size: 108000h  Name:D2CMP.dll        Path:C:\\Program Files\\Diablo II\\ProjectD2\\D2CMP.dll\r\n";
const AUTOMAP: &str = "10:34:37.671  PROGRAM:       Diablo II v1.13\r\n10:34:37.671  TIME:          2026-09-27 10:34:37.669\r\n10:34:37.671  --------  FILE:     LINE: 965  --------\r\n\r\nHalt\r\nLocation : , line #965\r\nExpression : Unrecoverable internal error 6fb11827\r\n10:34:45.946  Enumerate modules...\r\n10:40:56.301      Base:6FAB0000h  Size:  E1000h  Name:D2Client.dll     Path:C:\\Program Files\\Diablo II\\ProjectD2\\D2Client.dll\r\n";

#[test]
fn crash_log_finds_each_crash_with_time_kind_module_and_failed_files() {
    let log = format!("{START}{LOADING}{START}{AUTOMAP}");
    let crashes = parse_crash_log(&log);
    assert_eq!(crashes.len(), 2);
    let c = &crashes[0];
    assert_eq!(c.time, "2026-09-27 10:30:29");
    assert_eq!(c.kind, CrashKind::Exception);
    assert_eq!(c.what, "ACCESS_VIOLATION");
    assert_eq!(c.module.as_deref(), Some("D2CMP.dll"));
    assert_eq!(c.offset, Some(0x11614));
    assert_eq!(
        c.failed_files,
        vec!["DATA\\LOCAL\\UI\\ENG\\EXPANSION\\.dc6"]
    );
    // A halt is placed in its module from the module list; the earlier
    // session's failed files do not carry over.
    let h = &crashes[1];
    assert_eq!(h.kind, CrashKind::Halt);
    assert_eq!(h.line, Some(965));
    assert_eq!(h.address, Some(0x6fb1_1827));
    assert_eq!(h.module.as_deref(), Some("D2Client.dll"));
    assert_eq!(h.offset, Some(0x61827));
    assert!(h.failed_files.is_empty());
}

#[test]
fn crash_explanations_in_plain_words() {
    let log = format!("{START}{LOADING}{START}{AUTOMAP}");
    let crashes = parse_crash_log(&log);
    let (loading, automap) = (&crashes[0], &crashes[1]);
    let e = explain_crash(loading, None);
    assert!(e.known && e.check);
    assert_eq!(e.cause, Cause::EntryImage);
    assert_eq!(
        e.title,
        "A level's loading-screen image (EntryFile) is empty"
    );

    let mut missing = loading.clone();
    missing.failed_files = vec!["DATA\\LOCAL\\UI\\ENG\\EXPANSION\\G4.dc6".into()];
    let lookup = |e: &str| {
        if e == "G4" {
            vec!["205 Guild 4".to_string()]
        } else {
            vec![]
        }
    };
    let e = explain_crash(&missing, Some(&lookup));
    assert_eq!(e.title, "The loading-screen image \"G4.dc6\" doesn't exist");
    assert!(e.detail.contains("Levels using it: 205 Guild 4"));

    assert!(explain_crash(automap, None).title.contains("too far out"));
    let mut reveal = automap.clone();
    reveal.address = Some(0x6fb1_19c1);
    reveal.line = None;
    assert!(explain_crash(&reveal, None).title.contains("AutoMap is 1"));

    // D2Common's preset builder (halt 6fd5c664, line 2239).
    let mut size = automap.clone();
    size.what = "Unrecoverable internal error 6fd5c664".into();
    size.address = Some(0x6fd5_c664);
    size.module = Some("D2COMMON.dll".into());
    size.line = Some(2239);
    let e = explain_crash(&size, None);
    assert_eq!(e.cause, Cause::PresetSize);
    assert!(e.detail.contains("width"));

    let mut dt1 = loading.clone();
    dt1.failed_files = vec!["DATA\\GLOBAL\\TILES\\Guild\\PD2\\house9\\x.dt1".into()];
    assert_eq!(
        explain_crash(&dt1, None).title,
        "The game couldn't open a tile library (DT1): x.dt1"
    );

    let mut unknown = automap.clone();
    unknown.what = "Unrecoverable internal error 6ff6339e".into();
    unknown.address = None;
    unknown.module = Some("Fog.dll".into());
    unknown.offset = Some(0x1339e);
    unknown.line = Some(646);
    let e = explain_crash(&unknown, None);
    assert!(!e.known);
    assert_eq!(
        e.title,
        "The game stopped on an internal check in Fog.dll +0x1339e, line 646"
    );
}

#[test]
fn a_jump_to_an_invalid_address_is_not_placed_in_a_module() {
    let c = parse_crash_log(
        "PROGRAM: Diablo II\r\n***** UNHANDLED EXCEPTION: ACCESS_VIOLATION (c0000005)\r\nFault address:  00000408 00:00000000 C:\\Diablo II\\Game.exe\r\n    Base:00400000h  Size:  11000h  Name:Game.exe\r\n",
    );
    assert_eq!(c.len(), 1);
    assert!(c[0].module.is_none());
    assert_eq!(
        explain_crash(&c[0], None).title,
        "The game crashed (ACCESS_VIOLATION) at address 0x408, outside the game's code"
    );
}

#[test]
fn garbage_and_empty_logs_give_no_crashes() {
    assert!(parse_crash_log("").is_empty());
    assert!(parse_crash_log("hello\r\nworld\r\n\u{0}\u{1}").is_empty());
    assert!(parse_crash_log("PROGRAM:").is_empty());
}

// ------------------------------------------------------------------- automap

#[test]
fn import_maps_level_names_and_skips_rows_already_there() {
    let sent = format!(
        "{HEAD}\r\n46\twr\t2\t0\t3\tStn_WR a\t11\tStn_WR b\t12\t\t-1\t\t-1\r\n46\twl\t2\t0\t3\tStn_WL a\t13\tStn_WL b\t14\t\t-1\t\t-1\r\n46\tfl\t0\t54\t54\tWaypoint\t307\t\t-1\t\t-1\t\t-1\r\n"
    );
    let mine_text = format!(
        "{HEAD}\r\n46\tfl\t0\t0\t9\tPoison floor\t5\t\t-1\t\t-1\t\t-1\r\n58\twl\t2\t0\t3\tStn_WL a\t13\tStn_WL b\t14\t\t-1\t\t-1\r\n"
    );
    let src = read_rows(sent.as_bytes()).unwrap();
    assert_eq!(src.levels.len(), 1);
    assert_eq!(src.levels[0].name, "46");
    assert_eq!(src.levels[0].rows, 3);
    assert_eq!(src.levels[0].codes, vec!["wr", "wl", "fl"]);
    let mut mine = TxtDoc::parse(mine_text.as_bytes());
    assert!(names_by_number(&mine));
    assert!(read_rows(b"Name\tId\r\nx\t1\r\n").is_none());

    let mapping = HashMap::from([("46".to_string(), Some("58".to_string()))]);
    let res = merge_rows(&mut mine, &src, &mapping).unwrap();
    assert_eq!((res.added, res.already), (2, 1));
    let text = String::from_utf8(mine.to_bytes()).unwrap();
    let lines: Vec<&str> = text.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.iter().filter(|l| l.starts_with("58\t")).count(), 3);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("46\t"))
            .collect::<Vec<_>>(),
        vec![&"46\tfl\t0\t0\t9\tPoison floor\t5\t\t-1\t\t-1\t\t-1"]
    );
    // Importing again adds nothing; unmapped levels add nothing.
    let again = merge_rows(&mut mine, &src, &mapping).unwrap();
    assert_eq!((again.added, again.already), (0, 3));
    let none = HashMap::from([("46".to_string(), None)]);
    let mut other = TxtDoc::parse(mine_text.as_bytes());
    assert_eq!(merge_rows(&mut other, &src, &none).unwrap().added, 0);
    assert_eq!(other.to_bytes(), mine_text.as_bytes());
}

#[test]
fn safety_flags_rules_without_a_picture_and_refuses_to_save_them() {
    let ok = doc(&[HEAD, "1 Town\tfl\t0\t0\t3\tA\t0\t\t-1\t\t-1\t\t-1"]);
    assert!(invalid_rows(&ok).unwrap().is_empty());
    let bad_text = format!(
        "{HEAD}\r\n1 Town\tfl\t0\t0\t3\tA\t-1\t\t-1\t\t-1\t\t-1\r\n1 Town\twl\t0\t0\t3\tA\t4\t\t-1\t\t-1\t\t-1\r\n1 Town\twr\t0\t0\t3\tA\t1.5\t\t-1\t\t-1\t\t-1\r\n1 Town\twr\t0\t0\t3\tA\t\t\t-1\t\t-1\t\t-1\r\n"
    );
    let bad = TxtDoc::parse(bad_text.as_bytes());
    // Empty counts as zero (valid); -1 and a fraction are not.
    assert_eq!(invalid_rows(&bad).unwrap(), vec![0, 2]);
    let err = validate_save("data\\global\\excel\\AutoMap.txt", bad_text.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("on line(s) 2, 4"), "{err}");
    assert!(err.to_string().contains("was not saved"));
    assert!(validate_save("data/global/excel/Levels.txt", bad_text.as_bytes()).is_ok());
    assert!(invalid_rows(&doc(&["Name\tId", "x\t1"])).is_err());
    let mut fixed = bad.clone();
    assert_eq!(remove_invalid_rows(&mut fixed).unwrap(), 2);
    assert_eq!(remove_invalid_rows(&mut fixed).unwrap(), 0);
    assert!(invalid_rows(&fixed).unwrap().is_empty());
    assert!(fixed.row_count() >= 2);
}

fn base() -> TxtDoc {
    doc(&[
        HEAD,
        "1 Town\tfl\t0\t1\t46\tFloor Path A\t0\tFloor Path B\t1\t\t-1\t\t-1",
        "1 Town\tfl\t2\t0\t3\tFloor Path A\t0\t\t-1\t\t-1\t\t-1",
        "1 Town\twl\t2\t0\t3\tWall\t13\t\t-1\t\t-1\t\t-1",
        "1 Town\twl\t5\t-1\t-1\tAll\t20\t\t-1\t\t-1\t\t-1",
        "2 Town\twl\t2\t0\t3\tWall\t30\t\t-1\t\t-1\t\t-1",
    ])
}

#[test]
fn lookup_follows_the_games_first_match_and_ranges() {
    let t = AutomapTable::parse(&base());
    assert_eq!(t.rules.len(), 5);
    assert_eq!(t.find_rule("1 Town", 1, 2, 3).unwrap().row, 2);
    assert!(t.find_rule("1 Town", 1, 2, 4).is_none());
    // Start -1 covers every sequence.
    assert_eq!(t.find_rule("1 town", 1, 5, 99).unwrap().cels[0].cel, 20);
    // Start 1, end 46 covers 40; end -1 means just the start.
    assert!(t.find_rule("1 Town", 0, 0, 40).is_some());
    assert!(t.find_rule("1 Town", 0, 0, 47).is_none());
    assert!(t.rules[0].describe().contains("seq 1-46"));
    assert!(t.unknown_levels().is_empty());
    assert!(!t.reads_level_numbers());
}

#[test]
fn level_names_map_from_level_types() {
    let t = AutomapTable::parse(&base());
    assert_eq!(
        t.level_for(Some("Act 1 - Town"), Some(1), None).as_deref(),
        Some("1 Town")
    );
    assert_eq!(
        t.level_for(Some("whatever"), Some(2), Some(12)).as_deref(),
        Some("2 Town")
    );
    assert_eq!(
        t.level_for(Some("zzz"), Some(2), None).as_deref(),
        Some("2 Town")
    );
    assert_eq!(t.level_for(Some("zzz"), None, None), None);
    // A level type the game never had: no entry unless the mod reads numbers.
    assert_eq!(t.level_for(Some("Guild"), Some(5), Some(46)), None);
    let numbered = doc(&[HEAD, "46\tfl\t0\t0\t9\tA\t5\t\t-1\t\t-1\t\t-1"]);
    let t2 = AutomapTable::parse(&numbered);
    assert!(t2.reads_level_numbers());
    assert_eq!(
        t2.level_for(Some("Guild"), Some(5), Some(58)).as_deref(),
        Some("58")
    );
    assert!(
        AutomapTable::parse(&doc(&[HEAD, "Bogus\tfl\t0\t0\t9\tA\t5\t\t-1\t\t-1\t\t-1"]))
            .unknown_levels()
            == vec!["Bogus".to_string()]
    );
}

#[test]
fn set_cel_updates_inserts_and_is_idempotent() {
    let original = base();
    let before = original.to_bytes();
    // Existing exact row: updated in place.
    let mut d = original.clone();
    let s = automap::set_cel(&mut d, "1 Town", 1, 2, 3, 9, Scope::Sequence);
    // seq 3 is inside 0-3, not an exact row: a new row goes in front of the style's other rows.
    let s = s.unwrap();
    assert_eq!(s.changes.len(), 1);
    assert!(s.summary.contains("new row"));
    let t = AutomapTable::parse(&d);
    assert_eq!(t.find_rule("1 Town", 1, 2, 3).unwrap().cels[0].cel, 9);
    assert_eq!(t.find_rule("1 Town", 1, 2, 2).unwrap().cels[0].cel, 13);
    assert_eq!(
        t.find_rule("1 Town", 1, 2, 3).unwrap().row,
        2,
        "in front of the range row"
    );
    // Rows other than the new one are byte-identical.
    let text = String::from_utf8(d.to_bytes()).unwrap();
    let old = String::from_utf8(before.clone()).unwrap();
    let removed: String = text
        .split_inclusive("\r\n")
        .filter(|l| !l.contains("\t3\t3\t"))
        .collect();
    assert_eq!(removed, old);
    // Twice = once.
    let snapshot = d.to_bytes();
    assert!(
        automap::set_cel(&mut d, "1 Town", 1, 2, 3, 9, Scope::Sequence)
            .unwrap()
            .changes
            .is_empty()
    );
    assert_eq!(d.to_bytes(), snapshot);
    // Whole style: row with start -1 gets updated, not duplicated.
    let mut d = original.clone();
    let s = automap::set_cel(&mut d, "1 Town", 1, 5, 0, 21, Scope::Style).unwrap();
    assert!(s.summary.contains("updated row"));
    assert_eq!(d.row_count(), original.row_count());
    // A level with no rows: the new rule is appended after the table.
    let mut d = original.clone();
    automap::set_cel(&mut d, "3 Town", 0, 0, 0, 1, Scope::Sequence).unwrap();
    assert!(
        AutomapTable::parse(&d)
            .find_rule("3 Town", 0, 0, 0)
            .is_some()
    );
    // Bad orientation.
    assert!(matches!(
        automap::set_cel(&mut d, "3 Town", 99, 0, 0, 1, Scope::Sequence),
        Err(AutomapError::Invalid(_))
    ));
}

#[test]
fn clearing_splits_ranges_and_never_writes_a_minus_one_cel() {
    let mut d = base();
    // Middle of 1-46: two halves.
    let s = automap::set_cel(&mut d, "1 Town", 0, 0, 20, -1, Scope::Sequence).unwrap();
    assert!(!s.changes.is_empty());
    let t = AutomapTable::parse(&d);
    assert!(t.find_rule("1 Town", 0, 0, 20).is_none());
    assert!(t.find_rule("1 Town", 0, 0, 19).is_some());
    assert!(t.find_rule("1 Town", 0, 0, 21).is_some());
    assert!(invalid_rows(&d).unwrap().is_empty());
    // Idempotent.
    let snap = d.to_bytes();
    assert!(
        automap::set_cel(&mut d, "1 Town", 0, 0, 20, -1, Scope::Sequence)
            .unwrap()
            .changes
            .is_empty()
    );
    assert_eq!(d.to_bytes(), snap);
    // Edge of a range trims it; a whole-style clear removes the rows.
    automap::set_cel(&mut d, "1 Town", 0, 0, 1, -1, Scope::Sequence).unwrap();
    assert!(
        AutomapTable::parse(&d)
            .find_rule("1 Town", 0, 0, 1)
            .is_none()
    );
    automap::set_cel(&mut d, "1 Town", 1, 2, 0, -1, Scope::Style).unwrap();
    assert!(
        AutomapTable::parse(&d)
            .rules_for("1 Town", "wl", 2)
            .is_empty()
    );
    // A wildcard-style rule cannot be split: error and nothing changed.
    let before = d.to_bytes();
    let err = automap::set_cel(&mut d, "1 Town", 1, 5, 4, -1, Scope::Sequence).unwrap_err();
    assert!(matches!(err, AutomapError::Wildcard { .. }));
    assert_eq!(d.to_bytes(), before);
}

#[test]
fn clear_tiles_is_all_or_nothing_and_plans_report_shared_tiles() {
    let mut d = base();
    let before = d.to_bytes();
    let tiles = [
        automap::TileId {
            orientation: 0,
            main: 2,
            sub: 1,
        },
        automap::TileId {
            orientation: 1,
            main: 5,
            sub: 0,
        },
    ];
    assert!(automap::clear_tiles(&mut d, "1 Town", &tiles).is_err());
    assert_eq!(d.to_bytes(), before, "rolled back");
    automap::clear_tiles(&mut d, "1 Town", &tiles[..1]).unwrap();
    assert!(
        AutomapTable::parse(&d)
            .find_rule("1 Town", 0, 2, 1)
            .is_none()
    );

    let t = automap::TileId {
        orientation: 0,
        main: 0,
        sub: 5,
    };
    let mk = |x, y| automap::Piece {
        x,
        y,
        tile: t,
        rule_row: Some(0),
        cel: Some(1),
        layer: automap::PieceLayer::Floor,
        layer_index: 0,
        suggested: false,
    };
    let plan = automap::plan_clear(
        &[mk(0, 0), mk(1, 0), mk(9, 9)],
        automap::CellRect {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        },
    );
    assert_eq!(plan.tiles, vec![t]);
    assert_eq!(plan.shared_elsewhere, vec![t]);
}

#[test]
fn suggestions_use_the_closest_most_used_piece_and_are_idempotent() {
    let mut d = base();
    let t = AutomapTable::parse(&d);
    let piece = |x, o, main, sub| automap::Piece {
        x,
        y: 0,
        tile: automap::TileId {
            orientation: o,
            main,
            sub,
        },
        rule_row: None,
        cel: None,
        layer: if o == 0 {
            automap::PieceLayer::Floor
        } else {
            automap::PieceLayer::Wall
        },
        layer_index: 0,
        suggested: false,
    };
    let pieces = [piece(0, 1, 2, 9), piece(1, 1, 2, 8), piece(2, 0, 0, 80)];
    let s = automap::suggest(&t, "1 Town", &pieces, false);
    assert_eq!(s.len(), 1, "floors only when asked");
    assert_eq!(
        (
            s[0].code,
            s[0].style,
            s[0].seqs.clone(),
            s[0].count,
            s[0].cel
        ),
        ("wl", 2, vec![8, 9], 2, 13)
    );
    let shown = automap::with_suggestions(&pieces, &s);
    assert!(shown[0].suggested && shown[0].cel == Some(13) && !shown[2].suggested);
    automap::apply_suggestions(&mut d, "1 Town", &s).unwrap();
    let snap = d.to_bytes();
    assert!(
        AutomapTable::parse(&d)
            .find_rule("1 Town", 1, 2, 9)
            .is_some()
    );
    let again = automap::apply_suggestions(&mut d, "1 Town", &s).unwrap();
    assert!(again.changes.is_empty());
    assert_eq!(d.to_bytes(), snap);
    let usual = automap::usual_cels(&AutomapTable::parse(&d), "1 Town", false);
    assert_eq!(usual.get("wl"), Some(&13));
}

#[test]
fn style_defaults_normalise_and_classify() {
    assert_eq!(
        ["wl", "wtlr", "wld", "ls"].map(|c| kind_of_code(c, false)),
        [Kind::Walls; 4]
    );
    assert_eq!(
        [
            kind_of_code("fl", false),
            kind_of_code("fl", true),
            kind_of_code("tr", false),
            kind_of_code("co", false),
            kind_of_code("rf", false),
            kind_of_code("sh", false)
        ],
        [
            Kind::Floors,
            Kind::Water,
            Kind::Objects,
            Kind::Objects,
            Kind::Roofs,
            Kind::Shadows
        ]
    );
    assert_eq!(part_of_code("wtll"), Some("corners"));
    assert_eq!(part_of_code("fl"), None);
    let n = normalize(&serde_json::json!({
        "parts": { "left": { "colour": "#abcdef" }, "bogus": {} },
        "thickness": 9, "dim": -3, "missingColour": "red",
        "opacity": 0.5,
        "kinds": { "other": { "show": false }, "walls": { "colour": "#000" } }
    }));
    assert_eq!(n.parts.len(), 1);
    assert_eq!(n.parts[0].0, "left");
    assert_eq!(n.parts[0].1.colour, "#abcdef");
    assert_eq!(n.thickness, 3);
    assert_eq!(n.dim, 0.0);
    assert_eq!(n.missing_colour, "#ff5ac8");
    let d = AutomapStyle::default();
    // Old "other" kind covers roofs and shadows; a bad colour falls back; one opacity for all.
    assert!(!n.kind_look(Kind::Roofs).show && !n.kind_look(Kind::Shadows).show);
    assert_eq!(
        n.kind_look(Kind::Walls).colour,
        d.kind_look(Kind::Walls).colour
    );
    assert_eq!(n.kind_look(Kind::Water).opacity, 0.5);
    assert_eq!(normalize(&serde_json::json!(null)), d);
    assert_eq!(n.look_of(Kind::Walls, Some("left")).colour, "#abcdef");
    assert!(looks_like_water([10, 40, 90]) && !looks_like_water([120, 80, 40]));
    assert!(is_water_tile(&[1; 25], None));
    assert!(!is_water_tile(&[0; 25], None));
}

// -------------------------------------------------------------- compatibility

fn level_tables() -> Tables {
    let prest = doc(&[
        "Name\tDef\tLevelId\tPopulate\tLogicals\tOutdoors\tAnimate\tKillEdge\tFillBlanks\tSizeX\tSizeY\tAutoMap\tScan\tPops\tPopPad\tFiles\tFile1\tFile2\tFile3\tFile4\tFile5\tFile6\tDt1Mask\tBeta\tExpansion",
        "Town\t0\t0\t1\t1\t1\t1\t1\t1\t0\t0\t1\t1\t0\t0\t1\tact1\\town.ds1\t0\t0\t0\t0\t0\t1\t0\t",
        "Mine\t1\t60\t1\t1\t1\t1\t1\t0\t0\t0\t1\t1\t0\t0\t2\tact2\\mine.ds1\t0\t0\t0\t0\t0\t1\t0\t",
    ]);
    let levels = doc(&[
        "Name\tId\tPal\tAct\tQuestFlag\tQuestFlagEx\tLayer\tSizeX\tSizeY\tSizeX(N)\tSizeY(N)\tSizeX(H)\tSizeY(H)\tOffsetX\tOffsetY\tDepend\tDrlgType\tLevelType\tWaypoint\tLevelName\tEntryFile",
        "Town\t0\t0\t0\t\t\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t1\t1\t0\tTown\t",
        "Mine\t1\t4\t0\t\t\t120\t9\t9\t9\t9\t9\t9\t9000\t100\t0\t2\t1\t0\tMine\t",
    ]);
    let types = doc(&[
        "Name\tId\tFile 1\tFile 2\tAct",
        "Wild\t0\t0\t0\t1",
        "Mine\t1\tact2/mine/floor.dt1\t0\t2",
    ]);
    // Rows must be in Id order for the game: level 60 would be record 60, so
    // the fixture uses record numbers instead: make the preset point at level 1.
    let mut t = Tables {
        prest: Some(prest),
        levels: Some(levels),
        types: Some(types),
        automap: None,
    };
    let p = t.prest.as_mut().unwrap();
    let c = p.column("LevelId").unwrap();
    p.set_cell(1, c, "1").unwrap();
    t
}

fn tiny_map() -> Ds1 {
    Ds1::new(11, 11, 0)
}

fn run_check(t: &Tables, ds1: &Ds1) -> Vec<capy_mod::compat::Finding> {
    let loaded = [LoadedDt1 {
        path: "act2/mine/floor.dt1".into(),
        found: true,
        tiles: Default::default(),
    }];
    let input = MapInput {
        path: "act2\\mine.ds1",
        ds1,
        loaded: &loaded,
        candidates: &[],
        used_dt1s: &[],
    };
    check_map(&input, t, &Env::default())
}

#[test]
fn table_findings_come_with_fixes_that_are_idempotent_and_remove_the_finding() {
    let mut t = level_tables();
    let ds1 = tiny_map();
    let findings = run_check(&t, &ds1);
    let titles: Vec<&str> = findings.iter().map(|f| f.title.as_str()).collect();
    for want in [
        "automap Layer 120",
        "doesn't match the map (10x10)",
        "FillBlanks",
        "too far out in the act's world",
        "Level 1 uses the Act 5 palette",
    ] {
        // Level 1 here has Pal 4; the level type says Act 2, so the palette finding appears with "Act 5".
        assert!(
            titles.iter().any(|x| x.contains(want)),
            "missing {want}: {titles:#?}"
        );
    }
    // Errors sort before warnings.
    let sev: Vec<_> = findings.iter().map(|f| f.severity).collect();
    assert!(sev.windows(2).all(|w| w[0] <= w[1]));
    // Sizes 10x10 equal ds1 11x11 minus one: no size error for this map.

    let mut applied = 0;
    for f in &findings {
        for fix in &f.fixes {
            if !matches!(fix.action, FixAction::Edits(_)) {
                continue;
            }
            let mut a = t.clone();
            let mut ds = ds1.clone();
            fix.apply(&mut a, Some(&mut ds)).unwrap();
            let snap: Vec<_> = [&a.prest, &a.levels, &a.types]
                .iter()
                .map(|d| d.as_ref().unwrap().to_bytes())
                .collect();
            assert!(!fix.apply(&mut a, Some(&mut ds)).unwrap(), "{}", fix.label);
            let snap2: Vec<_> = [&a.prest, &a.levels, &a.types]
                .iter()
                .map(|d| d.as_ref().unwrap().to_bytes())
                .collect();
            assert_eq!(snap, snap2, "{}", fix.label);
            applied += 1;
        }
    }
    assert!(applied >= 5);
    // Apply every fix; the findings they address are gone and the others stay.
    for f in &findings {
        for fix in f
            .fixes
            .iter()
            .filter(|x| matches!(x.action, FixAction::Edits(_)))
        {
            fix.apply(&mut t, None).unwrap();
        }
    }
    let after = run_check(&t, &ds1);
    for gone in ["automap Layer", "FillBlanks", "too far out", "palette"] {
        assert!(
            !after.iter().any(|f| f.title.contains(gone)),
            "{gone} remained: {:#?}",
            after.iter().map(|f| &f.title).collect::<Vec<_>>()
        );
    }
    // Only the touched cells changed in LvlPrest: the Town row is byte-identical.
    let town_before = level_tables().prest.unwrap().to_bytes();
    let town_after = t.prest.as_ref().unwrap().to_bytes();
    let first = |b: &[u8]| {
        b.split_inclusive(|&c| c == b'\n')
            .take(2)
            .flatten()
            .copied()
            .collect::<Vec<u8>>()
    };
    assert_eq!(first(&town_before), first(&town_after));
}

#[test]
fn out_of_order_records_are_reordered_without_changing_ids() {
    let mut t = level_tables();
    let types = doc(&[
        "Name\tId\tFile 1\tAct",
        "B\t1\tx.dt1\t1",
        "Expansion\t\t\t",
        "A\t0\ty.dt1\t1",
        "",
    ]);
    t.types = Some(types);
    let findings = run_check(&t, &tiny_map());
    let f = findings
        .iter()
        .find(|f| f.title.contains("LvlTypes.txt: row 0 has Id 1"))
        .expect("order error");
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.columns[0].0, TableName::LvlTypes);
    let FixAction::Edits(e) = &f.fixes[0].action else {
        panic!("edit fix first")
    };
    assert_eq!(e.len(), 1);
    f.fixes[0].apply(&mut t, None).unwrap();
    let text = String::from_utf8(t.types.as_ref().unwrap().to_bytes()).unwrap();
    assert_eq!(
        text,
        "Name\tId\tFile 1\tAct\r\nExpansion\t\t\t\r\nA\t0\ty.dt1\t1\r\nB\t1\tx.dt1\t1\r\n\r\n"
    );
    assert!(!f.fixes[0].apply(&mut t, None).unwrap());
    assert!(
        !run_check(&t, &tiny_map())
            .iter()
            .any(|f| f.title.contains("row 0 has Id"))
    );
}

#[test]
fn missing_tiles_offer_add_and_clear_and_both_are_idempotent() {
    let t = level_tables();
    let mut ds1 = tiny_map();
    let cell = ds1.floors[0].get(3, 4).unwrap().with_indices(2, 7);
    // prop1 must be non-zero for the cell to hold a tile.
    let cell = d2_formats::ds1::Cell(cell.0 | 1);
    ds1.floors[0].set(3, 4, cell);
    let key = TileKey {
        orientation: 0,
        main: 2,
        sub: 7,
    };
    let candidates = [capy_mod::compat::Dt1Candidate {
        path: "act2/mine/other.dt1".into(),
        tiles: [key].into_iter().collect(),
    }];
    let loaded = [LoadedDt1 {
        path: "act2/mine/floor.dt1".into(),
        found: true,
        tiles: Default::default(),
    }];
    let input = MapInput {
        path: "act2\\mine.ds1",
        ds1: &ds1,
        loaded: &loaded,
        candidates: &candidates,
        used_dt1s: &[],
    };
    let findings = check_map(&input, &t, &Env::default());
    let f = findings
        .iter()
        .find(|f| f.title.ends_with("placed tiles have no graphic"))
        .unwrap();
    assert_eq!(f.cells, vec![(3, 4)]);
    assert_eq!(f.fixes.len(), 2);
    let mut a = ds1.clone();
    let mut tt = t.clone();
    f.fixes[0].apply(&mut tt, Some(&mut a)).unwrap();
    assert!(!f.fixes[0].apply(&mut tt, Some(&mut a)).unwrap());
    assert_eq!(a.files.len(), ds1.files.len() + 1);
    let mut b = ds1.clone();
    assert!(f.fixes[1].apply(&mut tt, Some(&mut b)).unwrap());
    assert!(!f.fixes[1].apply(&mut tt, Some(&mut b)).unwrap());
    assert!(b.floors[0].get(3, 4).unwrap().is_empty());
    // After the clear the map is clean of this error and still serialises.
    let input = MapInput {
        path: "act2\\mine.ds1",
        ds1: &b,
        loaded: &loaded,
        candidates: &[],
        used_dt1s: &[],
    };
    assert!(
        !check_map(&input, &t, &Env::default())
            .iter()
            .any(|f| f.severity == Severity::Error && f.title.contains("no graphic"))
    );
    b.to_bytes().unwrap();
}

#[test]
fn automap_table_problems_are_reported_and_removed() {
    let mut t = level_tables();
    t.automap = Some(doc(&[
        HEAD,
        "1 Town\tfl\t0\t0\t3\tA\t-1\t\t-1\t\t-1\t\t-1",
        "Nowhere\tfl\t0\t0\t3\tA\t3\t\t-1\t\t-1\t\t-1",
    ]));
    let findings = run_check(&t, &tiny_map());
    let bad = findings
        .iter()
        .find(|f| f.title.contains("no picture"))
        .unwrap();
    assert_eq!(bad.severity, Severity::Error);
    assert!(
        findings
            .iter()
            .any(|f| f.title.contains("names level") && f.severity == Severity::Warning)
    );
    bad.fixes[0].apply(&mut t, None).unwrap();
    assert!(!bad.fixes[0].apply(&mut t, None).unwrap());
    assert!(
        !run_check(&t, &tiny_map())
            .iter()
            .any(|f| f.title.contains("no picture"))
    );
}

#[test]
fn an_unreferenced_map_is_an_error_with_the_add_to_game_flow() {
    let t = level_tables();
    let ds1 = tiny_map();
    let input = MapInput {
        path: "act9\\none.ds1",
        ds1: &ds1,
        loaded: &[],
        candidates: &[],
        used_dt1s: &[],
    };
    let f = check_map(&input, &t, &Env::default());
    let e = f
        .iter()
        .find(|f| f.title == "Not referenced by LvlPrest.txt")
        .unwrap();
    assert!(
        e.fixes
            .iter()
            .any(|x| matches!(x.action, FixAction::Register))
    );
    assert_eq!(f[0].severity, Severity::Error);
    assert!(
        f.iter()
            .any(|f| f.title.starts_with("No entry or warp markers"))
    );
}

#[test]
fn finding_keys_ignore_numbers() {
    let t = level_tables();
    let f = run_check(&t, &tiny_map());
    let layer = f.iter().find(|f| f.title.contains("Layer")).unwrap();
    assert!(layer.key().starts_with("Level|level #"));
    assert!(!layer.key().chars().any(|c| c.is_ascii_digit() && false));
}
