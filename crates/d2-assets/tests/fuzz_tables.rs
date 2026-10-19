//! Fuzz-style tests for the code that indexes game tables by file-derived
//! ids: `ObjectCatalog` (Objects, ObjPreset, MonPreset, MonStats, MonStats2,
//! SuperUniques, string tables) and `MapCatalog` (LvlTypes, LvlPrest, Levels,
//! DS1 headers and cells, DT1 keys).
//!
//! Small synthetic tables get cells replaced by extreme values (huge, signed,
//! non-numeric, `Expansion` markers), both in the `.txt` column layout and in
//! the compiled (`.bin`-style) one, then every lookup is probed with
//! out-of-range acts, versions, kinds and ids. The private corpus' real tables
//! are probed the same way when present. Nothing may panic.
//!
//! Knobs: proptest cases follow `PROPTEST_CASES` (default 256, these build a
//! catalog per case so they are the slow ones).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use d2_assets::{
    AssetSource, FolderSource, GamePath, GameVersion, LegacyEntry, MapCatalog, ObjectCatalog,
    Overlay, PaletteShift,
};
use d2_formats::ds1::{Cell, Ds1, Object};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

const EXCEL: &str = r"data\global\excel\";
const TILES: &str = r"data\global\tiles\";

/// Cell values that stress numeric parsing and every id/row computation.
const CELLS: &[&str] = &[
    "",
    "0",
    "1",
    "-1",
    "2",
    "5",
    "6",
    "149",
    "150",
    "151",
    "255",
    "256",
    "65535",
    "65536",
    "2147483647",
    "-2147483648",
    "4294967295",
    "4294967296",
    "9223372036854775807",
    "-9223372036854775808",
    "99999999999999999999",
    "abc",
    "Expansion",
    "gheed",
    "fallen1",
    "Bishibosh",
    "place_nothing",
    "act1\\cave\\floor.dt1",
    "act1\\cave\\a.ds1",
    "C:\\",
    "0",
];

const ACTS: &[u8] = &[0, 1, 2, 4, 5, 200, 255];
const VERSIONS: &[i32] = &[i32::MIN, -1, 0, 4, 5, 6, 18, i32::MAX];
const KINDS: &[i32] = &[i32::MIN, -1, 0, 1, 2, 3];
const IDS: &[i32] = &[
    i32::MIN,
    -1_000_000,
    -751,
    -151,
    -150,
    -1,
    0,
    1,
    2,
    10,
    149,
    150,
    151,
    299,
    300,
    749,
    750,
    1000,
    65535,
    i32::MAX,
];

type Table = Vec<Vec<String>>;

fn table(rows: &[&[&str]]) -> Table {
    rows.iter()
        .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
        .collect()
}

fn tsv(table: &Table) -> Vec<u8> {
    let mut out = String::new();
    for row in table {
        out.push_str(&row.join("\t"));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

fn write(root: &Path, game_path: &str, bytes: &[u8]) {
    let target = GamePath::new(game_path).to_native(root);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, bytes).unwrap();
}

fn folder_overlay(root: &Path) -> Overlay {
    let layer: Box<dyn AssetSource> = Box::new(FolderSource::new(root, "fuzz"));
    Overlay::new(vec![layer])
}

/// `(table, row, column, value)`: row/column taken modulo the table's size,
/// with row == len meaning "append a row".
type CellEdit = (usize, usize, usize, &'static str);

fn apply_edits(tables: &mut [Table], edits: &[CellEdit]) {
    for &(t, row, col, value) in edits {
        let table = &mut tables[t % tables.len()];
        let width = table.first().map_or(1, Vec::len).max(1);
        // Never touch the header row: the column names decide the layout.
        let row = 1 + row % table.len();
        if row == table.len() {
            table.push(vec![String::new(); width]);
        }
        let cells = &mut table[row];
        let col = col % width;
        if cells.len() <= col {
            cells.resize(col + 1, String::new());
        }
        cells[col] = value.to_owned();
    }
}

fn cell_edits() -> impl Strategy<Value = Vec<CellEdit>> {
    prop::collection::vec(
        (
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
            prop::sample::select(CELLS),
        ),
        0..24,
    )
}

// ------------------------------------------------------------ ObjectCatalog

/// A `.tbl` holding the given pairs (same layout as `objects_synthetic.rs`).
fn tbl(entries: &[(&str, &str)]) -> Vec<u8> {
    const HEADER: usize = 0x15;
    const NODE: usize = 17;
    let h = entries.len();
    let n = h.max(1);
    let data_start = HEADER + h * 2 + n * NODE;
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for (key, text) in entries {
        let k = data_start + data.len();
        data.extend_from_slice(key.as_bytes());
        data.push(0);
        let t = data_start + data.len();
        data.extend_from_slice(text.as_bytes());
        data.push(0);
        offsets.push((k, t, text.len() + 1));
    }
    let mut out = vec![0u8; HEADER];
    out[2..4].copy_from_slice(&u16::try_from(h).unwrap().to_le_bytes());
    out[4..8].copy_from_slice(&u32::try_from(n).unwrap().to_le_bytes());
    out[8] = 1;
    out[9..13].copy_from_slice(&u32::try_from(data_start).unwrap().to_le_bytes());
    out[17..21].copy_from_slice(
        &u32::try_from(data_start + data.len())
            .unwrap()
            .to_le_bytes(),
    );
    for i in 0..h {
        out.extend_from_slice(&u16::try_from(i).unwrap().to_le_bytes());
    }
    for (i, (k, t, tl)) in offsets.iter().enumerate() {
        out.push(1);
        out.extend_from_slice(&u16::try_from(i).unwrap().to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(*k).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(*t).unwrap().to_le_bytes());
        out.extend_from_slice(&u16::try_from(*tl).unwrap().to_le_bytes());
    }
    if h == 0 {
        out.extend_from_slice(&[0u8; NODE]);
    }
    out.extend_from_slice(&data);
    out
}

/// Objects, ObjPreset, MonPreset, MonStats, MonStats2, SuperUniques. With
/// `compiled`, the monster tables use the `.bin` column layout (row links and
/// string ids), which the catalog detects by column name.
fn object_tables(compiled: bool) -> Vec<Table> {
    let mut objects = table(&[&[
        "Name",
        "description - not loaded",
        "Token",
        "SizeX",
        "SizeY",
        "Xoffset",
        "Yoffset",
        "FrameCnt0",
        "FrameCnt1",
        "FrameDelta0",
        "FrameDelta1",
        "OrderFlag0",
        "OrderFlag1",
        "Trans",
        "HD",
        "TR",
    ]]);
    for n in 0..12 {
        let row: Vec<String> = [
            format!("obj{n}"),
            format!("object {n}"),
            format!("T{n}"),
            "2".into(),
            "3".into(),
            "-4".into(),
            "7".into(),
            "10".into(),
            "20".into(),
            "0".into(),
            "64".into(),
            "1".into(),
            "2".into(),
            (n % 3).to_string(),
            "1".into(),
            "0".into(),
        ]
        .into();
        objects.push(row);
    }
    let objpreset = table(&[
        &["Act", "Id", "ObjectsId"],
        &["0", "0", "1"],
        &["0", "1", "2"],
        &["1", "0", "3"],
        &["4", "149", "11"],
    ]);
    let (monpreset, monstats, superuniques) = if compiled {
        (
            table(&[
                &["Act", "PlaceKind", "PlaceIndex"],
                &["1", "1", "0"],
                &["1", "2", "0"],
                &["1", "0", "3"],
                &["2", "1", "2"],
                &["5", "1", "4"],
            ]),
            table(&[
                &["NameStrId", "MonStatsExRow", "Code"],
                &["0", "0", "GH"],
                &["1", "1", "DC"],
                &["20001", "-1", "PS"],
                &["10000", "0", "CK"],
                &["65535", "1", "FA"],
            ]),
            table(&[&["NameStrId", "ClassRow"], &["1", "4"], &["2", "99"]]),
        )
    } else {
        (
            table(&[
                &["Act", "Place"],
                &["1", "gheed"],
                &["1", "Bishibosh"],
                &["1", "place_fallen"],
                &["2", "akara"],
                &["3", "Expansion"],
                &["5", "fallen1"],
            ]),
            table(&[
                &["Id", "NameStr", "MonStatsEx", "Code"],
                &["gheed", "Gheed", "", "GH"],
                &["cain1", "DeckardCain", "", "DC"],
                &["akara", "Akara", "", "PS"],
                &["Expansion", "", "", ""],
                &["fallen1", "Fallen", "fallen1", "FA"],
            ]),
            table(&[
                &["Superunique", "Name", "Class"],
                &["Bishibosh", "Bishibosh", "fallen1"],
            ]),
        )
    };
    let monstats2 = table(&[
        &["Id", "SizeX", "SizeY", "BaseW", "HD", "TR"],
        &["gheed", "1", "2", "", "0", "1"],
        &["fallen1", "1", "1", "1hs", "1", "1"],
    ]);
    vec![
        objects,
        objpreset,
        monpreset,
        monstats,
        monstats2,
        superuniques,
    ]
}

const OBJECT_STEMS: [&str; 6] = [
    "objects",
    "objpreset",
    "monpreset",
    "monstats",
    "monstats2",
    "superuniques",
];

fn write_object_tables(root: &Path, tables: &[Table]) {
    for (stem, t) in OBJECT_STEMS.iter().zip(tables) {
        write(root, &format!("{EXCEL}{stem}.txt"), &tsv(t));
    }
    let strings = tbl(&[("Gheed", "Gheed"), ("Akara", "Akara"), ("dummy", "x")]);
    for name in ["string.tbl", "patchstring.tbl", "expansionstring.tbl"] {
        write(root, &format!(r"data\local\lng\eng\{name}"), &strings);
    }
}

/// Resolves the probe grid; `full` adds the rarer acts, versions and kinds
/// (the per-case fuzz uses the smaller grid to stay fast).
fn probe_objects(catalog: &ObjectCatalog, full: bool) {
    let (acts, versions, kinds): (&[u8], &[i32], &[i32]) = if full {
        (ACTS, VERSIONS, KINDS)
    } else {
        (&[0, 4, 255], &[i32::MIN, 0, 5, 6, i32::MAX], &[1, 2, 3])
    };
    for &act in acts {
        for &version in versions {
            for &kind in kinds {
                for &id in IDS {
                    let object = Object {
                        kind,
                        id,
                        x: 0,
                        y: 0,
                        flags: 0,
                    };
                    let info = catalog.resolve(act, version, &object);
                    assert_eq!((info.ds1_kind, info.ds1_id), (kind, id));
                }
            }
        }
        let _ = catalog.placeable(act);
    }
}

#[test]
fn object_catalog_extreme_cells_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..Config::default()
    });
    println!(
        "object_catalog_extreme_cells_never_panic: {} cases",
        runner.config().cases
    );
    let strategy = (any::<bool>(), prop::option::of(0usize..6), cell_edits());
    let result = runner.run(&strategy, |(compiled, drop, edits)| {
        let mut tables = object_tables(compiled);
        apply_edits(&mut tables, &edits);
        // Start from a clean folder each case so a dropped table is absent.
        let root = dir.path().join("case");
        let _ = std::fs::remove_dir_all(&root);
        write_object_tables(&root, &tables);
        if let Some(i) = drop {
            let _ = std::fs::remove_file(
                GamePath::new(&format!("{EXCEL}{}.txt", OBJECT_STEMS[i])).to_native(&root),
            );
        }
        let overlay = folder_overlay(&root);
        for version in [GameVersion::V114d, GameVersion::V113c] {
            let (catalog, _) = ObjectCatalog::build(&overlay, version).unwrap();
            probe_objects(&catalog, false);
        }
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
}

#[test]
fn object_catalog_on_an_empty_overlay_resolves_everything() {
    let dir = tempfile::tempdir().unwrap();
    let (catalog, warnings) =
        ObjectCatalog::build(&folder_overlay(dir.path()), GameVersion::V114d).unwrap();
    assert!(!warnings.is_empty());
    probe_objects(&catalog, true);
}

#[test]
fn palette_shift_rows_never_index_out_of_range() {
    let bytes = vec![7u8; 256 * 3 + 5];
    for row in [0, 1, 2, 3, 4, u32::MAX / 256, u32::MAX - 1, u32::MAX] {
        let shift = PaletteShift {
            path: "x".to_owned(),
            row,
        };
        let got = shift.row_in(&bytes);
        assert_eq!(got.is_some(), row < 3, "row {row}");
        assert!(shift.row_in(&[]).is_none());
    }
}

// --------------------------------------------------------------- MapCatalog

fn map_tables() -> Vec<Table> {
    vec![
        table(&[
            &["Name", "Id", "Act", "File 1", "File 2", "File 3"],
            &["Null", "0", "1", "0", "0", "0"],
            &[
                "Cave",
                "1",
                "1",
                "act1\\cave\\floor.dt1",
                "act1\\cave\\wall.dt1",
                "0",
            ],
            &["Desert", "2", "2", "act2\\desert\\floor.dt1", "0", "0"],
            &["Lava", "35", "5", "act4\\lava\\floor.dt1", "0", "0"],
        ]),
        table(&[
            &[
                "Name",
                "Id",
                "LevelName",
                "LevelType",
                "DrlgType",
                "Act",
                "Pal",
            ],
            &["None", "0", "", "0", "0", "0", "0"],
            &["Cave", "1", "Cave Level", "1", "2", "0", "0"],
            &["Desert", "2", "Desert Level", "2", "2", "1", "2"],
            &["Lava", "3", "Lava Level", "35", "2", "4", "3"],
        ]),
        table(&[
            &[
                "Name", "Def", "LevelId", "Dt1Mask", "File1", "File2", "File3", "File4", "File5",
                "File6",
            ],
            &[
                "Cave a",
                "10",
                "1",
                "5",
                "act1\\cave\\a.ds1",
                "act1\\cave\\b.ds1",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Maze",
                "17",
                "0",
                "2",
                "act2\\desert\\z.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Lava",
                "12",
                "3",
                "1",
                "",
                "",
                "act4\\lava\\l.ds1",
                "",
                "",
                "",
            ],
        ]),
    ]
}

const MAP_STEMS: [&str; 3] = ["lvltypes", "levels", "lvlprest"];

/// A DT1 holding header-only tiles with the given keys (as in
/// `presets_synthetic.rs`).
fn dt1_bytes(keys: &[(i32, i32, i32)]) -> Vec<u8> {
    let mut out = vec![0u8; 276];
    out[0..4].copy_from_slice(&7i32.to_le_bytes());
    out[4..8].copy_from_slice(&6i32.to_le_bytes());
    out[268..272].copy_from_slice(&i32::try_from(keys.len()).unwrap().to_le_bytes());
    out[272..276].copy_from_slice(&276i32.to_le_bytes());
    for &(orientation, main, sub) in keys {
        let mut tile = [0u8; 96];
        tile[20..24].copy_from_slice(&orientation.to_le_bytes());
        tile[24..28].copy_from_slice(&main.to_le_bytes());
        tile[28..32].copy_from_slice(&sub.to_le_bytes());
        out.extend_from_slice(&tile);
    }
    out
}

/// DS1 paths (tile-relative) and their header acts, including out-of-range ones.
const DS1S: &[(&str, i32)] = &[
    ("act1\\cave\\a.ds1", 0),
    ("act1\\cave\\b.ds1", -1),
    ("act2\\desert\\z.ds1", 1),
    ("act4\\lava\\l.ds1", 99),
    ("elsewhere\\x.ds1", i32::MAX),
    ("elsewhere\\broken.ds1", 0),
];

/// Writes DT1s and DS1s (the tables are written per case).
fn write_map_assets(root: &Path) {
    let keys: &[(i32, i32, i32)] = &[
        (0, 1, 0),
        (19, 8, 0),
        (13, 0, 0),
        (0, 255, 255),
        (-1, -1, -1),
        (i32::MAX, i32::MIN, 0),
    ];
    for dt1 in [
        "act1\\cave\\floor.dt1",
        "act1\\cave\\wall.dt1",
        "act2\\desert\\floor.dt1",
    ] {
        write(root, &format!("{TILES}{dt1}"), &dt1_bytes(keys));
    }
    // A DT1 whose header claims a huge tile count.
    let mut lava = dt1_bytes(&[(0, 7, 0)]);
    lava[268..272].copy_from_slice(&i32::MAX.to_le_bytes());
    write(root, &format!("{TILES}act4\\lava\\floor.dt1"), &lava);
    for &(name, act) in DS1S {
        let bytes = if name.ends_with("broken.ds1") {
            b"not a ds1".to_vec()
        } else {
            let mut ds1 = Ds1::new(4, 4, act);
            let cells = [
                (0u8, 1u8, 0u8),
                (0, 255, 255),
                (18, 8, 4),
                (19, 8, 0),
                (13, 0, 0),
            ];
            for (i, &(orientation, main, sub)) in cells.iter().enumerate() {
                let (x, y) = (i as u32 % 4, i as u32 / 4);
                let cell = Cell(1).with_indices(main, sub);
                let _ = ds1.floors[0].set(x, y, cell);
                let _ = ds1.shadow.set(x, y, cell);
                let _ = ds1.walls[0].tiles.set(x, y, cell);
                let _ = ds1.walls[0].orientations.set(x, y, u32::from(orientation));
            }
            ds1.files = vec![
                b"act1\\cave\\floor.dt1".to_vec(),
                b"C:\\".to_vec(),
                vec![0xFF; 40],
            ];
            ds1.to_bytes().unwrap()
        };
        write(root, &format!("{TILES}{name}"), &bytes);
    }
}

fn probe_maps(catalog: &mut MapCatalog) {
    let mut names: Vec<GamePath> = DS1S
        .iter()
        .map(|(n, _)| GamePath::new(&format!("{TILES}{n}")))
        .collect();
    names.push(GamePath::new("nowhere.ds1"));
    names.push(GamePath::new(""));
    let mut ids: Vec<u32> = catalog.lvl_type_ids();
    ids.extend([0, 1, 35, u32::MAX]);
    for ds1 in &names {
        for info in catalog.lookup(ds1) {
            assert!(info.act <= 4 && info.palette_act <= 4, "{info:?}");
        }
        let _ = catalog.best(ds1);
        for &lvl_type in &ids {
            for def in [None, Some(0), Some(10), Some(17), Some(u32::MAX)] {
                if let Ok(info) = catalog.resolve_explicit(ds1, lvl_type, def) {
                    assert!(info.act <= 4 && info.palette_act <= 4, "{info:?}");
                }
            }
        }
    }
    let legacy: Vec<LegacyEntry> = names
        .iter()
        .zip([0, 1, 35, u32::MAX].iter().cycle())
        .map(|(ds1, &id)| LegacyEntry {
            lvl_type: id,
            lvlprest_def: id,
            ds1: ds1.clone(),
        })
        .collect();
    catalog.import_legacy(&legacy);
    catalog.set_override(names[0].clone(), u32::MAX, Some(u32::MAX));
    for ds1 in &names {
        for info in catalog.lookup(ds1) {
            assert!(info.act <= 4 && info.palette_act <= 4, "{info:?}");
        }
    }
    let _ = catalog.overrides();
}

#[test]
fn map_catalog_extreme_cells_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_map_assets(root);
    let names: Vec<GamePath> = DS1S
        .iter()
        .map(|(n, _)| GamePath::new(&format!("{TILES}{n}")))
        .chain([GamePath::new("missing\\m.ds1")])
        .collect();
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..Config::default()
    });
    println!(
        "map_catalog_extreme_cells_never_panic: {} cases",
        runner.config().cases
    );
    let result = runner.run(&cell_edits(), |edits| {
        let mut tables = map_tables();
        apply_edits(&mut tables, &edits);
        for (stem, t) in MAP_STEMS.iter().zip(&tables) {
            write(root, &format!("{EXCEL}{stem}.txt"), &tsv(t));
        }
        let overlay = folder_overlay(root);
        let (mut catalog, _) = MapCatalog::build(&overlay, GameVersion::V114d, &names).unwrap();
        probe_maps(&mut catalog);
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
}

// ---------------------------------------------------------- private corpus

/// The real vanilla and Median XL tables (extracted into the private corpus):
/// every out-of-range act/version/kind/id resolves without panicking.
#[test]
fn private_corpus_tables_with_out_of_range_ids() {
    let extracted = common::private_dir().join("extracted");
    for (game, version) in [("vanilla", GameVersion::V114d), ("mxl", GameVersion::V113c)] {
        let root = extracted.join(game);
        if !common::dir_available(&root) {
            continue;
        }
        let overlay = folder_overlay(&root);
        let (catalog, _) = ObjectCatalog::build(&overlay, version).unwrap();
        probe_objects(&catalog, true);
        let ds1 = [
            GamePath::new(r"data\global\tiles\act1\town\towne1.ds1"),
            GamePath::new(r"data\global\tiles\act5\siege\snowbarrfd1.ds1"),
        ];
        let (mut maps, _) = MapCatalog::build(&overlay, version, &ds1).unwrap();
        probe_maps(&mut maps);
        println!("{game}: probed");
    }
}
