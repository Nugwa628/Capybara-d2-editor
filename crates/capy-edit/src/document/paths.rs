//! NPC path edits. A path belongs to the object standing on its anchor
//! sub-tile; the DS1 stores no other link, so an object whose sub-tile is
//! shared with another object has no unambiguous path (the game drops it).
//!
//! Every method checks the request against the document first and the file
//! version last, so a `NeedsVersion` always means the same call succeeds
//! after `upgrade_version(required)`.

use std::collections::BTreeSet;

use d2_formats::ds1::{NpcPath, Object, PathPoint};

use super::Document;
use crate::error::{Result, invalid};
use crate::patch::{Patch, Step};
use crate::version;

/// First version that stores NPC paths.
const PATH_VERSION: i32 = 14;
/// First version that stores a path point's `action`.
const ACTION_VERSION: i32 = 15;

impl Document {
    fn object_at(&self, index: usize) -> Result<Object> {
        self.ds1.objects.get(index).copied().ok_or_else(|| {
            invalid(format!(
                "object index {index} is out of range ({} objects)",
                self.ds1.objects.len()
            ))
        })
    }

    /// `NeedsVersion` unless the file stores NPC paths and, for `Some(action)`
    /// other than 1, path point actions.
    fn require_path_version(&self, action: Option<i32>) -> Result<()> {
        let mut at_least = PATH_VERSION;
        if action.is_some_and(|a| a != 1) {
            at_least = ACTION_VERSION;
        }
        if self.ds1.version >= at_least {
            return Ok(());
        }
        let change = version::Change {
            path: Some(
                action
                    .map(|action| PathPoint { x: 0, y: 0, action })
                    .into_iter()
                    .collect(),
            ),
            ..version::Change::default()
        };
        Err(version::needs(&self.ds1, &change, at_least))
    }

    /// Position in `ds1.npc_paths` of the first path anchored on `object`'s sub-tile.
    fn path_position(&self, object: &Object) -> Option<usize> {
        self.ds1
            .npc_paths
            .iter()
            .position(|p| p.x == object.x && p.y == object.y)
    }

    /// Position of the path of the object at `object_index`, or `Format(Invalid)`.
    fn existing_path(&self, object_index: usize, object: &Object) -> Result<usize> {
        self.path_position(object)
            .ok_or_else(|| invalid(format!("object {object_index} has no NPC path")))
    }

    /// The NPC path of the object at `object_index`: the path anchored on its
    /// sub-tile. `None` when there is none or the index is out of range. If
    /// another object stands on the same sub-tile the path is still returned
    /// (the model keeps it), but see [`Document::add_path_point`].
    pub fn path_of(&self, object_index: usize) -> Option<&NpcPath> {
        let object = self.ds1.objects.get(object_index)?;
        self.ds1
            .npc_paths
            .iter()
            .find(|p| p.x == object.x && p.y == object.y)
    }

    /// Inserts `point` into the NPC path of the object at `object_index`,
    /// at point position `at` (`None` = the end), creating the path when the
    /// object has none (then `at` must be `None` or `Some(0)`).
    ///
    /// Errors: `Format(Invalid)` for a bad object index, a bad `at`, or when
    /// another object stands on the same sub-tile (the path would be ambiguous
    /// and the game would drop it); then `NeedsVersion` below v14, or below
    /// v15 for an `action` other than 1.
    pub fn add_path_point(
        &mut self,
        object_index: usize,
        point: PathPoint,
        at: Option<usize>,
    ) -> Result<()> {
        let object = self.object_at(object_index)?;
        let sharing = self
            .ds1
            .objects
            .iter()
            .enumerate()
            .any(|(i, o)| i != object_index && o.x == object.x && o.y == object.y);
        if sharing {
            return Err(invalid(format!(
                "object {object_index} shares its sub-tile ({}, {}) with another object, \
                 so an NPC path on it would be ambiguous",
                object.x, object.y
            )));
        }
        let step = match self.path_position(&object) {
            Some(pos) => {
                let before = self.ds1.npc_paths[pos].clone();
                let mut after = before.clone();
                let at = at.unwrap_or(after.points.len());
                if at > after.points.len() {
                    return Err(invalid(format!(
                        "path point position {at} is past the end ({} points)",
                        after.points.len()
                    )));
                }
                after.points.insert(at, point);
                Step::PathSet {
                    at: pos,
                    before,
                    after,
                }
            }
            None => {
                if at.is_some_and(|a| a != 0) {
                    return Err(invalid("a new path has no point to insert after"));
                }
                Step::PathInsert {
                    at: self.ds1.npc_paths.len(),
                    path: NpcPath {
                        x: object.x,
                        y: object.y,
                        points: vec![point],
                    },
                }
            }
        };
        self.require_path_version(Some(point.action))?;
        let mut patch = Patch::new("Add path point");
        patch.push(step);
        self.commit(patch)
    }

    /// Moves point `point_index` of the object's path to `(x, y)` (sub-tiles).
    /// `Format(Invalid)` when the object has no path or no such point (so a
    /// map below v14, which has no paths, gets `Format(Invalid)`).
    pub fn move_path_point(
        &mut self,
        object_index: usize,
        point_index: usize,
        x: i32,
        y: i32,
    ) -> Result<()> {
        self.edit_point("Move path point", object_index, point_index, None, |p| {
            p.x = x;
            p.y = y;
        })
    }

    /// Sets the action of point `point_index`. `Format(Invalid)` when the
    /// object has no path or no such point; an action other than 1 needs v15
    /// (`NeedsVersion { required: 15 }` on a v14 map).
    pub fn set_path_action(
        &mut self,
        object_index: usize,
        point_index: usize,
        action: i32,
    ) -> Result<()> {
        self.edit_point(
            "Set path action",
            object_index,
            point_index,
            Some(action),
            |p| {
                p.action = action;
            },
        )
    }

    fn edit_point(
        &mut self,
        label: &str,
        object_index: usize,
        point_index: usize,
        action: Option<i32>,
        change: impl FnOnce(&mut PathPoint),
    ) -> Result<()> {
        let object = self.object_at(object_index)?;
        let pos = self.existing_path(object_index, &object)?;
        let before = self.ds1.npc_paths[pos].clone();
        let mut after = before.clone();
        let len = after.points.len();
        let point = after.points.get_mut(point_index).ok_or_else(|| {
            invalid(format!(
                "path point {point_index} is out of range ({len} points)"
            ))
        })?;
        change(point);
        self.require_path_version(action)?;
        let mut patch = Patch::new(label);
        if before != after {
            patch.push(Step::PathSet {
                at: pos,
                before,
                after,
            });
        }
        self.commit(patch)
    }

    /// Deletes the points at `point_indices` (duplicates ignored). The path is
    /// removed when no point is left. An empty list is a no-op;
    /// `Format(Invalid)` when the object has no path or no such point.
    pub fn delete_path_points(
        &mut self,
        object_index: usize,
        point_indices: &[usize],
    ) -> Result<()> {
        let object = self.object_at(object_index)?;
        if point_indices.is_empty() {
            return Ok(());
        }
        let pos = self.existing_path(object_index, &object)?;
        let before = self.ds1.npc_paths[pos].clone();
        let doomed: BTreeSet<usize> = point_indices.iter().copied().collect();
        if let Some(bad) = doomed.iter().find(|&&i| i >= before.points.len()) {
            return Err(invalid(format!(
                "path point {bad} is out of range ({} points)",
                before.points.len()
            )));
        }
        self.require_path_version(None)?;
        let mut after = before.clone();
        after.points = before
            .points
            .iter()
            .enumerate()
            .filter(|(i, _)| !doomed.contains(i))
            .map(|(_, p)| *p)
            .collect();
        let mut patch = Patch::new("Delete path points");
        if after.points.is_empty() {
            patch.push(Step::PathRemove {
                at: pos,
                path: before,
            });
        } else {
            patch.push(Step::PathSet {
                at: pos,
                before,
                after,
            });
        }
        self.commit(patch)
    }
}
