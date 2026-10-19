//! Structural difference between two DS1 models.

use std::collections::{BTreeSet, HashMap};

use d2_formats::ds1::{Ds1, Object};

use crate::layer::{CellValue, LayerRef};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellChange {
    pub layer: LayerRef,
    pub x: u32,
    pub y: u32,
    pub before: CellValue,
    pub after: CellValue,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Diff {
    /// `(before, after)` as `(width, height)`.
    pub size: Option<((u32, u32), (u32, u32))>,
    pub version: Option<(i32, i32)>,
    pub layers_added: Vec<LayerRef>,
    pub layers_removed: Vec<LayerRef>,
    /// Over the common area of layers present in both, layer by layer in row order.
    pub cells: Vec<CellChange>,
    pub objects_added: Vec<Object>,
    pub objects_removed: Vec<Object>,
    /// Files, groups, paths, act, tag_type, preserved bytes, or the order of the objects.
    pub other_changed: bool,
}

impl Ds1Diff {
    pub fn is_empty(&self) -> bool {
        self.size.is_none()
            && self.version.is_none()
            && self.layers_added.is_empty()
            && self.layers_removed.is_empty()
            && self.cells.is_empty()
            && self.objects_added.is_empty()
            && self.objects_removed.is_empty()
            && !self.other_changed
    }
}

pub fn diff(before: &Ds1, after: &Ds1) -> Ds1Diff {
    let mut d = Ds1Diff::default();

    let (bs, a_s) = ((before.width, before.height), (after.width, after.height));
    if bs != a_s {
        d.size = Some((bs, a_s));
    }
    if before.version != after.version {
        d.version = Some((before.version, after.version));
    }

    let before_layers: BTreeSet<LayerRef> = LayerRef::all_in(before).into_iter().collect();
    let after_layers: BTreeSet<LayerRef> = LayerRef::all_in(after).into_iter().collect();
    d.layers_added = after_layers.difference(&before_layers).copied().collect();
    d.layers_removed = before_layers.difference(&after_layers).copied().collect();

    let w = before.width.min(after.width);
    let h = before.height.min(after.height);
    for layer in before_layers.intersection(&after_layers) {
        for y in 0..h {
            for x in 0..w {
                if let (Some(b), Some(a)) = (layer.get(before, x, y), layer.get(after, x, y))
                    && b != a
                {
                    d.cells.push(CellChange {
                        layer: *layer,
                        x,
                        y,
                        before: b,
                        after: a,
                    });
                }
            }
        }
    }

    let (added, removed) = object_difference(&before.objects, &after.objects);
    if added.is_empty() && removed.is_empty() && before.objects != after.objects {
        d.other_changed = true;
    }
    d.objects_added = added;
    d.objects_removed = removed;

    if before.act != after.act
        || before.tag_type != after.tag_type
        || before.files != after.files
        || before.groups != after.groups
        || before.npc_paths != after.npc_paths
        || before.preserved != after.preserved
    {
        d.other_changed = true;
    }
    d
}

/// Multiset difference: `(in after only, in before only)`, each in file order.
fn object_difference(before: &[Object], after: &[Object]) -> (Vec<Object>, Vec<Object>) {
    let mut counts: HashMap<Object, i64> = HashMap::new();
    for o in before {
        *counts.entry(*o).or_insert(0) += 1;
    }
    let mut added = Vec::new();
    for o in after {
        match counts.get_mut(o) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added.push(*o),
        }
    }
    let mut removed = Vec::new();
    let mut leftover = counts;
    for o in before {
        if let Some(n) = leftover.get_mut(o)
            && *n > 0
        {
            *n -= 1;
            removed.push(*o);
        }
    }
    (added, removed)
}
