//! One test per tool on small synthetic frames with exact expected pixels.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use capy_paint::{
    Dither, FillMode, FrameRef, GradientColors, GradientShape, Mask, OutlineColor, OutlineSide,
    Paint, PaintError, PaintSession, ShapeStyle,
};
use common::*;
use d2_formats::IndexedImage;

fn p(i: u8) -> Paint {
    Paint::Index(i)
}

// ---- pencil ----

#[test]
fn pencil_square_brushes() {
    let mut s = blank(5, 3);
    s.pencil(F0, &[(2, 1)], sq(1), p(5), None).unwrap();
    assert_eq!(render(&s), [".....", "..5..", "....."]);

    let mut s = blank(4, 4);
    s.pencil(F0, &[(1, 1)], sq(2), p(5), None).unwrap();
    assert_eq!(render(&s), ["....", ".55.", ".55.", "...."]);

    let mut s = blank(5, 5);
    s.pencil(F0, &[(2, 2)], sq(3), p(5), None).unwrap();
    assert_eq!(render(&s), [".....", ".555.", ".555.", ".555.", "....."]);
}

#[test]
fn pencil_round_brushes() {
    let mut s = blank(5, 5);
    s.pencil(F0, &[(2, 2)], round(3), p(5), None).unwrap();
    assert_eq!(render(&s), [".....", "..5..", ".555.", "..5..", "....."]);

    let mut s = blank(6, 6);
    s.pencil(F0, &[(1, 1)], round(4), p(5), None).unwrap();
    assert_eq!(
        render(&s),
        [".55...", "5555..", "5555..", ".55...", "......", "......"]
    );

    let mut s = blank(7, 7);
    s.pencil(F0, &[(3, 3)], round(5), p(5), None).unwrap();
    assert_eq!(
        render(&s),
        [
            ".......", "..555..", ".55555.", ".55555.", ".55555.", "..555..", "......."
        ]
    );
}

#[test]
fn pencil_joins_points_clips_and_erases() {
    let mut s = blank(5, 1);
    s.pencil(F0, &[(0, 0), (4, 0)], sq(1), p(5), None).unwrap();
    assert_eq!(render(&s), ["55555"]);
    // Erase with Transparent, including a point far outside.
    s.pencil(F0, &[(2, 0), (1000, 1000)], sq(1), Paint::Transparent, None)
        .unwrap();
    assert_eq!(render(&s), ["55.55"]);
}

#[test]
fn pencil_clips_at_the_border() {
    let mut s = blank(3, 3);
    s.pencil(F0, &[(-5, -5)], sq(1), p(5), None).unwrap();
    assert!(!s.can_undo(), "nothing painted, nothing recorded");
    s.pencil(F0, &[(-1, 0)], sq(3), p(5), None).unwrap();
    assert_eq!(render(&s), ["5..", "5..", "..."]);
}

#[test]
fn paint_index_zero_and_missing_frames_are_errors() {
    let mut s = blank(3, 3);
    assert!(matches!(
        s.pencil(F0, &[(0, 0)], sq(1), p(0), None),
        Err(PaintError::Invalid(_))
    ));
    let bad = FrameRef { dir: 3, frame: 0 };
    assert!(matches!(
        s.pencil(bad, &[(0, 0)], sq(1), p(1), None),
        Err(PaintError::NoFrame(f)) if f == bad
    ));
    assert!(s.pick(bad, (0, 0)).is_none());
}

// ---- line ----

#[test]
fn line_diagonal_shallow_and_steep() {
    let mut s = blank(5, 5);
    s.line(F0, (0, 0), (4, 4), sq(1), p(5), None).unwrap();
    assert_eq!(render(&s), ["5....", ".5...", "..5..", "...5.", "....5"]);

    let mut s = blank(5, 3);
    s.line(F0, (0, 0), (4, 2), sq(1), p(5), None).unwrap();
    assert_eq!(render(&s), ["5....", ".55..", "...55"]);
    let mut r = blank(5, 3);
    r.line(F0, (4, 2), (0, 0), sq(1), p(5), None).unwrap();
    assert_eq!(render(&r), render(&s), "direction does not matter");

    let mut s = blank(3, 5);
    s.line(F0, (0, 0), (2, 4), sq(1), p(5), None).unwrap();
    assert_eq!(render(&s), ["5..", ".5.", ".5.", "..5", "..5"]);
}

#[test]
fn line_with_thick_brush() {
    let mut s = blank(5, 4);
    s.line(F0, (1, 1), (3, 1), sq(2), p(5), None).unwrap();
    assert_eq!(render(&s), [".....", ".5555", ".5555", "....."]);
}

// ---- rectangle ----

#[test]
fn rectangle_outline_and_filled() {
    let mut s = blank(6, 5);
    s.rectangle(F0, (1, 1), (4, 3), ShapeStyle::Outline, sq(1), p(5), None)
        .unwrap();
    assert_eq!(
        render(&s),
        ["......", ".5555.", ".5..5.", ".5555.", "......"]
    );
    let mut s = blank(6, 5);
    s.rectangle(F0, (4, 3), (1, 1), ShapeStyle::Filled, sq(1), p(5), None)
        .unwrap();
    assert_eq!(
        render(&s),
        ["......", ".5555.", ".5555.", ".5555.", "......"]
    );
    let mut s = blank(4, 4);
    s.rectangle(F0, (-2, -2), (2, 2), ShapeStyle::Filled, sq(1), p(5), None)
        .unwrap();
    assert_eq!(render(&s), ["555.", "555.", "555.", "...."]);
}

// ---- ellipse ----

#[test]
fn ellipse_outline_and_filled() {
    let mut s = blank(5, 5);
    s.ellipse(F0, (0, 0), (4, 4), ShapeStyle::Filled, sq(1), p(5), None)
        .unwrap();
    assert_eq!(render(&s), [".555.", "55555", "55555", "55555", ".555."]);

    let mut s = blank(5, 5);
    s.ellipse(F0, (0, 0), (4, 4), ShapeStyle::Outline, sq(1), p(5), None)
        .unwrap();
    assert_eq!(render(&s), [".555.", "5...5", "5...5", "5...5", ".555."]);

    let mut s = blank(6, 4);
    s.ellipse(F0, (5, 3), (0, 0), ShapeStyle::Filled, sq(1), p(5), None)
        .unwrap();
    assert_eq!(render(&s), [".5555.", "555555", "555555", ".5555."]);
    let mut s = blank(6, 4);
    s.ellipse(F0, (0, 0), (5, 3), ShapeStyle::Outline, sq(1), p(5), None)
        .unwrap();
    assert_eq!(render(&s), [".5555.", "5....5", "5....5", ".5555."]);
}

// ---- fill / replace ----

#[test]
fn fill_contiguous_and_global() {
    let art = ["11.2", "1..2", "..22"];
    let mut s = session(&art);
    s.fill(F0, (0, 0), p(3), FillMode::Contiguous, None)
        .unwrap();
    assert_eq!(render(&s), ["33.2", "3..2", "..22"]);

    let mut s = session(&art);
    s.fill(F0, (2, 0), p(4), FillMode::Contiguous, None)
        .unwrap();
    assert_eq!(render(&s), ["1142", "1442", "4422"]);

    let mut s = session(&["1.1", "...", "1.1"]);
    s.fill(F0, (0, 0), p(2), FillMode::Contiguous, None)
        .unwrap();
    assert_eq!(render(&s), ["2.1", "...", "1.1"]);
    let mut s = session(&["1.1", "...", "1.1"]);
    s.fill(F0, (0, 0), p(2), FillMode::Global, None).unwrap();
    assert_eq!(render(&s), ["2.2", "...", "2.2"]);

    // Fill with Transparent erases a region; filling with the same colour is a no-op.
    let mut s = session(&["112", "112"]);
    s.fill(F0, (0, 0), Paint::Transparent, FillMode::Contiguous, None)
        .unwrap();
    assert_eq!(render(&s), ["..2", "..2"]);
    let before = s.revision();
    s.fill(F0, (2, 0), p(2), FillMode::Contiguous, None)
        .unwrap();
    assert_eq!(s.revision(), before);
    // Outside the frame: nothing happens.
    s.fill(F0, (99, 99), p(2), FillMode::Global, None).unwrap();
    assert_eq!(s.revision(), before);
}

#[test]
fn replace_color_in_frame_and_whole_sprite() {
    let a = image(&["12", "21"]);
    let b = image(&["11", ".1"]);
    let mut s = PaintSession::new(vec![vec![a.clone(), b.clone()]], test_palette()).unwrap();
    s.replace_color(Some(F0), p(1), p(3), None).unwrap();
    assert_eq!(render_at(&s, F0), ["32", "23"]);
    assert_eq!(render_at(&s, FrameRef { dir: 0, frame: 1 }), ["11", ".1"]);

    s.replace_color(None, p(1), p(4), None).unwrap();
    assert_eq!(render_at(&s, FrameRef { dir: 0, frame: 1 }), ["44", ".4"]);
    // One undo step for the whole-sprite replace.
    assert!(s.undo());
    assert_eq!(render_at(&s, FrameRef { dir: 0, frame: 1 }), ["11", ".1"]);
    assert_eq!(render_at(&s, F0), ["32", "23"]);

    // Transparent can be replaced too.
    s.replace_color(
        Some(FrameRef { dir: 0, frame: 1 }),
        Paint::Transparent,
        p(9),
        None,
    )
    .unwrap();
    assert_eq!(render_at(&s, FrameRef { dir: 0, frame: 1 }), ["11", "91"]);
}

#[test]
fn pick_reads_index_and_transparency() {
    let s = session(&["1."]);
    assert_eq!(s.pick(F0, (0, 0)), Some(p(1)));
    assert_eq!(s.pick(F0, (1, 0)), Some(Paint::Transparent));
    assert_eq!(s.pick(F0, (2, 0)), None);
    assert_eq!(s.pick(F0, (-1, 0)), None);
}

// ---- gradient ----

fn red_ramp(s: &PaintSession) -> capy_paint::RampId {
    let (id, _) = s.ramps().ramp_of(1).unwrap();
    assert_eq!(s.ramps().ramp(id), &[1, 2, 3, 4, 5, 6, 7, 8]);
    id
}

#[test]
fn gradient_linear_ramp_without_dither_walks_the_ramp() {
    let mut s = blank(8, 1);
    let id = red_ramp(&s);
    s.gradient(
        F0,
        (0.0, 0.0),
        (7.0, 0.0),
        GradientShape::Linear,
        GradientColors::Ramp(id),
        Dither::None,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["12345678"]);
}

#[test]
fn gradient_two_colours_hits_both_ends() {
    let mut s = blank(8, 2);
    s.gradient(
        F0,
        (0.0, 0.0),
        (7.0, 0.0),
        GradientShape::Linear,
        GradientColors::Two(1, 8),
        Dither::None,
        None,
    )
    .unwrap();
    let img = s.frame(F0).unwrap();
    assert_eq!(img.pixels[0], 1);
    assert_eq!(img.pixels[7], 8);
    assert!(
        img.alpha.iter().all(|&a| a == 255),
        "gradient output is opaque"
    );
    for row in img.pixels.chunks(8) {
        assert!(row.windows(2).all(|w| w[0] <= w[1]), "monotonic {row:?}");
        assert!(row.iter().all(|&i| (1..=8).contains(&i)));
    }
    // Rows are identical for a horizontal gradient.
    assert_eq!(img.pixels[..8], img.pixels[8..]);
}

#[test]
fn gradient_dither_modes_use_only_ramp_indices() {
    let ramp: Vec<u8> = (9..=16).collect();
    for shape in [GradientShape::Linear, GradientShape::Radial] {
        for dither in [Dither::None, Dither::Bayer4, Dither::FloydSteinberg] {
            let mut s = blank(12, 9);
            let (id, _) = s.ramps().ramp_of(9).unwrap();
            assert_eq!(s.ramps().ramp(id), ramp.as_slice());
            s.gradient(
                F0,
                (0.0, 0.0),
                (11.0, 8.0),
                shape,
                GradientColors::Ramp(id),
                dither,
                None,
            )
            .unwrap();
            let img = s.frame(F0).unwrap();
            assert!(img.alpha.iter().all(|&a| a == 255));
            assert!(
                img.pixels.iter().all(|i| ramp.contains(i)),
                "{shape:?}/{dither:?} left the ramp: {:?}",
                img.pixels
            );
            // A dithered gradient actually uses several steps.
            let distinct: std::collections::BTreeSet<_> = img.pixels.iter().collect();
            assert!(distinct.len() >= 4, "{shape:?}/{dither:?}: {distinct:?}");
        }
    }
}

#[test]
fn gradient_dithering_mixes_neighbouring_shades() {
    // Halfway between ramp steps 4 and 5 the ordered dither uses both.
    let (mut s, id) = {
        let s = blank(8, 8);
        let (id, _) = s.ramps().ramp_of(1).unwrap();
        (s, id)
    };
    // Constant t = 3.5/7: from == to is degenerate, so use a huge flat span.
    s.gradient(
        F0,
        (0.0, 0.0),
        (1000.0, 0.0),
        GradientShape::Linear,
        GradientColors::Ramp(id),
        Dither::None,
        None,
    )
    .unwrap();
    let flat = s.frame(F0).unwrap().pixels.clone();
    assert!(
        flat.iter().all(|&i| i == 1),
        "t is ~0 across 8 px of a 1000 px span"
    );

    let mut s = blank(8, 8);
    s.gradient(
        F0,
        (-3500.0, 0.0),
        (3500.0, 0.0),
        GradientShape::Linear,
        GradientColors::Ramp(id),
        Dither::Bayer4,
        None,
    )
    .unwrap();
    let used: std::collections::BTreeSet<u8> =
        s.frame(F0).unwrap().pixels.iter().copied().collect();
    assert!(
        used.len() >= 2 && used.iter().all(|i| (4..=5).contains(i)),
        "{used:?}"
    );
}

#[test]
fn gradient_radial_centre_and_edge() {
    let mut s = blank(5, 5);
    let id = red_ramp(&s);
    s.gradient(
        F0,
        (2.0, 2.0),
        (4.0, 2.0),
        GradientShape::Radial,
        GradientColors::Ramp(id),
        Dither::None,
        None,
    )
    .unwrap();
    let img = s.frame(F0).unwrap();
    assert_eq!(img.pixels[2 * 5 + 2], 1);
    assert_eq!(img.pixels[2 * 5 + 4], 8);
    assert_eq!(img.pixels[2 * 5], 8, "beyond the radius clamps to the end");
    assert!(
        (4..=5).contains(&img.pixels[2 * 5 + 3]),
        "halfway: {}",
        img.pixels[2 * 5 + 3]
    );
}

#[test]
fn gradient_errors() {
    let mut s = blank(4, 4);
    let nan = f32::NAN;
    assert!(
        s.gradient(
            F0,
            (nan, 0.0),
            (1.0, 1.0),
            GradientShape::Linear,
            GradientColors::Two(1, 2),
            Dither::None,
            None
        )
        .is_err()
    );
    assert!(
        s.gradient(
            F0,
            (0.0, 0.0),
            (1.0, 1.0),
            GradientShape::Linear,
            GradientColors::Two(0, 2),
            Dither::None,
            None
        )
        .is_err()
    );
    assert!(
        s.gradient(
            F0,
            (0.0, 0.0),
            (1.0, 1.0),
            GradientShape::Linear,
            GradientColors::Ramp(capy_paint::RampId(999)),
            Dither::None,
            None
        )
        .is_err()
    );
    assert!(!s.can_undo());
}

// ---- shade ----

#[test]
fn shade_darken_and_lighten_clamp_at_ramp_ends() {
    let mut s = session(&["18.4"]);
    s.shade(F0, &[(0, 0), (3, 0)], sq(1), -1, None).unwrap();
    assert_eq!(
        render(&s),
        ["17.3"],
        "1 is already the darkest; transparent untouched"
    );

    let mut s = session(&["18.4"]);
    s.shade(F0, &[(0, 0), (3, 0)], sq(1), 1, None).unwrap();
    assert_eq!(render(&s), ["28.5"], "8 is already the lightest");

    let mut s = session(&["18.4"]);
    s.shade(F0, &[(0, 0), (3, 0)], sq(1), -100, None).unwrap();
    assert_eq!(render(&s), ["11.1"]);
    let mut s = session(&["18.4"]);
    s.shade(F0, &[(0, 0), (3, 0)], sq(1), 100, None).unwrap();
    assert_eq!(render(&s), ["88.8"]);

    // A single point with a bigger brush shades only what it covers, once.
    let mut s = session(&["444", "444", "444"]);
    s.shade(F0, &[(1, 1), (1, 1)], round(3), 2, None).unwrap();
    assert_eq!(render(&s), ["464", "666", "464"]);
}

// ---- outline ----

const DOT: [&str; 5] = [".....", ".....", "..5..", ".....", "....."];

#[test]
fn outline_outside_with_and_without_diagonals() {
    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Outside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), [".....", "..2..", ".252.", "..2..", "....."]);

    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Outside,
        1,
        true,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), [".....", ".222.", ".252.", ".222.", "....."]);

    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Outside,
        2,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["..2..", ".222.", "22522", ".222.", "..2.."]);

    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Outside,
        2,
        true,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["22222", "22222", "22522", "22222", "22222"]);
}

#[test]
fn outline_inside() {
    let block = [".....", ".555.", ".555.", ".555.", "....."];
    let mut s = session(&block);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Inside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), [".....", ".222.", ".252.", ".222.", "....."]);

    // The frame edge counts as outside, so a cropped sprite is outlined all round.
    let mut s = session(&["555", "555", "555"]);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Inside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["222", "252", "222"]);

    // Thickness 2 on a 5x5 block leaves only the centre.
    let mut s = session(&["55555"; 5]);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Inside,
        2,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["22222", "22222", "22522", "22222", "22222"]);

    // Diagonals: a single pixel is a border by itself.
    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Inside,
        1,
        true,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), [".....", ".....", "..2..", ".....", "....."]);
}

#[test]
fn outline_auto_darker_uses_the_neighbouring_ramp() {
    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::AutoDarker(2),
        OutlineSide::Outside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), [".....", "..3..", ".353.", "..3..", "....."]);

    // Each ring pixel follows its own neighbour.
    let mut s = session(&["....", ".26.", "...."]);
    s.outline(
        F0,
        OutlineColor::AutoDarker(1),
        OutlineSide::Outside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), [".15.", "1265", ".15."]);

    // Inside: the border recolours itself, one step darker.
    let mut s = session(&["555", "555", "555"]);
    s.outline(
        F0,
        OutlineColor::AutoDarker(1),
        OutlineSide::Inside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["444", "454", "444"]);

    // Already darkest: clamped.
    let mut s = session(&["1"]);
    s.outline(
        F0,
        OutlineColor::AutoDarker(3),
        OutlineSide::Inside,
        1,
        false,
        None,
    )
    .unwrap();
    assert_eq!(render(&s), ["1"]);
}

#[test]
fn outline_with_a_mask_outlines_the_mask() {
    let mut s = blank(5, 5);
    let m = Mask::rect(5, 5, 2, 2, 2, 2);
    s.outline(
        F0,
        OutlineColor::Fixed(3),
        OutlineSide::Outside,
        1,
        false,
        Some(&m),
    )
    .unwrap();
    assert_eq!(render(&s), [".....", "..3..", ".3.3.", "..3..", "....."]);

    let mut s = blank(5, 5);
    let m = Mask::rect(5, 5, 1, 1, 3, 3);
    s.outline(
        F0,
        OutlineColor::Fixed(3),
        OutlineSide::Inside,
        1,
        true,
        Some(&m),
    )
    .unwrap();
    assert_eq!(render(&s), [".....", ".333.", ".3.3.", ".333.", "....."]);
}

#[test]
fn outline_edge_cases() {
    let mut s = session(&DOT);
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Outside,
        0,
        false,
        None,
    )
    .unwrap();
    assert!(!s.can_undo());
    assert!(
        s.outline(
            F0,
            OutlineColor::Fixed(0),
            OutlineSide::Outside,
            1,
            false,
            None
        )
        .is_err()
    );
    // Thickness is capped at 8 and the frame border clips the ring.
    s.outline(
        F0,
        OutlineColor::Fixed(2),
        OutlineSide::Outside,
        99,
        true,
        None,
    )
    .unwrap();
    assert!(s.frame(F0).unwrap().alpha.iter().all(|&a| a == 255));
}

// ---- clean_strays ----

#[test]
fn clean_strays_removes_specks_and_fills_holes() {
    let mut s = session(&["1....", ".....", "..22.", "..22.", "....."]);
    s.clean_strays(F0, None).unwrap();
    assert_eq!(render(&s), [".....", ".....", "..22.", "..22.", "....."]);

    let mut s = session(&["111", "1.1", "111"]);
    s.clean_strays(F0, None).unwrap();
    assert_eq!(render(&s), ["111", "111", "111"]);

    // No majority: the first neighbour in N, W, E, S order wins.
    let mut s = session(&["212", "3.4", "565"]);
    s.clean_strays(F0, None).unwrap();
    assert_eq!(render(&s), ["212", "314", "565"]);
    // Majority wins.
    let mut s = session(&["212", "2.2", "232"]);
    s.clean_strays(F0, None).unwrap();
    assert_eq!(render(&s), ["212", "222", "232"]);
    // A transparent pixel on the border is not a hole (a neighbour is missing).
    let mut s = session(&["111", "1.1"]);
    s.clean_strays(F0, None).unwrap();
    assert_eq!(render(&s), ["111", "1.1"]);
}

// ---- selection content ----

#[test]
fn magic_wand_contiguous_and_global() {
    let s = session(&["11.", "1.2", "..2"]);
    let m = s.magic_wand(F0, (0, 0), FillMode::Contiguous);
    assert_eq!(m.count(), 3);
    assert!(m.get(0, 0) && m.get(1, 0) && m.get(0, 1));
    let m = s.magic_wand(F0, (2, 1), FillMode::Global);
    assert_eq!((m.count(), m.get(2, 1), m.get(2, 2)), (2, true, true));
    let m = s.magic_wand(F0, (2, 0), FillMode::Contiguous);
    assert_eq!(m.count(), 1, "(2,0) is walled in by 1 and 2");
    let m = s.magic_wand(F0, (2, 0), FillMode::Global);
    assert_eq!(m.count(), 4, "all transparent pixels");
    assert!(s.magic_wand(F0, (9, 9), FillMode::Global).is_empty());
    assert!(
        s.magic_wand(FrameRef { dir: 9, frame: 9 }, (0, 0), FillMode::Global)
            .is_empty()
    );
}

#[test]
fn copy_cut_paste() {
    let art = [".....", ".12..", ".34..", "....."];
    let mut s = session(&art);
    let m = Mask::rect(5, 4, 1, 1, 2, 2);
    let clip = s.copy(F0, &m);
    assert_eq!(
        (clip.width, clip.height, clip.offset_x, clip.offset_y),
        (2, 2, 1, 1)
    );
    assert_eq!(render_img(&clip), ["12", "34"]);
    assert_eq!(render(&s), art, "copy does not modify");

    let cut = s.cut(F0, &m).unwrap();
    assert_eq!(cut, clip);
    assert_eq!(render(&s), [".....", ".....", ".....", "....."]);

    let written = s.paste(F0, &clip, (3, 2)).unwrap();
    assert_eq!(render(&s), [".....", ".....", "...12", "...34"]);
    assert_eq!(written.bounds(), Some((3, 2, 2, 2)));

    // Transparent clip pixels do not overwrite; pixels outside the frame are clipped.
    let hole = image(&["1.", ".2"]);
    let w = s.paste(F0, &hole, (3, 3)).unwrap();
    assert_eq!(render(&s), [".....", ".....", "...12", "...14"]);
    assert_eq!(w.count(), 1);
    assert_eq!(w.bounds(), Some((3, 3, 1, 1)));

    // Partial selection: unselected pixels are transparent in the clip.
    let s = session(&["12", "34"]);
    let mut diag = Mask::none(2, 2);
    diag.set(0, 0, true);
    diag.set(1, 1, true);
    assert_eq!(render_img(&s.copy(F0, &diag)), ["1.", ".4"]);
    assert_eq!(s.copy(F0, &Mask::none(2, 2)).width, 0);
}

#[test]
fn paste_rgba_maps_to_the_palette_without_touching_transparency() {
    let mut s = session(&["999", "999"]);
    #[rustfmt::skip]
    let rgba = [
        60, 10, 10, 255,    0, 0, 0, 0,
        10, 10, 228, 255,   0, 0, 0, 0,
    ];
    let w = s.paste_rgba(F0, 2, 2, &rgba, (0, 0), Dither::None).unwrap();
    assert_eq!(render(&s), ["199", "o99"]);
    assert_eq!(w.count(), 2);
}

#[test]
fn paste_rgba_never_produces_index_zero_and_validates_length() {
    let mut s = blank(4, 4);
    let black = [0u8, 0, 0, 255].repeat(4);
    for dither in [Dither::None, Dither::Bayer4, Dither::FloydSteinberg] {
        s.paste_rgba(F0, 2, 2, &black, (1, 1), dither).unwrap();
        assert_locked(&s);
        assert_eq!(
            s.frame(F0)
                .unwrap()
                .alpha
                .iter()
                .filter(|&&a| a == 255)
                .count(),
            4
        );
    }
    assert!(matches!(
        s.paste_rgba(F0, 2, 2, &black[..15], (0, 0), Dither::None),
        Err(PaintError::Invalid(_))
    ));
    // Alpha below 128 is transparent.
    let mut s = blank(2, 1);
    s.paste_rgba(
        F0,
        2,
        1,
        &[60, 10, 10, 127, 60, 10, 10, 128],
        (0, 0),
        Dither::None,
    )
    .unwrap();
    assert_eq!(render(&s), [".1"]);
}

#[test]
fn move_selection_lifts_and_drops() {
    let mut s = session(&[".1.", "...", "..."]);
    let m = Mask::rect(3, 3, 1, 0, 1, 0);
    let moved = s.move_selection(F0, &m, 1, 1).unwrap();
    assert_eq!(render(&s), ["...", "..1", "..."]);
    assert!(moved.get(2, 1) && moved.count() == 1);

    // Transparent selected pixels do not overwrite the destination.
    let mut s = session(&["1.2"]);
    let m = Mask::rect(3, 1, 0, 0, 1, 0);
    s.move_selection(F0, &m, 1, 0).unwrap();
    assert_eq!(render(&s), [".12"]);

    // Pushed off the frame: lost.
    let mut s = session(&["12"]);
    let m = Mask::all(2, 1);
    s.move_selection(F0, &m, 1, 0).unwrap();
    assert_eq!(render(&s), [".1"]);
    s.move_selection(F0, &Mask::all(2, 1), 100, 100).unwrap();
    assert_eq!(render(&s), [".."]);
}

#[test]
fn flip_whole_frame_and_selection() {
    let mut s = session(&["12.", "3.4"]);
    s.flip(F0, None, true).unwrap();
    assert_eq!(render(&s), [".21", "4.3"]);
    s.flip(F0, None, false).unwrap();
    assert_eq!(render(&s), ["4.3", ".21"]);

    let art = ["123", "456", "789"];
    let m = Mask::rect(3, 3, 0, 0, 1, 1);
    let mut s = session(&art);
    s.flip(F0, Some(&m), true).unwrap();
    assert_eq!(render(&s), ["213", "546", "789"]);
    let mut s = session(&art);
    s.flip(F0, Some(&m), false).unwrap();
    assert_eq!(render(&s), ["453", "126", "789"]);
    // Empty selection: nothing.
    s.flip(F0, Some(&Mask::none(3, 3)), true).unwrap();
}

#[test]
fn rotate90_whole_frame_and_selection() {
    let mut s = session(&["123", "456"]);
    s.rotate90(F0, None, true).unwrap();
    assert_eq!(render(&s), ["41", "52", "63"]);
    let img = s.frame(F0).unwrap();
    assert_eq!(
        (img.width, img.height, img.offset_x, img.offset_y),
        (2, 3, 0, -1)
    );

    let mut s = session(&["123", "456"]);
    s.rotate90(F0, None, false).unwrap();
    assert_eq!(render(&s), ["36", "25", "14"]);

    // Four quarter turns are the identity on the pixels.
    let mut s = session(&["12.", "3.4"]);
    for _ in 0..4 {
        s.rotate90(F0, None, true).unwrap();
    }
    assert_eq!(render(&s), ["12.", "3.4"]);

    let mut s = session(&["123", "456", "789"]);
    let m = Mask::rect(3, 3, 0, 0, 1, 1);
    s.rotate90(F0, Some(&m), true).unwrap();
    assert_eq!(render(&s), ["413", "526", "789"]);
}

// ---- frame structure ----

#[test]
fn resize_canvas_anchors_and_offsets() {
    let make = || {
        let mut img = image(&["12", "34"]);
        img.offset_x = 10;
        img.offset_y = 20;
        PaintSession::new(vec![vec![img]], test_palette()).unwrap()
    };
    let off = |s: &PaintSession| {
        let i = s.frame(F0).unwrap();
        (i.offset_x, i.offset_y)
    };
    let mut s = make();
    s.resize_canvas(F0, 4, 4, (-1, -1)).unwrap();
    assert_eq!(render(&s), ["12..", "34..", "....", "...."]);
    assert_eq!(off(&s), (10, 20));

    let mut s = make();
    s.resize_canvas(F0, 4, 4, (1, 1)).unwrap();
    assert_eq!(render(&s), ["....", "....", "..12", "..34"]);
    assert_eq!(off(&s), (8, 18));

    let mut s = make();
    s.resize_canvas(F0, 4, 4, (0, 0)).unwrap();
    assert_eq!(render(&s), ["....", ".12.", ".34.", "...."]);
    assert_eq!(off(&s), (9, 19));

    // Shrinking crops from the opposite side of the anchor.
    let mut s = session(&["1234", "5678", "9abc", "defg"]);
    s.resize_canvas(F0, 2, 2, (1, 1)).unwrap();
    assert_eq!(render(&s), ["bc", "fg"]);
    s.undo();
    assert_eq!(render(&s), ["1234", "5678", "9abc", "defg"]);

    assert!(matches!(
        s.resize_canvas(F0, 0, 4, (0, 0)),
        Err(PaintError::Invalid(_))
    ));
    assert!(s.resize_canvas(F0, 100_000, 4, (0, 0)).is_err());
}

#[test]
fn set_offset() {
    let mut s = session(&["1"]);
    s.set_offset(F0, -3, 7).unwrap();
    let i = s.frame(F0).unwrap();
    assert_eq!((i.offset_x, i.offset_y), (-3, 7));
    assert!(s.undo());
    assert_eq!(s.frame(F0).unwrap().offset_x, 0);
}

fn two_by_three() -> PaintSession {
    let f = |i: u8| image(&[&char::from_digit(u32::from(i), 36).unwrap().to_string()]);
    PaintSession::new(
        vec![vec![f(1), f(2), f(3)], vec![f(4), f(5), f(6)]],
        test_palette(),
    )
    .unwrap()
}

fn column(s: &PaintSession, dir: usize) -> String {
    s.frames()[dir]
        .iter()
        .map(|img| render_img(img).concat())
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn frame_insert_delete_move() {
    let mut s = two_by_three();
    s.insert_frame(1, None).unwrap();
    assert_eq!(
        (column(&s, 0), column(&s, 1)),
        ("1.23".to_string(), "4.56".to_string())
    );
    assert!(s.undo());
    assert_eq!(column(&s, 0), "123");

    s.insert_frame(0, Some(2)).unwrap();
    assert_eq!(
        (column(&s, 0), column(&s, 1)),
        ("3123".to_string(), "6456".to_string())
    );
    s.insert_frame(4, Some(0)).unwrap();
    assert_eq!(column(&s, 0), "31233");
    assert!(s.undo() && s.undo());

    s.delete_frame(1).unwrap();
    assert_eq!(
        (column(&s, 0), column(&s, 1)),
        ("13".to_string(), "46".to_string())
    );
    assert!(s.undo());
    assert_eq!(column(&s, 0), "123");

    s.move_frame(0, 2).unwrap();
    assert_eq!(
        (column(&s, 0), column(&s, 1)),
        ("231".to_string(), "564".to_string())
    );
    assert!(s.undo());
    s.move_frame(2, 0).unwrap();
    assert_eq!(column(&s, 0), "312");
    assert!(s.undo());
    assert_eq!(column(&s, 0), "123");
    s.move_frame(1, 1).unwrap();
    assert!(s.can_redo(), "a no-op does not discard the redo history");

    // Redo replays them.
    assert!(s.redo());
    assert_eq!(column(&s, 0), "312");
}

#[test]
fn frame_structure_errors() {
    let mut s = two_by_three();
    assert!(matches!(s.delete_frame(3), Err(PaintError::NoFrame(_))));
    assert!(matches!(
        s.insert_frame(4, None),
        Err(PaintError::NoFrame(_))
    ));
    assert!(matches!(
        s.insert_frame(0, Some(3)),
        Err(PaintError::NoFrame(_))
    ));
    assert!(matches!(s.move_frame(0, 3), Err(PaintError::NoFrame(_))));
    s.delete_frame(0).unwrap();
    s.delete_frame(0).unwrap();
    assert!(
        matches!(s.delete_frame(0), Err(PaintError::Invalid(_))),
        "last frame stays"
    );
    assert_eq!(column(&s, 1), "6");
    // Blank frames copy the size and offset of their neighbour.
    let mut img = image(&["12", "34"]);
    img.offset_x = -5;
    let mut s = PaintSession::new(vec![vec![img]], test_palette()).unwrap();
    s.insert_frame(1, None).unwrap();
    let blank = &s.frames()[0][1];
    assert_eq!((blank.width, blank.height, blank.offset_x), (2, 2, -5));
    assert!(blank.alpha.iter().all(|&a| a == 0));
}

#[test]
fn mirror_direction_flips_about_the_anchor() {
    let mut img = image(&["12.", "3.."]);
    img.offset_x = -1;
    img.offset_y = -4;
    let other = image(&["9"]);
    let mut s = PaintSession::new(vec![vec![img.clone()], vec![other]], test_palette()).unwrap();
    s.mirror_direction(0, 1).unwrap();
    let m = &s.frames()[1][0];
    assert_eq!(render_img(m), [".21", "..3"]);
    assert_eq!((m.width, m.height, m.offset_x, m.offset_y), (3, 2, -2, -4));
    assert_eq!(s.frames()[0][0], img, "the source is untouched");

    // Mirroring the mirror back reproduces the original: already equal, so no step.
    let rev = s.revision();
    s.mirror_direction(1, 0).unwrap();
    assert_eq!(s.frames()[0][0], img);
    assert_eq!(s.revision(), rev);

    assert!(matches!(
        s.mirror_direction(0, 0),
        Err(PaintError::Invalid(_))
    ));
    assert!(matches!(
        s.mirror_direction(0, 5),
        Err(PaintError::NoFrame(_))
    ));
    assert!(s.undo());
    assert_eq!(render_img(&s.frames()[1][0]), ["9"]);
    assert!(!s.undo());
}

// ---- masks restrict every tool ----

const FILLED: [&str; 4] = ["1.2.3.", ".4.5.6", "7.8.9.", ".1.2.3"];

fn left_half() -> Mask {
    Mask::rect(6, 4, 0, 0, 2, 3)
}

fn check_restricted(name: &str, op: impl Fn(&mut PaintSession, &Mask)) {
    let mut s = session(&FILLED);
    let before = s.frame(F0).unwrap().clone();
    let m = left_half();
    op(&mut s, &m);
    let after = s.frame(F0).unwrap();
    assert_ne!(
        *after, before,
        "{name}: the operation should change something"
    );
    for y in 0..4i32 {
        for x in 3..6i32 {
            let i = (y * 6 + x) as usize;
            assert_eq!(
                (after.pixels[i], after.alpha[i]),
                (before.pixels[i], before.alpha[i]),
                "{name}: pixel ({x},{y}) outside the mask changed"
            );
        }
    }
    assert_locked(&s);
}

#[test]
fn masks_restrict_every_tool() {
    check_restricted("pencil", |s, m| {
        s.pencil(F0, &[(0, 0), (5, 3)], sq(3), p(9), Some(m))
            .unwrap()
    });
    check_restricted("line", |s, m| {
        s.line(F0, (0, 3), (5, 0), sq(2), p(9), Some(m)).unwrap()
    });
    check_restricted("rect outline", |s, m| {
        s.rectangle(
            F0,
            (0, 0),
            (5, 3),
            ShapeStyle::Outline,
            sq(1),
            p(9),
            Some(m),
        )
        .unwrap();
    });
    check_restricted("rect filled", |s, m| {
        s.rectangle(F0, (0, 0), (5, 3), ShapeStyle::Filled, sq(1), p(9), Some(m))
            .unwrap();
    });
    check_restricted("ellipse outline", |s, m| {
        s.ellipse(
            F0,
            (0, 0),
            (5, 3),
            ShapeStyle::Outline,
            sq(1),
            p(9),
            Some(m),
        )
        .unwrap();
    });
    check_restricted("ellipse filled", |s, m| {
        s.ellipse(F0, (0, 0), (5, 3), ShapeStyle::Filled, sq(1), p(9), Some(m))
            .unwrap();
    });
    check_restricted("fill global", |s, m| {
        s.fill(F0, (1, 0), p(9), FillMode::Global, Some(m)).unwrap();
    });
    check_restricted("fill contiguous", |s, m| {
        s.fill(F0, (1, 0), p(9), FillMode::Contiguous, Some(m))
            .unwrap();
    });
    check_restricted("replace colour", |s, m| {
        s.replace_color(Some(F0), Paint::Transparent, p(9), Some(m))
            .unwrap();
    });
    check_restricted("replace colour (all)", |s, m| {
        s.replace_color(None, p(1), p(9), Some(m)).unwrap();
    });
    for dither in [Dither::None, Dither::Bayer4, Dither::FloydSteinberg] {
        check_restricted("gradient", |s, m| {
            let (id, _) = s.ramps().ramp_of(1).unwrap();
            s.gradient(
                F0,
                (0.0, 0.0),
                (5.0, 0.0),
                GradientShape::Linear,
                GradientColors::Ramp(id),
                dither,
                Some(m),
            )
            .unwrap();
        });
    }
    check_restricted("shade", |s, m| {
        s.shade(F0, &[(0, 0), (5, 3)], sq(3), 2, Some(m)).unwrap()
    });
    check_restricted("outline inside", |s, m| {
        s.outline(
            F0,
            OutlineColor::Fixed(9),
            OutlineSide::Inside,
            1,
            true,
            Some(m),
        )
        .unwrap();
    });
    check_restricted("clean strays", |s, m| s.clean_strays(F0, Some(m)).unwrap());
}

#[test]
fn masked_content_moves_stay_inside_the_mask_box() {
    let art = ["123", "456", "789"];
    let m = Mask::rect(3, 3, 0, 0, 1, 1);
    for op in 0..4 {
        let mut s = session(&art);
        match op {
            0 => s.flip(F0, Some(&m), true).unwrap(),
            1 => s.flip(F0, Some(&m), false).unwrap(),
            2 => s.rotate90(F0, Some(&m), true).unwrap(),
            _ => s.rotate90(F0, Some(&m), false).unwrap(),
        }
        let r = render(&s);
        assert_eq!(
            (&r[2][..], &r[0][2..], &r[1][2..]),
            ("789", "3", "6"),
            "op {op}"
        );
    }
}

#[test]
fn extreme_inputs_do_not_panic() {
    let mut s = blank(6, 6);
    let big = i32::MAX;
    let small = i32::MIN;
    let b = sq(u32::MAX);
    s.pencil(F0, &[(small, small), (big, big), (0, 0)], b, p(3), None)
        .unwrap();
    s.line(F0, (small, 0), (big, 5), b, p(3), None).unwrap();
    s.rectangle(
        F0,
        (small, small),
        (big, big),
        ShapeStyle::Outline,
        b,
        p(3),
        None,
    )
    .unwrap();
    s.ellipse(
        F0,
        (small, small),
        (big, big),
        ShapeStyle::Filled,
        b,
        p(3),
        None,
    )
    .unwrap();
    s.ellipse(
        F0,
        (small, small),
        (big, big),
        ShapeStyle::Outline,
        round(7),
        p(3),
        None,
    )
    .unwrap();
    s.fill(F0, (small, big), p(3), FillMode::Global, None)
        .unwrap();
    s.shade(F0, &[(small, big), (big, small)], b, i32::MIN, None)
        .unwrap();
    s.gradient(
        F0,
        (-1e30, 0.0),
        (1e30, 1e30),
        GradientShape::Radial,
        GradientColors::Two(1, 2),
        Dither::FloydSteinberg,
        None,
    )
    .unwrap();
    let clip = image(&["12", "34"]);
    s.paste(F0, &clip, (big, small)).unwrap();
    s.paste(F0, &clip, (small, big)).unwrap();
    s.move_selection(F0, &Mask::all(6, 6), big, small).unwrap();
    s.move_selection(F0, &Mask::all(2, 2), small, big).unwrap();
    s.set_offset(F0, small, big).unwrap();
    s.rotate90(F0, None, true).unwrap();
    s.resize_canvas(F0, 3, 3, (big, small)).unwrap();
    s.insert_frame(0, None).unwrap();
    s.mirror_direction(0, 0).unwrap_err();
    // A mask that does not match the frame is read as "unselected outside".
    let wrong = Mask::all(1, 1);
    s.pencil(F0, &[(0, 0), (2, 2)], sq(1), p(2), Some(&wrong))
        .unwrap();
    s.flip(F0, Some(&Mask::all(40, 40)), true).unwrap();
    s.rotate90(F0, Some(&Mask::all(40, 40)), true).unwrap();
    assert_locked(&s);

    // Degenerate sessions.
    assert!(
        PaintSession::new(
            vec![vec![IndexedImage::new(1, 1, 0, 0)], vec![]],
            test_palette()
        )
        .is_err()
    );
    let mut bad = IndexedImage::new(2, 2, 0, 0);
    bad.pixels.pop();
    assert!(PaintSession::new(vec![vec![bad]], test_palette()).is_err());
    let mut empty = PaintSession::new(vec![], test_palette()).unwrap();
    assert!(empty.insert_frame(0, None).is_err());
    assert!(empty.pick(F0, (0, 0)).is_none());
}

#[test]
fn new_normalises_input() {
    let mut img = IndexedImage::new(3, 1, 0, 0);
    img.pixels = vec![0, 5, 7];
    img.alpha = vec![255, 128, 0];
    let s = PaintSession::new(vec![vec![img]], test_palette()).unwrap();
    let i = s.frame(F0).unwrap();
    assert_eq!(
        i.alpha,
        [0, 255, 0],
        "opaque index 0 -> transparent; any alpha -> 255"
    );
    assert_eq!(i.pixels, [0, 5, 0], "transparent pixels carry index 0");
}
