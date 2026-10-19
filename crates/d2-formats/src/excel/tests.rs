use super::*;
use crate::txt::{Row, TxtTable};

const BOTH: [ExcelVersion; 2] = [ExcelVersion::V113c, ExcelVersion::V114d];

fn file(count: usize, record: usize, fill: impl Fn(usize, &mut [u8])) -> Vec<u8> {
    let mut out = (count as u32).to_le_bytes().to_vec();
    for i in 0..count {
        let mut rec = vec![0u8; record];
        fill(i, &mut rec);
        out.extend_from_slice(&rec);
    }
    out
}

fn put_str(rec: &mut [u8], off: usize, s: &str) {
    rec[off..off + s.len()].copy_from_slice(s.as_bytes());
}

fn put_u32(rec: &mut [u8], off: usize, v: u32) {
    rec[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn every_column_lies_inside_its_record() {
    for kind in ExcelTableKind::all() {
        if *kind == ExcelTableKind::ObjPreset {
            continue;
        }
        let l = layout(*kind);
        assert_eq!(l.record_size, record_size(*kind, ExcelVersion::V113c));
        for c in &l.columns {
            assert!(
                c.offset + c.field.reach() <= l.record_size,
                "{kind:?} {}",
                c.name
            );
        }
        let mut names: Vec<&str> = l.columns.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "{kind:?} has duplicate column names");
    }
    let defs = leveldefs_layout();
    assert!(
        defs.columns
            .iter()
            .all(|c| c.offset + c.field.width() <= LEVELDEFS_SIZE)
    );
}

#[test]
fn columns_do_not_overlap() {
    // Fields of one record must not share bytes (Id/Index excepted).
    for kind in [
        ExcelTableKind::LvlTypes,
        ExcelTableKind::LvlPrest,
        ExcelTableKind::LvlWarp,
        ExcelTableKind::Objects,
        ExcelTableKind::Levels,
        ExcelTableKind::MonPreset,
        ExcelTableKind::MonStats,
        ExcelTableKind::MonStats2,
        ExcelTableKind::SuperUniques,
    ] {
        let l = layout(kind);
        let mut used = vec![None::<&str>; l.record_size];
        for c in &l.columns {
            for (b, slot) in used
                .iter_mut()
                .enumerate()
                .skip(c.offset)
                .take(c.field.width())
            {
                assert!(
                    slot.is_none(),
                    "{kind:?}: {} overlaps {:?} at {b:#x}",
                    c.name,
                    slot
                );
                *slot = Some(c.name.as_str());
            }
        }
    }
}

#[test]
fn stems_and_sizes() {
    let stems: Vec<_> = ExcelTableKind::all()
        .iter()
        .map(|k| k.file_stem())
        .collect();
    assert_eq!(
        stems,
        [
            "lvltypes",
            "lvlprest",
            "lvlwarp",
            "objects",
            "objpreset",
            "levels",
            "monpreset",
            "monstats",
            "monstats2",
            "superuniques"
        ]
    );
    for v in BOTH {
        assert_eq!(record_size(ExcelTableKind::LvlTypes, v), 0x788);
        assert_eq!(record_size(ExcelTableKind::Objects, v), 0x1C0);
        assert_eq!(record_size(ExcelTableKind::MonPreset, v), 4);
        assert_eq!(record_size(ExcelTableKind::MonStats, v), 424);
        assert_eq!(record_size(ExcelTableKind::MonStats2, v), 308);
        assert_eq!(record_size(ExcelTableKind::SuperUniques, v), 52);
        assert_eq!(leveldefs_record_size(v), 156);
    }
}

#[test]
fn synthetic_lvltypes_and_lvlprest() {
    let types = file(2, LVLTYPES_SIZE, |i, r| {
        put_str(r, 0, if i == 0 { "0" } else { "Act1/Town/Floor.dt1" });
        put_str(r, 60, "Act1/Town/Objects.dt1");
        put_u32(r, 0x780, 1 + i as u32);
        put_u32(r, 0x784, i as u32);
    });
    let t = decode_bin(ExcelTableKind::LvlTypes, ExcelVersion::V113c, &types).unwrap();
    assert_eq!(t.len(), 2);
    let r1 = t.row(1).unwrap();
    assert_eq!(r1.get("Id"), Some("1"));
    assert_eq!(r1.get("File 1"), Some("Act1/Town/Floor.dt1"));
    assert_eq!(r1.get("File 2"), Some("Act1/Town/Objects.dt1"));
    assert_eq!(r1.get("File 3"), Some(""));
    assert_eq!(r1.get_i64("Act"), Some(2));
    assert_eq!(r1.get_i64("Expansion"), Some(1));
    assert_eq!(t.row(0).unwrap().get("File 1"), Some("0"));

    // a 60-byte name with no NUL is taken whole
    let full = file(1, LVLTYPES_SIZE, |_, r| r[..60].fill(b'x'));
    let t = decode_bin(ExcelTableKind::LvlTypes, ExcelVersion::V114d, &full).unwrap();
    assert_eq!(t.row(0).unwrap().get("File 1").map(str::len), Some(60));

    let prest = file(1, LVLPREST_SIZE, |_, r| {
        put_u32(r, 0, 7);
        put_u32(r, 4, (-1i32) as u32);
        put_u32(r, 40, 8);
        put_str(r, 68, "Act1/Town/TownN1.ds1");
        put_u32(r, 428, 0xFFFF_FFFF);
    });
    let t = decode_bin(ExcelTableKind::LvlPrest, ExcelVersion::V113c, &prest).unwrap();
    let r = t.row(0).unwrap();
    assert_eq!(r.get_i64("Def"), Some(7));
    assert_eq!(r.get_i64("LevelId"), Some(-1));
    assert_eq!(r.get_i64("SizeX"), Some(8));
    assert_eq!(r.get("File1"), Some("Act1/Town/TownN1.ds1"));
    assert_eq!(r.get_i64("Dt1Mask"), Some(0xFFFF_FFFF));
}

#[test]
fn synthetic_objpreset_and_levels() {
    let bytes = file(750, 4, |i, r| put_u32(r, 0, (i * 2) as u32));
    let t = decode_bin(ExcelTableKind::ObjPreset, ExcelVersion::V113c, &bytes).unwrap();
    assert_eq!(t.len(), 750);
    let r = t.row(151).unwrap();
    assert_eq!(r.get_i64("Act"), Some(1));
    assert_eq!(r.get_i64("Id"), Some(1));
    assert_eq!(r.get_i64("ObjectsId"), Some(302));
    assert_eq!(t.row(749).unwrap().get_i64("Act"), Some(4));

    let levels = file(2, LEVELS_SIZE, |i, r| {
        r[0..2].copy_from_slice(&(i as u16).to_le_bytes());
        r[2] = 3;
        r[3] = 4;
        put_str(r, 245, "Rogue Encampment");
    });
    let defs = file(2, LEVELDEFS_SIZE, |i, r| {
        put_u32(r, 48, 2);
        put_u32(r, 52, 10 + i as u32);
    });
    let t = decode_levels_bin(ExcelVersion::V113c, &levels, &defs).unwrap();
    let r = t.row(1).unwrap();
    assert_eq!(r.get_i64("Id"), Some(1));
    assert_eq!(r.get_i64("Pal"), Some(3));
    assert_eq!(r.get_i64("Act"), Some(4));
    assert_eq!(r.get_i64("LevelType"), Some(11));
    assert_eq!(r.get_i64("DrlgType"), Some(2));
    assert_eq!(r.get("LevelName"), Some("Rogue Encampment"));
    // levels.bin alone has no LevelType
    let alone = decode_bin(ExcelTableKind::Levels, ExcelVersion::V113c, &levels).unwrap();
    assert!(alone.column("LevelType").is_none());
    // mismatched record counts are refused
    let one = file(1, LEVELDEFS_SIZE, |_, _| {});
    assert!(decode_levels_bin(ExcelVersion::V113c, &levels, &one).is_err());
}

#[test]
fn bad_sizes_are_rejected() {
    for kind in ExcelTableKind::all() {
        let size = record_size(*kind, ExcelVersion::V113c);
        assert!(decode_bin(*kind, ExcelVersion::V113c, &[]).is_err());
        assert!(decode_bin(*kind, ExcelVersion::V113c, &[1, 0]).is_err());
        // count says 1 but there are two records / half a record
        let mut two = file(2, size, |_, _| {});
        two[..4].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_bin(*kind, ExcelVersion::V113c, &two).is_err());
        let half = file(1, size / 2, |_, _| {});
        assert!(decode_bin(*kind, ExcelVersion::V113c, &half).is_err());
        // a huge count must not allocate or overflow
        let mut huge = file(1, size, |_, _| {});
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_bin(*kind, ExcelVersion::V113c, &huge).is_err());
        // an empty table is fine
        let empty = file(0, size, |_, _| {});
        assert!(
            decode_bin(*kind, ExcelVersion::V113c, &empty)
                .unwrap()
                .is_empty()
        );
        assert_eq!(detect_version(*kind, &empty), Some(ExcelVersion::V113c));
        assert_eq!(detect_version(*kind, &two), None);
    }
}

// ---------------------------------------------------------------------------
// corpus

/// `.txt` rows without the `Expansion` separator the compiler skips.
fn record_rows(t: &TxtTable) -> Vec<usize> {
    t.rows()
        .filter(|r| {
            let cells = r.cells();
            !(cells.first().map(String::as_str) == Some("Expansion")
                && cells.iter().skip(1).all(String::is_empty))
        })
        .map(|r| r.index())
        .collect()
}

/// Compares every decoded column against the `.txt`. Returns (columns, cells).
fn compare_with_txt(label: &str, bin: &TxtTable, txt: &TxtTable) -> (usize, usize) {
    let rows = record_rows(txt);
    assert_eq!(bin.len(), rows.len(), "{label}: record count");
    let mut columns = 0;
    let mut cells = 0;
    let mut failures = Vec::new();
    for (bi, header) in bin.headers().iter().enumerate() {
        let Some(ti) = txt.column(header) else {
            panic!("{label}: decoded column {header:?} has no .txt column");
        };
        // The txt `Id` of Objects is informational (three vanilla rows differ);
        // the game addresses records by index.
        let skip_id = label == "objects" && header == "Id";
        columns += 1;
        for (k, &trow) in rows.iter().enumerate() {
            let b = &bin.row(k).unwrap().cells()[bi];
            let t = &txt.row(trow).unwrap().cells()[ti];
            let ok = b == t
                // the compiler writes 0 for a blank numeric cell
                || (t.is_empty() && b == "0")
                || skip_id
                // objects.bin keeps a two-character token; row 0 is 'NU0'
                || (label == "objects" && header == "Token" && t.starts_with(b.as_str()));
            if !ok {
                failures.push(format!(
                    "{label}: record {k} column {header:?}: bin {b:?} != txt {t:?}"
                ));
                break;
            }
            cells += 1;
        }
    }
    assert!(failures.is_empty(), "{:#?}", failures);
    (columns, cells)
}

fn read(path: &std::path::Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

#[test]
fn vanilla_bin_equals_txt() {
    let Some(dir) = crate::testutil::extracted("vanilla") else {
        println!("skipped: extracted/vanilla missing");
        return;
    };
    let excel = dir.join("data").join("global").join("excel");
    let mut report = Vec::new();
    for kind in ExcelTableKind::all() {
        if is_mon_kind(*kind) {
            continue; // cross-table columns: see vanilla_mon_bins_equal_txt
        }
        let stem = kind.file_stem();
        let (Some(bin), Some(txt)) = (
            read(&excel.join(format!("{stem}.bin"))),
            read(&excel.join(format!("{stem}.txt"))),
        ) else {
            println!("vanilla {stem}: only one form present, not compared");
            continue;
        };
        let txt = TxtTable::parse(&txt).unwrap();
        let version = detect_version(*kind, &bin)
            .unwrap_or_else(|| panic!("{stem}: no version fits {} bytes", bin.len()));
        let table = if *kind == ExcelTableKind::Levels {
            let defs = read(&excel.join("leveldefs.bin")).expect("leveldefs.bin");
            decode_levels_bin(version, &bin, &defs).unwrap()
        } else {
            decode_bin(*kind, version, &bin).unwrap()
        };
        let (columns, cells) = compare_with_txt(stem, &table, &txt);
        report.push(format!(
            "{stem}: {} records, {columns} columns, {cells} cells equal ({version:?})",
            table.len()
        ));
    }
    println!("vanilla .bin vs .txt:\n  {}", report.join("\n  "));
}

#[test]
fn median_xl_bins_decode_as_1_13c() {
    let Some(dir) = crate::testutil::extracted("mxl") else {
        println!("skipped: extracted/mxl missing");
        return;
    };
    let excel = dir.join("data").join("global").join("excel");
    for kind in ExcelTableKind::all() {
        let stem = kind.file_stem();
        let Some(bin) = read(&excel.join(format!("{stem}.bin"))) else {
            println!("mxl {stem}.bin: not present");
            continue;
        };
        assert_eq!(
            detect_version(*kind, &bin),
            Some(ExcelVersion::V113c),
            "{stem}"
        );
        let table = if *kind == ExcelTableKind::Levels {
            let defs = read(&excel.join("leveldefs.bin")).expect("leveldefs.bin");
            decode_levels_bin(ExcelVersion::V113c, &bin, &defs).unwrap()
        } else {
            decode_bin(*kind, ExcelVersion::V113c, &bin).unwrap()
        };
        println!(
            "mxl {stem}: {} records, {} columns",
            table.len(),
            table.headers().len()
        );
        assert!(!table.is_empty(), "{stem}");
    }
}

// ---------------------------------------------------------------------------
// monster tables

use crate::tbl::StringTable;
use std::collections::HashMap;
use std::path::Path;

fn is_mon_kind(kind: ExcelTableKind) -> bool {
    matches!(
        kind,
        ExcelTableKind::MonPreset
            | ExcelTableKind::MonStats
            | ExcelTableKind::MonStats2
            | ExcelTableKind::SuperUniques
    )
}

/// The records of a `.bin` (after the count word).
fn records(bytes: &[u8], size: usize) -> std::slice::ChunksExact<'_, u8> {
    bytes[4..].chunks_exact(size)
}

fn put_u16(rec: &mut [u8], off: usize, v: u16) {
    rec[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn synthetic_monster_tables() {
    let preset = file(3, MONPRESET_SIZE, |i, r| {
        r[0] = 1 + i as u8;
        r[1] = i as u8;
        put_u16(r, 2, 300 + i as u16);
    });
    let t = decode_bin(ExcelTableKind::MonPreset, ExcelVersion::V113c, &preset).unwrap();
    let r = t.row(2).unwrap();
    assert_eq!(r.get_i64("Act"), Some(3));
    assert_eq!(r.get_i64("PlaceKind"), Some(2));
    assert_eq!(r.get_i64("PlaceIndex"), Some(302));

    let stats = file(2, MONSTATS_SIZE, |i, r| {
        put_u16(r, 2, 0xFFFF);
        put_u16(r, 6, 5382 + i as u16);
        put_u32(r, 12, 1 << 8 | 1 << 25);
        put_str(r, 16, "SK  ");
        put_u16(r, 24, 1446);
        r[34] = (-3i8) as u8;
        put_u16(r, 170, 40);
    });
    let t = decode_bin(ExcelTableKind::MonStats, ExcelVersion::V114d, &stats).unwrap();
    let r = t.row(1).unwrap();
    assert_eq!(r.get_i64("hcIdx"), Some(1));
    assert_eq!(r.get_i64("BaseIdRow"), Some(-1));
    assert_eq!(r.get_i64("NameStrId"), Some(5383));
    assert_eq!(r.get("Code"), Some("SK"));
    assert_eq!(r.get_i64("MonStatsExRow"), Some(1446));
    assert_eq!(r.get_i64("npc"), Some(1));
    assert_eq!(r.get_i64("enabled"), Some(1));
    assert_eq!(r.get_i64("boss"), Some(0));
    assert_eq!(r.get_i64("spawnx"), Some(-3));
    assert_eq!(r.get_i64("Level"), Some(40));

    let stats2 = file(1, MONSTATS2_SIZE, |_, r| {
        put_u32(r, 232, 0b1011_1111);
        put_u32(r, 240, 1 << 15);
        put_str(r, 16, "1hs ");
        r[8] = 2;
        r[9] = 3;
        put_u32(r, 236, 9);
        r[244] = 8;
    });
    let t = decode_bin(ExcelTableKind::MonStats2, ExcelVersion::V113c, &stats2).unwrap();
    let r = t.row(0).unwrap();
    assert_eq!(r.get("BaseW"), Some("1hs"));
    assert_eq!(r.get_i64("SizeX"), Some(2));
    assert_eq!(r.get_i64("SizeY"), Some(3));
    assert_eq!(r.get_i64("TotalPieces"), Some(9));
    assert_eq!(r.get_i64("dDT"), Some(8));
    let on: Vec<&str> = ["HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH"]
        .into_iter()
        .filter(|c| r.get_i64(c) == Some(1))
        .collect();
    assert_eq!(on, ["HD", "TR", "LG", "RA", "LA", "RH", "SH"]);
    assert_eq!(r.get_i64("mRN"), Some(1));
    assert_eq!(r.get_i64("mDT"), Some(0));

    let supers = file(1, SUPERUNIQUES_SIZE, |_, r| {
        put_u16(r, 2, 2869);
        put_u16(r, 4, 58);
        put_u32(r, 12, 8);
        r[37] = 1;
    });
    let t = decode_bin(ExcelTableKind::SuperUniques, ExcelVersion::V113c, &supers).unwrap();
    let r = t.row(0).unwrap();
    assert_eq!(r.get_i64("NameStrId"), Some(2869));
    assert_eq!(r.get_i64("ClassRow"), Some(58));
    assert_eq!(r.get_i64("Mod1"), Some(8));
    assert_eq!(r.get_i64("EClass"), Some(1));

    assert_eq!(monplace_code(0), Some("place_nothing"));
    assert_eq!(monplace_code(36), Some("place_group100"));
    assert_eq!(monplace_code(37), None);
}

/// Rows of a monster `.txt` as the compiler sees them: the `Expansion`
/// separator is skipped whatever its other cells hold (`monstats2.txt` has an
/// `EOL` cell on it).
fn mon_rows(t: &TxtTable) -> Vec<usize> {
    t.rows()
        .filter(|r| {
            !r.cells()
                .first()
                .is_some_and(|c| c.trim().eq_ignore_ascii_case("expansion"))
        })
        .map(|r| r.index())
        .collect()
}

/// Lower-cased text of `column` to its record number (first match wins).
fn row_numbers(t: &TxtTable, column: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    let col = t.column(column).unwrap();
    for (k, r) in mon_rows(t).into_iter().enumerate() {
        let text = t.row(r).unwrap().cells()[col].trim().to_ascii_lowercase();
        map.entry(text).or_insert(k);
    }
    map
}

/// The `string.tbl`, `patchstring.tbl`, `expansionstring.tbl` of one install.
struct Strings([Option<StringTable>; 3]);

impl Strings {
    fn load(root: &Path) -> Strings {
        let dir = root.join("data").join("local").join("lng").join("eng");
        let one = |name: &str| read(&dir.join(name)).and_then(|b| StringTable::parse(&b).ok());
        Strings([
            one("string.tbl"),
            one("patchstring.tbl"),
            one("expansionstring.tbl"),
        ])
    }

    /// The key a compiled string id stands for.
    fn key(&self, id: i64) -> Option<String> {
        let (table, base) = if id >= 20000 {
            (2, 20000)
        } else if id >= 10000 {
            (1, 10000)
        } else {
            (0, 0)
        };
        let id = u16::try_from(id - base).ok()?;
        let (key, _) = self.0.get(table)?.as_ref()?.entry_by_id(id)?;
        Some(key.to_owned())
    }

    fn has_key(&self, key: &str) -> bool {
        self.0.iter().flatten().any(|t| t.get(key).is_some())
    }

    /// Whether the compiled id `cell` stands for the `.txt` key `key`. A blank
    /// or unknown key is compiled as `dummy`.
    fn matches(&self, cell: &str, key: &str) -> bool {
        let Some(found) = cell.parse::<i64>().ok().and_then(|id| self.key(id)) else {
            return false;
        };
        let key = key.trim();
        if key.is_empty() || !self.has_key(key) {
            found.eq_ignore_ascii_case("dummy")
        } else {
            found.eq_ignore_ascii_case(key)
        }
    }
}

/// Compares `bin` to `txt` column by column (both over the compiled rows).
/// Columns `special` answers for are compared by it; every other decoded
/// column must be a `.txt` column of the same name and equal to it. Returns
/// (columns, cells).
fn compare_mon(
    label: &str,
    bin: &TxtTable,
    txt: &TxtTable,
    special: impl Fn(&str, &str, Row<'_>) -> Option<bool>,
) -> (usize, usize) {
    let rows = mon_rows(txt);
    assert_eq!(bin.len(), rows.len(), "{label}: record count");
    let mut cells = 0;
    let mut failures = Vec::new();
    for (bi, header) in bin.headers().iter().enumerate() {
        let ti = txt.column(header);
        for (k, &trow) in rows.iter().enumerate() {
            let b = &bin.row(k).unwrap().cells()[bi];
            let row = txt.row(trow).unwrap();
            let ok = match special(header, b, row) {
                Some(ok) => ok,
                None => {
                    let ti = ti.unwrap_or_else(|| {
                        panic!("{label}: decoded column {header:?} has no .txt column")
                    });
                    let t = &row.cells()[ti];
                    b == t || (t.trim().is_empty() && b == "0")
                }
            };
            if !ok {
                failures.push(format!(
                    "{label}: record {k} column {header:?}: bin {b:?} vs txt row {trow}"
                ));
                break;
            }
            cells += 1;
        }
    }
    assert!(failures.is_empty(), "{:#?}", failures);
    (bin.headers().len(), cells)
}

fn load_pair(excel: &Path, stem: &str) -> Option<(Vec<u8>, TxtTable)> {
    let bin = read(&excel.join(format!("{stem}.bin")))?;
    let txt = TxtTable::parse(&read(&excel.join(format!("{stem}.txt")))?).ok()?;
    Some((bin, txt))
}

#[test]
fn vanilla_mon_bins_equal_txt() {
    let Some(dir) = crate::testutil::extracted("vanilla") else {
        println!("skipped: extracted/vanilla missing");
        return;
    };
    let excel = dir.join("data").join("global").join("excel");
    let (Some((preset_bin, preset)), Some((stats_bin, stats)), Some((stats2_bin, stats2))) = (
        load_pair(&excel, "monpreset"),
        load_pair(&excel, "monstats"),
        load_pair(&excel, "monstats2"),
    ) else {
        println!(
            "skipped: vanilla monpreset/monstats2 .bin+.txt missing \
             (re-run scripts/extract_fixtures.py --profile vanilla)"
        );
        return;
    };
    let Some((supers_bin, supers)) = load_pair(&excel, "superuniques") else {
        println!("skipped: vanilla superuniques missing");
        return;
    };
    let monplace = TxtTable::parse(&read(&excel.join("monplace.txt")).unwrap()).unwrap();
    let strings = Strings::load(&dir);
    assert!(
        strings.0.iter().all(Option::is_some),
        "vanilla string tables"
    );

    let stats_ids = row_numbers(&stats, "Id");
    let stats2_ids = row_numbers(&stats2, "Id");
    let place_ids = row_numbers(&monplace, "code");
    let super_ids = row_numbers(&supers, "Superunique");
    let mut report = Vec::new();

    // MonStats
    let version = detect_version(ExcelTableKind::MonStats, &stats_bin).unwrap();
    assert_eq!(version, ExcelVersion::V113c);
    let bin = decode_bin(ExcelTableKind::MonStats, version, &stats_bin).unwrap();
    let row_ref = |row: Row<'_>, column: &str, map: &HashMap<String, usize>, b: &str| {
        let text = row.get(column).unwrap_or("").trim().to_ascii_lowercase();
        let want = if text.is_empty() {
            Some(-1)
        } else {
            map.get(&text).map(|&r| r as i64)
        };
        want.is_some() && b.parse::<i64>().ok() == want
    };
    let (columns, cells) = compare_mon("monstats", &bin, &stats, |h, b, row| match h {
        "BaseIdRow" => Some(row_ref(row, "BaseId", &stats_ids, b)),
        "NextInClassRow" => Some(row_ref(row, "NextInClass", &stats_ids, b)),
        "spawnRow" => Some(row_ref(row, "spawn", &stats_ids, b)),
        "minion1Row" => Some(row_ref(row, "minion1", &stats_ids, b)),
        "minion2Row" => Some(row_ref(row, "minion2", &stats_ids, b)),
        "MonStatsExRow" => Some(row_ref(row, "MonStatsEx", &stats2_ids, b)),
        // `Lilith` was added to patchstring.tbl after monstats.bin was compiled,
        // so the vanilla bin still holds the id of `dummy` for uberandariel.
        "NameStrId" if row.get("Id") == Some("uberandariel") => Some(b == "5382"),
        "NameStrId" => Some(strings.matches(b, row.get("NameStr").unwrap_or(""))),
        _ => None,
    });
    report.push(format!(
        "monstats: {} records of {} bytes, {columns} columns, {cells} cells equal",
        bin.len(),
        record_size(ExcelTableKind::MonStats, version)
    ));
    // The stored word at offset 0 is the record index except in the last 11
    // vanilla records (stale: index - 1, and 617 for record 723).
    let stale: Vec<usize> = records(&stats_bin, MONSTATS_SIZE)
        .enumerate()
        .filter(|(i, rec)| usize::from(u16::from_le_bytes([rec[0], rec[1]])) != *i)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(stale, (723..734).collect::<Vec<_>>());

    // MonStats2
    let version = detect_version(ExcelTableKind::MonStats2, &stats2_bin).unwrap();
    let bin = decode_bin(ExcelTableKind::MonStats2, version, &stats2_bin).unwrap();
    let (columns, cells) = compare_mon("monstats2", &bin, &stats2, |_, _, _| None);
    report.push(format!(
        "monstats2: {} records of {} bytes, {columns} columns, {cells} cells equal",
        bin.len(),
        record_size(ExcelTableKind::MonStats2, version)
    ));
    // The word at offset 0 is the record index.
    for (i, rec) in records(&stats2_bin, MONSTATS2_SIZE).enumerate() {
        assert_eq!(usize::from(u16::from_le_bytes([rec[0], rec[1]])), i);
    }

    // SuperUniques
    let version = detect_version(ExcelTableKind::SuperUniques, &supers_bin).unwrap();
    let bin = decode_bin(ExcelTableKind::SuperUniques, version, &supers_bin).unwrap();
    let (columns, cells) = compare_mon("superuniques", &bin, &supers, |h, b, row| match h {
        "ClassRow" => Some(row_ref(row, "Class", &stats_ids, b)),
        "NameStrId" => Some(strings.matches(b, row.get("Name").unwrap_or(""))),
        _ => None,
    });
    report.push(format!(
        "superuniques: {} records of {} bytes, {columns} columns, {cells} cells equal",
        bin.len(),
        record_size(ExcelTableKind::SuperUniques, version)
    ));

    // MonPreset: `Place` is looked up in SuperUniques (when its Class exists),
    // then MonStats, then MonPlace.
    let version = detect_version(ExcelTableKind::MonPreset, &preset_bin).unwrap();
    let bin = decode_bin(ExcelTableKind::MonPreset, version, &preset_bin).unwrap();
    let place_of = |row: Row<'_>| -> Option<(i64, i64)> {
        let place = row.get("Place").unwrap_or("").trim().to_ascii_lowercase();
        let class_known = |s: usize| {
            let class = supers.row(mon_rows(&supers)[s]).unwrap().get("Class");
            stats_ids.contains_key(&class.unwrap_or("").trim().to_ascii_lowercase())
        };
        if let Some(&s) = super_ids.get(&place)
            && class_known(s)
        {
            return Some((2, s as i64));
        }
        if let Some(&m) = stats_ids.get(&place) {
            return Some((1, m as i64));
        }
        place_ids.get(&place).map(|&p| (0, p as i64))
    };
    let (columns, cells) = compare_mon("monpreset", &bin, &preset, |h, b, row| match h {
        "PlaceKind" => Some(place_of(row).map(|p| p.0) == b.parse().ok()),
        "PlaceIndex" => Some(place_of(row).map(|p| p.1) == b.parse().ok()),
        _ => None,
    });
    report.push(format!(
        "monpreset: {} records of {} bytes, {columns} columns, {cells} cells equal",
        bin.len(),
        record_size(ExcelTableKind::MonPreset, version)
    ));
    for row in bin.rows() {
        if row.get_i64("PlaceKind") == Some(0) {
            let index = usize::try_from(row.get_i64("PlaceIndex").unwrap()).unwrap();
            assert!(monplace_code(index).is_some());
        }
    }
    // The built-in MonPlace names are monplace.txt's.
    for (code, &index) in &place_ids {
        assert_eq!(
            monplace_code(index).map(str::to_ascii_lowercase).as_deref(),
            Some(code.as_str())
        );
    }
    println!("vanilla monster .bin vs .txt:\n  {}", report.join("\n  "));
}

#[test]
fn median_xl_mon_bins_are_consistent() {
    let Some(dir) = crate::testutil::extracted("mxl") else {
        println!("skipped: extracted/mxl missing");
        return;
    };
    let excel = dir.join("data").join("global").join("excel");
    let load = |kind: ExcelTableKind| {
        let bytes = read(&excel.join(format!("{}.bin", kind.file_stem()))).unwrap();
        assert_eq!(detect_version(kind, &bytes), Some(ExcelVersion::V113c));
        let table = decode_bin(kind, ExcelVersion::V113c, &bytes).unwrap();
        println!(
            "mxl {}: {} records of {} bytes, {} columns",
            kind.file_stem(),
            table.len(),
            record_size(kind, ExcelVersion::V113c),
            table.headers().len()
        );
        (bytes, table)
    };
    let (preset_bytes, preset) = load(ExcelTableKind::MonPreset);
    let (stats_bytes, stats) = load(ExcelTableKind::MonStats);
    let (stats2_bytes, stats2) = load(ExcelTableKind::MonStats2);
    let (supers_bytes, supers) = load(ExcelTableKind::SuperUniques);
    // The words at offset 0 are the record indices.
    for (i, rec) in records(&stats_bytes, MONSTATS_SIZE).enumerate() {
        assert_eq!(usize::from(u16::from_le_bytes([rec[0], rec[1]])), i);
    }
    for (i, rec) in records(&stats2_bytes, MONSTATS2_SIZE).enumerate() {
        assert_eq!(usize::from(u16::from_le_bytes([rec[0], rec[1]])), i);
    }
    let _ = (preset_bytes, supers_bytes);

    // Every reference points inside its target table.
    let inside = |value: Option<i64>, len: usize| {
        value.is_some_and(|v| v == -1 || usize::try_from(v).is_ok_and(|v| v < len))
    };
    let mut bad = Vec::new();
    for row in preset.rows() {
        let (kind, index) = (row.get_i64("PlaceKind"), row.get_i64("PlaceIndex"));
        let len = match kind {
            Some(0) => 37,
            Some(1) => stats.len(),
            Some(2) => supers.len(),
            _ => 0,
        };
        if !inside(index, len) || index == Some(-1) {
            bad.push(format!("monpreset {} {kind:?} {index:?}", row.index()));
        }
        assert!((1..=5).contains(&row.get_i64("Act").unwrap()));
    }
    for row in stats.rows() {
        for (column, len) in [
            ("BaseIdRow", stats.len()),
            ("NextInClassRow", stats.len()),
            ("spawnRow", stats.len()),
            ("minion1Row", stats.len()),
            ("minion2Row", stats.len()),
            ("MonStatsExRow", stats2.len()),
        ] {
            if !inside(row.get_i64(column), len) {
                bad.push(format!("monstats {} {column}", row.index()));
            }
        }
    }
    for row in supers.rows() {
        if !inside(row.get_i64("ClassRow"), stats.len()) {
            bad.push(format!("superuniques {} ClassRow", row.index()));
        }
    }
    assert!(bad.is_empty(), "{:#?}", &bad[..bad.len().min(20)]);

    // How many names resolve through Median XL's own string tables.
    let strings = Strings::load(&dir);
    let named = |t: &TxtTable| {
        let ok = t
            .rows()
            .filter(|r| {
                r.get_i64("NameStrId")
                    .and_then(|id| strings.key(id))
                    .is_some_and(|k| !k.eq_ignore_ascii_case("dummy"))
            })
            .count();
        format!("{ok}/{}", t.len())
    };
    println!(
        "mxl names resolved: monstats {}, superuniques {}",
        named(&stats),
        named(&supers)
    );
}
