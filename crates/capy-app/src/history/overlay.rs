//! The visual tile diff: what changed between a snapshot and the current map,
//! per cell and per layer, in the form the Map panel paints (green added,
//! red removed, yellow changed) and the History panel counts.

use std::collections::BTreeMap;

use capy_edit::{CellValue, Ds1Diff, LayerRef};
use d2_formats::ds1::Ds1;

/// How one cell (or object) differs, from the snapshot to now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Change {
    /// Empty in the snapshot, a tile now.
    Added,
    /// A tile in the snapshot, empty now.
    Removed,
    /// A different tile (or flags, or orientation).
    Changed,
}

impl Change {
    pub const ALL: [Change; 3] = [Change::Added, Change::Removed, Change::Changed];

    /// Overlay colour, RGBA (unmultiplied).
    pub fn rgba(self) -> [u8; 4] {
        match self {
            Change::Added => [60, 220, 90, 120],
            Change::Removed => [235, 60, 60, 130],
            Change::Changed => [245, 210, 50, 120],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Removed => "removed",
            Change::Changed => "changed",
        }
    }

    pub fn sign(self) -> &'static str {
        match self {
            Change::Added => "+",
            Change::Removed => "-",
            Change::Changed => "~",
        }
    }
}

/// Layer-aware classification of one cell change.
pub fn classify(layer: LayerRef, before: CellValue, after: CellValue) -> Change {
    match (layer.is_empty_value(before), layer.is_empty_value(after)) {
        (true, false) => Change::Added,
        (false, true) => Change::Removed,
        _ => Change::Changed,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.added + self.removed + self.changed
    }

    pub fn get(&self, change: Change) -> usize {
        match change {
            Change::Added => self.added,
            Change::Removed => self.removed,
            Change::Changed => self.changed,
        }
    }

    fn bump(&mut self, change: Change) {
        match change {
            Change::Added => self.added += 1,
            Change::Removed => self.removed += 1,
            Change::Changed => self.changed += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayCell {
    pub layer: LayerRef,
    pub x: u32,
    pub y: u32,
    pub change: Change,
}

/// An object that exists on one side only, in sub-tile coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectMark {
    pub x: i32,
    pub y: i32,
    pub change: Change,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffOverlay {
    /// Every differing cell, by layer then position.
    pub cells: Vec<OverlayCell>,
    pub objects: Vec<ObjectMark>,
    pub per_layer: BTreeMap<LayerRef, Counts>,
    /// Map-level differences in words (size, version, layers, other data).
    pub notes: Vec<String>,
}

fn layer_name(layer: LayerRef) -> String {
    crate::doc::layer_label(layer)
}

/// Every non-empty cell of `layer` in `ds1`, marked `change`.
fn whole_layer(ds1: &Ds1, layer: LayerRef, change: Change, out: &mut Vec<OverlayCell>) {
    for y in 0..ds1.height {
        for x in 0..ds1.width {
            if let Some(v) = layer.get(ds1, x, y)
                && !layer.is_empty_value(v)
            {
                out.push(OverlayCell {
                    layer,
                    x,
                    y,
                    change,
                });
            }
        }
    }
}

impl DiffOverlay {
    /// The overlay of `diff = capy_edit::diff(before, after)`, where `before`
    /// is the snapshot and `after` the current map. A layer present on one
    /// side only counts all its tiles as added (or removed).
    pub fn build(diff: &Ds1Diff, before: &Ds1, after: &Ds1) -> DiffOverlay {
        let mut cells: Vec<OverlayCell> = diff
            .cells
            .iter()
            .map(|c| OverlayCell {
                layer: c.layer,
                x: c.x,
                y: c.y,
                change: classify(c.layer, c.before, c.after),
            })
            .collect();
        for &layer in &diff.layers_added {
            whole_layer(after, layer, Change::Added, &mut cells);
        }
        for &layer in &diff.layers_removed {
            whole_layer(before, layer, Change::Removed, &mut cells);
        }
        cells.sort_by_key(|c| (c.layer, c.y, c.x));
        let mut per_layer: BTreeMap<LayerRef, Counts> = BTreeMap::new();
        for c in &cells {
            per_layer.entry(c.layer).or_default().bump(c.change);
        }
        let objects = diff
            .objects_added
            .iter()
            .map(|o| (o, Change::Added))
            .chain(diff.objects_removed.iter().map(|o| (o, Change::Removed)))
            .map(|(o, change)| ObjectMark {
                x: o.x,
                y: o.y,
                change,
            })
            .collect();
        let mut notes = Vec::new();
        if let Some(((w0, h0), (w1, h1))) = diff.size {
            notes.push(format!(
                "Size {w0} x {h0} then, {w1} x {h1} now (cells compared over the common area)"
            ));
        }
        if let Some((v0, v1)) = diff.version {
            notes.push(format!("DS1 version {v0} then, {v1} now"));
        }
        for &l in &diff.layers_added {
            notes.push(format!("{} added since", layer_name(l)));
        }
        for &l in &diff.layers_removed {
            notes.push(format!("{} removed since", layer_name(l)));
        }
        if diff.other_changed {
            notes.push("Other data differs (files, groups, NPC paths, act or tag type)".to_owned());
        }
        DiffOverlay {
            cells,
            objects,
            per_layer,
            notes,
        }
    }

    /// The overlay between two maps.
    pub fn between(before: &Ds1, after: &Ds1) -> DiffOverlay {
        DiffOverlay::build(&capy_edit::diff(before, after), before, after)
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.objects.is_empty() && self.notes.is_empty()
    }

    /// Differing cells on `layer`.
    pub fn count(&self, layer: LayerRef) -> usize {
        self.per_layer.get(&layer).map_or(0, Counts::total)
    }

    pub fn objects_count(&self, change: Change) -> usize {
        self.objects.iter().filter(|o| o.change == change).count()
    }

    /// The `n`-th differing cell of `layer` (wrapping), for "jump to".
    pub fn nth_of(&self, layer: LayerRef, n: usize) -> Option<(u32, u32)> {
        let total = self.count(layer);
        if total == 0 {
            return None;
        }
        self.cells
            .iter()
            .filter(|c| c.layer == layer)
            .nth(n % total)
            .map(|c| (c.x, c.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capy_edit::CellChange;
    use d2_formats::ds1::{Cell, Object};

    fn v(cell: u32) -> CellValue {
        CellValue {
            cell,
            orientation: 0,
        }
    }

    fn tile(main: u8, sub: u8) -> u32 {
        Cell(1).with_indices(main, sub).0
    }

    #[test]
    fn cells_are_coloured_by_kind_per_layer() {
        let a = tile(3, 2);
        let b = tile(4, 0);
        let diff = Ds1Diff {
            cells: vec![
                CellChange {
                    layer: LayerRef::Floor(0),
                    x: 1,
                    y: 1,
                    before: CellValue::EMPTY,
                    after: v(a),
                },
                CellChange {
                    layer: LayerRef::Floor(0),
                    x: 2,
                    y: 1,
                    before: v(a),
                    after: v(b),
                },
                CellChange {
                    layer: LayerRef::Wall(0),
                    x: 0,
                    y: 3,
                    before: v(b),
                    after: CellValue::EMPTY,
                },
                // Same tile, different orientation: changed.
                CellChange {
                    layer: LayerRef::Wall(0),
                    x: 1,
                    y: 3,
                    before: CellValue {
                        cell: a,
                        orientation: 1,
                    },
                    after: CellValue {
                        cell: a,
                        orientation: 2,
                    },
                },
                // Tag cells are empty only when 0.
                CellChange {
                    layer: LayerRef::Tag,
                    x: 0,
                    y: 0,
                    before: v(0),
                    after: v(7),
                },
            ],
            objects_added: vec![Object {
                kind: 1,
                id: 5,
                x: 12,
                y: 7,
                flags: 0,
            }],
            objects_removed: vec![Object {
                kind: 2,
                id: 9,
                x: 3,
                y: 4,
                flags: 0,
            }],
            ..Ds1Diff::default()
        };
        let empty = Ds1::new(4, 4, 0);
        let o = DiffOverlay::build(&diff, &empty, &empty);
        let kinds: Vec<(LayerRef, u32, u32, Change)> = o
            .cells
            .iter()
            .map(|c| (c.layer, c.x, c.y, c.change))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (LayerRef::Floor(0), 1, 1, Change::Added),
                (LayerRef::Floor(0), 2, 1, Change::Changed),
                (LayerRef::Wall(0), 0, 3, Change::Removed),
                (LayerRef::Wall(0), 1, 3, Change::Changed),
                (LayerRef::Tag, 0, 0, Change::Added),
            ]
        );
        assert_eq!(
            o.per_layer[&LayerRef::Floor(0)],
            Counts {
                added: 1,
                removed: 0,
                changed: 1
            }
        );
        assert_eq!(o.count(LayerRef::Wall(0)), 2);
        assert_eq!(o.count(LayerRef::Shadow), 0);
        // Green, red, yellow.
        assert_eq!(Change::Added.rgba()[..3], [60, 220, 90]);
        assert_eq!(Change::Removed.rgba()[..3], [235, 60, 60]);
        assert_eq!(Change::Changed.rgba()[..3], [245, 210, 50]);
        // Object markers on both sides.
        assert_eq!(
            o.objects,
            vec![
                ObjectMark {
                    x: 12,
                    y: 7,
                    change: Change::Added
                },
                ObjectMark {
                    x: 3,
                    y: 4,
                    change: Change::Removed
                },
            ]
        );
        assert_eq!(o.objects_count(Change::Added), 1);
        // Jumping cycles through a layer's cells.
        assert_eq!(o.nth_of(LayerRef::Wall(0), 0), Some((0, 3)));
        assert_eq!(o.nth_of(LayerRef::Wall(0), 3), Some((1, 3)));
        assert_eq!(o.nth_of(LayerRef::Shadow, 0), None);
    }

    #[test]
    fn real_maps_diff_with_whole_layers_and_notes() {
        let mut before = Ds1::new(6, 5, 0);
        before.floors[0].set(1, 1, Cell(1).with_indices(3, 2));
        before.floors[0].set(2, 2, Cell(1).with_indices(3, 2));
        let mut after = before.clone();
        after.floors[0].set(1, 1, Cell(1).with_indices(5, 0)); // changed
        after.floors[0].set(2, 2, Cell(0)); // removed
        after.floors[0].set(4, 4, Cell(1).with_indices(1, 1)); // added
        assert!(after.add_wall_layer().is_ok());
        let w = after.walls.len() - 1;
        after.walls[w].tiles.set(0, 0, Cell(1).with_indices(2, 2));
        let o = DiffOverlay::between(&before, &after);
        assert_eq!(
            o.per_layer[&LayerRef::Floor(0)],
            Counts {
                added: 1,
                removed: 1,
                changed: 1
            }
        );
        let new_wall = LayerRef::Wall(u8::try_from(w).expect("small"));
        assert_eq!(
            o.count(new_wall),
            1,
            "a new layer counts its tiles as added"
        );
        assert!(
            o.notes.iter().any(|n| n.contains("added since")),
            "{:?}",
            o.notes
        );
        assert!(DiffOverlay::between(&before, &before).is_empty());
    }
}
