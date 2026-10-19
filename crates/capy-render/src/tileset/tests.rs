#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::testutil::{dt1, iso_block, tile, tileset};

fn key(orientation: u8, main: u8, sub: u8) -> TileKey {
    TileKey {
        orientation,
        main,
        sub,
    }
}

fn r(dt1: usize, tile: usize) -> TileRef {
    TileRef { dt1, tile }
}

#[test]
fn exact_key_beats_every_fallback() {
    let set = tileset(vec![dt1(vec![
        tile(19, 3, 1, 0, 1),
        tile(18, 3, 1, 0, 2),
        tile(19, 3, 0, 0, 3),
    ])]);
    assert_eq!(set.resolve(key(18, 3, 1)), Some(r(0, 1)));
    assert_eq!(set.resolve(key(19, 3, 1)), Some(r(0, 0)));
}

#[test]
fn orientation_18_and_19_fall_back_to_each_other() {
    let set = tileset(vec![dt1(vec![tile(19, 3, 1, 0, 1), tile(18, 4, 2, 0, 2)])]);
    // 18 -> 19 with the same sub index (ds1.c:154-175).
    assert_eq!(set.resolve(key(18, 3, 1)), Some(r(0, 0)));
    // 19 -> 18.
    assert_eq!(set.resolve(key(19, 4, 2)), Some(r(0, 1)));
}

#[test]
fn sub_zero_fallback_uses_the_other_orientation() {
    // "trick of O=18 M=3 I=1 ---> O=19 M=3 I=0" (ds1.c:153).
    let set = tileset(vec![dt1(vec![tile(19, 3, 0, 0, 1), tile(18, 5, 0, 0, 2)])]);
    assert_eq!(set.resolve(key(18, 3, 1)), Some(r(0, 0)));
    assert_eq!(set.resolve(key(19, 5, 7)), Some(r(0, 1)));
    // The same orientation with sub 0 is not tried (ds1.c:177-192 only looks
    // at the swapped orientation).
    let set = tileset(vec![dt1(vec![tile(18, 3, 0, 0, 1)])]);
    assert_eq!(set.resolve(key(18, 3, 1)), None);
}

#[test]
fn fallbacks_do_not_apply_to_other_orientations() {
    let set = tileset(vec![dt1(vec![tile(1, 3, 0, 0, 1), tile(0, 2, 0, 0, 2)])]);
    assert_eq!(set.resolve(key(1, 3, 5)), None, "sub 0 is only for 18/19");
    assert_eq!(set.resolve(key(0, 2, 1)), None);
    assert_eq!(set.resolve(key(2, 3, 0)), None);
}

#[test]
fn highest_rarity_wins() {
    let set = tileset(vec![dt1(vec![
        tile(0, 1, 1, 1, 1),
        tile(0, 1, 1, 3, 2),
        tile(0, 1, 1, 2, 3),
    ])]);
    assert_eq!(set.resolve(key(0, 1, 1)), Some(r(0, 1)));
}

#[test]
fn rarity_ties_go_to_the_first_in_load_order() {
    let set = tileset(vec![
        dt1(vec![tile(0, 1, 1, 2, 1), tile(0, 1, 1, 2, 2)]),
        dt1(vec![tile(0, 1, 1, 2, 3)]),
    ]);
    assert_eq!(set.resolve(key(0, 1, 1)), Some(r(0, 0)));
    // A later DT1 with a higher rarity still wins.
    let set = tileset(vec![
        dt1(vec![tile(0, 1, 1, 2, 1)]),
        dt1(vec![tile(0, 1, 1, 5, 2)]),
    ]);
    assert_eq!(set.resolve(key(0, 1, 1)), Some(r(1, 0)));
}

#[test]
fn all_zero_rarity_uses_the_last_tile_of_the_first_dt1() {
    let set = tileset(vec![
        dt1(vec![
            tile(0, 4, 0, 0, 1),
            tile(0, 9, 9, 0, 9),
            tile(0, 4, 0, 0, 2),
        ]),
        dt1(vec![tile(0, 4, 0, 0, 3)]),
    ]);
    assert_eq!(set.resolve(key(0, 4, 0)), Some(r(0, 2)));
    // The first DT1 that has the key, not DT1 number 0 as such.
    let set = tileset(vec![
        dt1(vec![tile(0, 9, 9, 0, 9)]),
        dt1(vec![tile(0, 4, 0, 0, 1), tile(0, 4, 0, 0, 2)]),
        dt1(vec![tile(0, 4, 0, 0, 3)]),
    ]);
    assert_eq!(set.resolve(key(0, 4, 0)), Some(r(1, 1)));
}

#[test]
fn a_single_nonzero_rarity_beats_zero_rarity_tiles() {
    let set = tileset(vec![dt1(vec![tile(0, 1, 1, 0, 1), tile(0, 1, 1, 1, 2)])]);
    assert_eq!(set.resolve(key(0, 1, 1)), Some(r(0, 1)));
}

#[test]
fn candidates_are_in_load_order() {
    let set = tileset(vec![
        dt1(vec![
            tile(0, 1, 1, 0, 1),
            tile(0, 2, 2, 0, 1),
            tile(0, 1, 1, 0, 2),
        ]),
        dt1(vec![tile(0, 1, 1, 0, 3)]),
    ]);
    assert_eq!(
        set.candidates(key(0, 1, 1)),
        &[r(0, 0), r(0, 2), r(1, 0)][..]
    );
    assert!(set.candidates(key(0, 7, 7)).is_empty());
}

#[test]
fn reference_quirk_skips_the_group_head() {
    // misc.c:156-187. In one DT1 the group is sorted by rarity, so a tie makes
    // the head the first of equals; the quirk then takes the next one.
    let quirk = tileset(vec![dt1(vec![
        tile(0, 1, 1, 1, 1),
        tile(0, 1, 1, 1, 2),
        tile(0, 1, 1, 1, 3),
    ])])
    .with_variant_rule(VariantRule::ReferenceQuirk);
    assert_eq!(quirk.variant_rule(), VariantRule::ReferenceQuirk);
    assert_eq!(quirk.resolve(key(0, 1, 1)), Some(r(0, 1)));
    let documented = tileset(vec![dt1(vec![
        tile(0, 1, 1, 1, 1),
        tile(0, 1, 1, 1, 2),
        tile(0, 1, 1, 1, 3),
    ])]);
    assert_eq!(documented.variant_rule(), VariantRule::Documented);
    assert_eq!(documented.resolve(key(0, 1, 1)), Some(r(0, 0)));
}

#[test]
fn reference_quirk_prefers_a_later_dt1_over_a_higher_head() {
    // Head = lowest slot. dt1#0 has rarity 5, dt1#1 has rarity 3: the head is
    // never compared, so the quirk shows the 3.
    let build = |rule| {
        tileset(vec![
            dt1(vec![tile(0, 1, 1, 5, 1)]),
            dt1(vec![tile(0, 1, 1, 3, 2)]),
        ])
        .with_variant_rule(rule)
    };
    assert_eq!(
        build(VariantRule::Documented).resolve(key(0, 1, 1)),
        Some(r(0, 0))
    );
    assert_eq!(
        build(VariantRule::ReferenceQuirk).resolve(key(0, 1, 1)),
        Some(r(1, 0))
    );
}

#[test]
fn reference_quirk_agrees_on_zero_rarity_and_single_tiles() {
    let build = |rule| {
        tileset(vec![
            dt1(vec![
                tile(0, 4, 0, 0, 1),
                tile(0, 4, 0, 0, 2),
                tile(0, 6, 0, 3, 3),
            ]),
            dt1(vec![tile(0, 4, 0, 0, 4)]),
        ])
        .with_variant_rule(rule)
    };
    for rule in [VariantRule::Documented, VariantRule::ReferenceQuirk] {
        let set = build(rule);
        assert_eq!(set.resolve(key(0, 4, 0)), Some(r(0, 1)), "{rule:?}");
        assert_eq!(set.resolve(key(0, 6, 0)), Some(r(0, 2)), "{rule:?}");
    }
}

#[test]
fn animated_floor_shows_frame_zero() {
    // Frames of one key with rarity 0..=2; the representative is the highest
    // rarity and the reference steps back `rarity` blocks.
    let mut frames = vec![
        tile(0, 1, 1, 2, 3),
        tile(0, 1, 1, 0, 1),
        tile(0, 1, 1, 1, 2),
    ];
    for t in &mut frames {
        t.animated = 1;
    }
    let set = tileset(vec![dt1(frames)]);
    let rep = set.resolve(key(0, 1, 1)).unwrap();
    assert_eq!(rep, r(0, 0));
    assert_eq!(set.animation_frame0(&rep), r(0, 1));
    // Non-animated tiles are untouched.
    let plain = tileset(vec![dt1(vec![tile(0, 1, 1, 2, 1), tile(0, 1, 1, 0, 2)])]);
    let rep = plain.resolve(key(0, 1, 1)).unwrap();
    assert_eq!(plain.animation_frame0(&rep), rep);
}

#[test]
fn corner_pair_is_the_orientation_4_tile() {
    let set = tileset(vec![dt1(vec![
        tile(3, 2, 1, 0, 1),
        tile(4, 2, 1, 0, 2),
        tile(4, 2, 2, 0, 3),
    ])]);
    assert_eq!(set.corner_pair(&r(0, 0)), Some(r(0, 1)));
    assert_eq!(set.corner_pair(&r(0, 1)), None, "only orientation 3 pairs");
    let lonely = tileset(vec![dt1(vec![tile(3, 2, 1, 0, 1)])]);
    assert_eq!(lonely.corner_pair(&r(0, 0)), None);
}

#[test]
fn geometry_follows_the_reference_classes() {
    let g = |t: &Tile| TileGeometry::of(t);
    let floor = g(&tile(0, 0, 0, 0, 1));
    assert_eq!(
        (
            floor.class,
            floor.width,
            floor.height,
            floor.y_add,
            floor.zero_line
        ),
        (TileClass::Floor, 160, 80, 0, 0)
    );
    assert_eq!(floor.bitmap_top(1), 0);
    // Upper wall, stored height -128: +32 -> 96 high, y_add = 96, zero_line =
    // 96; the bitmap top sits 16 above the cell top.
    let wall = g(&tile(1, 0, 0, 0, 1));
    assert_eq!(
        (wall.class, wall.height, wall.y_add, wall.zero_line),
        (TileClass::UpperWall, 96, 96, 96)
    );
    assert_eq!(wall.bitmap_top(1), -16);
    assert_eq!(wall.bitmap_top(4), 20 - 24);
    // Lower wall: y_add and zero_line are hard-coded to 96.
    let lower = g(&tile(16, 0, 0, 0, 1));
    assert_eq!((lower.height, lower.y_add, lower.zero_line), (32, 96, 96));
    assert_eq!(lower.bitmap_top(1), -16);
    // Shadow: stored -96 -> 64 high, hangs off the cell's bottom corner.
    let shadow = g(&tile(13, 0, 0, 0, 1));
    assert_eq!(
        (shadow.class, shadow.height, shadow.zero_line),
        (TileClass::Shadow, 64, 64)
    );
    assert_eq!(shadow.bitmap_top(1), 80 - 64);
    // Roof: like a floor, lifted by roof_height (preview.c:1367).
    let mut roof_tile = tile(15, 0, 0, 0, 1);
    roof_tile.roof_height = 40;
    let roof = g(&roof_tile);
    assert_eq!(
        (roof.class, roof.height, roof.y_add),
        (TileClass::Roof, 80, 0)
    );
    assert_eq!(roof.bitmap_top(1), -40);
    assert_eq!(roof.bitmap_top(4), -10);
    // Special tiles are 160 wide whatever the file says (dt1.c:380).
    let mut special_tile = tile(10, 0, 0, 0, 1);
    special_tile.width = 32;
    assert_eq!(g(&special_tile).width, 160);
}

#[test]
fn empty_geometry_draws_nothing() {
    let mut t = tile(0, 0, 0, 0, 1);
    t.height = 0;
    assert!(TileGeometry::of(&t).is_empty());
    let set = tileset(vec![dt1(vec![t])]);
    let img = set.image(&r(0, 0)).unwrap();
    assert_eq!((img.width, img.height), (0, 0));
}

#[test]
fn image_is_clipped_to_the_bitmap() {
    // A floor bitmap is 160x80; a block hanging off the right edge and one
    // above the top are cut.
    let mut t = tile(0, 0, 0, 0, 7);
    t.blocks = vec![iso_block(150, 0, 7), iso_block(0, -20, 7)];
    let set = tileset(vec![dt1(vec![t])]);
    let img = set.image(&r(0, 0)).unwrap();
    assert!(img.offset_x >= 0 && img.offset_y >= 0);
    assert!(i64::from(img.offset_x) + i64::from(img.width) <= 160);
    assert!(i64::from(img.offset_y) + i64::from(img.height) <= 80);
    assert!(img.alpha.iter().any(|&a| a != 0));
}

#[test]
fn images_are_cached() {
    let set = tileset(vec![dt1(vec![tile(0, 0, 0, 0, 1)])]);
    let a = set.image(&r(0, 0)).unwrap();
    let b = set.image(&r(0, 0)).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn unreadable_dt1_is_a_warning_and_keeps_indices_aligned() {
    let root = std::env::temp_dir().join("capy-render-no-such-folder");
    let overlay = Overlay::new(vec![Box::new(d2_assets::FolderSource::new(&root, "none"))]);
    let (set, warnings) = TileSet::load(
        &overlay,
        &[GamePath::new("data\\global\\tiles\\missing.dt1")],
    )
    .unwrap();
    assert_eq!(set.dt1_paths().len(), 1);
    assert_eq!(warnings.len(), 1);
    assert!(set.resolve(key(0, 0, 0)).is_none());
}
