use super::*;
use crate::txt::TxtTable;

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
                c.offset + c.field.width() <= l.record_size,
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
            "levels"
        ]
    );
    for v in BOTH {
        assert_eq!(record_size(ExcelTableKind::LvlTypes, v), 0x788);
        assert_eq!(record_size(ExcelTableKind::Objects, v), 0x1C0);
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
