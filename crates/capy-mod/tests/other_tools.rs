//! Cube recipes, entering text and custom objects: synthetic cases translated
//! from the reference editor's tests, plus the vanilla tables where present.

mod common;

use std::collections::{HashMap, HashSet};

use capy_mod::cube::{
    CubeItemInput, CubeTables, MapTarget, RecipeTables, TemplateRisk, find_map_recipes,
    map_item_level, normalize_input, plan_cube_item, remove_rows, same_recipe, tagged_items,
    tagged_recipes, template_risk,
};
use capy_mod::custom_object::{
    CUSTOM_MARK, CustomObjectOptions, custom_object_files, custom_object_row, free_token,
    game_rows, guess_frames, object_slots, preset_row, split_strip,
};
use capy_mod::entry_text::{
    EntryImage, entry_dc6, entry_frames, levels_using_entry_file, render_entry_text, set_area_name,
    set_entry_file, suggest_entry_name,
};
use common::{TestResult, corpus, old_rows_unchanged, read_doc, read_game_file};
use d2_formats::dc6::Dc6;
use d2_formats::tbl::TblDoc;
use d2_formats::txt::TxtDoc;

fn cell(doc: &TxtDoc, r: usize, col: &str) -> String {
    doc.column(col)
        .and_then(|c| doc.cell(r, c))
        .unwrap_or_default()
}

fn row_of(doc: &TxtDoc, col: &str, v: &str) -> Option<usize> {
    (0..doc.row_count()).find(|&r| cell(doc, r, col) == v)
}

// ------------------------------------------------------------ cube recipes

#[test]
fn cube_helpers() {
    assert_eq!(normalize_input("\"Gem4, qty=3\""), "gem4,qty=3");
    assert_eq!(
        normalize_input("pk1,sock,qty=2"),
        normalize_input("pk1,qty=2,sock")
    );
    assert!(matches!(
        template_risk("elix", ""),
        Some(TemplateRisk::Blocked(_))
    ));
    assert!(matches!(
        template_risk("t1m", ""),
        Some(TemplateRisk::ModType(_))
    ));
    assert_eq!(template_risk("key", ""), None);
}

#[test]
fn cube_item_against_the_vanilla_tables() -> TestResult {
    let Some(root) = corpus("vanilla") else {
        return Ok(());
    };
    let misc = read_doc(&root, "misc.txt")?;
    let cube = read_doc(&root, "cubemain.txt")?;
    let key = row_of(&misc, "code", "key").ok_or("key row")?;
    let tables = CubeTables {
        misc: misc.clone(),
        cube: cube.clone(),
        levels: None,
        strings: None,
    };
    let input = CubeItemInput {
        template_row: key,
        name: "Guild Key".into(),
        code: "gk01".into(),
        inputs: vec!["key".into(), "isc".into(), "hp1".into()],
        numinputs: 3,
        map_name: "guild3".into(),
        accept_mod_type: false,
        map: None,
    };
    let out = plan_cube_item(&tables, &input)?;
    // Existing rows stay where they were; the item and recipe are appended and tagged.
    assert!(old_rows_unchanged(&misc, &out.tables.misc));
    assert!(old_rows_unchanged(&cube, &out.tables.cube));
    let n = row_of(&out.tables.misc, "code", "gk01").ok_or("new item")?;
    assert!(n >= misc.row_count().saturating_sub(2));
    assert_eq!(cell(&out.tables.misc, n, "name"), "Guild Key");
    assert_eq!(cell(&out.tables.misc, n, "spawnable"), "0");
    assert_eq!(cell(&out.tables.misc, n, "type"), "key");
    assert_ne!(cell(&out.tables.misc, n, "normcode"), "key");
    let cr = row_of(&out.tables.cube, "output", "gk01").ok_or("new recipe")?;
    let got: Vec<String> = ["input 1", "input 2", "input 3", "numinputs", "enabled"]
        .iter()
        .map(|c| cell(&out.tables.cube, cr, c))
        .collect();
    assert_eq!(got, ["key", "isc", "hp1", "3", "1"]);
    let codes: Vec<String> = tagged_items(&out.tables.misc)
        .into_iter()
        .map(|i| i.2)
        .collect();
    assert_eq!(codes, ["gk01"]);
    assert_eq!(tagged_recipes(&out.tables.cube).len(), 1);
    // Removing the recipe gives back the original table.
    let back = remove_rows(&out.tables.cube, &[cr])?;
    assert_eq!(back.to_bytes(), cube.to_bytes());
    // The same code again is refused, and so is a recipe the cube would never run.
    let again = plan_cube_item(
        &out.tables,
        &CubeItemInput {
            name: "Other".into(),
            inputs: vec!["key".into(), "isc".into(), "hp2".into()],
            ..input.clone()
        },
    );
    assert!(again.is_err());

    let r = (0..cube.row_count())
        .find(|&i| cell(&cube, i, "enabled") == "1" && !cell(&cube, i, "input 2").is_empty())
        .ok_or("enabled multi-input recipe")?;
    let inputs: Vec<String> = ["input 2", "input 1", "input 3", "input 4"]
        .iter()
        .map(|c| cell(&cube, r, c))
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        same_recipe(&cube, &inputs),
        Some(cell(&cube, r, "description"))
    );
    let clash = plan_cube_item(
        &tables,
        &CubeItemInput {
            code: "gk02".into(),
            numinputs: inputs.len() as u32,
            inputs,
            ..input.clone()
        },
    );
    assert!(clash.err().is_some_and(|e| {
        e.to_string()
            .contains("already uses exactly these ingredients")
    }));

    // Elixirs are never copied; a mod item type needs confirmation.
    if let Some(elx) = row_of(&misc, "code", "elx") {
        let e = plan_cube_item(
            &tables,
            &CubeItemInput {
                template_row: elx,
                code: "gk03".into(),
                inputs: vec!["key".into(), "hp2".into()],
                ..input.clone()
            },
        );
        assert!(e.err().is_some_and(|e| e.to_string().contains("Elixirs")));
    }
    let mut modded = misc.clone();
    let tcol = modded.column("type").ok_or("type col")?;
    modded.set_cell(key, tcol, "t1m")?;
    let mt = CubeTables {
        misc: modded,
        cube: cube.clone(),
        levels: None,
        strings: None,
    };
    let ask = CubeItemInput {
        code: "gk04".into(),
        inputs: vec!["key".into(), "hp3".into()],
        ..input.clone()
    };
    assert!(plan_cube_item(&mt, &ask).is_err());
    assert!(
        plan_cube_item(
            &mt,
            &CubeItemInput {
                accept_mod_type: true,
                ..ask
            }
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn map_item_opens_the_level_and_recipes_are_found() -> TestResult {
    let Some(root) = corpus("vanilla") else {
        return Ok(());
    };
    let mut misc = read_doc(&root, "misc.txt")?;
    let mut cube = read_doc(&root, "cubemain.txt")?;
    let levels = read_doc(&root, "levels.txt")?;
    let prest = read_doc(&root, "lvlprest.txt")?;
    let strings = TblDoc::parse(&read_game_file(
        &root,
        r"data\local\lng\eng\patchstring.tbl",
    )?)?;
    let key = row_of(&misc, "code", "key").ok_or("key row")?;
    // A PD2-style map item: a mod type, pSpell 12, len = the level it opens (38), and a recipe making it.
    for (c, v) in [("type", "t1m"), ("pSpell", "12"), ("len", "38")] {
        let col = misc.column(c).ok_or(c)?;
        misc.set_cell(key, col, v)?;
    }
    let mut row: Vec<String> = vec![String::new(); cube.headers().len()];
    for (c, v) in [
        ("description", "ID scroll -> Tristram Map"),
        ("enabled", "1"),
        ("version", "100"),
        ("numinputs", "1"),
        ("input 1", "isc,qty=1"),
        ("output", "key,nor"),
        ("lvl", "99"),
        ("ilvl", "100"),
    ] {
        row[cube.column(c).ok_or(c)?] = v.into();
    }
    let refs: Vec<&str> = row.iter().map(String::as_str).collect();
    cube.insert_row(cube.row_count(), &refs)?;
    assert_eq!(map_item_level(&misc, key, Some(&levels)), Some(38));

    let tables = CubeTables {
        misc: misc.clone(),
        cube: cube.clone(),
        levels: Some(levels.clone()),
        strings: Some(strings.clone()),
    };
    let input = CubeItemInput {
        template_row: key,
        name: "Guild Hall Map".into(),
        code: "gh01".into(),
        inputs: vec!["tbk".into(), "hp1".into()],
        numinputs: 2,
        map_name: "guild3".into(),
        accept_mod_type: false,
        map: Some(MapTarget {
            level_id: 25,
            level_title: Some("Guild Hall".into()),
        }),
    };
    let out = plan_cube_item(&tables, &input)?;
    let n = row_of(&out.tables.misc, "code", "gh01").ok_or("item")?;
    let got: Vec<String> = ["len", "pSpell", "type", "namestr"]
        .iter()
        .map(|c| cell(&out.tables.misc, n, c))
        .collect();
    assert_eq!(got, ["25", "12", "t1m", "capy_gh01"]);
    let cr = (0..out.tables.cube.row_count())
        .find(|&i| cell(&out.tables.cube, i, "output").starts_with("gh01"))
        .ok_or("recipe")?;
    let got: Vec<String> = ["output", "lvl", "ilvl"]
        .iter()
        .map(|c| cell(&out.tables.cube, cr, c))
        .collect();
    assert_eq!(got, ["gh01,nor", "99", "100"]);
    let lv = out.tables.levels.as_ref().ok_or("levels")?;
    let lr = row_of(lv, "Id", "25").ok_or("level 25")?;
    assert_eq!(cell(lv, lr, "LevelName"), "capy_lvl_25");
    assert_eq!(cell(lv, lr, "LevelWarp"), "capy_lvl_25");
    let tbl = out.tables.strings.as_ref().ok_or("strings")?;
    assert_eq!(tbl.get("capy_gh01"), Some("Guild Hall Map"));
    assert_eq!(tbl.get("capy_lvl_25"), Some("Guild Hall"));
    // Without the map being a level in the game, a map item asks for it first.
    let no_map = plan_cube_item(
        &tables,
        &CubeItemInput {
            map: None,
            code: "gh02".into(),
            inputs: vec!["tbk".into(), "hp2".into()],
            ..input
        },
    );
    assert!(
        no_map
            .err()
            .is_some_and(|e| e.to_string().contains("Add it to the game first"))
    );

    // Which recipes make the items of a map: the level 38 map item built above.
    let (prest_row, ds1) = (0..prest.row_count())
        .find_map(|r| (cell(&prest, r, "LevelId") == "38").then(|| (r, cell(&prest, r, "File1"))))
        .ok_or("level 38 preset")?;
    let _ = prest_row;
    let found = find_map_recipes(
        &RecipeTables {
            prest: &prest,
            levels: &levels,
            misc: &misc,
            cube: &cube,
            weapons: None,
            armor: None,
            types: None,
        },
        &ds1,
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].description, "ID scroll -> Tristram Map");
    assert_eq!(found[0].inputs[0].code, "isc");
    assert_eq!(found[0].inputs[0].quantity, 1);
    assert_eq!(found[0].outputs[0].code, "key");
    Ok(())
}

// ------------------------------------------------------------ entering text

#[test]
fn entry_names_and_frames() -> TestResult {
    assert_eq!(
        suggest_entry_name("Na-Krul's Abyss", |_| false),
        "nakrulsaby"
    );
    assert_eq!(suggest_entry_name("guild5", |n| n == "guild5"), "guild52");
    assert_eq!(suggest_entry_name("!!!", |_| false), "entry");

    // A 300-pixel line is cut 256 + 44, and the DC6 reads back to the same pixels.
    let (w, h) = (300u32, 5u32);
    let pixels: Vec<u8> = (0..w * h).map(|i| (i % 7) as u8).collect();
    let img = EntryImage {
        width: w,
        height: h,
        pixels: pixels.clone(),
        missing: vec![],
    };
    let frames = entry_frames(&img);
    assert_eq!(
        frames.iter().map(|f| f.width).collect::<Vec<_>>(),
        [256, 44]
    );
    let dc6 = Dc6::parse(&entry_dc6(&img)?.to_bytes()?)?;
    let mut back = vec![0u8; (w * h) as usize];
    let mut x0 = 0;
    for f in &dc6.frames {
        let r = f.render()?;
        for y in 0..r.height {
            for x in 0..r.width {
                back[(y * w + x0 + x) as usize] = r.pixels[(y * r.width + x) as usize];
            }
        }
        x0 += r.width;
    }
    assert_eq!(back, pixels);

    // Glyphs are placed by their advance widths; unknown characters are reported.
    let glyph = |c: char| {
        (c == 'a').then(|| {
            let mut im = d2_formats::IndexedImage::new(2, 2, 0, 0);
            im.pixels.fill(9);
            im.alpha.fill(255);
            capy_mod::entry_text::Glyph {
                advance: 3,
                image: im,
            }
        })
    };
    let line = render_entry_text(2, glyph, "aa?a");
    assert_eq!(line.width, 9);
    assert_eq!(line.missing, ['?']);
    assert_eq!(line.pixels[..9], [9, 9, 0, 9, 9, 0, 9, 9, 0]);
    Ok(())
}

#[test]
fn area_name_in_the_tbl() -> TestResult {
    let Some(root) = corpus("vanilla") else {
        return Ok(());
    };
    let levels = read_doc(&root, "levels.txt")?;
    let strings = TblDoc::parse(&read_game_file(
        &root,
        r"data\local\lng\eng\patchstring.tbl",
    )?)?;
    // A level of its own key: the key it already names is edited.
    let id = 38u32;
    let out = set_area_name(&levels, &strings, id, "Tristram Ruins", None)?;
    assert_eq!(out.strings.get(&out.key), Some("Tristram Ruins"));
    let changed = (0..levels.row_count())
        .filter(|&r| levels.row_cells(r) != out.levels.row_cells(r))
        .count();
    assert!(changed <= 1, "only the level's own row may change");
    // Again with the same text: nothing changes.
    let twice = set_area_name(
        &out.levels,
        &out.strings,
        id,
        "Tristram Ruins",
        Some(&out.key),
    )?;
    assert!(twice.changes.is_empty(), "{:?}", twice.changes);
    assert_eq!(twice.levels, out.levels);
    assert_eq!(twice.strings, out.strings);
    // An explicit key.
    let own = set_area_name(&levels, &strings, id, "Mine", Some("capy_test_key"))?;
    assert_eq!(own.strings.get("capy_test_key"), Some("Mine"));
    let row = row_of(&own.levels, "Id", &id.to_string()).ok_or("row")?;
    assert_eq!(cell(&own.levels, row, "LevelName"), "capy_test_key");
    assert_eq!(cell(&own.levels, row, "LevelWarp"), "capy_test_key");
    // Written tbl parses back with the string.
    let bytes = own.strings.to_bytes()?;
    assert_eq!(TblDoc::parse(&bytes)?.get("capy_test_key"), Some("Mine"));
    assert!(set_area_name(&levels, &strings, id, "  ", None).is_err());
    assert!(set_area_name(&levels, &strings, 99999, "x", None).is_err());

    let mut lv = levels.clone();
    let path = set_entry_file(&mut lv, id, "tristram2")?;
    assert_eq!(path, "data/local/ui/eng/expansion/tristram2.dc6");
    assert_eq!(levels_using_entry_file(&lv, "TRISTRAM2"), [id]);
    assert!(set_entry_file(&mut lv, id, "bad name").is_err());
    Ok(())
}

// ------------------------------------------------------------ custom objects

const OBJECTS: &str = "Name\tdescription - not loaded\tId\tToken\tFrameCnt0\tFrameDelta0\tLit0\tMode0\tOperateFn\tTR\tSelectable0\tExtra\r\n\
dummy\ttest\t0\t\t0\t0\t0\t0\t0\t0\t0\tkeep\r\n\
Barrel\tBarrel\t1\tB1\t1\t256\t0\t1\t5\t1\t1\tkeep\r\n\
Expansion\r\n\
Portal\tportal\t2\tPP\t1\t256\t0\t1\t0\t1\t1\tkeep\r\n\
Unused\tunused thing\t3\tUU\t1\t256\t0\t1\t0\t1\t0\tkeep\r\n";

fn opts() -> CustomObjectOptions {
    CustomObjectOptions {
        name: "Lantern".into(),
        token: "zz".into(),
        frames: 4,
        speed: 128,
        light: 6,
        light_colour: [255, 200, 120],
        flicker: true,
        blocks: true,
        size: 2,
        draw_under: false,
        act: 0,
    }
}

fn presets(n: usize) -> Vec<Vec<i32>> {
    vec![
        (0..150)
            .map(|i| if i < n { i as i32 } else { -1 })
            .collect(),
    ]
}

#[test]
fn custom_object_table_side() -> TestResult {
    let doc = TxtDoc::parse(OBJECTS.as_bytes());
    // The Expansion line does not count.
    assert_eq!(game_rows(&doc), [0, 1, 3, 4]);
    // 150+ is the objects.txt row; negative ids reach into earlier acts.
    let p = vec![
        (0..150).collect::<Vec<i32>>(),
        (0..150).map(|i| 1000 + i).collect(),
    ];
    assert_eq!(preset_row(&p, 0, 5), 5);
    assert_eq!(preset_row(&p, 0, 155), 5);
    assert_eq!(preset_row(&p, 1, -145), 5);

    // Only rows nothing uses are free.
    let slots = object_slots(
        &presets(4),
        &doc,
        0,
        &HashMap::from([(1, 3)]),
        &HashSet::from([2]),
    );
    let view: Vec<_> = slots.iter().map(|s| (s.id, s.row, s.free)).collect();
    assert_eq!(view, [(1, 1, false), (2, 2, false), (3, 3, true)]);
    assert!(slots[0].functions && slots[0].uses == 3);
    assert!(slots[1].in_group);

    // Rewritten as a plain decoration; unknown columns and other rows stay.
    let out = custom_object_row(&doc, 3, &opts())?;
    let line = game_rows(&out.objects)[3];
    assert_eq!(cell(&out.objects, line, "Name"), "Lantern");
    assert_eq!(
        cell(&out.objects, line, "description - not loaded"),
        format!("{CUSTOM_MARK}Lantern")
    );
    assert_eq!(cell(&out.objects, line, "Token"), "zz");
    assert_eq!(cell(&out.objects, line, "FrameCnt0"), "4");
    assert_eq!(cell(&out.objects, line, "OperateFn"), "0");
    assert_eq!(cell(&out.objects, line, "Selectable0"), "0");
    assert_eq!(cell(&out.objects, line, "Extra"), "keep");
    let (a, b) = (doc.to_bytes(), out.objects.to_bytes());
    let (a, b) = (String::from_utf8(a)?, String::from_utf8(b)?);
    for (i, (x, y)) in a.split("\r\n").zip(b.split("\r\n")).enumerate() {
        if i != 5 {
            assert_eq!(x, y, "line {i}");
        }
    }
    // A custom row is offered again to edit even when a map uses it; same options = no change.
    let again = custom_object_row(&out.objects, 3, &opts())?;
    assert!(again.changes.is_empty());
    let s = object_slots(
        &presets(4),
        &out.objects,
        0,
        &HashMap::from([(3, 2)]),
        &HashSet::new(),
    );
    let mine = s.iter().find(|s| s.row == 3).ok_or("slot")?;
    assert!(mine.custom && mine.free && mine.name == "Lantern");
    assert!(custom_object_row(&doc, 9, &opts()).is_err());
    assert!(
        custom_object_row(
            &doc,
            3,
            &CustomObjectOptions {
                token: "toolong".into(),
                ..opts()
            }
        )
        .is_err()
    );

    // A token no row or folder uses.
    let t = free_token(&doc, &HashSet::from(["zz".to_owned()]))?;
    assert_eq!(t, "zy");
    Ok(())
}

#[test]
fn custom_object_pictures() -> TestResult {
    // Frames of a strip from the gaps between them.
    let strip = |x: u32| x % 8 == 3 || x % 8 == 4;
    assert_eq!(guess_frames(24, 4, |x, _| strip(x)), 3);
    assert_eq!(guess_frames(16, 16, |x, _| x == 3 || x == 12), 2);
    assert_eq!(guess_frames(16, 8, |x, _| x == 3 || x == 12), 2);
    assert_eq!(guess_frames(9, 9, |_, _| true), 1);

    let px: Vec<u8> = (0..12).collect();
    let f = split_strip(6, 2, &px, 3)?;
    let pix: Vec<Vec<u8>> = f.iter().map(|x| x.pixels.clone()).collect();
    assert_eq!(
        pix,
        [vec![0, 1, 6, 7], vec![2, 3, 8, 9], vec![4, 5, 10, 11]]
    );
    assert!(split_strip(7, 2, &[0; 14], 3).is_err());

    // The files: paths, a COF plan, and a DC6 that reads back where the picture stands.
    let frames = split_strip(
        8,
        3,
        &(0..24)
            .map(|i| {
                if i % 8 < 4 {
                    10 + (i % 4) as u8
                } else {
                    20 + (i % 4) as u8
                }
            })
            .collect::<Vec<_>>(),
        2,
    )?;
    let plan = custom_object_files("ZZ", &frames, 2, None)?;
    assert_eq!(plan.cof.path, "data/global/objects/zz/cof/zznuhth.cof");
    assert_eq!(plan.dc6_path, "data/global/objects/zz/tr/zztrlitnuhth.dc6");
    assert_eq!(
        (
            plan.cof.x_min,
            plan.cof.x_max,
            plan.cof.y_min,
            plan.cof.y_max
        ),
        (-2, 2, -1, 2)
    );
    assert_eq!(plan.cof.frames_per_direction, 2);
    let dc6 = Dc6::parse(&plan.dc6)?;
    assert_eq!(dc6.frames.len(), 2);
    for (i, fr) in dc6.frames.iter().enumerate() {
        let r = fr.render()?;
        assert_eq!((r.width, r.height), (4, 3));
        assert_eq!(r.pixels, frames[i].pixels);
        assert_eq!(r.offset_x, -2);
    }
    assert!(custom_object_files("zz", &[], 0, None).is_err());
    Ok(())
}
