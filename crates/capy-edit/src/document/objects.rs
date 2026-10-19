//! Object edits. Objects are addressed by their index in `Ds1::objects`.

use std::collections::BTreeSet;

use d2_formats::ds1::{Ds1, NpcPath, Object};

use super::Document;
use crate::error::{EditError, Result, invalid};
use crate::patch::{Patch, Step};

/// Lowest version that can store all of `objects` (`None` when `ds1` can).
pub(crate) fn version_needed(ds1: &Ds1, objects: &[Object]) -> Option<i32> {
    let mut required = i32::MIN;
    if !objects.is_empty() {
        required = 2;
        if objects.iter().any(|o| o.flags != 0) {
            required = 6;
        }
    }
    (required > ds1.version).then_some(required)
}

/// Sorted, de-duplicated indices, all of which must exist.
fn checked_indices(ds1: &Ds1, indices: &[usize]) -> Result<Vec<usize>> {
    let sorted: BTreeSet<usize> = indices.iter().copied().collect();
    if let Some(bad) = sorted.iter().find(|&&i| i >= ds1.objects.len()) {
        return Err(invalid(format!(
            "object index {bad} is out of range ({} objects)",
            ds1.objects.len()
        )));
    }
    Ok(sorted.into_iter().collect())
}

/// Whether every object standing on the path's sub-tile is in `set` (and there is one).
fn path_anchored_in(ds1: &Ds1, path: &NpcPath, set: &BTreeSet<usize>) -> bool {
    let mut owners = ds1
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.x == path.x && o.y == path.y)
        .map(|(i, _)| i)
        .peekable();
    owners.peek().is_some() && owners.all(|i| set.contains(&i))
}

/// Steps that delete the objects at `indices` (sorted, valid) and the NPC
/// paths that were anchored only on them.
pub(crate) fn removal_steps(ds1: &Ds1, indices: &[usize]) -> Vec<Step> {
    let set: BTreeSet<usize> = indices.iter().copied().collect();
    let items: Vec<(usize, Object)> = indices
        .iter()
        .filter_map(|&i| ds1.objects.get(i).map(|o| (i, *o)))
        .collect();
    let mut steps = Vec::new();
    let paths: Vec<NpcPath> = ds1
        .npc_paths
        .iter()
        .filter(|p| !path_anchored_in(ds1, p, &set))
        .cloned()
        .collect();
    if !items.is_empty() {
        steps.push(Step::ObjectsRemove { items });
    }
    if paths.len() != ds1.npc_paths.len() {
        steps.push(Step::Paths {
            before: ds1.npc_paths.clone(),
            after: paths,
        });
    }
    steps
}

impl Document {
    /// Appends `object` and returns its index. `NeedsVersion` for a file below
    /// version 2 (no objects) or, for an object with flags, below version 6.
    pub fn add_object(&mut self, object: Object) -> Result<usize> {
        if let Some(required) = version_needed(&self.ds1, &[object]) {
            return Err(EditError::NeedsVersion {
                current: self.ds1.version,
                required,
            });
        }
        let index = self.ds1.objects.len();
        let mut patch = Patch::new("Add object");
        patch.push(Step::ObjectsInsert {
            items: vec![(index, object)],
        });
        self.commit(patch)?;
        Ok(index)
    }

    /// Moves the objects at `indices` by `(dx, dy)` sub-tiles (5 per cell).
    /// NPC paths anchored only on moved objects move with them. Objects may
    /// end up outside the map.
    pub fn move_objects(&mut self, indices: &[usize], dx: i32, dy: i32) -> Result<()> {
        let indices = checked_indices(&self.ds1, indices)?;
        if (dx == 0 && dy == 0) || indices.is_empty() {
            return Err(EditError::Empty);
        }
        let overflow = || invalid("object coordinate overflow");
        let mut edits = Vec::with_capacity(indices.len());
        for &i in &indices {
            let Some(&before) = self.ds1.objects.get(i) else {
                continue;
            };
            let after = Object {
                x: before.x.checked_add(dx).ok_or_else(overflow)?,
                y: before.y.checked_add(dy).ok_or_else(overflow)?,
                ..before
            };
            edits.push((i, before, after));
        }
        let set: BTreeSet<usize> = indices.iter().copied().collect();
        let mut paths = self.ds1.npc_paths.clone();
        let mut paths_moved = false;
        for path in &mut paths {
            if path_anchored_in(&self.ds1, path, &set) {
                path.x = path.x.checked_add(dx).ok_or_else(overflow)?;
                path.y = path.y.checked_add(dy).ok_or_else(overflow)?;
                paths_moved = true;
            }
        }
        let mut patch = Patch::new("Move objects");
        patch.push(Step::ObjectsSet { edits });
        if paths_moved {
            patch.push(Step::Paths {
                before: self.ds1.npc_paths.clone(),
                after: paths,
            });
        }
        self.commit(patch)
    }

    /// Deletes the objects at `indices`. NPC paths anchored only on them are
    /// removed too (both come back on undo).
    pub fn delete_objects(&mut self, indices: &[usize]) -> Result<()> {
        let indices = checked_indices(&self.ds1, indices)?;
        let mut patch = Patch::new("Delete objects");
        for step in removal_steps(&self.ds1, &indices) {
            patch.push(step);
        }
        self.commit(patch)
    }
}
