//! `presets` against a synthetic overlay: small `.txt` tables and tiny DS1
//! files in a loose folder.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use d2_assets::{
    FolderSource, GamePath, GameVersion, LegacyEntry, MapCatalog, MapInfoSource, Overlay,
    parse_legacy_ini,
};
use d2_formats::ds1::{Cell, Ds1};

const TILES: &str = "data\\global\\tiles\\";

fn gp(tile_relative: &str) -> GamePath {
    GamePath::new(&format!("{TILES}{tile_relative}"))
}

fn write(root: &Path, game_path: &str, bytes: &[u8]) {
    let target = GamePath::new(game_path).to_native(root);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, bytes).unwrap();
}

fn excel(root: &Path, stem: &str, text: &str) {
    write(
        root,
        &format!("data\\global\\excel\\{stem}.txt"),
        text.as_bytes(),
    );
}

/// Tab-separated table from rows of cells.
fn tsv(rows: &[&[&str]]) -> String {
    let mut out = String::new();
    for row in rows {
        out.push_str(&row.join("\t"));
        out.push_str("\r\n");
    }
    out
}

fn ds1_bytes(act: i32) -> Vec<u8> {
    Ds1::new(2, 2, act).to_bytes().unwrap()
}

/// DS1s that exist on disk (the catalog's "known" set).
const DS1_FILES: &[(&str, i32)] = &[
    ("act1\\cave\\a.ds1", 0),
    ("act1\\cave\\shared.ds1", 0),
    ("act1\\cave\\m.ds1", 0),
    ("act1\\cave\\n.ds1", 0),
    ("act2\\desert\\z.ds1", 1),
    ("act2\\desert\\y.ds1", 1),
    ("act1\\cave\\orphan.ds1", 0),
    ("act2\\desert\\orphan.ds1", 1),
    ("act4\\lava\\other.ds1", 3),
    ("act4\\lava\\l.ds1", 3),
    ("act3\\w.ds1", 2),
    ("act5\\nolevel.ds1", 3),
    ("other\\nowhere.ds1", 2),
];

fn write_tables(root: &Path) {
    // LvlTypes: Act is the stored, 1-based value.
    excel(
        root,
        "lvltypes",
        &tsv(&[
            &["Name", "Id", "Act", "File 1", "File 2", "File 3", "File 4"],
            &["Null", "0", "1", "0", "0", "0", "0"],
            &[
                "Cave",
                "1",
                "1",
                "act1\\cave\\floor.dt1",
                "act1\\cave\\wall.dt1",
                "act1\\cave\\extra.dt1",
                "act1\\cave\\ghost.dt1",
            ],
            &["Desert", "2", "2", "act2\\desert\\floor.dt1", "0", "0", "0"],
            &[
                "CaveB",
                "3",
                "1",
                "act1\\cave\\floor.dt1",
                "act1\\cave\\wall.dt1",
                "0",
                "0",
            ],
            &[
                "Desert2",
                "4",
                "2",
                "act2\\desert\\floor2.dt1",
                "0",
                "0",
                "0",
            ],
            &[
                "DesertC",
                "5",
                "2",
                "act1\\cave\\floor.dt1",
                "act2\\desert\\floor3.dt1",
                "0",
                "0",
            ],
            &["Lava", "35", "5", "act4\\lava\\floor.dt1", "0", "0", "0"],
        ]),
    );
    excel(
        root,
        "levels",
        &tsv(&[
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
            &["Weird", "4", "", "99", "2", "2", "2"],
        ]),
    );
    // Rows deliberately out of Def order.
    excel(
        root,
        "lvlprest",
        &tsv(&[
            &[
                "Name", "Def", "LevelId", "Dt1Mask", "File1", "File2", "File3", "File4", "File5",
                "File6",
            ],
            &[
                "Desert shared",
                "11",
                "2",
                "15",
                "act1\\cave\\shared.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Cave a",
                "10",
                "1",
                "5",
                "act1\\cave\\a.ds1",
                "act1\\cave\\shared.ds1",
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
            &[
                "Weird",
                "13",
                "4",
                "255",
                "act3\\w.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "No level",
                "14",
                "99",
                "255",
                "act5\\nolevel.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Ghost",
                "15",
                "1",
                "9",
                "act1\\cave\\m.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Maze piece",
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
                "Maze piece 2",
                "18",
                "0",
                "1",
                "act2\\desert\\y.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Orphan",
                "19",
                "0",
                "255",
                "orphan\\p.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "Cave cov",
                "20",
                "1",
                "5",
                "act1\\cave\\cov.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
            &[
                "No file 1",
                "16",
                "1",
                "14",
                "act1\\cave\\n.ds1",
                "0",
                "0",
                "0",
                "0",
                "0",
            ],
        ]),
    );
}

/// A DT1 holding header-only tiles with the given (orientation, main, sub)
/// keys (no sub-blocks): enough for key sets.
fn dt1_bytes(keys: &[KeySpec]) -> Vec<u8> {
    let mut out = vec![0u8; 276];
    out[0..4].copy_from_slice(&7i32.to_le_bytes());
    out[4..8].copy_from_slice(&6i32.to_le_bytes());
    out[268..272].copy_from_slice(&(keys.len() as i32).to_le_bytes());
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

/// Each DT1's tile keys; unique per file so coverage tells them apart.
type KeySpec = (i32, i32, i32);
const DT1_FILES: &[(&str, &[KeySpec])] = &[
    ("act1\\cave\\floor.dt1", &[(0, 1, 0), (0, 1, 1)]),
    ("act1\\cave\\wall.dt1", &[(1, 2, 0), (19, 8, 0)]),
    ("act1\\cave\\extra.dt1", &[(2, 3, 0)]),
    ("act2\\desert\\floor.dt1", &[(0, 5, 0)]),
    ("act2\\desert\\floor2.dt1", &[(0, 6, 0)]),
    ("act4\\lava\\floor.dt1", &[(0, 7, 0)]),
];

fn write_dt1_stubs(root: &Path) {
    for (name, keys) in DT1_FILES {
        write(root, &format!("{TILES}{name}"), &dt1_bytes(keys));
    }
}

struct Synthetic {
    _dir: tempfile::TempDir,
    overlay: Overlay,
}

fn synthetic() -> Synthetic {
    let dir = tempfile::tempdir().unwrap();
    write_tables(dir.path());
    write_dt1_stubs(dir.path());
    for (name, act) in DS1_FILES {
        write(dir.path(), &format!("{TILES}{name}"), &ds1_bytes(*act));
    }
    for (name, act, cells, refs) in CELL_DS1S {
        write(
            dir.path(),
            &format!("{TILES}{name}"),
            &ds1_with_cells(*act, cells, refs),
        );
    }
    let overlay = Overlay::new(vec![Box::new(FolderSource::new(dir.path(), "synthetic"))]);
    Synthetic { _dir: dir, overlay }
}

/// 'F' floor, 'S' shadow, 'W' wall (the second number is its orientation).
type CellSpec = (char, u8, u8, u8);

/// Cells are placed row by row on a 4x4 map.
fn ds1_with_cells(act: i32, cells: &[CellSpec], refs: &[&str]) -> Vec<u8> {
    let mut ds1 = Ds1::new(4, 4, act);
    for (i, &(kind, a, main, sub)) in cells.iter().enumerate() {
        let (x, y) = (i as u32 % 4, i as u32 / 4);
        let cell = Cell(1).with_indices(main, sub);
        match kind {
            'F' => assert!(ds1.floors[0].set(x, y, cell)),
            'S' => assert!(ds1.shadow.set(x, y, cell)),
            'W' => {
                assert!(ds1.walls[0].tiles.set(x, y, cell));
                assert!(ds1.walls[0].orientations.set(x, y, u32::from(a)));
            }
            _ => panic!("bad cell kind"),
        }
    }
    ds1.files = refs.iter().map(|r| r.as_bytes().to_vec()).collect();
    ds1.to_bytes().unwrap()
}

/// DS1s with cells or a tileset list, for the coverage tests.
const CELL_DS1S: &[(&str, i32, &[CellSpec], &[&str])] = &[
    // Desert dir: types 2, 4, 5 tie on directory; floor2.dt1 (type 4) has the tile.
    ("act2\\desert\\cov.ds1", 1, &[('F', 0, 6, 0)], &[]),
    // Same tie, no cells: the DS1's own tileset list names floor2.dt1.
    (
        "act2\\desert\\ref.ds1",
        1,
        &[],
        &["act2\\desert\\floor2.dt1", "DS1EDIT_WRKSPC_x"],
    ),
    // No directory candidate: only lava floor.dt1 has (0,7,0).
    ("elsewhere\\tc.ds1", 3, &[('F', 0, 7, 0)], &[]),
    // 1 of 2 cells resolves: below the 0.90 bar.
    (
        "elsewhere\\low.ds1",
        2,
        &[('F', 0, 7, 0), ('F', 0, 99, 0)],
        &[],
    ),
    // Wall 18/3/sub 4 resolves through 19/3/sub 0? No: cave wall.dt1 holds 19/8/0,
    // so wall (18, 8, 4) falls back 18->19, sub 0.
    ("elsewhere\\wall.ds1", 0, &[('W', 18, 8, 4)], &[]),
    // A LvlPrest row (def 19) with LevelId 0 and no directory candidate.
    ("orphan\\p.ds1", 1, &[('F', 0, 5, 0), ('F', 0, 5, 0)], &[]),
    // A normal LvlPrest map with cells: floor.dt1 + extra.dt1 hold 1 of 2.
    (
        "act1\\cave\\cov.ds1",
        0,
        &[('F', 0, 1, 0), ('F', 0, 9, 9)],
        &[],
    ),
];

fn catalog() -> (Synthetic, MapCatalog, Vec<String>) {
    let s = synthetic();
    let names: Vec<GamePath> = DS1_FILES
        .iter()
        .map(|(n, _)| gp(n))
        .chain(CELL_DS1S.iter().map(|(n, ..)| gp(n)))
        .collect();
    let (catalog, warnings) = MapCatalog::build(&s.overlay, GameVersion::V114d, &names).unwrap();
    (s, catalog, warnings)
}

#[test]
fn builds_clean() {
    let (_s, catalog, warnings) = catalog();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(catalog.lvl_type_ids(), vec![0, 1, 2, 3, 4, 5, 35]);
    assert!(catalog.overrides().is_empty());
}

#[test]
fn lvlprest_source_masks_dt1_and_acts() {
    let (_s, catalog, _) = catalog();
    let a = catalog.best(&gp("act1\\cave\\a.ds1")).unwrap();
    assert_eq!(a.source, MapInfoSource::LvlPrest);
    assert_eq!(a.lvlprest_def, Some(10));
    assert_eq!(a.lvlprest_slot, Some(0));
    assert_eq!(a.level_id, Some(1));
    assert_eq!(a.level_name.as_deref(), Some("Cave Level"));
    assert_eq!(a.lvl_type, Some(1));
    assert_eq!((a.act, a.palette_act), (0, 0));
    // Mask 0b0101 = File1 + File3 of the Cave type; File 2 and 4 are off.
    assert_eq!(
        a.dt1_files,
        vec![gp("act1\\cave\\floor.dt1"), gp("act1\\cave\\extra.dt1")]
    );
    assert!(a.missing_dt1.is_empty());
}

#[test]
fn dt1_mask_gates_file1_and_reports_missing() {
    let (_s, catalog, _) = catalog();
    // Mask 0b1001 = File1 + File4; File4 (ghost.dt1) is not on disk.
    let m = catalog.best(&gp("act1\\cave\\m.ds1")).unwrap();
    assert_eq!(
        m.dt1_files,
        vec![gp("act1\\cave\\floor.dt1"), gp("act1\\cave\\ghost.dt1")]
    );
    assert_eq!(m.missing_dt1, vec![gp("act1\\cave\\ghost.dt1")]);
    // Mask 0b1110: bit 0 clear, so File 1 (floor.dt1) is off too.
    let n = catalog.best(&gp("act1\\cave\\n.ds1")).unwrap();
    assert_eq!(
        n.dt1_files,
        vec![
            gp("act1\\cave\\wall.dt1"),
            gp("act1\\cave\\extra.dt1"),
            gp("act1\\cave\\ghost.dt1")
        ]
    );
    let lava = catalog.best(&gp("act4\\lava\\l.ds1")).unwrap();
    assert_eq!(lava.dt1_files, vec![gp("act4\\lava\\floor.dt1")]);
}

#[test]
fn several_lvlprest_rows_lowest_def_first() {
    let (_s, catalog, _) = catalog();
    let all = catalog.lookup(&gp("act1\\cave\\shared.ds1"));
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|i| i.source == MapInfoSource::LvlPrest));
    assert_eq!(all[0].lvlprest_def, Some(10));
    assert_eq!(all[0].lvlprest_slot, Some(1));
    assert_eq!(all[0].lvl_type, Some(1));
    assert_eq!(all[1].lvlprest_def, Some(11));
    assert_eq!(all[1].lvlprest_slot, Some(0));
    assert_eq!(all[1].lvl_type, Some(2));
    assert_eq!(all[1].act, 1);
}

#[test]
fn lvl_type_35_uses_the_act4_palette() {
    let (_s, catalog, _) = catalog();
    let lava = catalog.best(&gp("act4\\lava\\l.ds1")).unwrap();
    assert_eq!(lava.lvlprest_slot, Some(2));
    assert_eq!(lava.lvl_type, Some(35));
    // LvlTypes stores Act 5 -> index 4, but the palette is Act 4's (index 3):
    // the Level's own Pal says so, as in vanilla (Hell 1-3, Pandemonium 3).
    assert_eq!(lava.act, 4);
    assert_eq!(lava.palette_act, 3);
}

#[test]
fn level_pal_overrides_the_lvl_type_act() {
    // Median XL case (Level 241: LvlTypes act 5, Pal 2): a real Level's Pal
    // wins over the LvlType act. The fixture's Desert level stores Pal 2.
    let (_s, catalog, _) = catalog();
    let all = catalog.lookup(&gp(r"act1\cave\shared.ds1"));
    let desert = all.iter().find(|i| i.level_id == Some(2)).unwrap();
    assert_eq!(desert.lvl_type, Some(2));
    assert_eq!((desert.act, desert.palette_act), (1, 2));
}

#[test]
fn act_falls_back_to_level_then_header() {
    let (_s, catalog, _) = catalog();
    // LevelType 99 does not exist: the Level's own act (2) is used.
    let w = catalog.best(&gp("act3\\w.ds1")).unwrap();
    assert_eq!(w.source, MapInfoSource::LvlPrest);
    assert_eq!(w.lvl_type, None);
    assert_eq!(w.act, 2);
    assert!(w.dt1_files.is_empty());
    // LevelId 99 does not exist either: the DS1 header says act 3.
    let n = catalog.best(&gp("act5\\nolevel.ds1")).unwrap();
    assert_eq!(n.source, MapInfoSource::LvlPrest);
    assert_eq!(n.level_id, None);
    assert_eq!(n.lvl_type, None);
    assert_eq!(n.act, 3);
}

#[test]
fn path_guess_prefers_most_shared_dt1s_then_lowest_id() {
    let (_s, catalog, _) = catalog();
    // Cave (id 1, 4 files in the directory) beats CaveB (id 3, 2 files).
    let cave = catalog.best(&gp("act1\\cave\\orphan.ds1")).unwrap();
    assert_eq!(cave.source, MapInfoSource::PathGuess);
    assert_eq!(cave.lvl_type, Some(1));
    assert_eq!(cave.lvlprest_def, None);
    assert_eq!(cave.lvlprest_slot, None);
    assert_eq!(cave.level_id, None);
    assert_eq!(cave.act, 0);
    // No mask applies: all four File slots, in slot order.
    assert_eq!(
        cave.dt1_files,
        vec![
            gp("act1\\cave\\floor.dt1"),
            gp("act1\\cave\\wall.dt1"),
            gp("act1\\cave\\extra.dt1"),
            gp("act1\\cave\\ghost.dt1"),
        ]
    );
    assert_eq!(cave.missing_dt1, vec![gp("act1\\cave\\ghost.dt1")]);
    // Desert (2) and Desert2 (4) tie on one file each: the lowest id wins.
    let desert = catalog.best(&gp("act2\\desert\\orphan.ds1")).unwrap();
    assert_eq!(desert.source, MapInfoSource::PathGuess);
    assert_eq!(desert.lvl_type, Some(2));
    assert_eq!(desert.act, 1);
}

#[test]
fn lvlprest_rows_without_a_level_guess_the_type_from_their_mask() {
    let (_s, catalog, _) = catalog();
    // LevelId 0 (Levels -> LvlType 0, no tilesets). Mask 0b10 enables File 2
    // only: of the three types with a slot in the directory, only DesertC (5)
    // has File 2 there. A plain directory guess would say Desert (2).
    let z = catalog.best(&gp("act2\\desert\\z.ds1")).unwrap();
    assert_eq!(z.source, MapInfoSource::LvlPrest);
    assert_eq!(z.lvlprest_def, Some(17));
    assert_eq!(z.lvl_type, Some(5));
    assert_eq!(z.dt1_files, vec![gp("act2\\desert\\floor3.dt1")]);
    // Mask 0b01: File 1; Desert (2) and Desert2 (4) tie, lowest id wins.
    let y = catalog.best(&gp("act2\\desert\\y.ds1")).unwrap();
    assert_eq!(y.source, MapInfoSource::LvlPrest);
    assert_eq!(y.lvl_type, Some(2));
    assert_eq!(y.dt1_files, vec![gp("act2\\desert\\floor.dt1")]);
}

#[test]
fn path_guess_applies_the_35_palette_rule() {
    let (_s, catalog, _) = catalog();
    let other = catalog.best(&gp("act4\\lava\\other.ds1")).unwrap();
    assert_eq!(other.source, MapInfoSource::PathGuess);
    assert_eq!(other.lvl_type, Some(35));
    assert_eq!((other.act, other.palette_act), (4, 3));
}

#[test]
fn ds1_header_is_the_last_resort() {
    let (_s, catalog, _) = catalog();
    let nowhere = catalog.lookup(&gp("other\\nowhere.ds1"));
    assert_eq!(nowhere.len(), 1);
    let info = &nowhere[0];
    assert_eq!(info.source, MapInfoSource::Ds1Header);
    assert_eq!(info.act, 2);
    assert_eq!(info.palette_act, 2);
    assert_eq!(info.lvl_type, None);
    assert!(info.dt1_files.is_empty() && info.missing_dt1.is_empty());
}

#[test]
fn unknown_ds1_has_no_interpretation() {
    let (_s, catalog, _) = catalog();
    assert!(catalog.lookup(&gp("nope\\none.ds1")).is_empty());
    assert!(catalog.best(&gp("nope\\none.ds1")).is_none());
}

#[test]
fn overrides_set_clear_and_order() {
    let (_s, mut catalog, _) = catalog();
    let orphan = gp("act1\\cave\\orphan.ds1");
    catalog.set_override(orphan.clone(), 2, None);
    let all = catalog.lookup(&orphan);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].source, MapInfoSource::Override);
    assert_eq!(all[0].lvl_type, Some(2));
    assert_eq!(all[0].act, 1);
    assert_eq!(all[1].source, MapInfoSource::PathGuess);
    assert_eq!(
        catalog.best(&orphan).unwrap().source,
        MapInfoSource::Override
    );
    assert_eq!(catalog.overrides(), vec![(orphan.clone(), 2, None)]);

    // Override with a Def applies its mask and finds the slot.
    let shared = gp("act1\\cave\\shared.ds1");
    catalog.set_override(shared.clone(), 1, Some(10));
    let best = catalog.best(&shared).unwrap();
    assert_eq!(best.source, MapInfoSource::Override);
    assert_eq!(best.lvlprest_def, Some(10));
    assert_eq!(best.lvlprest_slot, Some(1));
    assert_eq!(best.dt1_files.len(), 2);
    assert_eq!(catalog.lookup(&shared).len(), 3);

    // A Def that does not exist degrades to all File slots.
    catalog.set_override(shared.clone(), 1, Some(9999));
    assert_eq!(catalog.best(&shared).unwrap().dt1_files.len(), 4);
    // A LvlType that does not exist is ignored.
    catalog.set_override(shared.clone(), 777, None);
    assert_eq!(
        catalog.best(&shared).unwrap().source,
        MapInfoSource::LvlPrest
    );

    catalog.clear_override(&orphan);
    catalog.clear_override(&shared);
    assert!(catalog.overrides().is_empty());
    assert_eq!(
        catalog.best(&orphan).unwrap().source,
        MapInfoSource::PathGuess
    );
    // Clearing again is a no-op.
    catalog.clear_override(&orphan);
}

#[test]
fn resolve_explicit_matches_the_command_line_form() {
    let (_s, catalog, _) = catalog();
    let shared = gp("act1\\cave\\shared.ds1");
    let info = catalog.resolve_explicit(&shared, 1, Some(10)).unwrap();
    assert_eq!(info.source, MapInfoSource::Override);
    assert_eq!((info.lvl_type, info.lvlprest_def), (Some(1), Some(10)));
    assert_eq!(info.lvlprest_slot, Some(1));
    assert_eq!(
        info.dt1_files,
        vec![gp("act1\\cave\\floor.dt1"), gp("act1\\cave\\extra.dt1")]
    );
    // Type 1 with Def 12 (mask 1): only File 1.
    let one = catalog.resolve_explicit(&shared, 1, Some(12)).unwrap();
    assert_eq!(one.dt1_files, vec![gp("act1\\cave\\floor.dt1")]);
    assert_eq!(one.lvlprest_slot, None);
    // Without a Def: every File slot.
    let all = catalog.resolve_explicit(&shared, 1, None).unwrap();
    assert_eq!(all.dt1_files.len(), 4);
    assert_eq!(all.lvlprest_def, None);
    // Errors, not panics.
    assert!(catalog.resolve_explicit(&shared, 777, None).is_err());
    assert!(catalog.resolve_explicit(&shared, 1, Some(9999)).is_err());
    // The ds1 need not be known.
    assert!(
        catalog
            .resolve_explicit(&gp("nope\\none.ds1"), 2, None)
            .is_ok()
    );
}

#[test]
fn import_legacy_counts_known_ds1s_only() {
    let (_s, mut catalog, _) = catalog();
    let entries = vec![
        LegacyEntry {
            lvl_type: 2,
            lvlprest_def: 11,
            ds1: gp("act1\\cave\\shared.ds1"),
        },
        LegacyEntry {
            lvl_type: 1,
            lvlprest_def: 10,
            ds1: gp("nope\\none.ds1"),
        },
        LegacyEntry {
            lvl_type: 1,
            lvlprest_def: 10,
            ds1: gp("act1\\cave\\a.ds1"),
        },
    ];
    assert_eq!(catalog.import_legacy(&entries), 2);
    let overrides = catalog.overrides();
    assert_eq!(overrides.len(), 2);
    // Sorted by path.
    assert_eq!(overrides[0].0, gp("act1\\cave\\a.ds1"));
    assert_eq!(overrides[1], (gp("act1\\cave\\shared.ds1"), 2, Some(11)));
    let best = catalog.best(&gp("act1\\cave\\shared.ds1")).unwrap();
    assert_eq!(best.source, MapInfoSource::Override);
    assert_eq!(best.lvl_type, Some(2));
    assert_eq!(best.lvlprest_def, Some(11));
}

#[test]
fn source_order_is_preference_order() {
    assert!(MapInfoSource::Override < MapInfoSource::LvlPrest);
    assert!(MapInfoSource::LvlPrest < MapInfoSource::PathGuess);
    assert!(MapInfoSource::PathGuess < MapInfoSource::TileCoverage);
    assert!(MapInfoSource::TileCoverage < MapInfoSource::Ds1Header);
}

// ------------------------------------------------------------ tile coverage

#[test]
fn coverage_ranks_directory_candidates_first() {
    let (_s, catalog, _) = catalog();
    // Types 2, 4 and 5 tie on the directory (lowest id would be 2), but only
    // type 4's floor2.dt1 has the DS1's one tile.
    let info = catalog.best(&gp("act2\\desert\\cov.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::PathGuess);
    assert_eq!(info.lvl_type, Some(4));
    assert_eq!(info.coverage, Some(1.0));
}

#[test]
fn tileset_list_breaks_coverage_ties() {
    let (_s, catalog, _) = catalog();
    // No cells, so coverage cannot tell types 2/4/5 apart; the DS1's own
    // file list names floor2.dt1 (type 4).
    let info = catalog.best(&gp("act2\\desert\\ref.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::PathGuess);
    assert_eq!(info.lvl_type, Some(4));
    assert_eq!(info.coverage, None);
}

#[test]
fn tile_coverage_source_without_directory_candidates() {
    let (_s, catalog, _) = catalog();
    let info = catalog.best(&gp("elsewhere\\tc.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::TileCoverage);
    assert_eq!(info.lvl_type, Some(35));
    assert_eq!(info.coverage, Some(1.0));
    assert_eq!((info.act, info.palette_act), (4, 3));
    assert_eq!(info.dt1_files, vec![gp("act4\\lava\\floor.dt1")]);
    assert!(info.lvlprest_def.is_none());
}

#[test]
fn low_coverage_falls_to_the_header() {
    let (_s, catalog, _) = catalog();
    let info = catalog.best(&gp("elsewhere\\low.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::Ds1Header);
    assert_eq!(info.act, 2);
    assert_eq!(info.coverage, None);
}

#[test]
fn wall_fallbacks_count_as_covered() {
    let (_s, catalog, _) = catalog();
    // Wall (18, 8, 4): DT1 has (19, 8, 0) only: orientation swap, then sub 0.
    let info = catalog.best(&gp("elsewhere\\wall.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::TileCoverage);
    assert_eq!(info.lvl_type, Some(1));
    assert_eq!(info.coverage, Some(1.0));
}

#[test]
fn lvlprest_row_without_a_level_uses_the_coverage_winner() {
    let (_s, catalog, _) = catalog();
    let info = catalog.best(&gp("orphan\\p.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::LvlPrest);
    assert_eq!(info.lvlprest_def, Some(19));
    assert_eq!(info.lvl_type, Some(2));
    assert_eq!(info.dt1_files, vec![gp("act2\\desert\\floor.dt1")]);
    assert_eq!(info.coverage, Some(1.0));
}

#[test]
fn coverage_is_reported_for_plain_lvlprest_maps() {
    let (_s, catalog, _) = catalog();
    let info = catalog.best(&gp("act1\\cave\\cov.ds1")).unwrap();
    assert_eq!(info.source, MapInfoSource::LvlPrest);
    assert_eq!(info.coverage, Some(0.5));
    // Overrides carry none.
    let explicit = catalog
        .resolve_explicit(&gp("act1\\cave\\cov.ds1"), 1, Some(20))
        .unwrap();
    assert_eq!(explicit.coverage, None);
}

// ------------------------------------------------------------ malformed tables

#[test]
fn empty_overlay_gives_warnings_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = Overlay::new(vec![Box::new(FolderSource::new(dir.path(), "empty"))]);
    let (catalog, warnings) = MapCatalog::build(&overlay, GameVersion::V114d, &[]).unwrap();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(catalog.lvl_type_ids().is_empty());
    assert!(catalog.best(&gp("x\\y.ds1")).is_none());
}

#[test]
fn missing_columns_become_warnings() {
    let dir = tempfile::tempdir().unwrap();
    excel(
        dir.path(),
        "lvltypes",
        &tsv(&[&["Name", "Act"], &["x", "1"]]),
    );
    excel(
        dir.path(),
        "lvlprest",
        &tsv(&[&["Name", "Def"], &["x", "1"]]),
    );
    excel(
        dir.path(),
        "levels",
        &tsv(&[&["Id", "LevelType"], &["1", "1"]]),
    );
    write(dir.path(), &format!("{TILES}a\\b.ds1"), &ds1_bytes(1));
    let overlay = Overlay::new(vec![Box::new(FolderSource::new(dir.path(), "bad"))]);
    let known = [gp("a\\b.ds1")];
    let (catalog, warnings) = MapCatalog::build(&overlay, GameVersion::V114d, &known).unwrap();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    // Everything falls back to the DS1 header.
    let info = catalog.best(&known[0]).unwrap();
    assert_eq!(info.source, MapInfoSource::Ds1Header);
    assert_eq!(info.act, 1);
}

#[test]
fn garbage_cells_and_unreadable_ds1_become_warnings() {
    let dir = tempfile::tempdir().unwrap();
    excel(
        dir.path(),
        "lvltypes",
        &tsv(&[&["Id", "Act", "File 1"], &["abc", "1", "x.dt1"]]),
    );
    excel(dir.path(), "lvlprest", "not a table at all");
    excel(dir.path(), "levels", "");
    write(dir.path(), &format!("{TILES}a\\bad.ds1"), b"\x01\x02");
    let overlay = Overlay::new(vec![Box::new(FolderSource::new(dir.path(), "bad"))]);
    let known = [gp("a\\bad.ds1")];
    let (catalog, warnings) = MapCatalog::build(&overlay, GameVersion::V114d, &known).unwrap();
    assert!(warnings.len() >= 3, "{warnings:?}");
    assert!(
        warnings.iter().any(|w| w.contains("bad.ds1")),
        "{warnings:?}"
    );
    // Still answers: header unreadable -> act 0.
    let info = catalog.best(&known[0]).unwrap();
    assert_eq!(info.source, MapInfoSource::Ds1Header);
    assert_eq!(info.act, 0);
}

// ------------------------------------------------------------ legacy ini

#[test]
fn legacy_ini_maps_paths() {
    let (entries, warnings) = parse_legacy_ini(
        "1 1 assets/tiles/ACT1/TOWN/townE1.ds1\n\
         7 167 assets/tiles/expansion/Siege/snowbarrFD1.ds1\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        entries,
        vec![
            LegacyEntry {
                lvl_type: 1,
                lvlprest_def: 1,
                ds1: GamePath::new("data\\global\\tiles\\act1\\town\\towne1.ds1"),
            },
            LegacyEntry {
                lvl_type: 7,
                lvlprest_def: 167,
                ds1: GamePath::new("data\\global\\tiles\\expansion\\siege\\snowbarrfd1.ds1"),
            },
        ]
    );
}

#[test]
fn legacy_ini_skips_blanks_and_comments_and_handles_crlf() {
    let text = "\u{feff}# header\r\n\r\n   \r\n; another\r\n\
                 2   3\tassets\\tiles\\ACT2\\Town\\a b.ds1  \r\n\
                 5 6 assets/tiles/act1/x.ds1";
    let (entries, warnings) = parse_legacy_ini(text);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].ds1.as_str(),
        "data\\global\\tiles\\act2\\town\\a b.ds1"
    );
    assert_eq!((entries[1].lvl_type, entries[1].lvlprest_def), (5, 6));
}

#[test]
fn legacy_ini_bad_lines_are_warnings() {
    let text = "1 1 assets/tiles/act1/ok.ds1\n\
                1 1\n\
                x 1 assets/tiles/act1/a.ds1\n\
                1 -4 assets/tiles/act1/b.ds1\n\
                1 1 assets/tiles/act1/c.dt1\n\
                lonely\n\
                3 4 assets/tiles/act1/fine.ds1\n";
    let (entries, warnings) = parse_legacy_ini(text);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].ds1.file_name(), "fine.ds1");
    assert_eq!(warnings.len(), 5, "{warnings:?}");
    assert!(warnings[0].starts_with("line 2:"), "{warnings:?}");
    assert!(warnings[1].starts_with("line 3:"), "{warnings:?}");
    assert!(warnings[4].starts_with("line 6:"), "{warnings:?}");
}

#[test]
fn legacy_ini_empty_text() {
    let (entries, warnings) = parse_legacy_ini("");
    assert!(entries.is_empty() && warnings.is_empty());
}
