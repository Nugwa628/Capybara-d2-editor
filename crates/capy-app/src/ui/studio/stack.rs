//! Stack painting (ds1-studio's Alt brush, `editTools.ts stackPaintEdits`):
//! each cell goes on the first empty layer of the active layer's kind (the
//! active layer first, then the others front to back), adding a layer while
//! the map has room (4 walls, 2 floors). With every layer taken the tile on
//! the active layer is replaced. `placed` remembers the layer a cell got in
//! this stroke, so going over it again repaints it there.

use std::collections::{BTreeMap, HashMap};

use capy_edit::LayerRef;
use d2_formats::ds1::{Ds1, MAX_FLOOR_LAYERS, MAX_WALL_LAYERS};

use crate::edit::ops::EditOp;
use crate::state::AppState;

/// Cells stacked in one stroke: cell -> layer index it went to.
pub type Placed = HashMap<(u32, u32), u8>;

/// `(cell, layer index)` per painted cell, layers needed, cells replaced.
pub type StackPlan = (Vec<((u32, u32), u8)>, usize, usize);

/// Whether `layer` can be stacked on (shadows and tags have one layer).
pub fn stackable(layer: LayerRef) -> bool {
    matches!(layer, LayerRef::Floor(_) | LayerRef::Wall(_))
}

fn kind_layer(like: LayerRef, n: u8) -> LayerRef {
    match like {
        LayerRef::Wall(_) => LayerRef::Wall(n),
        _ => LayerRef::Floor(n),
    }
}

/// The layer index each cell of `cells` is stacked on, and how many layers
/// of the kind the map needs. Pure: reads `ds1` only.
pub fn plan(ds1: &Ds1, layer: LayerRef, cells: &[(u32, u32)], placed: &mut Placed) -> StackPlan {
    let (max, have, active) = match layer {
        LayerRef::Wall(i) => (MAX_WALL_LAYERS, ds1.walls.len(), i),
        LayerRef::Floor(i) => (MAX_FLOOR_LAYERS, ds1.floors.len(), i),
        _ => return (Vec::new(), 0, 0),
    };
    let order: Vec<u8> = std::iter::once(active)
        .chain((0..max as u8).filter(|n| *n != active))
        .collect();
    let mut need = have;
    let mut replaced = 0;
    let mut out = Vec::new();
    for &(x, y) in cells {
        if x >= ds1.width || y >= ds1.height {
            continue;
        }
        let n = match placed.get(&(x, y)) {
            Some(n) => *n,
            None => {
                let free = order.iter().copied().find(|&n| {
                    usize::from(n) >= have
                        || kind_layer(layer, n)
                            .get(ds1, x, y)
                            .is_none_or(|v| kind_layer(layer, n).is_empty_value(v))
                });
                let n = free.unwrap_or_else(|| {
                    replaced += 1;
                    active
                });
                placed.insert((x, y), n);
                n
            }
        };
        need = need.max(usize::from(n) + 1);
        out.push(((x, y), n));
    }
    (out, need, replaced)
}

/// Paints the current value on the free layers of `cells`. False when an
/// edit failed.
pub fn paint_cells(
    state: &mut AppState,
    index: usize,
    layer: LayerRef,
    cells: &[(u32, u32)],
    placed: &mut Placed,
) -> bool {
    let (targets, need, replaced) = plan(&state.docs[index].ds1, layer, cells, placed);
    if targets.is_empty() {
        return true;
    }
    let have = match layer {
        LayerRef::Wall(_) => state.docs[index].ds1.walls.len(),
        _ => state.docs[index].ds1.floors.len(),
    };
    if need > have && !state.exec(index, EditOp::AddLayer(kind_layer(layer, need as u8 - 1))) {
        return false;
    }
    let value = state.edit.current;
    let mut by_layer: BTreeMap<u8, Vec<(u32, u32, capy_edit::CellValue)>> = BTreeMap::new();
    for ((x, y), n) in targets {
        by_layer.entry(n).or_default().push((x, y, value));
    }
    for (n, cells) in by_layer {
        if !state.exec(
            index,
            EditOp::Paint {
                layer: kind_layer(layer, n),
                cells,
            },
        ) {
            return false;
        }
    }
    if replaced > 0 {
        state.notify(format!(
            "{replaced} cell(s) had no free layer: replaced on the active layer"
        ));
    }
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use d2_formats::ds1::Cell;

    fn map() -> Ds1 {
        let mut d = Ds1::new(4, 4, 0);
        d.floors[0].set(1, 1, Cell(0x0000_0101));
        d
    }

    #[test]
    fn an_occupied_cell_goes_to_the_next_layer_and_a_free_one_stays() {
        let d = map();
        let mut placed = Placed::new();
        let (t, need, replaced) = plan(&d, LayerRef::Floor(0), &[(1, 1), (2, 2)], &mut placed);
        assert_eq!(t, vec![((1, 1), 1), ((2, 2), 0)]);
        assert_eq!((need, replaced), (d.floors.len().max(2), 0));
    }

    #[test]
    fn a_cell_gone_over_twice_keeps_its_layer() {
        let d = map();
        let mut placed = Placed::new();
        plan(&d, LayerRef::Floor(0), &[(1, 1)], &mut placed);
        let (t, ..) = plan(&d, LayerRef::Floor(0), &[(1, 1)], &mut placed);
        assert_eq!(t, vec![((1, 1), 1)]);
    }

    #[test]
    fn a_full_stack_replaces_the_active_layer() {
        let mut d = map();
        d.add_floor_layer().ok();
        d.floors[1].set(1, 1, Cell(0x0000_0102));
        let mut placed = Placed::new();
        let (t, _, replaced) = plan(&d, LayerRef::Floor(0), &[(1, 1)], &mut placed);
        assert_eq!((t, replaced), (vec![((1, 1), 0)], 1));
    }
}
