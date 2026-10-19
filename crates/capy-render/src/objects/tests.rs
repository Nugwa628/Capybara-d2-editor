#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::order::{DrawEntry, DrawPass, Visibility, draw_list_with_objects};
use crate::testutil::{cell, dt1, palette, set_floor, set_wall, tile, tileset};
use d2_assets::{FolderSource, GameVersion};
use d2_formats::dc6::Dc6;

#[test]
fn anchor_matches_the_reference_formula() {
    // preview.c:2084-2086 at 1:1: ((x - y + 5) * 16, 4 + (x + y + 1) * 8).
    assert_eq!(object_anchor(0, 0, 1), (80, 12));
    assert_eq!(object_anchor(2, 2, 1), (80, 44));
    assert_eq!(object_anchor(7, 3, 1), (144, 92));
    // 1:4: tile_w 40, tile_h 20, the constant 4 is not scaled.
    assert_eq!(object_anchor(0, 0, 4), (20, 6));
    // negative operands truncate toward zero like C
    assert_eq!(object_anchor(0, 0, 8).0, (-2 * -20) / 10 + (3 * 20) / 10);
}

#[test]
fn frame_index_follows_the_tick_rule() {
    assert_eq!(frame_index(None, 256, 8), 0);
    assert_eq!(frame_index(Some(3), 256, 8), 3);
    assert_eq!(frame_index(Some(9), 256, 8), 1);
    // half speed: one frame per two ticks
    assert_eq!(frame_index(Some(5), 128, 8), 2);
    // a zero count is treated as 1
    assert_eq!(frame_index(Some(5), 256, 0), 0);
}

#[test]
fn drawing_order_is_tile_then_sub_tile_table_then_index() {
    let frame = Arc::new(ObjectFrame {
        layers: Vec::new(),
        offset: (0, 0),
        order_flag: 0,
    });
    let key = |x, y, index| {
        PlacedObject {
            index,
            kind: 2,
            id: 0,
            x,
            y,
            frame: Arc::clone(&frame),
        }
        .sort_key()
    };
    // row first
    assert!(key(9, 0, 0) < key(0, 5, 1));
    // then column
    assert!(key(4, 0, 0) < key(5, 0, 1));
    // within a tile: table[sy][sx], (sx=1, sy=0) = 2 and (sx=0, sy=1) = 1
    assert!(key(0, 1, 0) < key(1, 0, 1));
    // equal sub-tile: file order
    assert!(key(2, 2, 3) < key(2, 2, 4));
}

#[test]
fn file_direction_uses_the_editor_tables() {
    assert_eq!(file_direction(1, 0), 0);
    assert_eq!(file_direction(4, 2), 2);
    assert_eq!(file_direction(8, 0), 4);
    assert_eq!(file_direction(8, 1), 0);
    assert_eq!(file_direction(16, 0), 4);
    assert_eq!(file_direction(32, 1), 16);
}

#[test]
fn darken_scales_the_destination_like_the_reference_shadow() {
    // out = dst * level / 32 over an opaque destination.
    let out = Blend::Darken(10).apply([200, 100, 32, 255], [255, 255, 255]);
    assert_eq!(out[3], 255);
    assert!((i32::from(out[0]) - 62).abs() <= 1, "{out:?}");
    assert!((i32::from(out[1]) - 31).abs() <= 1, "{out:?}");
    assert!(out[2] <= 11, "{out:?}");
    // the source colour does not matter
    assert_eq!(
        Blend::Darken(28).apply([100, 100, 100, 255], [0, 0, 0]),
        Blend::Darken(28).apply([100, 100, 100, 255], [250, 10, 90])
    );
    // over nothing it leaves a translucent black pixel
    let out = Blend::Darken(10).apply([0, 0, 0, 0], [9, 9, 9]);
    assert_eq!((out[0], out[1], out[2]), (0, 0, 0));
    assert!(out[3] > 150 && out[3] < 190, "{out:?}");
}

// ----- an object drawn from a synthetic game folder

const SPRITE_PATH: &str = "data/global/objects/fn/tr/fntrlitnuhth.dc6";
const COF_PATH: &str = "data/global/objects/fn/cof/fnnuhth.cof";

/// A one direction, two frame COF with the layer TR (shadow on).
fn cof_bytes() -> Vec<u8> {
    let mut b = vec![1u8, 2, 1, 20];
    b.extend_from_slice(&[0; 24]);
    // component TR, shadow, selectable, transparent, effect, "HTH\0"
    b.extend_from_slice(&[1, 1, 0, 0, 0, b'H', b'T', b'H', 0]);
    b.extend_from_slice(&[0, 0]); // keyframes
    b.extend_from_slice(&[1, 1]); // priority: TR in both frames
    b
}

fn sprite_bytes() -> Vec<u8> {
    let frame = |pixel: u8| {
        let mut img = IndexedImage::new(4, 4, -2, -6);
        img.pixels.fill(pixel);
        img.alpha.fill(255);
        img
    };
    Dc6::from_frames(&[vec![frame(10), frame(20)]])
        .unwrap()
        .to_bytes()
        .unwrap()
}

fn write(root: &std::path::Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn game(dir: &std::path::Path) -> Overlay {
    write(dir, COF_PATH, &cof_bytes());
    write(dir, SPRITE_PATH, &sprite_bytes());
    Overlay::new(vec![Box::new(FolderSource::new(dir, "synthetic"))])
}

/// Act 1 (0), object type 2 id 0: DS1Edit's obj.txt maps it to the rogue
/// fountain, token FN, mode NU, TR skin LIT.
fn ds1_with(objects: &[(i32, i32)]) -> Ds1 {
    let mut ds1 = Ds1::new(3, 2, 0);
    ds1.add_wall_layer().unwrap();
    ds1.add_floor_layer().unwrap();
    for &(x, y) in objects {
        ds1.objects.push(Object {
            kind: 2,
            id: 0,
            x,
            y,
            flags: 0,
        });
    }
    ds1
}

#[test]
fn an_object_resolves_loads_and_composites() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = game(dir.path());
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let ds1 = ds1_with(&[(7, 3)]);
    let (placed, missing) = sprites.place(&ds1, &ObjectOptions::default());
    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(placed.len(), 1);
    let frame = &placed[0].frame;
    assert_eq!(frame.layers.len(), 1);
    let layer = &frame.layers[0];
    assert_eq!(layer.component, 1);
    assert_eq!(layer.shadow, Some(10));
    assert_eq!(layer.box_origin, (-2, -6));
    assert_eq!(layer.image.pixels[0], 10, "frame 0 by default");
    // the animation tick advances the frame
    let ticked = ObjectOptions {
        animation_tick: Some(1),
        ..ObjectOptions::default()
    };
    let (placed, _) = sprites.place(&ds1, &ticked);
    assert_eq!(placed[0].frame.layers[0].image.pixels[0], 20);
    // pinned frames win over the tick
    let pinned = ObjectOptions {
        animation_tick: Some(1),
        frame_overrides: Some(vec![Some(0)].into()),
        ..ObjectOptions::default()
    };
    let (placed, _) = sprites.place(&ds1, &pinned);
    assert_eq!(placed[0].frame.layers[0].image.pixels[0], 10);
    // the 1:1 composite holds the 4x4 square at the layer offset
    let (img, origin) = frame.composite(&palette()).unwrap();
    assert_eq!((img.width, img.height), (4, 4));
    assert_eq!(origin, (-2, -6));
    assert_eq!(img.get(0, 0), Some([10, 5, 245, 255]));
}

#[test]
fn objects_outside_the_map_and_unresolved_ones_are_handled() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = game(dir.path());
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let mut ds1 = ds1_with(&[(7, 3), (15, 3), (-1, 0)]);
    ds1.objects.push(Object {
        kind: 9,
        id: 1,
        x: 1,
        y: 1,
        flags: 0,
    });
    let (placed, missing) = sprites.place(&ds1, &ObjectOptions::default());
    // x = 15 is past the 3 tile (15 sub-tile) width and x = -1 is negative:
    // dropped without a report, like ds1.c:975.
    assert_eq!(placed.len(), 1);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].index, 3);
    assert!(matches!(
        missing[0].reason,
        MissingReason::Unresolved | MissingReason::NoSprite
    ));
}

#[test]
fn a_missing_cof_is_reported_not_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = game(dir.path());
    std::fs::remove_file(dir.path().join(COF_PATH)).unwrap();
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let ds1 = ds1_with(&[(7, 3)]);
    let (placed, missing) = sprites.place(&ds1, &ObjectOptions::default());
    assert!(placed.is_empty());
    assert!(matches!(missing[0].reason, MissingReason::CofNotFound(_)));
}

fn tiles() -> crate::tileset::TileSet {
    tileset(vec![dt1(vec![tile(0, 1, 0, 0, 10), tile(1, 1, 0, 0, 12)])])
}

fn summary(list: &crate::order::DrawList) -> Vec<(DrawPass, bool, u32, u32)> {
    list.entries
        .iter()
        .map(|e| match e {
            DrawEntry::Tile(t) => (t.pass, false, t.x, t.y),
            DrawEntry::Object(o) => (o.pass, true, o.tile.0, o.tile.1),
        })
        .collect()
}

#[test]
fn objects_interleave_with_upper_walls_per_tile() {
    // preview.c:3675-3697: per tile the upper walls, then the tile's objects;
    // shadows come first as a pass of their own.
    let dir = tempfile::tempdir().unwrap();
    let overlay = game(dir.path());
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    // objects on tile (0,0), on tile (1,0) (walls there too) and on (2,1),
    // a tile with no wall at all
    let mut ds1 = ds1_with(&[(3, 3), (7, 2), (12, 6)]);
    for x in 0..2 {
        set_floor(&mut ds1, 0, x, 0, cell(1, 1, 0));
        set_wall(&mut ds1, 0, x, 0, 1, cell(1, 1, 0));
    }
    let opts = ObjectOptions::default();
    let list = draw_list_with_objects(
        &ds1,
        &tiles(),
        &Visibility::default(),
        Some((&sprites, &opts)),
    )
    .unwrap();
    assert!(list.missing_objects.is_empty());
    use DrawPass::*;
    assert_eq!(
        summary(&list),
        vec![
            (Floors, false, 0, 0),
            (Floors, false, 1, 0),
            // shadows: every object, drawing order
            (ObjectShadows, true, 0, 0),
            (ObjectShadows, true, 1, 0),
            (ObjectShadows, true, 2, 1),
            (UpperWalls, false, 0, 0),
            (UpperWalls, true, 0, 0),
            (UpperWalls, false, 1, 0),
            (UpperWalls, true, 1, 0),
            (UpperWalls, true, 2, 1),
        ]
    );
    // order is the position in the list
    for (i, e) in list.entries.iter().enumerate() {
        let order = match e {
            DrawEntry::Tile(t) => t.order,
            DrawEntry::Object(o) => o.order,
        };
        assert_eq!(order as usize, i);
    }
}

#[test]
fn hidden_objects_draw_and_report_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = game(dir.path());
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let ds1 = ds1_with(&[(7, 3), (1, 1)]);
    let opts = ObjectOptions::default();
    let vis = Visibility {
        objects: false,
        ..Visibility::default()
    };
    let list = draw_list_with_objects(&ds1, &tiles(), &vis, Some((&sprites, &opts))).unwrap();
    assert!(list.entries.is_empty());
    assert!(list.missing_objects.is_empty());
}
