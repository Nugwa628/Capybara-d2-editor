//! Object edits. Objects are addressed by their index in `Ds1::objects`.

use std::collections::BTreeSet;

use d2_formats::ds1::{Ds1, NpcPath, Object};

use super::Document;
use crate::error::{EditError, Result, invalid};
use crate::patch::{Patch, Step};
use crate::version;

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

/// The `NeedsVersion` error for a result holding `objects`, when the file
/// version cannot store them (`None` when it can).
fn objects_need(ds1: &Ds1, objects: &[Object]) -> Option<EditError> {
    let required = version_needed(ds1, objects)?;
    let change = version::Change {
        objects: objects.to_vec(),
        ..version::Change::default()
    };
    Some(version::needs(ds1, &change, required))
}

/// `Format(Invalid)` when moving objects as `moves` (index, before, after)
/// would put an NPC path anchor in doubt: a destination sub-tile would hold
/// two objects and a path, or two paths. `paths_after` is the path list after
/// the move and `moved_paths` the indices in it that moved with the objects.
/// Only a destination that already held an object or a path that is not
/// moving is checked, so an ambiguous anchor moved as a whole stays allowed.
fn check_anchors(
    ds1: &Ds1,
    moves: &[(usize, Object, Object)],
    paths_after: &[NpcPath],
    moved_paths: &BTreeSet<usize>,
) -> Result<()> {
    let mut positions: Vec<(i32, i32)> = ds1.objects.iter().map(|o| (o.x, o.y)).collect();
    let mut moved = BTreeSet::new();
    for &(i, before, after) in moves {
        if (before.x, before.y) == (after.x, after.y) {
            continue;
        }
        if let Some(p) = positions.get_mut(i) {
            *p = (after.x, after.y);
            moved.insert(i);
        }
    }
    match anchor_conflicts(&positions, &moved, paths_after, moved_paths)
        .into_iter()
        .next()
    {
        Some(detail) => Err(invalid(detail)),
        None => Ok(()),
    }
}

/// One message per sub-tile where appending `objects` (indices from
/// `ds1.objects.len()` on, as `add_object` and `paste` number them) would put
/// an NPC path anchor in doubt, worded as for [`check_anchors`]. Empty when
/// the placement is fine.
pub(crate) fn placement_conflicts(ds1: &Ds1, objects: &[Object]) -> Vec<String> {
    let positions: Vec<(i32, i32)> = ds1
        .objects
        .iter()
        .chain(objects)
        .map(|o| (o.x, o.y))
        .collect();
    let placed: BTreeSet<usize> = (ds1.objects.len()..positions.len()).collect();
    anchor_conflicts(&positions, &placed, &ds1.npc_paths, &BTreeSet::new())
}

/// The anchor check shared by moves and placements. `positions` are every
/// object's sub-tile after the edit, `placed` the objects that moved there
/// (or are new), `paths_after` the path list after the edit and
/// `moved_paths` the indices in it that moved with the objects. A
/// destination is checked only when it holds an object or a path that did
/// not move; it is reported when it would hold two objects and a path, or
/// two paths. Returns one message per such sub-tile, in sub-tile order.
fn anchor_conflicts(
    positions: &[(i32, i32)],
    placed: &BTreeSet<usize>,
    paths_after: &[NpcPath],
    moved_paths: &BTreeSet<usize>,
) -> Vec<String> {
    let destinations: BTreeSet<(i32, i32)> = placed
        .iter()
        .filter_map(|&i| positions.get(i).copied())
        .collect();
    let mut conflicts = Vec::new();
    for (x, y) in destinations {
        let objects: Vec<usize> = (0..positions.len())
            .filter(|&i| positions[i] == (x, y))
            .collect();
        let paths: Vec<usize> = (0..paths_after.len())
            .filter(|&k| (paths_after[k].x, paths_after[k].y) == (x, y))
            .collect();
        let resident = objects.iter().copied().find(|i| !placed.contains(i));
        let resident_path = paths.iter().any(|k| !moved_paths.contains(k));
        if resident.is_none() && !resident_path {
            continue;
        }
        let Some(mover) = objects.iter().copied().find(|i| placed.contains(i)) else {
            continue;
        };
        let brings_path = paths.iter().any(|k| moved_paths.contains(k));
        let detail = match resident {
            Some(other) if !paths.is_empty() => {
                if brings_path {
                    format!(
                        "object {mover} has an NPC path and would share sub-tile ({x}, {y}) \
                         with object {other}"
                    )
                } else {
                    format!(
                        "object {mover} would share sub-tile ({x}, {y}) with object {other} \
                         that has an NPC path"
                    )
                }
            }
            _ if paths.len() >= 2 => format!(
                "object {mover} would leave {} NPC paths anchored on sub-tile ({x}, {y})",
                paths.len()
            ),
            _ if objects.len() >= 2 && !paths.is_empty() => {
                let other = objects
                    .iter()
                    .copied()
                    .find(|&i| i != mover)
                    .unwrap_or(mover);
                format!(
                    "objects {mover} and {other} would share sub-tile ({x}, {y}), \
                     where an NPC path is anchored"
                )
            }
            _ => continue,
        };
        conflicts.push(format!(
            "{detail}; NPC paths are keyed by sub-tile, so the path would be ambiguous"
        ));
    }
    conflicts
}

impl Document {
    /// Appends `object` and returns its index. `NeedsVersion` for a file below
    /// version 2 (no objects) or, for an object with flags, below version 6.
    ///
    /// `Format(Invalid)` (and nothing changes) when the object would make an
    /// NPC path anchor ambiguous: its sub-tile holds an object with a path, or
    /// two paths. Joining an object without a path, or a stray path with no
    /// object (the new object takes it over), is allowed.
    pub fn add_object(&mut self, object: Object) -> Result<usize> {
        if let Some(detail) = placement_conflicts(&self.ds1, &[object]).into_iter().next() {
            return Err(invalid(detail));
        }
        if let Some(e) = objects_need(&self.ds1, &[object]) {
            return Err(e);
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
    ///
    /// `Format(Invalid)` (and nothing moves) when a moved object would land on
    /// a sub-tile that holds another object or a path and the result would
    /// leave two objects with a path there, or two paths: NPC paths are keyed
    /// by sub-tile, so the anchor would be ambiguous. Moves where no path is
    /// involved are allowed, and so is moving an already ambiguous sub-tile as
    /// a whole.
    pub fn move_objects(&mut self, indices: &[usize], dx: i32, dy: i32) -> Result<()> {
        let indices = checked_indices(&self.ds1, indices)?;
        if (dx == 0 && dy == 0) || indices.is_empty() {
            return Ok(());
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
        let moved_paths: BTreeSet<usize> = self
            .ds1
            .npc_paths
            .iter()
            .enumerate()
            .filter(|(_, p)| path_anchored_in(&self.ds1, p, &set))
            .map(|(k, _)| k)
            .collect();
        check_anchors(&self.ds1, &edits, &paths, &moved_paths)?;
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

    /// Replaces the object at `index` (type, id, flags and position). If the
    /// position changes, the NPC path anchored only on this object follows it
    /// (a path on a sub-tile shared with another object stays put, as with
    /// [`Document::move_objects`]). Non-zero flags need v6 (`NeedsVersion`
    /// with the lowest version the map can be upgraded to that holds them);
    /// a bad index is `Format(Invalid)`. Setting an identical object is a no-op.
    ///
    /// `Format(Invalid)` when the new position would leave two objects with an
    /// NPC path, or two paths, on one sub-tile (moving a pathed object onto
    /// another object, or any object onto a pathed object's sub-tile).
    pub fn set_object(&mut self, index: usize, object: Object) -> Result<()> {
        let Some(&before) = self.ds1.objects.get(index) else {
            return Err(invalid(format!(
                "object index {index} is out of range ({} objects)",
                self.ds1.objects.len()
            )));
        };
        if before == object {
            return Ok(());
        }
        let edits = vec![(index, before, object)];
        let mut path_step = None;
        if (before.x, before.y) != (object.x, object.y) {
            let set: BTreeSet<usize> = std::iter::once(index).collect();
            let anchored = self.ds1.npc_paths.iter().position(|p| {
                path_anchored_in(&self.ds1, p, &set) && p.x == before.x && p.y == before.y
            });
            let mut paths_after = self.ds1.npc_paths.clone();
            let mut moved_paths = BTreeSet::new();
            if let Some(at) = anchored
                && let Some(path) = paths_after.get_mut(at)
            {
                let old = path.clone();
                path.x = object.x;
                path.y = object.y;
                moved_paths.insert(at);
                path_step = Some(Step::PathSet {
                    at,
                    before: old,
                    after: path.clone(),
                });
            }
            check_anchors(&self.ds1, &edits, &paths_after, &moved_paths)?;
        }
        if let Some(e) = objects_need(&self.ds1, &[object]) {
            return Err(e);
        }
        let mut patch = Patch::new("Edit object");
        patch.push(Step::ObjectsSet { edits });
        if let Some(step) = path_step {
            patch.push(step);
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
