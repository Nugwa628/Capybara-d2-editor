//! `capy mcp`: a scripted session edits a vanilla map and the saved file
//! equals the same `capy-edit` calls made directly. Uses the extracted corpus
//! (`CAPY_PRIVATE_DIR`) as a folder layer; skips when it is absent.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::io::Cursor;
use std::path::PathBuf;

use capy_edit::{Brush, CellRect, CellValue, Document, LayerRef, Rng, TileKey};
use d2_assets::profile::LayerSpec;
use d2_assets::{GamePath, GameVersion, Profile};
use d2_formats::ds1::{Cell, Ds1, Object};
use serde_json::{Value, json};

const MAP: &str = "data/global/tiles/act1/outdoors/cott6.ds1";

fn corpus() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\zffit\Capybara-d2-editor\testdata\private"));
    let d = dir.join("extracted").join("vanilla");
    d.is_dir().then_some(d)
}

fn run(home: &std::path::Path, profile: &Profile, calls: &[Value]) -> Vec<Value> {
    let input: String = calls.iter().map(|c| format!("{c}\n")).collect();
    let mut out = Vec::new();
    capy_cli::mcp::serve(
        home.to_path_buf(),
        profile.clone(),
        &mut Cursor::new(input),
        &mut out,
    )
    .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn call(id: u32, name: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}

fn text_of(r: &Value) -> String {
    r["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

#[test]
fn protocol_lists_the_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: "t".into(),
        name: "t".into(),
        game_version: GameVersion::V113c,
        layers: vec![LayerSpec::Saves],
    };
    let replies = run(
        tmp.path(),
        &profile,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":4,"method":"nope"}),
        ],
    );
    assert_eq!(replies.len(), 4, "notifications get no reply");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
    let names: Vec<&str> = replies[2]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for n in [
        "list_maps",
        "open_map",
        "new_map",
        "paint",
        "set_cell",
        "copy_area",
        "find_replace",
        "reroll",
        "resize_map",
        "set_cell_flags",
        "add_object",
        "edit_objects",
        "undo",
        "redo",
        "check_map",
        "render_map",
        "save_map",
        "regions",
        "no_spawn_area",
        "list_tiles",
        "list_placeable",
        "add_layer",
        "clear",
        "history",
        "tile_template",
        "create_tile",
        "import_tile_image",
        "reshape_tile",
        "new_dt1",
        "list_own_tiles",
        "inspect_tile",
    ] {
        assert!(names.contains(&n), "{n}");
    }
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn a_scripted_session_equals_the_same_capy_edit_calls() {
    let Some(root) = corpus() else {
        eprintln!("skipped: corpus not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: "t".into(),
        name: "t".into(),
        game_version: GameVersion::V113c,
        layers: vec![LayerSpec::Saves, LayerSpec::Folder { path: root.clone() }],
    };
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "open_map", json!({"path": MAP})),
            call(
                2,
                "paint",
                json!({"layer":"floor1","tiles":[{"main":3,"sub":1}],"rect":{"x0":1,"y0":1,"x1":2,"y1":3}}),
            ),
            call(
                3,
                "set_cell",
                json!({"layer":"floor1","x":0,"y":0,"hidden":true}),
            ),
            call(
                4,
                "find_replace",
                json!({"layer":"floor","from":{"main":3,"sub":1},"to":{"main":4,"sub":2},"rect":{"x0":0,"y0":0,"x1":1,"y1":9}}),
            ),
            call(5, "add_object", json!({"type":2,"id":5,"x":12,"y":13})),
            call(
                6,
                "copy_area",
                json!({"rect":{"x0":0,"y0":0,"x1":2,"y1":2},"to_x":5,"to_y":5}),
            ),
            call(
                7,
                "set_cell_flags",
                json!({"layer":"floor1","cells":[[4,4],[5,5]],"unwalkable":true}),
            ),
            call(8, "undo", json!({})),
            call(9, "redo", json!({})),
            call(10, "check_map", json!({})),
            call(11, "render_map", json!({"scale":0.25,"objects":false})),
            call(12, "save_map", json!({})),
        ],
    );
    for (i, reply) in r.iter().enumerate() {
        assert!(
            reply["result"]["isError"].is_null(),
            "call {}: {}",
            i + 1,
            text_of(reply)
        );
    }
    let render = &r[10]["result"]["content"];
    assert_eq!(render[1]["type"], "image", "{render}");
    assert!(
        render[1]["data"].as_str().unwrap().starts_with("iVBOR"),
        "PNG base64"
    );
    assert!(
        text_of(&r[9]).contains("Walkable area"),
        "{}",
        text_of(&r[9])
    );

    // The same edits made directly.
    let mut ds1 = Ds1::parse(&std::fs::read(root.join(MAP)).unwrap()).unwrap();
    ds1.objects.shrink_to_fit();
    let mut doc = Document::new(ds1);
    doc.set_clip_context(Some(GamePath::new(MAP).to_string()), None);
    let floor = LayerRef::Floor(0);
    doc.paint_rect(
        floor,
        CellRect {
            x0: 1,
            y0: 1,
            x1: 2,
            y1: 3,
        },
        &[Brush::floor(3, 1)],
        &mut Rng::new(0),
    )
    .unwrap();
    let cur = floor.get(doc.ds1(), 0, 0).unwrap();
    doc.paint(
        floor,
        &[(
            0,
            0,
            CellValue {
                cell: Cell(cur.cell).with_hidden(true).0,
                orientation: cur.orientation,
            },
        )],
    )
    .unwrap();
    let mut sel = capy_edit::Selection::new();
    sel.box_select(
        doc.ds1(),
        CellRect {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 9,
        },
        &[LayerRef::Floor(0)],
        true,
        capy_edit::SelectOp::Replace,
    );
    doc.replace_tiles(
        &[LayerRef::Floor(0)],
        TileKey {
            orientation: 0,
            main: 3,
            sub: 1,
        },
        TileKey {
            orientation: 0,
            main: 4,
            sub: 2,
        },
        Some(&sel),
    )
    .unwrap();
    doc.add_object(Object {
        kind: 2,
        id: 5,
        x: 12,
        y: 13,
        flags: 0,
    })
    .unwrap();
    let mut sel = capy_edit::Selection::new();
    let layers = LayerRef::all_in(doc.ds1());
    sel.box_select(
        doc.ds1(),
        CellRect {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 2,
        },
        &layers,
        false,
        capy_edit::SelectOp::Replace,
    );
    let b = sel.bounds().unwrap();
    let clip = doc.copy(&sel);
    doc.paste(
        &clip,
        (5 + b.x0 as i32, 5 + b.y0 as i32),
        capy_edit::PasteMode::Overwrite,
    )
    .unwrap();
    let mut sel = capy_edit::Selection::new();
    sel.insert(floor, 4, 4);
    sel.insert(floor, 5, 5);
    doc.set_cell_flags(&sel, None, Some(true)).unwrap();
    let expected = doc.ds1().to_bytes().unwrap();

    let saved = tmp
        .path()
        .join("saves")
        .join("t")
        .join("data/global/tiles/act1/outdoors/cott6.ds1");
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        expected,
        "MCP result differs from direct capy-edit calls"
    );
    let changed =
        std::fs::read_to_string(tmp.path().join("saves").join("t").join("CHANGED.txt")).unwrap();
    assert!(changed.contains("cott6.ds1"));

    // A second session sees the saved map first (the saves layer wins) and can undo nothing.
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "open_map", json!({"path": MAP})),
            call(2, "undo", json!({})),
        ],
    );
    assert!(text_of(&r[1]).contains("Nothing to undo"));
}

#[test]
fn errors_are_tool_errors_not_protocol_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: "t".into(),
        name: "t".into(),
        game_version: GameVersion::V113c,
        layers: vec![LayerSpec::Saves],
    };
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "paint", json!({"layer":"floor1"})),
            call(2, "bogus", json!({})),
        ],
    );
    assert_eq!(r[0]["result"]["isError"], true);
    assert!(text_of(&r[0]).contains("No map is open"));
    assert_eq!(r[1]["result"]["isError"], true);
}

#[test]
fn spawn_tile_layer_clear_and_history_tools_in_a_scripted_session() {
    let Some(root) = corpus() else {
        eprintln!("skipped: corpus not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: "t".into(),
        name: "t".into(),
        game_version: GameVersion::V113c,
        layers: vec![LayerSpec::Saves, LayerSpec::Folder { path: root.clone() }],
    };
    let rect = json!({"x0":0,"y0":0,"x1":7,"y1":7});
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "open_map", json!({"path": MAP})),
            call(2, "regions", json!({"rect": rect})),
            call(3, "list_tiles", json!({"kind":"floor","limit":5})),
            call(
                4,
                "list_tiles",
                json!({"kind":"wall","orientation":1,"limit":3}),
            ),
            call(5, "list_placeable", json!({"type":2,"limit":5})),
            call(6, "no_spawn_area", json!({"cells":[[2,2]]})),
            call(7, "regions", json!({"rect": rect, "spawning_only": true})),
            call(8, "add_layer", json!({"kind":"wall"})),
            call(
                9,
                "clear",
                json!({"rect":{"x0":0,"y0":0,"x1":3,"y1":3},"layers":["floor1"],"objects":true}),
            ),
            call(10, "history", json!({})),
            call(11, "undo", json!({"steps":3})),
            call(12, "history", json!({})),
            call(13, "add_layer", json!({"kind":"floor","count":3})),
            call(14, "no_spawn_area", json!({})),
        ],
    );
    let t = |i: usize| text_of(&r[i]);
    assert!(t(1).contains("room") && t(1).contains("seed"), "{}", t(1));
    assert!(t(2).contains("tile"), "{}", t(2));
    assert!(t(3).contains("orientation 1 (left wall)"), "{}", t(3));
    assert!(r[4]["result"]["isError"].is_null(), "{}", t(4));
    assert!(t(5).contains("covered"), "{}", t(5));
    assert!(t(6).contains("room"), "{}", t(6));
    assert!(t(7).contains("added 1 wall layer"), "{}", t(7));
    assert!(t(8).contains("cleared"), "{}", t(8));
    let h = t(9);
    assert!(h.contains("1. ") && h.contains("Clear area"), "{h}");
    assert!(t(10).contains("Undid"), "{}", t(10));
    assert!(t(11).contains("(undone)"), "{}", t(11));
    assert_eq!(r[12]["result"]["isError"], true, "too many floor layers");
    assert_eq!(r[13]["result"]["isError"], true, "no cells given");
}

// ------------------------------------------------------------------ tile creation

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    capy_render::RgbaImage {
        width: w,
        height: h,
        pixels: rgba.repeat((w * h) as usize),
    }
    .to_png()
    .unwrap()
}

/// The `(orientation, main, sub)` of each "Tile oO mM sS:" line.
fn tile_numbers(t: &str) -> Vec<(u32, u32, u32)> {
    t.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("Tile o")?;
            let key = rest.split(':').next()?;
            let mut it = key.split(' ');
            let o = it.next()?.parse().ok()?;
            let m = it.next()?.strip_prefix('m')?.parse().ok()?;
            let s = it.next()?.strip_prefix('s')?.parse().ok()?;
            Some((o, m, s))
        })
        .collect()
}

fn files_named(dir: &std::path::Path, suffix: &str, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files_named(&p, suffix, out);
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(suffix))
        {
            out.push(p);
        }
    }
}

fn ok(r: &[Value], i: usize) -> String {
    assert!(
        r[i]["result"]["isError"].is_null(),
        "call {}: {}",
        i + 1,
        text_of(&r[i])
    );
    text_of(&r[i])
}

fn err(r: &[Value], i: usize, needle: &str) {
    assert_eq!(
        r[i]["result"]["isError"],
        true,
        "call {} should fail: {}",
        i + 1,
        text_of(&r[i])
    );
    assert!(
        text_of(&r[i]).contains(needle),
        "call {}: {:?} not in {}",
        i + 1,
        needle,
        text_of(&r[i])
    );
}

#[test]
fn tile_creation_tools_create_place_and_save() {
    let Some(root) = corpus() else {
        eprintln!("skipped: corpus not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: "t".into(),
        name: "t".into(),
        game_version: GameVersion::V113c,
        layers: vec![LayerSpec::Saves, LayerSpec::Folder { path: root.clone() }],
    };
    // An upper wall 96 high: index 40, its top row index 0 drawn opaque
    // (transparent_index 255), one pixel of 255 (empty).
    let mut wall = vec![40u8; 160 * 96];
    wall[..160].fill(0);
    wall[500] = 255;
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "open_map", json!({"path": MAP})),
            call(2, "tile_template", json!({"orientation":0,"mask":"base64"})),
            call(
                3,
                "tile_template",
                json!({"orientation":3,"height":64,"corner_pair":true,"mask":"png"}),
            ),
            call(4, "create_tile", json!({"orientation":0,"fill":30})),
            call(
                5,
                "create_tile",
                json!({"orientation":1,"height":96,"transparent_index":255,"pixels":{"data":b64(&wall),"width":160,"height":96},"rarity":2}),
            ),
            call(
                6,
                "create_tile",
                json!({"orientation":3,"height":64,"corner_pair":true,"fill":50}),
            ),
            call(
                7,
                "create_tile",
                json!({"orientation":16,"height":32,"fill":60}),
            ),
            call(
                8,
                "import_tile_image",
                json!({"orientation":0,"data":b64(&png(40, 20, [200, 40, 30, 255])),"placement":"fit"}),
            ),
            call(9, "list_own_tiles", json!({})),
            call(10, "list_tiles", json!({"kind":"floor","dt1":"_custom"})),
            call(11, "save_map", json!({})),
        ],
    );
    let t2 = ok(&r, 1);
    assert!(
        t2.contains("160x79") && t2.contains("Mask (160x79 bytes"),
        "{t2}"
    );
    ok(&r, 2);
    assert_eq!(r[2]["result"]["content"][1]["type"], "image");
    let floor = ok(&r, 3);
    let wall_t = ok(&r, 4);
    let corner = ok(&r, 5);
    let lower = ok(&r, 6);
    let imported = ok(&r, 7);
    assert!(
        floor.contains("LvlTypes") || floor.contains("appended"),
        "{floor}"
    );
    assert!(
        floor.contains("Files written") && floor.contains("_custom.dt1"),
        "{floor}"
    );
    assert!(
        wall_t.contains("palette index 0") && wall_t.contains("Warnings"),
        "{wall_t}"
    );
    assert!(imported.contains("Imported a 40x20 picture"), "{imported}");
    let (f, w, c, l, im) = (
        tile_numbers(&floor),
        tile_numbers(&wall_t),
        tile_numbers(&corner),
        tile_numbers(&lower),
        tile_numbers(&imported),
    );
    assert_eq!(
        (f.len(), w.len(), c.len(), l.len(), im.len()),
        (1, 1, 2, 1, 1)
    );
    assert_eq!((f[0].0, w[0].0, l[0].0), (0, 1, 16));
    let corner_o: Vec<u32> = c.iter().map(|k| k.0).collect();
    assert!(corner_o.contains(&3) && corner_o.contains(&4), "{corner}");
    assert_eq!(
        (c[0].1, c[0].2),
        (c[1].1, c[1].2),
        "corner halves share a number"
    );
    assert_ne!(
        (f[0].1, f[0].2),
        (im[0].1, im[0].2),
        "two floors, two numbers"
    );
    let own = ok(&r, 8);
    assert!(
        own.contains(": 6 tile(s)") && own.contains("the open map loads it"),
        "{own}"
    );
    let lt = ok(&r, 9);
    assert!(lt.contains(&format!("{}/{} ", f[0].1, f[0].2)), "{lt}");
    ok(&r, 10);

    // The own DT1 parses and writes back byte for byte; it is registered.
    let saves = tmp.path().join("saves").join("t");
    let mut own_files = Vec::new();
    files_named(&saves, "_custom.dt1", &mut own_files);
    assert_eq!(own_files.len(), 1, "{own_files:?}");
    let bytes = std::fs::read(&own_files[0]).unwrap();
    let dt1 = d2_formats::dt1::Dt1::parse(&bytes).unwrap();
    assert_eq!(dt1.tiles.len(), 6);
    assert_eq!(dt1.to_bytes().unwrap(), bytes, "byte-identical write-back");
    let changed = std::fs::read_to_string(saves.join("CHANGED.txt")).unwrap();
    for n in ["_custom.dt1", "lvltypes.bin", "lvlprest.bin", "cott6.ds1"] {
        assert!(changed.contains(n), "{n} not in CHANGED.txt: {changed}");
    }
    let rel = own_files[0]
        .strip_prefix(&saves)
        .unwrap()
        .to_string_lossy()
        .replace('/', "\\");
    let own_game = GamePath::new(&rel);
    let overlay = profile.open(&tmp.path().join("saves")).unwrap();
    let (catalog, _) =
        d2_assets::MapCatalog::build(&overlay, GameVersion::V113c, &[GamePath::new(MAP)]).unwrap();
    let info = catalog.best(&GamePath::new(MAP)).unwrap();
    assert!(
        info.dt1_files.contains(&own_game),
        "LvlTypes/LvlPrest register {own_game}: {:?}",
        info.dt1_files
    );
    let saved_map = Ds1::parse(&std::fs::read(saves.join(MAP)).unwrap()).unwrap();
    let tg1 = own_game.as_str().replace(".dt1", ".tg1");
    assert!(
        saved_map.files.iter().any(|f| String::from_utf8_lossy(f)
            .to_ascii_lowercase()
            .ends_with(&tg1)),
        "the DS1 lists the own DT1"
    );

    // A new session places the tiles, saves, inspects, reshapes, and makes a library.
    let (fm, fs) = (f[0].1, f[0].2);
    let (wm, ws) = (w[0].1, w[0].2);
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "open_map", json!({"path": MAP})),
            call(
                2,
                "list_tiles",
                json!({"kind":"wall","orientation":1,"dt1":"_custom"}),
            ),
            call(
                3,
                "paint",
                json!({"layer":"floor1","tiles":[{"main":fm,"sub":fs}],"cells":[[1,1]]}),
            ),
            call(
                4,
                "paint",
                json!({"layer":"wall1","tiles":[{"main":wm,"sub":ws,"orientation":1}],"cells":[[2,2]]}),
            ),
            call(
                5,
                "render_map",
                json!({"rect":{"x0":0,"y0":0,"x1":3,"y1":3}}),
            ),
            call(6, "save_map", json!({})),
            call(
                7,
                "inspect_tile",
                json!({"orientation":0,"main":fm,"sub":fs}),
            ),
            call(
                8,
                "inspect_tile",
                json!({"orientation":1,"main":wm,"sub":ws,"render":"file","name":"wall_test"}),
            ),
            call(
                9,
                "reshape_tile",
                json!({"orientation":1,"main":wm,"sub":ws,"new_height":160}),
            ),
            call(
                10,
                "new_dt1",
                json!({"name":"mylib","folder":"act1/outdoors","register":true}),
            ),
            call(
                11,
                "create_tile",
                json!({"orientation":0,"fill":12,"dt1":"act1/outdoors/mylib.dt1"}),
            ),
            call(12, "list_tiles", json!({"kind":"floor","dt1":"mylib"})),
            call(
                13,
                "list_own_tiles",
                json!({"dt1":"act1/outdoors/mylib.dt1"}),
            ),
            call(14, "check_map", json!({})),
        ],
    );
    let lt = ok(&r, 1);
    assert!(lt.contains(&format!("{wm}/{ws} orientation 1")), "{lt}");
    ok(&r, 2);
    ok(&r, 3);
    ok(&r, 4);
    ok(&r, 5);
    let ins = ok(&r, 6);
    assert!(
        ins.contains("Sub-tile flags") && ins.contains("Picture: 160x79"),
        "{ins}"
    );
    assert_eq!(r[6]["result"]["content"][1]["type"], "image");
    let ins = ok(&r, 7);
    assert!(ins.contains("Wrote"), "{ins}");
    assert!(tmp.path().join("renders").join("wall_test.png").is_file());
    let re = ok(&r, 8);
    assert!(
        re.contains("Reshaped") && re.contains("160 px tall"),
        "{re}"
    );
    let nd = ok(&r, 9);
    assert!(nd.contains("mylib.dt1") && nd.contains("Dt1Mask"), "{nd}");
    let lib = ok(&r, 10);
    let n = tile_numbers(&lib);
    assert_eq!(n.len(), 1, "{lib}");
    let lt = ok(&r, 11);
    assert!(
        lt.contains("mylib.dt1"),
        "the registered library loads: {lt}"
    );
    assert!(
        !lt.contains("variants"),
        "a tile added to a loaded library avoids the map's other numbers: {lt}"
    );
    let own = ok(&r, 12);
    assert!(
        own.contains(": 1 tile(s)") && own.contains("writable"),
        "{own}"
    );
    ok(&r, 13);

    let saved_map = Ds1::parse(&std::fs::read(saves.join(MAP)).unwrap()).unwrap();
    let c = saved_map.floors[0].get(1, 1).unwrap();
    assert_eq!(
        (u32::from(c.main_index()), u32::from(c.sub_index())),
        (fm, fs),
        "the new floor is placed and saved"
    );
    let dt1 = d2_formats::dt1::Dt1::parse(&std::fs::read(&own_files[0]).unwrap()).unwrap();
    let reshaped = dt1
        .tiles
        .iter()
        .find(|t| t.orientation == 1 && t.main_index == wm as i32)
        .unwrap();
    assert_eq!(
        reshaped.height,
        -(160 + 32),
        "stored height of a 160 px wall"
    );
    let mut libs = Vec::new();
    files_named(&saves, "mylib.dt1", &mut libs);
    let lb = std::fs::read(&libs[0]).unwrap();
    assert_eq!(
        d2_formats::dt1::Dt1::parse(&lb)
            .unwrap()
            .to_bytes()
            .unwrap(),
        lb
    );
}

#[test]
fn tile_creation_errors() {
    let Some(root) = corpus() else {
        eprintln!("skipped: corpus not found");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: "t".into(),
        name: "t".into(),
        game_version: GameVersion::V113c,
        layers: vec![LayerSpec::Saves, LayerSpec::Folder { path: root.clone() }],
    };
    let big = png(5000, 2, [255, 255, 255, 255]);
    let r = run(
        tmp.path(),
        &profile,
        &[
            call(1, "create_tile", json!({"orientation":0,"fill":3})),
            call(2, "open_map", json!({"path": MAP})),
            call(3, "create_tile", json!({"orientation":25})),
            call(4, "create_tile", json!({"orientation":1})),
            call(5, "create_tile", json!({"orientation":1,"height":5000})),
            call(
                6,
                "create_tile",
                json!({"orientation":1,"height":32,"corner_pair":true}),
            ),
            call(
                7,
                "create_tile",
                json!({"orientation":0,"pixels":{"data":"not base64!","width":2,"height":2}}),
            ),
            call(
                8,
                "create_tile",
                json!({"orientation":0,"pixels":{"data":b64(&[1, 2, 3]),"width":2,"height":2}}),
            ),
            call(
                9,
                "create_tile",
                json!({"orientation":0,"pixels":{"data":"AAAA","width":5000,"height":1}}),
            ),
            call(
                10,
                "import_tile_image",
                json!({"orientation":0,"data":b64(&big)}),
            ),
            call(
                11,
                "import_tile_image",
                json!({"orientation":0,"data":b64(b"hello there"),"format":"png"}),
            ),
            call(
                12,
                "new_dt1",
                json!({"name":"../evil","folder":"act1/outdoors"}),
            ),
            call(
                13,
                "new_dt1",
                json!({"name":"con","folder":"act1/outdoors"}),
            ),
            call(14, "new_dt1", json!({"name":"ok","folder":"../../windows"})),
            call(
                15,
                "create_tile",
                json!({"orientation":0,"fill":3,"dt1":"../../x.dt1"}),
            ),
            call(
                16,
                "create_tile",
                json!({"orientation":0,"fill":3,"dt1":"c:/windows/x.dt1"}),
            ),
            call(
                17,
                "create_tile",
                json!({"orientation":0,"fill":3,"dt1":"act1/outdoors/blank.dt1"}),
            ),
            call(
                18,
                "reshape_tile",
                json!({"dt1":"act1/outdoors/blank.dt1","index":0,"new_orientation":15}),
            ),
            call(
                19,
                "inspect_tile",
                json!({"dt1":"act1/outdoors/blank.dt1","index":0,"render":"file","name":"../x"}),
            ),
            call(
                20,
                "import_tile_image",
                json!({"orientation":0,"path":"/etc/passwd"}),
            ),
            call(
                21,
                "new_dt1",
                json!({"name":"blank","folder":"act1/outdoors"}),
            ),
            call(22, "create_tile", json!({"orientation":0,"main":64})),
            call(
                23,
                "import_tile_image",
                json!({"orientation":0,"data":b64(b"hello there")}),
            ),
        ],
    );
    err(&r, 0, "No map is open");
    err(&r, 2, "orientation must be");
    err(&r, 3, "height is required");
    err(&r, 4, "height must be");
    err(&r, 5, "corner_pair needs orientation 3 or 4");
    err(&r, 6, "not valid base64");
    err(&r, 7, "holds 3 bytes");
    err(&r, 8, "pixels.width must be");
    err(&r, 9, "at most 4096");
    err(&r, 10, "cannot be read");
    err(&r, 11, "name");
    err(&r, 12, "reserves");
    err(&r, 13, "folder");
    err(&r, 14, "not a safe DT1 path");
    err(&r, 15, "not a safe DT1 path");
    err(&r, 16, "not one of this profile's own DT1s");
    err(&r, 17, "not one of this profile's own DT1s");
    err(&r, 18, "use only letters");
    err(&r, 19, "only .png");
    err(&r, 20, "already exists");
    err(&r, 21, "main must be");
    err(&r, 22, "not PNG, BMP, GIF or PCX");
    // Nothing was written outside the saves.
    assert!(!tmp.path().join("x.dt1").exists());
    assert!(
        !tmp.path()
            .join("saves")
            .join("t")
            .join("data")
            .join("global")
            .join("tiles")
            .join("act1")
            .join("outdoors")
            .join("blank.dt1")
            .exists()
    );
}
