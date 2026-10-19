use super::*;
use crate::excel::{self, ExcelTableKind, ExcelVersion};

fn table(text: &str) -> TxtTable {
    TxtTable::parse(text.as_bytes()).unwrap()
}

fn tabs(cells: &[&str]) -> String {
    cells.join("\t")
}

#[test]
fn lvl_types_from_txt() {
    let mut text = String::new();
    let mut header = vec!["Name", "Id"];
    let file_headers: Vec<String> = (1..=32).map(|n| format!("File {n}")).collect();
    header.extend(file_headers.iter().map(String::as_str));
    header.extend(["Beta", "Act", "Expansion"]);
    text.push_str(&tabs(&header));
    text.push_str("\r\n");
    let row = |name: &str, id: &str, f1: &str, f2: &str, act: &str, exp: &str| {
        let mut cells = vec![name, id, f1, f2];
        cells.extend(std::iter::repeat_n("0", 30));
        cells.extend(["0", act, exp]);
        tabs(&cells) + "\r\n"
    };
    text.push_str(&row("None", "0", "0", "0", "0", "0"));
    text.push_str(&row(
        "Act 1 - Town",
        "1",
        "Act1/Town/Floor.dt1",
        "Act1/Town/Objects.dt1",
        "1",
        "0",
    ));
    text.push_str("Expansion\t\t\r\n");
    text.push_str(&row(
        "Act 5 - Town",
        "29",
        "Expansion/town/blacksmith.dt1",
        "",
        "5",
        "",
    ));
    let t = lvl_types(&table(&text)).unwrap();
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].files.len(), 32);
    assert!(t[0].files.iter().all(Option::is_none));
    assert_eq!(t[1].id, 1);
    assert_eq!(t[1].name, "Act 1 - Town");
    assert_eq!(t[1].act, 1);
    assert_eq!(t[1].files[0].as_deref(), Some("Act1/Town/Floor.dt1"));
    assert_eq!(t[1].files[1].as_deref(), Some("Act1/Town/Objects.dt1"));
    assert_eq!(t[1].files[2], None);
    assert!(!t[1].expansion);
    // after the separator row: expansion even though the cell is blank
    assert_eq!(t[2].row, 3);
    assert!(t[2].expansion);
    assert_eq!(t[2].files[1], None, "blank slot");
}

#[test]
fn lvl_types_accepts_unspaced_file_columns_and_reports_problems() {
    let t = lvl_types(&table("Id\tFile1\tFile2\tAct\n3\ta.dt1\t0\t2\n")).unwrap();
    assert_eq!(t[0].files[0].as_deref(), Some("a.dt1"));
    assert_eq!(t[0].files[1], None);
    assert_eq!(t[0].name, "");

    assert!(matches!(
        lvl_types(&table("Name\tAct\nx\t1\n")),
        Err(FormatError::MissingColumn { .. })
    ));
    assert!(matches!(
        lvl_types(&table("Id\nabc\n")),
        Err(FormatError::Invalid { .. })
    ));
    assert!(matches!(
        lvl_types(&table("Id\n-4\n")),
        Err(FormatError::Invalid { .. })
    ));
    assert!(lvl_types(&table("Id\tAct\n")).unwrap().is_empty());
}

#[test]
fn lvl_prests_from_txt() {
    let text = "Name\tDef\tLevelId\tPopulate\tLogicals\tOutdoors\tAnimate\tKillEdge\tFillBlanks\tSizeX\tSizeY\tAutoMap\tScan\tPops\tPopPad\tFiles\tFile1\tFile2\tFile3\tFile4\tFile5\tFile6\tDt1Mask\tBeta\tExpansion\r\n\
Null\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\r\n\
Act 1 Town E1\t1\t1\t1\t0\t1\t0\t1\t0\t57\t41\t1\t0\t2\t-3\t2\tAct1/Town/TownE1.ds1\t0\t\t\t\t\t1023\t0\t0\r\n\
Expansion\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\r\n\
Act 5\t900\t109\t\t\t\t\t\t\t30\t30\t\t\t\t\t1\tExpansion/x.ds1\t\t\t\t\t\t4294967295\t0\t1\r\n";
    let p = lvl_prests(&table(text)).unwrap();
    assert_eq!(p.len(), 3);
    let town = &p[1];
    assert_eq!((town.def, town.level_id), (1, 1));
    assert_eq!(town.name, "Act 1 Town E1");
    assert_eq!((town.size_x, town.size_y), (57, 41));
    assert!(town.populate && !town.logicals && town.outdoors && !town.animate);
    assert!(town.kill_edge && !town.fill_blanks && town.auto_map && !town.scan);
    assert_eq!((town.pops, town.pop_pad), (2, -3));
    assert_eq!(town.files.len(), 6);
    assert_eq!(town.files[0].as_deref(), Some("Act1/Town/TownE1.ds1"));
    assert!(town.files[1..].iter().all(Option::is_none));
    assert_eq!(town.dt1_mask, 1023);
    assert!(!town.expansion);
    let exp = &p[2];
    assert_eq!(exp.row, 3);
    assert_eq!(exp.def, 900);
    assert_eq!(exp.dt1_mask, u32::MAX);
    assert!(exp.expansion);
    assert!(!exp.populate, "blank flag reads as false");

    assert!(matches!(
        lvl_prests(&table("Def\tLevelId\n1\t2\n")),
        Err(FormatError::MissingColumn { column, .. }) if column == "Dt1Mask"
    ));
    assert!(matches!(
        lvl_prests(&table("Def\tLevelId\tDt1Mask\n1\t2\t99999999999\n")),
        Err(FormatError::Invalid { .. })
    ));
    // a signed spelling of an all-ones mask
    let p = lvl_prests(&table("Def\tLevelId\tDt1Mask\n1\t2\t-1\n")).unwrap();
    assert_eq!(p[0].dt1_mask, u32::MAX);
}

#[test]
fn levels_from_txt() {
    let text = "Name\tId\tPal\tAct\tLevelName\tLevelType\tDrlgType\r\n\
Null\t0\t0\t0\t\t0\t0\r\n\
Act 1 - Town\t1\t0\t0\tRogue Encampment\t1\t2\r\n\
Expansion\t\t\t\t\t\t\r\n\
Act 5 - Harrogath\t109\t4\t4\t\t34\t1\r\n";
    let l = levels(&table(text)).unwrap();
    assert_eq!(l.len(), 3);
    assert_eq!(l[1].name, "Rogue Encampment");
    assert_eq!((l[1].id, l[1].act, l[1].pal), (1, 0, 0));
    assert_eq!((l[1].level_type, l[1].drlg_type), (1, 2));
    assert_eq!(l[2].row, 3);
    assert_eq!(l[2].name, "Act 5 - Harrogath", "falls back to Name");
    assert_eq!((l[2].act, l[2].pal), (4, 4));

    assert!(matches!(
        levels(&table("Id\tAct\n1\t0\n")),
        Err(FormatError::MissingColumn { .. })
    ));
}

fn type_with_files(files: &[Option<&str>]) -> LvlType {
    let mut f: Vec<Option<String>> = files.iter().map(|x| x.map(str::to_string)).collect();
    f.resize(32, None);
    LvlType {
        row: 0,
        id: 1,
        name: String::new(),
        act: 1,
        files: f,
        expansion: false,
    }
}

fn prest_with_mask(mask: u32) -> LvlPrest {
    LvlPrest {
        row: 0,
        def: 1,
        level_id: 1,
        name: String::new(),
        size_x: 0,
        size_y: 0,
        populate: false,
        logicals: false,
        outdoors: false,
        animate: false,
        kill_edge: false,
        fill_blanks: false,
        auto_map: false,
        scan: false,
        pops: 0,
        pop_pad: 0,
        files: vec![None; 6],
        dt1_mask: mask,
        expansion: false,
    }
}

#[test]
fn dt1_files_follow_the_mask_in_slot_order() {
    let lt = type_with_files(&[Some("a"), Some("b"), Some("c"), None, Some("e")]);
    let pick = |mask: u32| -> Vec<&str> { prest_with_mask(mask).dt1_files(&lt).collect() };
    assert_eq!(pick(0), Vec::<&str>::new());
    assert_eq!(pick(0b1), ["a"]);
    assert_eq!(pick(0b101), ["a", "c"]);
    assert_eq!(
        pick(0b11111),
        ["a", "b", "c", "e"],
        "empty slot 4 yields nothing"
    );
    assert_eq!(pick(0b10), ["b"]);
    assert_eq!(pick(u32::MAX), ["a", "b", "c", "e"]);

    let mut full = type_with_files(&[]);
    full.files[31] = Some("last".to_string());
    full.files[0] = Some("first".to_string());
    assert_eq!(
        prest_with_mask(1 << 31)
            .dt1_files(&full)
            .collect::<Vec<_>>(),
        ["last"]
    );
    // fewer than 32 slots must not panic
    let mut short = type_with_files(&[Some("a")]);
    short.files.truncate(1);
    assert_eq!(prest_with_mask(u32::MAX).dt1_files(&short).count(), 1);
}

#[test]
fn bin_decoded_tables_parse_like_txt() {
    // build one record of each kind by hand and run it through the decoder
    let mut types = (1u32).to_le_bytes().to_vec();
    let mut rec = vec![0u8; excel::record_size(ExcelTableKind::LvlTypes, ExcelVersion::V113c)];
    rec[..4].copy_from_slice(b"a.dt");
    rec[60..64].copy_from_slice(b"b.dt");
    rec[0x780..0x784].copy_from_slice(&3u32.to_le_bytes());
    rec[0x784..0x788].copy_from_slice(&1u32.to_le_bytes());
    types.extend_from_slice(&rec);
    let t = excel::decode_bin(ExcelTableKind::LvlTypes, ExcelVersion::V113c, &types).unwrap();
    let parsed = lvl_types(&t).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].files[0].as_deref(), Some("a.dt"));
    assert_eq!(parsed[0].files[1].as_deref(), Some("b.dt"));
    assert_eq!(
        (parsed[0].act, parsed[0].expansion, parsed[0].name.as_str()),
        (3, true, "")
    );
}

// ---------------------------------------------------------------------------
// corpus

fn read(path: &std::path::Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

#[test]
fn vanilla_txt_tables_parse() {
    let Some(dir) = crate::testutil::extracted("vanilla") else {
        println!("skipped: extracted/vanilla missing");
        return;
    };
    let excel_dir = dir.join("data").join("global").join("excel");
    let types_txt = TxtTable::parse(&read(&excel_dir.join("lvltypes.txt")).unwrap()).unwrap();
    let prest_txt = TxtTable::parse(&read(&excel_dir.join("lvlprest.txt")).unwrap()).unwrap();
    let levels_txt = TxtTable::parse(&read(&excel_dir.join("levels.txt")).unwrap()).unwrap();
    let types = lvl_types(&types_txt).unwrap();
    let prests = lvl_prests(&prest_txt).unwrap();
    let lvls = levels(&levels_txt).unwrap();
    println!(
        "vanilla txt: {} lvltypes, {} lvlprest, {} levels",
        types.len(),
        prests.len(),
        lvls.len()
    );
    assert_eq!(types.len(), 36);
    assert_eq!(prests.len(), 1091);
    assert_eq!(lvls.len(), 137);
    assert!(types.iter().filter(|t| t.expansion).count() >= 7);
    assert_eq!(lvls[1].name, "Rogue Encampment");

    // Walk every preset through Levels -> LvlTypes and select its DT1 files.
    let by_level: std::collections::HashMap<u32, &Level> = lvls.iter().map(|l| (l.id, l)).collect();
    let by_type: std::collections::HashMap<u32, &LvlType> =
        types.iter().map(|t| (t.id, t)).collect();
    let (mut resolved, mut selected, mut unresolved_levels, mut empty_slots) = (0, 0, 0, 0);
    for p in &prests {
        if p.level_id <= 0 {
            continue;
        }
        let Some(level) = by_level.get(&(p.level_id as u32)) else {
            unresolved_levels += 1;
            continue;
        };
        let Some(lt) = by_type.get(&level.level_type) else {
            unresolved_levels += 1;
            continue;
        };
        resolved += 1;
        let files: Vec<&str> = p.dt1_files(lt).collect();
        selected += files.len();
        empty_slots += (0..32)
            .filter(|&i| p.dt1_mask & (1 << i) != 0 && lt.files[i].is_none())
            .count();
        // the selection is exactly the set bits that have a file
        let expected = (0..32)
            .filter(|&i| p.dt1_mask & (1 << i) != 0 && lt.files[i].is_some())
            .count();
        assert_eq!(files.len(), expected);
    }
    println!(
        "vanilla presets resolved to a LvlType: {resolved} ({selected} dt1 selections, \
         {empty_slots} mask bits on empty slots, {unresolved_levels} unresolved)"
    );
    assert!(resolved >= 30, "only {resolved} presets name a level");

    // The first town preset selects Floor.dt1 and friends under Act1/Town.
    let town = prests.iter().find(|p| p.def == 1).unwrap();
    let level = by_level[&(town.level_id as u32)];
    let files: Vec<&str> = town.dt1_files(by_type[&level.level_type]).collect();
    assert!(!files.is_empty());
    // townE1.ds1's own file list names the same Town/Outdoors tilesets
    assert!(
        files
            .iter()
            .any(|f| f.eq_ignore_ascii_case("Act1/Town/Floor.dt1")),
        "{files:?}"
    );
    assert!(files.len() >= 8, "{files:?}");
}

#[test]
fn median_xl_bin_tables_parse() {
    let Some(dir) = crate::testutil::extracted("mxl") else {
        println!("skipped: extracted/mxl missing");
        return;
    };
    let excel_dir = dir.join("data").join("global").join("excel");
    let v = ExcelVersion::V113c;
    let bin = |name: &str| read(&excel_dir.join(name)).unwrap();
    let types =
        lvl_types(&excel::decode_bin(ExcelTableKind::LvlTypes, v, &bin("lvltypes.bin")).unwrap())
            .unwrap();
    let prests =
        lvl_prests(&excel::decode_bin(ExcelTableKind::LvlPrest, v, &bin("lvlprest.bin")).unwrap())
            .unwrap();
    let lvls =
        levels(&excel::decode_levels_bin(v, &bin("levels.bin"), &bin("leveldefs.bin")).unwrap())
            .unwrap();
    println!(
        "mxl bin: {} lvltypes, {} lvlprest, {} levels",
        types.len(),
        prests.len(),
        lvls.len()
    );
    assert_eq!((types.len(), prests.len(), lvls.len()), (52, 1212, 280));
    assert!(matches!(
        levels(&excel::decode_bin(ExcelTableKind::Levels, v, &bin("levels.bin")).unwrap()),
        Err(FormatError::MissingColumn { .. })
    ));
    let by_level: std::collections::HashMap<u32, &Level> = lvls.iter().map(|l| (l.id, l)).collect();
    let by_type: std::collections::HashMap<u32, &LvlType> =
        types.iter().map(|t| (t.id, t)).collect();
    let mut resolved = 0;
    let mut unresolved = Vec::new();
    for p in &prests {
        if p.level_id <= 0 {
            continue;
        }
        let lt = by_level
            .get(&(p.level_id as u32))
            .and_then(|l| by_type.get(&l.level_type));
        match lt {
            Some(lt) => {
                resolved += 1;
                let _ = p.dt1_files(lt).count();
            }
            None => unresolved.push((p.def, p.level_id)),
        }
    }
    println!(
        "mxl presets resolved to a LvlType: {resolved}, unresolved: {}",
        unresolved.len()
    );
    assert!(unresolved.is_empty(), "{unresolved:?}");
}
