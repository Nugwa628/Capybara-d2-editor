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
        frames_per_direction: 1,
        directions: 1,
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
    // the full tables (main.c:331-340)
    let row = |n: u8| (0..n).map(|u| file_direction(n, u)).collect::<Vec<_>>();
    assert_eq!(row(8), DIR8);
    assert_eq!(row(16), DIR16);
    assert_eq!(row(32), DIR32);
    assert_eq!(row(4), [0, 1, 2, 3]);
    // out of range user directions, and other COF direction counts
    assert_eq!(file_direction(4, 9), 3);
    assert_eq!(file_direction(8, 8), 4);
    assert_eq!(file_direction(1, 5), 0);
    assert_eq!(file_direction(0, 3), 0);
    assert_eq!(file_direction(2, 1), 0);
    for n in [1u8, 4, 8, 16, 32] {
        assert!((0..=255).all(|u| file_direction(n, u) < n));
    }
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
    assert_eq!(frame.frames_per_direction, 2, "the COF's frame count");
    assert_eq!(frame.directions, 1, "the COF's direction count");
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

/// A four direction, one frame COF (layer TR) and a matching DC6 whose
/// direction `d` is filled with pixel `30 + d`.
fn four_direction_game(dir: &std::path::Path) -> Overlay {
    let mut cof = vec![1u8, 1, 4, 20];
    cof.extend_from_slice(&[0; 24]);
    cof.extend_from_slice(&[1, 1, 0, 0, 0, b'H', b'T', b'H', 0]);
    cof.push(0); // keyframes
    cof.extend_from_slice(&[1, 1, 1, 1]); // priority: TR in every direction
    let frame = |pixel: u8| {
        let mut img = IndexedImage::new(4, 4, -2, -6);
        img.pixels.fill(pixel);
        img.alpha.fill(255);
        img
    };
    let dc6 = Dc6::from_frames(&[
        vec![frame(30)],
        vec![frame(31)],
        vec![frame(32)],
        vec![frame(33)],
    ])
    .unwrap()
    .to_bytes()
    .unwrap();
    write(dir, COF_PATH, &cof);
    write(dir, SPRITE_PATH, &dc6);
    Overlay::new(vec![Box::new(FolderSource::new(dir, "synthetic"))])
}

#[test]
fn object_frames_carry_the_cof_direction_count() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = four_direction_game(dir.path());
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let ds1 = ds1_with(&[(7, 3)]);
    let (placed, missing) = sprites.place(&ds1, &ObjectOptions::default());
    assert!(missing.is_empty(), "{missing:?}");
    let frame = &placed[0].frame;
    assert_eq!(frame.directions, 4);
    assert_eq!(frame.frames_per_direction, 1);
    // Every user direction maps onto one of the four file directions.
    let info = catalog.resolve(ds1.effective_act(), ds1.version, &ds1.objects[0]);
    let mut seen = std::collections::BTreeSet::new();
    for user_dir in 0..32 {
        let f = sprites
            .frame_of_info(&info, &ObjectOptions::default(), Some(0), user_dir)
            .unwrap();
        assert_eq!(f.directions, 4);
        seen.insert(f.layers[0].image.pixels[0]);
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), [30, 31, 32, 33]);
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
    // frames_of_info reports the same reason.
    let info = catalog.resolve(ds1.effective_act(), ds1.version, &ds1.objects[0]);
    assert!(matches!(
        sprites.frames_of_info(&info, 0),
        Err(MissingReason::CofNotFound(_))
    ));
}

#[test]
fn frames_of_info_holds_every_frame_and_the_animation_speed() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = game(dir.path());
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let ds1 = ds1_with(&[(7, 3)]);
    let mut info = catalog.resolve(ds1.effective_act(), ds1.version, &ds1.objects[0]);
    let (frames, delta) = sprites.frames_of_info(&info, 0).unwrap();
    assert_eq!(frames.len(), 2, "the COF's frames per direction");
    assert_eq!(frames[0].layers[0].image.pixels[0], 10);
    assert_eq!(frames[1].layers[0].image.pixels[0], 20);
    assert_eq!(
        delta,
        info.frame_delta.filter(|d| *d != 0).unwrap_or(256),
        "no AnimData here: the Objects.txt FrameDelta, else 256"
    );
    // The same cached frames, and the speed frame_of_info animates with.
    for (f, frame) in (0u8..).zip(&frames) {
        let one = sprites
            .frame_of_info(&info, &ObjectOptions::default(), Some(f), 0)
            .unwrap();
        assert!(Arc::ptr_eq(&one, frame));
    }
    for tick in 0..40 {
        let opts = ObjectOptions {
            animation_tick: Some(tick),
            ..ObjectOptions::default()
        };
        let one = sprites.frame_of_info(&info, &opts, None, 0).unwrap();
        let k = usize::from(frame_index(Some(tick), delta, 2));
        assert!(Arc::ptr_eq(&one, &frames[k]), "tick {tick}");
    }
    // A slower FrameDelta is reported as is; 0 means 256.
    info.frame_delta = Some(64);
    assert_eq!(sprites.frames_of_info(&info, 0).unwrap().1, 64);
    info.frame_delta = Some(0);
    assert_eq!(sprites.frames_of_info(&info, 0).unwrap().1, 256);
    // An object without a sprite.
    info.token = None;
    assert!(sprites.frames_of_info(&info, 0).is_err());
}

fn assert_static_send_sync<T: Send + Sync + 'static>(_: &T) {}

#[test]
fn owned_sprites_keep_their_cache_across_threads() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = Arc::new(game(dir.path()));
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let catalog = Arc::new(catalog);
    let owned: OwnedObjectSprites =
        ObjectSprites::new_shared(Arc::clone(&overlay), Arc::clone(&catalog));
    assert_static_send_sync(&owned);
    assert!(owned.uses(&overlay, &catalog));
    let (other, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    assert!(!owned.uses(&overlay, &other));

    let ds1 = ds1_with(&[(7, 3)]);
    let (placed, missing) = owned.place(&ds1, &ObjectOptions::default());
    assert!(missing.is_empty(), "{missing:?}");
    // Same results as the borrowing variant.
    let borrowed = ObjectSprites::new(&overlay, &catalog);
    assert!(borrowed.uses(&overlay, &catalog));
    let (again, _) = borrowed.place(&ds1, &ObjectOptions::default());
    assert_eq!(placed[0].frame, again[0].frame);

    // The cache lives with the value: another thread gets the same Arcs.
    let owned = Arc::new(owned);
    let first = Arc::clone(&placed[0].frame);
    let info = catalog.resolve(ds1.effective_act(), ds1.version, &ds1.objects[0]);
    let worker = {
        let owned = Arc::clone(&owned);
        std::thread::spawn(move || owned.frames_of_info(&info, 0).unwrap().0)
    };
    let frames = worker.join().unwrap();
    assert!(Arc::ptr_eq(&frames[0], &first));
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

#[test]
fn shift_palette_maps_visible_pixels_only() {
    let mut row = [0u8; 256];
    for (i, v) in row.iter_mut().enumerate() {
        *v = 255 - i as u8;
    }
    row[0] = 0;
    let mut img = IndexedImage::new(4, 1, 0, 0);
    img.pixels.copy_from_slice(&[0, 10, 20, 30]);
    img.alpha.copy_from_slice(&[255, 255, 0, 255]);
    let mut frames = vec![img];
    shift_palette(&mut frames, &row);
    // index 0 and the alpha-0 pixel stay; the rest go through the row
    assert_eq!(frames[0].pixels, [0, 245, 20, 225]);
    assert_eq!(frames[0].alpha, [255, 255, 0, 255]);
}

/// Act 1 type 1 id 36 is Coldcrow in DS1Edit's obj.txt: monsters token CR,
/// mode NU, class BOW, TR and S1 skin LIT, `RandTransforms.dat` Index 4.
const CR_COF: &str = "data/global/monsters/cr/cof/crnubow.cof";
const CR_TR: &str = "data/global/monsters/cr/tr/crtrlitnubow.dc6";
const CR_S1: &str = "data/global/monsters/cr/s1/crs1litnubow.dc6";
const RAND_TRANSFORMS: &str = "data/global/monsters/randtransforms.dat";

/// One direction, one frame: TR opaque (trans_a 0), S1 alpha-blended
/// (trans_a 1, effect 5 = drawn normally), S1 on top.
fn coldcrow_cof() -> Vec<u8> {
    let mut b = vec![2u8, 1, 1, 20];
    b.extend_from_slice(&[0; 24]);
    b.extend_from_slice(&[1, 0, 0, 0, 0, b'B', b'O', b'W', 0]);
    b.extend_from_slice(&[8, 0, 0, 1, 5, b'B', b'O', b'W', 0]);
    b.push(0); // keyframe
    b.extend_from_slice(&[1, 8]); // priority: TR, then S1
    b
}

fn one_frame_dc6(pixel: u8, x: i32) -> Vec<u8> {
    let mut img = IndexedImage::new(2, 2, x, -2);
    img.pixels.fill(pixel);
    img.alpha.fill(255);
    Dc6::from_frames(&[vec![img]]).unwrap().to_bytes().unwrap()
}

/// Rows of `i ^ k` for k = 0, 1, 2, ... (index 0 kept at 0).
fn colormap_file(rows: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for k in 0..rows {
        out.extend((0..256usize).map(|i| if i == 0 { 0 } else { (i ^ (k * 16)) as u8 }));
    }
    out
}

fn coldcrow_game(dir: &std::path::Path, colormap: Option<Vec<u8>>) -> Overlay {
    write(dir, CR_COF, &coldcrow_cof());
    write(dir, CR_TR, &one_frame_dc6(10, -2));
    write(dir, CR_S1, &one_frame_dc6(20, 0));
    if let Some(bytes) = colormap {
        write(dir, RAND_TRANSFORMS, &bytes);
    }
    Overlay::new(vec![Box::new(FolderSource::new(dir, "synthetic"))])
}

fn coldcrow_ds1() -> Ds1 {
    let mut ds1 = ds1_with(&[]);
    ds1.objects.push(Object {
        kind: 1,
        id: 36,
        x: 7,
        y: 3,
        flags: 0,
    });
    ds1
}

/// The (TR, S1) pixel indices Coldcrow draws with.
fn coldcrow_pixels(colormap: Option<Vec<u8>>) -> (u8, u8) {
    let dir = tempfile::tempdir().unwrap();
    let overlay = coldcrow_game(dir.path(), colormap);
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let ds1 = coldcrow_ds1();
    let info = catalog.resolve(0, ds1.version, &ds1.objects[0]);
    assert_eq!(info.token.as_deref(), Some("CR"));
    assert_eq!(info.colormap.as_ref().map(|c| c.row), Some(3));
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let (placed, missing) = sprites.place(&ds1, &ObjectOptions::default());
    assert!(missing.is_empty(), "{missing:?}");
    let layers = &placed[0].frame.layers;
    assert_eq!(
        layers.iter().map(|l| l.component).collect::<Vec<_>>(),
        [1, 8]
    );
    (layers[0].image.pixels[0], layers[1].image.pixels[0])
}

#[test]
fn unique_monsters_draw_through_their_palette_shift() {
    // Index 4 is row 3: i ^ 48. Only the opaque layer (trans_a == 0) shifts.
    assert_eq!(coldcrow_pixels(Some(colormap_file(4))), (10 ^ 48, 20));
    // ... and the composite looks the shifted colour up in the palette
    let dir = tempfile::tempdir().unwrap();
    let overlay = coldcrow_game(dir.path(), Some(colormap_file(4)));
    let (catalog, _) = ObjectCatalog::build(&overlay, GameVersion::V113c).unwrap();
    let sprites = ObjectSprites::new(&overlay, &catalog);
    let (placed, _) = sprites.place(&coldcrow_ds1(), &ObjectOptions::default());
    let pal = palette();
    let (img, origin) = placed[0].frame.composite(&pal).unwrap();
    assert_eq!(origin, (-2, -2));
    let rgb = |i: u8| {
        let c = pal.colors[usize::from(i)];
        Some([c[0], c[1], c[2], 255])
    };
    assert_eq!(img.get(0, 0), rgb(10 ^ 48));
    assert_eq!(img.get(2, 0), rgb(20));
}

#[test]
fn a_missing_or_short_colormap_draws_unshifted() {
    assert_eq!(coldcrow_pixels(None), (10, 20));
    // three rows: Index 4 (row 3) is past the end of the file
    assert_eq!(coldcrow_pixels(Some(colormap_file(3))), (10, 20));
    // one byte short of the fourth row
    let mut short = colormap_file(4);
    short.pop();
    assert_eq!(coldcrow_pixels(Some(short)), (10, 20));
}
