//! The editing tools as plain state machines, with no egui or Bevy types so
//! they test fast: the tool list, the modifier-to-[`SelectOp`] mapping, box
//! and lasso gestures, brush footprints and the layer scope of a selection.

use capy_edit::{CellRect, LayerRef, SelectOp};

/// The tools of the toolbar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Rectangle selection.
    #[default]
    SelectBox,
    Lasso,
    /// Select every cell with the hovered cell's tile ID.
    SelectById,
    /// Select every cell identical to the hovered one.
    SelectIdentical,
    Paint,
    Erase,
    /// Pick the hovered cell as the current value.
    Eyedropper,
    /// Select, move and delete objects.
    Objects,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::SelectBox => "Select box",
            Tool::Lasso => "Lasso",
            Tool::SelectById => "Select by tile ID",
            Tool::SelectIdentical => "Select identical",
            Tool::Paint => "Paint",
            Tool::Erase => "Erase",
            Tool::Eyedropper => "Eyedropper",
            Tool::Objects => "Objects",
        }
    }

    /// Tools that change the selection.
    pub fn selects(self) -> bool {
        matches!(
            self,
            Tool::SelectBox | Tool::Lasso | Tool::SelectById | Tool::SelectIdentical
        )
    }

    /// Tools that edit cells while the button is held.
    pub fn brushes(self) -> bool {
        matches!(self, Tool::Paint | Tool::Erase)
    }
}

/// Keyboard modifiers as the tools see them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Shift adds, Ctrl subtracts, Shift+Ctrl intersects (the same as the
/// Sprites editor), nothing replaces. Alt also intersects (kept as an alias)
/// unless Shift or Ctrl alone says otherwise.
pub fn select_op(mods: Mods) -> SelectOp {
    match (mods.shift, mods.ctrl) {
        (true, true) => SelectOp::Intersect,
        (true, false) => SelectOp::Add,
        (false, true) => SelectOp::Subtract,
        (false, false) if mods.alt => SelectOp::Intersect,
        (false, false) => SelectOp::Replace,
    }
}

/// The tool-strip hint for the selection modifiers.
pub const SELECT_MODS_HINT: &str = "Shift adds, Ctrl subtracts, Shift+Ctrl (or Alt) intersects";

/// Which layers a selection tool looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectScope {
    /// Only the active layer.
    #[default]
    Active,
    /// Every visible layer of the map.
    Visible,
}

impl SelectScope {
    pub fn label(self) -> &'static str {
        match self {
            SelectScope::Active => "Active layer",
            SelectScope::Visible => "All visible layers",
        }
    }
}

/// The layers a selection tool works on: `active` for [`SelectScope::Active`],
/// every layer of `existing` that `visible` accepts otherwise.
pub fn scope_layers(
    scope: SelectScope,
    active: LayerRef,
    existing: &[LayerRef],
    visible: impl Fn(LayerRef) -> bool,
) -> Vec<LayerRef> {
    match scope {
        SelectScope::Active => vec![active],
        SelectScope::Visible => existing.iter().copied().filter(|&l| visible(l)).collect(),
    }
}

/// A rectangle being dragged out, in cells (not clamped while it is dragged).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxDrag {
    pub start: (i32, i32),
    pub current: (i32, i32),
}

impl BoxDrag {
    pub fn new(cell: (i32, i32)) -> BoxDrag {
        BoxDrag {
            start: cell,
            current: cell,
        }
    }

    pub fn update(&mut self, cell: (i32, i32)) {
        self.current = cell;
    }

    /// The dragged rectangle clipped to a `width` x `height` map, or `None`
    /// when it lies entirely outside.
    pub fn rect(&self, width: u32, height: u32) -> Option<CellRect> {
        let (x0, x1) = (
            self.start.0.min(self.current.0),
            self.start.0.max(self.current.0),
        );
        let (y0, y1) = (
            self.start.1.min(self.current.1),
            self.start.1.max(self.current.1),
        );
        let (w, h) = (i64::from(width), i64::from(height));
        if width == 0 || height == 0 || i64::from(x1) < 0 || i64::from(y1) < 0 {
            return None;
        }
        if i64::from(x0) >= w || i64::from(y0) >= h {
            return None;
        }
        let clip = |v: i32, max: i64| u32::try_from(i64::from(v).clamp(0, max - 1)).unwrap_or(0);
        Some(CellRect::new(
            (clip(x0, w), clip(y0, h)),
            (clip(x1, w), clip(y1, h)),
        ))
    }
}

/// The points of a lasso gesture, in tile space (cell `(x, y)` spans
/// `[x, x + 1]`), thinned so that a long drag stays small.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LassoPath {
    points: Vec<(f32, f32)>,
}

/// Points closer than this to the previous one (in cells) are dropped.
pub const LASSO_MIN_STEP: f32 = 0.2;

impl LassoPath {
    pub fn new(start: (f32, f32)) -> LassoPath {
        LassoPath {
            points: vec![start],
        }
    }

    /// Adds `p` unless it is too close to the last point. Returns whether it
    /// was kept.
    pub fn push(&mut self, p: (f32, f32)) -> bool {
        let Some(&(lx, ly)) = self.points.last() else {
            self.points.push(p);
            return true;
        };
        let (dx, dy) = (p.0 - lx, p.1 - ly);
        if dx * dx + dy * dy < LASSO_MIN_STEP * LASSO_MIN_STEP {
            return false;
        }
        self.points.push(p);
        true
    }

    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// A polygon needs at least three points.
    pub fn is_polygon(&self) -> bool {
        self.points.len() >= 3
    }
}

/// Every cell of the straight line from `a` to `b` (Bresenham, both ends
/// included), so a fast stroke leaves no gaps.
pub fn line_cells(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::new();
    loop {
        out.push((x, y));
        if (x, y) == b {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

/// The cells a square brush of `size` covers around `center`, inside a
/// `width` x `height` map.
pub fn brush_cells(center: (i32, i32), size: u32, width: u32, height: u32) -> Vec<(u32, u32)> {
    let size = i32::try_from(size.clamp(1, 64)).unwrap_or(1);
    let lo = -((size - 1) / 2);
    let mut out = Vec::new();
    for dy in lo..lo + size {
        for dx in lo..lo + size {
            let (x, y) = (center.0 + dx, center.1 + dy);
            if let (Ok(ux), Ok(uy)) = (u32::try_from(x), u32::try_from(y))
                && ux < width
                && uy < height
            {
                out.push((ux, uy));
            }
        }
    }
    out
}

/// The cells of a whole brush stroke segment from `from` (if any) to `to`,
/// without duplicates, in drawing order.
pub fn stroke_cells(
    from: Option<(i32, i32)>,
    to: (i32, i32),
    size: u32,
    width: u32,
    height: u32,
) -> Vec<(u32, u32)> {
    let centers = match from {
        Some(f) => line_cells(f, to),
        None => vec![to],
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in centers {
        for cell in brush_cells(c, size, width, height) {
            if seen.insert(cell) {
                out.push(cell);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_map_to_select_ops() {
        let none = Mods::default();
        assert_eq!(select_op(none), SelectOp::Replace);
        assert_eq!(
            select_op(Mods {
                shift: true,
                ..none
            }),
            SelectOp::Add
        );
        assert_eq!(select_op(Mods { ctrl: true, ..none }), SelectOp::Subtract);
        // Shift+Ctrl intersects, as in the Sprites editor; Alt is an alias.
        assert_eq!(
            select_op(Mods {
                shift: true,
                ctrl: true,
                alt: false
            }),
            SelectOp::Intersect
        );
        assert_eq!(select_op(Mods { alt: true, ..none }), SelectOp::Intersect);
        assert_eq!(
            select_op(Mods {
                shift: true,
                ctrl: true,
                alt: true
            }),
            SelectOp::Intersect
        );
        // Shift or Ctrl alone wins over the Alt alias.
        assert_eq!(
            select_op(Mods {
                ctrl: true,
                alt: true,
                shift: false
            }),
            SelectOp::Subtract
        );
        assert_eq!(
            select_op(Mods {
                shift: true,
                alt: true,
                ctrl: false
            }),
            SelectOp::Add
        );
    }

    #[test]
    fn map_and_sprite_editors_agree_on_the_modifiers() {
        use crate::sprites::editor::Mods as SpriteMods;
        use capy_paint::MaskOp;
        for (shift, ctrl) in [(false, false), (true, false), (false, true), (true, true)] {
            let map = select_op(Mods {
                shift,
                ctrl,
                alt: false,
            });
            let sprite = SpriteMods { shift, ctrl }.mask_op();
            let same = matches!(
                (map, sprite),
                (SelectOp::Replace, MaskOp::Replace)
                    | (SelectOp::Add, MaskOp::Add)
                    | (SelectOp::Subtract, MaskOp::Subtract)
                    | (SelectOp::Intersect, MaskOp::Intersect)
            );
            assert!(same, "shift {shift} ctrl {ctrl}: {map:?} vs {sprite:?}");
        }
    }

    #[test]
    fn box_drag_normalises_and_clips() {
        let mut d = BoxDrag::new((5, 7));
        assert_eq!(d.rect(10, 10), Some(CellRect::new((5, 7), (5, 7))));
        d.update((2, 3));
        assert_eq!(d.rect(10, 10), Some(CellRect::new((2, 3), (5, 7))));
        // Dragging past the map edge clips to it.
        d.update((-4, 40));
        assert_eq!(d.rect(10, 10), Some(CellRect::new((0, 7), (5, 9))));
        // Entirely outside selects nothing.
        let outside = BoxDrag {
            start: (-5, -5),
            current: (-1, 3),
        };
        assert_eq!(outside.rect(10, 10), None);
        let beyond = BoxDrag {
            start: (10, 2),
            current: (14, 3),
        };
        assert_eq!(beyond.rect(10, 10), None);
        assert_eq!(BoxDrag::new((0, 0)).rect(0, 0), None);
    }

    #[test]
    fn lasso_thins_points_and_needs_three() {
        let mut l = LassoPath::new((1.0, 1.0));
        assert!(!l.push((1.05, 1.0)), "too close to the start");
        assert!(l.push((2.0, 1.0)));
        assert!(!l.is_polygon());
        assert!(!l.push((2.0, 1.1)));
        assert!(l.push((2.0, 2.0)));
        assert!(l.is_polygon());
        assert_eq!(l.points(), &[(1.0, 1.0), (2.0, 1.0), (2.0, 2.0)]);
    }

    #[test]
    fn lasso_polygon_selects_through_capy_edit() {
        use capy_edit::Selection;
        use d2_formats::ds1::{Cell, Ds1};
        let mut ds1 = Ds1::new(6, 6, 0);
        for y in 0..6 {
            for x in 0..6 {
                ds1.floors[0].set(x, y, Cell(1).with_indices(1, 0));
            }
        }
        let mut l = LassoPath::new((0.0, 0.0));
        for p in [(3.0, 0.0), (3.0, 2.0), (0.0, 2.0)] {
            l.push(p);
        }
        let mut sel = Selection::new();
        sel.lasso(&ds1, l.points(), &[LayerRef::Floor(0)], SelectOp::Replace);
        assert_eq!(sel.len(), 6, "a 3 x 2 block of cell centres");
        assert!(sel.contains(LayerRef::Floor(0), 2, 1));
        assert!(!sel.contains(LayerRef::Floor(0), 3, 1));
    }

    #[test]
    fn tool_kinds() {
        // Keys live in the command registry (`ui::commands`) now.
        assert!(Tool::SelectBox.selects() && !Tool::Paint.selects());
        assert!(Tool::Erase.brushes() && !Tool::Eyedropper.brushes());
    }

    #[test]
    fn lines_have_no_gaps_and_include_both_ends() {
        let cells = line_cells((0, 0), (5, 2));
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert_eq!(cells.last(), Some(&(5, 2)));
        for pair in cells.windows(2) {
            let (dx, dy) = ((pair[1].0 - pair[0].0).abs(), (pair[1].1 - pair[0].1).abs());
            assert!(dx <= 1 && dy <= 1 && (dx, dy) != (0, 0), "{pair:?}");
        }
        assert_eq!(line_cells((3, 3), (3, 3)), vec![(3, 3)]);
        assert_eq!(line_cells((2, 0), (0, 0)), vec![(2, 0), (1, 0), (0, 0)]);
    }

    #[test]
    fn brush_footprints_clip_to_the_map() {
        assert_eq!(brush_cells((4, 4), 1, 10, 10), vec![(4, 4)]);
        assert_eq!(brush_cells((4, 4), 3, 10, 10).len(), 9);
        assert_eq!(brush_cells((0, 0), 3, 10, 10).len(), 4, "corner clips");
        assert!(brush_cells((-5, 4), 1, 10, 10).is_empty());
        assert_eq!(brush_cells((4, 4), 2, 10, 10).len(), 4);
        let stroke = stroke_cells(Some((0, 0)), (4, 0), 2, 10, 10);
        let unique: std::collections::HashSet<_> = stroke.iter().collect();
        assert_eq!(unique.len(), stroke.len(), "no duplicates");
        assert_eq!(stroke_cells(None, (2, 2), 1, 10, 10), vec![(2, 2)]);
    }

    #[test]
    fn scope_picks_layers() {
        let existing = [LayerRef::Floor(0), LayerRef::Shadow, LayerRef::Wall(0)];
        assert_eq!(
            scope_layers(SelectScope::Active, LayerRef::Wall(0), &existing, |_| false),
            vec![LayerRef::Wall(0)]
        );
        assert_eq!(
            scope_layers(SelectScope::Visible, LayerRef::Wall(0), &existing, |l| {
                l != LayerRef::Shadow
            }),
            vec![LayerRef::Floor(0), LayerRef::Wall(0)]
        );
    }
}
