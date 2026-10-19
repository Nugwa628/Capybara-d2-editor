//! Invertible changes to a [`Ds1`].
//!
//! A [`Patch`] is a list of steps. Cell edits store the before and after
//! value of every changed cell (never a whole layer), so a patch costs memory
//! in proportion to what it changed. Only operations that destroy data
//! (removing a layer, shrinking the map) keep the removed data.

use d2_formats::Grid;
use d2_formats::ds1::{Cell, Ds1, Group, NpcPath, Object, WallLayer};

use crate::error::{EditError, Result, invalid};
use crate::layer::{CellValue, LayerRef};

/// One changed cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CellEdit {
    pub x: u32,
    pub y: u32,
    pub before: CellValue,
    pub after: CellValue,
}

/// A whole layer's content (for adding and removing layers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayerData {
    Wall(WallLayer),
    Floor(Grid<Cell>),
    Tag(Grid<u32>),
}

impl LayerData {
    /// An empty layer of the kind `layer` names, `None` for the shadow layer.
    pub fn empty(layer: LayerRef, width: u32, height: u32) -> Option<LayerData> {
        match layer {
            LayerRef::Wall(_) => Some(LayerData::Wall(WallLayer {
                tiles: Grid::new(width, height),
                orientations: Grid::new(width, height),
            })),
            LayerRef::Floor(_) => Some(LayerData::Floor(Grid::new(width, height))),
            LayerRef::Tag => Some(LayerData::Tag(Grid::new(width, height))),
            LayerRef::Shadow => None,
        }
    }

    /// A copy of the content of `layer` in `ds1`, `None` for the shadow layer
    /// or a layer that does not exist.
    pub fn of(ds1: &Ds1, layer: LayerRef) -> Option<LayerData> {
        match layer {
            LayerRef::Wall(i) => ds1
                .walls
                .get(usize::from(i))
                .map(|w| LayerData::Wall(w.clone())),
            LayerRef::Floor(i) => ds1
                .floors
                .get(usize::from(i))
                .map(|f| LayerData::Floor(f.clone())),
            LayerRef::Tag => ds1.tag.as_ref().map(|t| LayerData::Tag(t.clone())),
            LayerRef::Shadow => None,
        }
    }
}

/// The small header fields a layer or version change can touch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub version: i32,
    pub tag_type: i32,
    pub unknown_v9_13: [i32; 2],
    pub pre_group_dword: i32,
    pub groups: Vec<Group>,
}

impl Header {
    pub fn of(ds1: &Ds1) -> Header {
        Header {
            version: ds1.version,
            tag_type: ds1.tag_type,
            unknown_v9_13: ds1.preserved.unknown_v9_13,
            pre_group_dword: ds1.preserved.pre_group_dword,
            groups: ds1.groups.clone(),
        }
    }

    fn write(&self, ds1: &mut Ds1) {
        ds1.version = self.version;
        ds1.tag_type = self.tag_type;
        ds1.preserved.unknown_v9_13 = self.unknown_v9_13;
        ds1.preserved.pre_group_dword = self.pre_group_dword;
        ds1.groups = self.groups.clone();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Cell edits on one layer, applied in order (reverted in reverse order).
    Cells {
        layer: LayerRef,
        edits: Vec<CellEdit>,
    },
    /// Adds the layer at `layer`'s index (revert removes it).
    InsertLayer {
        layer: LayerRef,
        data: LayerData,
    },
    /// Removes the layer at `layer`'s index (revert puts `data` back).
    RemoveLayer {
        layer: LayerRef,
        data: LayerData,
    },
    /// Changes the map size, keeping the top-left. `cut` holds the non-default
    /// cells a shrink threw away.
    Resize {
        old: (u32, u32),
        new: (u32, u32),
        cut: Vec<(LayerRef, u32, u32, CellValue)>,
    },
    Header {
        before: Header,
        after: Header,
    },
    /// Inserts objects at these (ascending) indices.
    ObjectsInsert {
        items: Vec<(usize, Object)>,
    },
    /// Removes the objects at these (ascending) indices.
    ObjectsRemove {
        items: Vec<(usize, Object)>,
    },
    /// Replaces objects in place: `(index, before, after)`.
    ObjectsSet {
        edits: Vec<(usize, Object, Object)>,
    },
    Paths {
        before: Vec<NpcPath>,
        after: Vec<NpcPath>,
    },
    /// Swaps the whole model (restore-from-snapshot).
    Replace {
        before: Box<Ds1>,
        after: Box<Ds1>,
    },
}

/// An invertible change. `apply` then `revert` restores the DS1 exactly
/// (`PartialEq`), including layer counts, `version`, and object order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    label: String,
    steps: Vec<Step>,
}

impl Patch {
    pub(crate) fn new(label: impl Into<String>) -> Patch {
        Patch {
            label: label.into(),
            steps: Vec::new(),
        }
    }

    /// A patch that swaps the whole model `before` for `after`. Costs two full
    /// copies of the map: meant for restoring snapshots, not for editing.
    pub fn replace(label: impl Into<String>, before: Ds1, after: Ds1) -> Patch {
        let mut p = Patch::new(label);
        if before != after {
            p.steps.push(Step::Replace {
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        p
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub(crate) fn push(&mut self, step: Step) {
        self.steps.push(step);
    }

    pub(crate) fn push_cells(&mut self, layer: LayerRef, edits: Vec<CellEdit>) {
        if !edits.is_empty() {
            self.steps.push(Step::Cells { layer, edits });
        }
    }

    /// Appends `other`'s steps (a later change) to this patch, merging
    /// consecutive edits of the same layer.
    pub(crate) fn absorb(&mut self, other: Patch) {
        for step in other.steps {
            match (self.steps.last_mut(), step) {
                (
                    Some(Step::Cells {
                        layer: last,
                        edits: last_edits,
                    }),
                    Step::Cells { layer, edits },
                ) if *last == layer => last_edits.extend(edits),
                (_, step) => self.steps.push(step),
            }
        }
    }

    /// Applies the patch. On failure the document is left as it was.
    pub fn apply(&self, ds1: &mut Ds1) -> Result<()> {
        for (i, step) in self.steps.iter().enumerate() {
            if let Err(e) = step.run(ds1, true) {
                for done in self.steps.iter().take(i).rev() {
                    let _ = done.run(ds1, false);
                }
                return Err(e);
            }
        }
        Ok(())
    }

    /// Undoes [`Patch::apply`]. On failure the document is left as it was.
    pub fn revert(&self, ds1: &mut Ds1) -> Result<()> {
        let n = self.steps.len();
        for (k, step) in self.steps.iter().rev().enumerate() {
            if let Err(e) = step.run(ds1, false) {
                for done in self.steps.iter().skip(n - k) {
                    let _ = done.run(ds1, true);
                }
                return Err(e);
            }
        }
        Ok(())
    }
}

fn mismatch(detail: &str) -> EditError {
    invalid(format!("patch does not match the document: {detail}"))
}

impl Step {
    /// Forward (`fwd`) or backward. A failing step changes nothing.
    fn run(&self, ds1: &mut Ds1, fwd: bool) -> Result<()> {
        match self {
            Step::Cells { layer, edits } => set_cells(ds1, *layer, edits, fwd),
            Step::InsertLayer { layer, data } => {
                if fwd {
                    insert_layer(ds1, *layer, data)
                } else {
                    remove_layer(ds1, *layer)
                }
            }
            Step::RemoveLayer { layer, data } => {
                if fwd {
                    remove_layer(ds1, *layer)
                } else {
                    insert_layer(ds1, *layer, data)
                }
            }
            Step::Resize { old, new, cut } => {
                let (from, to) = if fwd { (old, new) } else { (new, old) };
                if (ds1.width, ds1.height) != *from {
                    return Err(mismatch("map size"));
                }
                resize_all(ds1, to.0, to.1);
                if !fwd {
                    for &(layer, x, y, value) in cut {
                        if !layer.set(ds1, x, y, value) {
                            return Err(mismatch("cut cell"));
                        }
                    }
                }
                Ok(())
            }
            Step::Header { before, after } => {
                let (expect, set) = if fwd {
                    (before, after)
                } else {
                    (after, before)
                };
                if ds1.version != expect.version {
                    return Err(mismatch("version"));
                }
                set.write(ds1);
                Ok(())
            }
            Step::ObjectsInsert { items } => {
                if fwd {
                    insert_objects(ds1, items)
                } else {
                    remove_objects(ds1, items)
                }
            }
            Step::ObjectsRemove { items } => {
                if fwd {
                    remove_objects(ds1, items)
                } else {
                    insert_objects(ds1, items)
                }
            }
            Step::ObjectsSet { edits } => {
                if edits.iter().any(|(i, _, _)| *i >= ds1.objects.len()) {
                    return Err(mismatch("object index"));
                }
                for (i, before, after) in edits {
                    if let Some(slot) = ds1.objects.get_mut(*i) {
                        *slot = if fwd { *after } else { *before };
                    }
                }
                Ok(())
            }
            Step::Paths { before, after } => {
                ds1.npc_paths = if fwd { after.clone() } else { before.clone() };
                Ok(())
            }
            Step::Replace { before, after } => {
                *ds1 = if fwd {
                    (**after).clone()
                } else {
                    (**before).clone()
                };
                Ok(())
            }
        }
    }
}

fn set_cells(ds1: &mut Ds1, layer: LayerRef, edits: &[CellEdit], fwd: bool) -> Result<()> {
    if !layer.exists_in(ds1) {
        return Err(EditError::NoSuchLayer(layer));
    }
    if let Some(e) = edits.iter().find(|e| layer.get(ds1, e.x, e.y).is_none()) {
        return Err(EditError::OutOfBounds {
            x: i64::from(e.x),
            y: i64::from(e.y),
            width: ds1.width,
            height: ds1.height,
        });
    }
    if fwd {
        for e in edits {
            layer.set(ds1, e.x, e.y, e.after);
        }
    } else {
        for e in edits.iter().rev() {
            layer.set(ds1, e.x, e.y, e.before);
        }
    }
    Ok(())
}

fn insert_layer(ds1: &mut Ds1, layer: LayerRef, data: &LayerData) -> Result<()> {
    match (layer, data) {
        (LayerRef::Wall(i), LayerData::Wall(w)) if usize::from(i) <= ds1.walls.len() => {
            ds1.walls.insert(usize::from(i), w.clone());
            Ok(())
        }
        (LayerRef::Floor(i), LayerData::Floor(f)) if usize::from(i) <= ds1.floors.len() => {
            ds1.floors.insert(usize::from(i), f.clone());
            Ok(())
        }
        (LayerRef::Tag, LayerData::Tag(t)) if ds1.tag.is_none() => {
            ds1.tag = Some(t.clone());
            Ok(())
        }
        _ => Err(mismatch("cannot insert layer")),
    }
}

fn remove_layer(ds1: &mut Ds1, layer: LayerRef) -> Result<()> {
    if !layer.exists_in(ds1) || layer == LayerRef::Shadow {
        return Err(EditError::NoSuchLayer(layer));
    }
    match layer {
        LayerRef::Wall(i) => {
            ds1.walls.remove(usize::from(i));
        }
        LayerRef::Floor(i) => {
            ds1.floors.remove(usize::from(i));
        }
        LayerRef::Tag => ds1.tag = None,
        LayerRef::Shadow => {}
    }
    Ok(())
}

fn resize_all(ds1: &mut Ds1, w: u32, h: u32) {
    for wall in &mut ds1.walls {
        wall.tiles = wall.tiles.resized(w, h, Cell::EMPTY);
        wall.orientations = wall.orientations.resized(w, h, 0);
    }
    for floor in &mut ds1.floors {
        *floor = floor.resized(w, h, Cell::EMPTY);
    }
    ds1.shadow = ds1.shadow.resized(w, h, Cell::EMPTY);
    if let Some(tag) = &mut ds1.tag {
        *tag = tag.resized(w, h, 0);
    }
    ds1.width = w;
    ds1.height = h;
}

fn insert_objects(ds1: &mut Ds1, items: &[(usize, Object)]) -> Result<()> {
    let len = ds1.objects.len();
    if items
        .iter()
        .enumerate()
        .any(|(k, (i, _))| *i > len + k || (k > 0 && items.get(k - 1).is_some_and(|p| p.0 >= *i)))
    {
        return Err(mismatch("object insert index"));
    }
    for (i, object) in items {
        ds1.objects.insert(*i, *object);
    }
    Ok(())
}

fn remove_objects(ds1: &mut Ds1, items: &[(usize, Object)]) -> Result<()> {
    let len = ds1.objects.len();
    if items
        .iter()
        .enumerate()
        .any(|(k, (i, _))| *i >= len || (k > 0 && items.get(k - 1).is_some_and(|p| p.0 >= *i)))
    {
        return Err(mismatch("object remove index"));
    }
    for (i, _) in items.iter().rev() {
        ds1.objects.remove(*i);
    }
    Ok(())
}
