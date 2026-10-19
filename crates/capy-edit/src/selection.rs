//! Cell selections: box, lasso, by tile id and "select identical".

use std::collections::BTreeSet;

use d2_formats::ds1::{Cell, Ds1};

use crate::layer::{CellValue, LayerRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectOp {
    Replace,
    Add,
    Subtract,
    Intersect,
}

/// An inclusive rectangle of cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl CellRect {
    /// The rectangle with corners `a` and `b` (in any order).
    pub fn new(a: (u32, u32), b: (u32, u32)) -> CellRect {
        CellRect {
            x0: a.0.min(b.0),
            y0: a.1.min(b.1),
            x1: a.0.max(b.0),
            y1: a.1.max(b.1),
        }
    }
}

type CellKey = (LayerRef, u32, u32);

/// A set of `(layer, x, y)`. Only cells that exist and are non-empty are ever
/// selected by the query methods, except `box_select(.., include_empty = true)`
/// (paste targets, erase areas). A `layers` argument that is empty selects
/// nothing; layers the map does not have are skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    cells: BTreeSet<CellKey>,
}

impl Selection {
    pub fn new() -> Selection {
        Selection::default()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn contains(&self, layer: LayerRef, x: u32, y: u32) -> bool {
        self.cells.contains(&(layer, x, y))
    }

    /// In `(layer, x, y)` order.
    pub fn iter(&self) -> impl Iterator<Item = (LayerRef, u32, u32)> + '_ {
        self.cells.iter().copied()
    }

    /// The distinct layers that hold selected cells, sorted.
    pub fn layers(&self) -> Vec<LayerRef> {
        let mut out: Vec<LayerRef> = self.cells.iter().map(|c| c.0).collect();
        out.dedup();
        out
    }

    /// The smallest rectangle holding every selected cell.
    pub fn bounds(&self) -> Option<CellRect> {
        let mut it = self.cells.iter();
        let first = it.next()?;
        let mut r = CellRect {
            x0: first.1,
            y0: first.2,
            x1: first.1,
            y1: first.2,
        };
        for &(_, x, y) in it {
            r.x0 = r.x0.min(x);
            r.y0 = r.y0.min(y);
            r.x1 = r.x1.max(x);
            r.y1 = r.y1.max(y);
        }
        Some(r)
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// Adds one cell; `false` when it was already selected.
    pub fn insert(&mut self, layer: LayerRef, x: u32, y: u32) -> bool {
        self.cells.insert((layer, x, y))
    }

    fn combine(&mut self, op: SelectOp, found: BTreeSet<CellKey>) {
        match op {
            SelectOp::Replace => self.cells = found,
            SelectOp::Add => self.cells.extend(found),
            SelectOp::Subtract => self.cells.retain(|c| !found.contains(c)),
            SelectOp::Intersect => self.cells.retain(|c| found.contains(c)),
        }
    }

    /// Every cell of `layers` inside `rect` (clipped to the map); non-empty
    /// cells only unless `include_empty`.
    pub fn box_select(
        &mut self,
        ds1: &Ds1,
        rect: CellRect,
        layers: &[LayerRef],
        include_empty: bool,
        op: SelectOp,
    ) {
        let found = scan(ds1, layers, rect, |layer, _, _, v| {
            include_empty || !layer.is_empty_value(v)
        });
        self.combine(op, found);
    }

    /// Cells whose centre (x+0.5, y+0.5) lies inside the polygon (even-odd rule).
    pub fn lasso(&mut self, ds1: &Ds1, polygon: &[(f32, f32)], layers: &[LayerRef], op: SelectOp) {
        let found = lasso_cells(ds1, polygon, layers);
        self.combine(op, found);
    }

    /// Every cell whose (main, sub) index (and orientation, when given: the raw
    /// orientation dword, which is 0 outside walls) matches. The tag layer has
    /// no tile ids and never matches.
    pub fn by_tile_id(
        &mut self,
        ds1: &Ds1,
        main: u8,
        sub: u8,
        orientation: Option<u32>,
        layers: &[LayerRef],
        op: SelectOp,
    ) {
        let layers: Vec<LayerRef> = layers
            .iter()
            .copied()
            .filter(|l| *l != LayerRef::Tag)
            .collect();
        let found = scan(ds1, &layers, whole_map(ds1), |layer, _, _, v| {
            let c = Cell(v.cell);
            !layer.is_empty_value(v)
                && c.main_index() == main
                && c.sub_index() == sub
                && orientation.is_none_or(|o| v.orientation == o)
        });
        self.combine(op, found);
    }

    /// Every cell in `layers` whose `CellValue` (indices, orientation and flag
    /// bits) equals the seed cell's. An empty or missing seed selects nothing.
    pub fn identical(
        &mut self,
        ds1: &Ds1,
        seed: (LayerRef, u32, u32),
        layers: &[LayerRef],
        op: SelectOp,
    ) {
        let found = match seed.0.get(ds1, seed.1, seed.2) {
            Some(want) if !seed.0.is_empty_value(want) => {
                scan(ds1, layers, whole_map(ds1), |layer, _, _, v| {
                    v == want && !layer.is_empty_value(v)
                })
            }
            _ => BTreeSet::new(),
        };
        self.combine(op, found);
    }
}

fn whole_map(ds1: &Ds1) -> CellRect {
    CellRect {
        x0: 0,
        y0: 0,
        x1: ds1.width.saturating_sub(1),
        y1: ds1.height.saturating_sub(1),
    }
}

/// The existing, distinct layers among `layers`.
fn existing(ds1: &Ds1, layers: &[LayerRef]) -> Vec<LayerRef> {
    let mut ls: Vec<LayerRef> = layers
        .iter()
        .copied()
        .filter(|l| l.exists_in(ds1))
        .collect();
    ls.sort();
    ls.dedup();
    ls
}

/// Cells of `layers` inside `rect` (clipped to the map) that `keep` accepts.
fn scan(
    ds1: &Ds1,
    layers: &[LayerRef],
    rect: CellRect,
    mut keep: impl FnMut(LayerRef, u32, u32, CellValue) -> bool,
) -> BTreeSet<CellKey> {
    let mut out = BTreeSet::new();
    if ds1.width == 0 || ds1.height == 0 {
        return out;
    }
    let x1 = rect.x1.min(ds1.width - 1);
    let y1 = rect.y1.min(ds1.height - 1);
    for layer in existing(ds1, layers) {
        for y in rect.y0..=y1 {
            for x in rect.x0..=x1 {
                if let Some(v) = layer.get(ds1, x, y)
                    && keep(layer, x, y, v)
                {
                    out.insert((layer, x, y));
                }
            }
        }
    }
    out
}

fn lasso_cells(ds1: &Ds1, polygon: &[(f32, f32)], layers: &[LayerRef]) -> BTreeSet<CellKey> {
    if polygon.len() < 3 || polygon.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return BTreeSet::new();
    }
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for &(px, py) in polygon {
        min_x = min_x.min(f64::from(px));
        max_x = max_x.max(f64::from(px));
        min_y = min_y.min(f64::from(py));
        max_y = max_y.max(f64::from(py));
    }
    if max_x < 0.0 || max_y < 0.0 {
        return BTreeSet::new();
    }
    // `as u32` saturates, so huge polygons just clip to the map.
    let rect = CellRect {
        x0: (min_x - 1.0).max(0.0) as u32,
        y0: (min_y - 1.0).max(0.0) as u32,
        x1: (max_x + 1.0) as u32,
        y1: (max_y + 1.0) as u32,
    };
    scan(ds1, layers, rect, |layer, x, y, v| {
        !layer.is_empty_value(v)
            && point_in_polygon(polygon, (f64::from(x) + 0.5, f64::from(y) + 0.5))
    })
}

/// Even-odd rule.
fn point_in_polygon(polygon: &[(f32, f32)], (px, py): (f64, f64)) -> bool {
    let n = polygon.len();
    let mut inside = false;
    for (&(xi, yi), &(xj, yj)) in polygon.iter().zip(polygon.iter().cycle().skip(n - 1)) {
        let (xi, yi, xj, yj) = (f64::from(xi), f64::from(yi), f64::from(xj), f64::from(yj));
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
    }
    inside
}
