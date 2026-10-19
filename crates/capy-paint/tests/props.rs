//! Property tests: the palette lock holds after any sequence of operations,
//! and undo / redo obey their laws.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use capy_paint::{
    Dither, FillMode, FrameRef, GradientColors, GradientShape, Mask, OutlineColor, OutlineSide,
    Paint, PaintSession, RampId, ShapeStyle, reduce_dcc_cells,
};
use common::*;
use d2_formats::IndexedImage;
use proptest::prelude::*;

type Pt = (i32, i32);

/// A mask recipe, built against the target frame's size when applied.
#[derive(Clone, Debug)]
struct MaskSpec {
    kind: u8,
    a: Pt,
    b: Pt,
}

impl MaskSpec {
    fn build(&self, w: u32, h: u32) -> Mask {
        match self.kind % 4 {
            0 => Mask::rect(w, h, self.a.0, self.a.1, self.b.0, self.b.1),
            1 => Mask::ellipse(w, h, self.a.0, self.a.1, self.b.0, self.b.1),
            2 => Mask::lasso(
                w,
                h,
                &[
                    (self.a.0 as f32, self.a.1 as f32),
                    (self.b.0 as f32, self.a.1 as f32),
                    (self.b.0 as f32, self.b.1 as f32),
                ],
            ),
            _ => {
                let mut m = Mask::rect(w, h, self.a.0, self.a.1, self.b.0, self.b.1);
                m.invert();
                m
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Pencil(FrameRef, Vec<Pt>, u32, bool, Paint, Option<MaskSpec>),
    Line(FrameRef, Pt, Pt, u32, bool, Paint, Option<MaskSpec>),
    Rect(FrameRef, Pt, Pt, bool, u32, Paint, Option<MaskSpec>),
    Ellipse(FrameRef, Pt, Pt, bool, u32, Paint, Option<MaskSpec>),
    Fill(FrameRef, Pt, Paint, bool, Option<MaskSpec>),
    Replace(Option<FrameRef>, Paint, Paint, Option<MaskSpec>),
    Gradient(
        FrameRef,
        (f32, f32),
        (f32, f32),
        bool,
        GradientColors,
        u8,
        Option<MaskSpec>,
    ),
    Shade(FrameRef, Vec<Pt>, u32, i32, Option<MaskSpec>),
    Outline(FrameRef, Option<u8>, i32, bool, u32, bool, Option<MaskSpec>),
    Clean(FrameRef, Option<MaskSpec>),
    Cut(FrameRef, MaskSpec),
    Paste(FrameRef, FrameRef, MaskSpec, Pt),
    PasteRgba(FrameRef, Vec<u8>, Pt, u8),
    Move(FrameRef, MaskSpec, i32, i32),
    Flip(FrameRef, Option<MaskSpec>, bool),
    Rotate(FrameRef, Option<MaskSpec>, bool),
    Resize(FrameRef, u32, u32, Pt),
    Offset(FrameRef, i32, i32),
    Insert(usize, Option<usize>),
    Delete(usize),
    MoveFrame(usize, usize),
    Mirror(usize, usize),
    Reduce,
}

fn pt() -> impl Strategy<Value = Pt> {
    (-3i32..11, -3i32..9)
}

fn fref() -> BoxedStrategy<FrameRef> {
    (0usize..3, 0usize..4)
        .prop_map(|(dir, frame)| FrameRef { dir, frame })
        .boxed()
}

fn paint() -> impl Strategy<Value = Paint> {
    prop_oneof![
        1 => Just(Paint::Transparent),
        1 => Just(Paint::Index(0)),
        6 => (1u8..=255).prop_map(Paint::Index),
        6 => (1u8..=24).prop_map(Paint::Index),
    ]
}

fn mask() -> impl Strategy<Value = Option<MaskSpec>> {
    proptest::option::of(mask_spec())
}

fn mask_spec() -> impl Strategy<Value = MaskSpec> {
    (any::<u8>(), pt(), pt()).prop_map(|(kind, a, b)| MaskSpec { kind, a, b })
}

fn colors() -> impl Strategy<Value = GradientColors> {
    prop_oneof![
        (0u8..=255, 0u8..=255).prop_map(|(a, b)| GradientColors::Two(a, b)),
        (0u16..6).prop_map(|r| GradientColors::Ramp(RampId(r))),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    let f = fref();
    let strokes = prop_oneof![
        (
            f.clone(),
            proptest::collection::vec(pt(), 0..5),
            0u32..5,
            any::<bool>(),
            paint(),
            mask()
        )
            .prop_map(|(f, p, s, r, c, m)| Op::Pencil(f, p, s, r, c, m)),
        (
            f.clone(),
            pt(),
            pt(),
            0u32..5,
            any::<bool>(),
            paint(),
            mask()
        )
            .prop_map(|(f, a, b, s, r, c, m)| Op::Line(f, a, b, s, r, c, m)),
        (
            f.clone(),
            pt(),
            pt(),
            any::<bool>(),
            0u32..4,
            paint(),
            mask()
        )
            .prop_map(|(f, a, b, fl, s, c, m)| Op::Rect(f, a, b, fl, s, c, m)),
        (
            f.clone(),
            pt(),
            pt(),
            any::<bool>(),
            0u32..4,
            paint(),
            mask()
        )
            .prop_map(|(f, a, b, fl, s, c, m)| Op::Ellipse(f, a, b, fl, s, c, m)),
        (f.clone(), pt(), paint(), any::<bool>(), mask())
            .prop_map(|(f, a, c, g, m)| Op::Fill(f, a, c, g, m)),
        (proptest::option::of(f.clone()), paint(), paint(), mask())
            .prop_map(|(f, a, b, m)| Op::Replace(f, a, b, m)),
    ];
    let effects = prop_oneof![
        (
            f.clone(),
            (-4.0f32..12.0, -4.0f32..10.0),
            (-4.0f32..12.0, -4.0f32..10.0),
            any::<bool>(),
            colors(),
            0u8..3,
            mask()
        )
            .prop_map(|(f, a, b, s, c, d, m)| Op::Gradient(f, a, b, s, c, d, m)),
        (
            f.clone(),
            proptest::collection::vec(pt(), 0..4),
            1u32..4,
            -4i32..5,
            mask()
        )
            .prop_map(|(f, p, s, n, m)| Op::Shade(f, p, s, n, m)),
        (
            f.clone(),
            proptest::option::of(0u8..=30),
            -3i32..4,
            any::<bool>(),
            0u32..10,
            any::<bool>(),
            mask()
        )
            .prop_map(|(f, c, n, i, t, d, m)| Op::Outline(f, c, n, i, t, d, m)),
        (f.clone(), mask()).prop_map(|(f, m)| Op::Clean(f, m)),
    ];
    let content = prop_oneof![
        (f.clone(), mask_spec()).prop_map(|(f, m)| Op::Cut(f, m)),
        (f.clone(), f.clone(), mask_spec(), pt()).prop_map(|(a, b, m, p)| Op::Paste(a, b, m, p)),
        (
            f.clone(),
            proptest::collection::vec(any::<u8>(), 4 * 12),
            pt(),
            0u8..3
        )
            .prop_map(|(f, d, p, dt)| Op::PasteRgba(f, d, p, dt)),
        (f.clone(), mask_spec(), -4i32..5, -4i32..5).prop_map(|(f, m, x, y)| Op::Move(f, m, x, y)),
        (f.clone(), mask(), any::<bool>()).prop_map(|(f, m, h)| Op::Flip(f, m, h)),
        (f.clone(), mask(), any::<bool>()).prop_map(|(f, m, c)| Op::Rotate(f, m, c)),
    ];
    let structure = prop_oneof![
        (f.clone(), 0u32..14, 0u32..12, (-1i32..2, -1i32..2))
            .prop_map(|(f, w, h, a)| Op::Resize(f, w, h, a)),
        (f, -5i32..5, -5i32..5).prop_map(|(f, x, y)| Op::Offset(f, x, y)),
        (0usize..5, proptest::option::of(0usize..4)).prop_map(|(a, c)| Op::Insert(a, c)),
        (0usize..4).prop_map(Op::Delete),
        (0usize..4, 0usize..4).prop_map(|(a, b)| Op::MoveFrame(a, b)),
        (0usize..3, 0usize..3).prop_map(|(a, b)| Op::Mirror(a, b)),
        Just(Op::Reduce),
    ];
    prop_oneof![4 => strokes, 3 => effects, 3 => content, 2 => structure]
}

fn dims(s: &PaintSession, f: FrameRef) -> (u32, u32) {
    s.frame(f).map_or((8, 6), |i| (i.width, i.height))
}

fn apply(s: &mut PaintSession, op: &Op) {
    use capy_paint::{Brush, BrushShape};
    let brush = |size: u32, round: bool| Brush {
        size,
        shape: if round {
            BrushShape::Round
        } else {
            BrushShape::Square
        },
    };
    let style = |filled: bool| {
        if filled {
            ShapeStyle::Filled
        } else {
            ShapeStyle::Outline
        }
    };
    // Failures (bad frame, index 0, ...) are part of the input space.
    match op {
        Op::Pencil(f, pts, size, r, c, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.pencil(*f, pts, brush(*size, *r), *c, m.as_ref());
        }
        Op::Line(f, a, b, size, r, c, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.line(*f, *a, *b, brush(*size, *r), *c, m.as_ref());
        }
        Op::Rect(f, a, b, fl, size, c, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.rectangle(*f, *a, *b, style(*fl), brush(*size, false), *c, m.as_ref());
        }
        Op::Ellipse(f, a, b, fl, size, c, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.ellipse(*f, *a, *b, style(*fl), brush(*size, true), *c, m.as_ref());
        }
        Op::Fill(f, a, c, g, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let mode = if *g {
                FillMode::Global
            } else {
                FillMode::Contiguous
            };
            let _ = s.fill(*f, *a, *c, mode, m.as_ref());
        }
        Op::Replace(f, a, b, m) => {
            let (w, h) = f.map_or((8, 6), |f| dims(s, f));
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.replace_color(*f, *a, *b, m.as_ref());
        }
        Op::Gradient(f, a, b, shape, cols, d, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let shape = if *shape {
                GradientShape::Radial
            } else {
                GradientShape::Linear
            };
            let dither =
                [Dither::None, Dither::Bayer4, Dither::FloydSteinberg][usize::from(*d) % 3];
            let _ = s.gradient(*f, *a, *b, shape, cols.clone(), dither, m.as_ref());
        }
        Op::Shade(f, pts, size, n, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.shade(*f, pts, brush(*size, true), *n, m.as_ref());
        }
        Op::Outline(f, c, n, inside, t, d, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let color = c.map_or(OutlineColor::AutoDarker(*n), OutlineColor::Fixed);
            let side = if *inside {
                OutlineSide::Inside
            } else {
                OutlineSide::Outside
            };
            let _ = s.outline(*f, color, side, *t, *d, m.as_ref());
        }
        Op::Clean(f, m) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.clean_strays(*f, m.as_ref());
        }
        Op::Cut(f, m) => {
            let (w, h) = dims(s, *f);
            let _ = s.cut(*f, &m.build(w, h));
        }
        Op::Paste(f, from, m, at) => {
            let (w, h) = dims(s, *from);
            let clip = s.copy(*from, &m.build(w, h));
            let _ = s.paste(*f, &clip, *at);
        }
        Op::PasteRgba(f, data, at, d) => {
            let dither =
                [Dither::None, Dither::Bayer4, Dither::FloydSteinberg][usize::from(*d) % 3];
            let _ = s.paste_rgba(*f, 4, 12, data, *at, dither);
        }
        Op::Move(f, m, dx, dy) => {
            let (w, h) = dims(s, *f);
            let _ = s.move_selection(*f, &m.build(w, h), *dx, *dy);
        }
        Op::Flip(f, m, hz) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.flip(*f, m.as_ref(), *hz);
        }
        Op::Rotate(f, m, cw) => {
            let (w, h) = dims(s, *f);
            let m = m.as_ref().map(|m| m.build(w, h));
            let _ = s.rotate90(*f, m.as_ref(), *cw);
        }
        Op::Resize(f, w, h, a) => {
            let _ = s.resize_canvas(*f, *w, *h, *a);
        }
        Op::Offset(f, x, y) => {
            let _ = s.set_offset(*f, *x, *y);
        }
        Op::Insert(at, c) => {
            let _ = s.insert_frame(*at, *c);
        }
        Op::Delete(i) => {
            let _ = s.delete_frame(*i);
        }
        Op::MoveFrame(a, b) => {
            let _ = s.move_frame(*a, *b);
        }
        Op::Mirror(a, b) => {
            let _ = s.mirror_direction(*a, *b);
        }
        Op::Reduce => {
            let _ = reduce_dcc_cells(s);
        }
    }
}

fn arb_frames() -> impl Strategy<Value = Vec<Vec<IndexedImage>>> {
    proptest::collection::vec(0u8..40, 2 * 3 * 8 * 6).prop_map(|px| {
        let mut it = px.into_iter();
        (0..2)
            .map(|_| {
                (0..3)
                    .map(|_| {
                        let mut img = IndexedImage::new(8, 6, 0, 0);
                        for i in 0..48 {
                            let v = it.next().unwrap_or(0);
                            img.pixels[i] = v;
                            img.alpha[i] = if v == 0 { 0 } else { 255 };
                        }
                        img
                    })
                    .collect()
            })
            .collect()
    })
}

fn assert_shape(s: &PaintSession) {
    let n = s.frames().first().map_or(0, Vec::len);
    assert!(
        s.frames().iter().all(|d| d.len() == n),
        "frame counts diverged"
    );
    assert_locked(s);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// After every operation (successful or not) each pixel is transparent or
    /// a palette index 1..=255, with consistent buffers.
    #[test]
    fn palette_lock_holds_after_random_operations(
        frames in arb_frames(),
        ops in proptest::collection::vec(op(), 1..30),
    ) {
        let mut s = PaintSession::new(frames, test_palette()).unwrap();
        assert_shape(&s);
        for o in &ops {
            apply(&mut s, o);
            assert_shape(&s);
        }
        while s.undo() {
            assert_shape(&s);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// undo-all restores the original, redo-all reproduces the final state,
    /// and undo then redo is the identity at every step. Units are single
    /// operations or groups of 1-3 operations (one undo step each).
    #[test]
    fn undo_redo_laws(
        frames in arb_frames(),
        units in proptest::collection::vec((any::<bool>(), proptest::collection::vec(op(), 1..4)), 1..14),
    ) {
        let mut s = PaintSession::new(frames, test_palette()).unwrap();
        let initial = s.frames().to_vec();
        prop_assert!(!s.is_dirty());
        let mut states = vec![initial.clone()];
        let record = |s: &mut PaintSession, before: u64, states: &mut Vec<Vec<Vec<IndexedImage>>>| {
            if s.revision() != before {
                states.push(s.frames().to_vec());
            }
        };
        for (grouped, ops) in &units {
            if *grouped {
                let before = s.revision();
                s.begin_group("group");
                for o in ops {
                    apply(&mut s, o);
                }
                s.end_group();
                record(&mut s, before, &mut states);
            } else {
                for o in ops {
                    let before = s.revision();
                    apply(&mut s, o);
                    record(&mut s, before, &mut states);
                }
            }
        }
        // Every recorded unit is exactly one undo step.
        let mut steps = 0;
        while s.undo() {
            steps += 1;
        }
        prop_assert_eq!(steps, states.len() - 1);
        // Everything undone: the original is back and the session is clean.
        prop_assert_eq!(s.frames(), initial.as_slice());
        prop_assert!(!s.can_undo());
        prop_assert!(!s.is_dirty());
        // Redo everything.
        let mut redone = 0;
        while s.redo() {
            redone += 1;
        }
        prop_assert_eq!(redone, steps);
        prop_assert_eq!(s.frames(), states.last().unwrap().as_slice());
        prop_assert!(!s.can_redo());

        // Step by step, back and forth: undo lands on the previous state, redo
        // returns to this one.
        for i in (1..=steps).rev() {
            prop_assert!(s.undo());
            prop_assert_eq!(s.frames(), states[i - 1].as_slice());
            prop_assert!(s.redo());
            prop_assert_eq!(s.frames(), states[i].as_slice());
            prop_assert!(s.undo());
        }
        prop_assert_eq!(s.frames(), states[0].as_slice());
    }
}

#[test]
fn dirty_tracking() {
    let mut s = blank(4, 4);
    assert!(!s.is_dirty());
    let b = sq(1);
    s.pencil(F0, &[(1, 1)], b, Paint::Index(3), None).unwrap();
    assert!(s.is_dirty());
    s.mark_saved();
    assert!(!s.is_dirty());
    s.pencil(F0, &[(2, 2)], b, Paint::Index(3), None).unwrap();
    assert!(s.is_dirty());
    assert!(s.undo());
    assert!(!s.is_dirty(), "back at the saved state");
    assert!(s.undo());
    assert!(s.is_dirty(), "before the saved state");
    assert!(s.redo());
    assert!(!s.is_dirty());
    // A new edit after undoing past the save point can never be clean again
    // by undoing back to the same depth.
    assert!(s.undo());
    s.pencil(F0, &[(0, 0)], b, Paint::Index(4), None).unwrap();
    s.mark_saved();
    s.pencil(F0, &[(3, 3)], b, Paint::Index(5), None).unwrap();
    assert!(s.undo());
    assert!(!s.is_dirty());
    // A no-op does not dirty the session.
    s.pencil(F0, &[(0, 0)], b, Paint::Index(4), None).unwrap();
    assert!(!s.is_dirty());
    // Palette swaps are not undoable but do dirty the session.
    s.set_palette(test_palette());
    assert!(s.is_dirty());
    s.mark_saved();
    assert!(!s.is_dirty());
    // A display-only swap never dirties and records nothing, but still moves
    // the revision so palette caches refresh.
    let (r, undo, redo) = (s.revision(), s.undo_label().map(str::to_owned), s.can_redo());
    let mut other = test_palette();
    other.colors[3] = [1, 2, 3];
    s.set_display_palette(other.clone());
    assert!(!s.is_dirty());
    assert_eq!(s.palette(), &other);
    assert_eq!(s.ramps().ramp_of(3), capy_paint::Ramps::build(&other).ramp_of(3));
    assert!(s.revision() > r);
    assert_eq!(s.undo_label().map(str::to_owned), undo);
    assert_eq!(s.can_redo(), redo);
    // It does not clear a pending real palette change either.
    s.set_palette(test_palette());
    s.set_display_palette(other);
    assert!(s.is_dirty());
    s.mark_saved();
    // Revision moves on every change, including undo and redo.
    let r = s.revision();
    s.undo();
    assert!(s.revision() > r);
}

#[test]
fn groups_merge_into_one_step_and_nest() {
    let mut s = blank(6, 1);
    s.begin_group("stroke");
    s.pencil(F0, &[(0, 0)], sq(1), Paint::Index(1), None)
        .unwrap();
    s.begin_group("inner");
    s.pencil(F0, &[(1, 0)], sq(1), Paint::Index(2), None)
        .unwrap();
    s.end_group();
    assert!(s.can_undo(), "pending group edits can be undone");
    s.pencil(F0, &[(2, 0)], sq(1), Paint::Index(3), None)
        .unwrap();
    s.end_group();
    assert_eq!(render(&s), ["123..."]);
    assert_eq!(s.undo_label(), Some("stroke"));
    assert!(s.undo());
    assert_eq!(render(&s), ["......"]);
    assert!(!s.can_undo());
    assert!(s.redo());
    assert_eq!(render(&s), ["123..."]);

    // An empty group and an unbalanced end leave nothing behind.
    let mut s = blank(2, 2);
    s.begin_group("nothing");
    s.end_group();
    s.end_group();
    assert!(!s.can_undo());
    // A group whose edits cancel still records one (harmless) step.
    s.begin_group("net zero");
    s.pencil(F0, &[(0, 0)], sq(1), Paint::Index(1), None)
        .unwrap();
    s.pencil(F0, &[(0, 0)], sq(1), Paint::Transparent, None)
        .unwrap();
    s.end_group();
    assert!(s.undo());
    assert!(!s.undo());

    // Undo inside an open group closes it first.
    let mut s = blank(3, 1);
    s.begin_group("open");
    s.pencil(F0, &[(0, 0)], sq(1), Paint::Index(1), None)
        .unwrap();
    assert!(s.undo());
    assert_eq!(render(&s), ["..."]);
    s.end_group(); // unbalanced now, ignored
    assert!(!s.can_undo());
}

#[test]
fn no_ops_record_nothing() {
    let mut s = session(&["11", "22"]);
    let b = sq(1);
    let rev = s.revision();
    s.pencil(F0, &[(0, 0)], b, Paint::Index(1), None).unwrap();
    s.replace_color(None, Paint::Index(7), Paint::Index(8), None)
        .unwrap();
    s.set_offset(F0, 0, 0).unwrap();
    s.resize_canvas(F0, 2, 2, (0, 0)).unwrap();
    assert!(!s.can_undo());
    assert!(!s.can_redo());
    assert!(!s.is_dirty());
    let _ = rev;
}
