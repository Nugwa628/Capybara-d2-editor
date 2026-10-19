//! NPC path edits. A path belongs to the object standing on its anchor
//! sub-tile; the DS1 stores no other link, so an object whose sub-tile is
//! shared with another object has no unambiguous path (the game drops it).

use std::collections::BTreeSet;

use d2_formats::ds1::{NpcPath, Object, PathPoint};

use super::Document;
use crate::error::{EditError, Result, invalid};
use crate::patch::{Patch, Step};

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

    fn require_path_version(&self) -> Result<()> {
        if self.ds1.version < PATH_VERSION {
            return Err(EditError::NeedsVersion {
                current: self.ds1.version,
                required: PATH_VERSION,
            });
        }
        Ok(())
    }

    fn require_action_version(&self, action: i32) -> Result<()> {
        if action != 1 && self.ds1.version < ACTION_VERSION {
            return Err(EditError::NeedsVersion {
                current: self.ds1.version,
                required: ACTION_VERSION,
            });
        }
        Ok(())
    }

    /// Position in `ds1.npc_paths` of the first path anchored on `object`'s sub-tile.
    fn path_position(&self, object: &Object) -> Option<usize> {
        self.ds1
            .npc_paths
            .iter()
            .position(|p| p.x == object.x && p.y == object.y)
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
    /// Errors: `NeedsVersion { required: 14 }` below v14; `required: 15` for an
    /// `action` other than 1 below v15; `Format(Invalid)` for a bad object
    /// index, a bad `at`, or when another object stands on the same sub-tile
    /// (the path would be ambiguous and the game would drop it).
    pub fn add_path_point(
        &mut self,
        object_index: usize,
        point: PathPoint,
        at: Option<usize>,
    ) -> Result<()> {
        let object = self.object_at(object_index)?;
        self.require_path_version()?;
        self.require_action_version(point.action)?;
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
        let mut patch = Patch::new("Add path point");
        match self.path_position(&object) {
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
                patch.push(Step::PathSet {
                    at: pos,
                    before,
                    after,
                });
            }
            None => {
                if at.is_some_and(|a| a != 0) {
                    return Err(invalid("a new path has no point to insert after"));
                }
                patch.push(Step::PathInsert {
                    at: self.ds1.npc_paths.len(),
                    path: NpcPath {
                        x: object.x,
                        y: object.y,
                        points: vec![point],
                    },
                });
            }
        }
        self.commit(patch)
    }

    /// Moves point `point_index` of the object's path to `(x, y)` (sub-tiles).
    /// `Format(Invalid)` when the object has no path or no such point.
    pub fn move_path_point(
        &mut self,
        object_index: usize,
        point_index: usize,
        x: i32,
        y: i32,
    ) -> Result<()> {
        self.object_at(object_index)?;
        self.require_path_version()?;
        self.edit_point("Move path point", object_index, point_index, |p| {
            p.x = x;
            p.y = y;
        })
    }

    /// Sets the action of point `point_index`. An action other than 1 needs v15
    /// (`NeedsVersion { required: 15 }` on a v14 map).
    pub fn set_path_action(
        &mut self,
        object_index: usize,
        point_index: usize,
        action: i32,
    ) -> Result<()> {
        self.object_at(object_index)?;
        self.require_path_version()?;
        self.require_action_version(action)?;
        self.edit_point("Set path action", object_index, point_index, |p| {
            p.action = action;
        })
    }

    fn edit_point(
        &mut self,
        label: &str,
        object_index: usize,
        point_index: usize,
        change: impl FnOnce(&mut PathPoint),
    ) -> Result<()> {
        let object = self.object_at(object_index)?;
        let pos = self
            .path_position(&object)
            .ok_or_else(|| invalid(format!("object {object_index} has no NPC path")))?;
        let before = self.ds1.npc_paths[pos].clone();
        let mut after = before.clone();
        let len = after.points.len();
        let point = after.points.get_mut(point_index).ok_or_else(|| {
            invalid(format!(
                "path point {point_index} is out of range ({len} points)"
            ))
        })?;
        change(point);
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
    /// removed when no point is left.
    pub fn delete_path_points(
        &mut self,
        object_index: usize,
        point_indices: &[usize],
    ) -> Result<()> {
        let object = self.object_at(object_index)?;
        self.require_path_version()?;
        if point_indices.is_empty() {
            return Ok(());
        }
        let pos = self
            .path_position(&object)
            .ok_or_else(|| invalid(format!("object {object_index} has no NPC path")))?;
        let before = self.ds1.npc_paths[pos].clone();
        let doomed: BTreeSet<usize> = point_indices.iter().copied().collect();
        if let Some(bad) = doomed.iter().find(|&&i| i >= before.points.len()) {
            return Err(invalid(format!(
                "path point {bad} is out of range ({} points)",
                before.points.len()
            )));
        }
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
