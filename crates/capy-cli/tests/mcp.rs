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
