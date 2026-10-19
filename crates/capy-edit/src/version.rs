//! The lowest DS1 version that can hold the result of an edit: the value of
//! [`EditError::NeedsVersion`]'s `required`.
//!
//! An edit's own gate (4 for a second wall, 6 for object flags, ...) is not
//! enough: `Document::upgrade_version` must also be able to reach that
//! version (a v3 map with tag cells cannot become v4..=9, which store no tag
//! layer), and the edit must then succeed on the upgraded model. The search
//! replays both on a skeleton of the document (every grid 1x1, the tag layer
//! reduced to one cell saying whether it holds anything) and asks the writer
//! whether the result fits, so it costs the same on any map size. It follows
//! [`Ds1::min_version_for_model`]'s candidate list.

use d2_formats::Grid;
use d2_formats::ds1::{Ds1, NpcPath, Object, PathPoint, Preserved, WallLayer};

use crate::error::EditError;
use crate::layer::LayerRef;
use crate::layer_ops;

/// The versions at which some part of the layout appears (as in
/// `Ds1::min_version_for_model`).
const GATES: [i32; 12] = [2, 3, 4, 6, 8, 10, 12, 13, 14, 15, 16, 18];

/// What an edit adds to the model, for the version search.
#[derive(Clone, Debug, Default)]
pub(crate) struct Change {
    /// Layers the edit writes to; missing ones are created as
    /// `layer_ops::add_steps` would.
    pub layers: Vec<LayerRef>,
    /// Objects the result holds that the current model may not (added or rewritten).
    pub objects: Vec<Object>,
    /// `Some(points)` when the result holds an NPC path with these points.
    pub path: Option<Vec<PathPoint>>,
}

/// What `upgrade_version` does with an existing tag layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TagFate {
    /// The new version stores it as it is.
    Keep,
    /// All zero and the new version has no room for it: removed.
    Drop,
    /// Kept by setting `tag_type` to 1 (version 10 or later).
    Retype,
    /// Non-empty and the new version (4..=9) cannot store it.
    Blocked,
}

/// The fate of a tag layer (`all_zero` = it holds nothing) when the file
/// becomes `version` with the given `tag_type`.
pub(crate) fn tag_fate(version: i32, tag_type: i32, all_zero: bool) -> TagFate {
    if version < 4 || (version >= 10 && (1..=2).contains(&tag_type)) {
        TagFate::Keep
    } else if all_zero {
        TagFate::Drop
    } else if version >= 10 {
        TagFate::Retype
    } else {
        TagFate::Blocked
    }
}

/// The `NeedsVersion` error for `change` on `ds1`, given that the edit's own
/// gate asks for at least `at_least`.
pub(crate) fn needs(ds1: &Ds1, change: &Change, at_least: i32) -> EditError {
    EditError::NeedsVersion {
        current: ds1.version,
        required: required(ds1, change, at_least),
    }
}

/// The smallest version `>= at_least` (and above the current one) to which
/// `ds1` can be upgraded so that `change` then succeeds and the result can be
/// written. When none can (the model is inconsistent), `max(at_least, 18)`,
/// like `Ds1::min_version_for_model`.
pub(crate) fn required(ds1: &Ds1, change: &Change, at_least: i32) -> i32 {
    let at_least = at_least.max(ds1.version.saturating_add(1));
    let base = skeleton(ds1);
    std::iter::once(at_least)
        .chain(GATES.iter().copied().filter(|&g| g > at_least))
        .find(|&v| fits(&base, change, v))
        .unwrap_or(at_least.max(18))
}

/// `ds1` with every grid shrunk to 1x1. A tag layer keeps one cell, non-zero
/// exactly when the real layer holds something.
fn skeleton(ds1: &Ds1) -> Ds1 {
    let tag = ds1.tag.as_ref().map(|t| {
        let mut g = Grid::new(1, 1);
        g.set(0, 0, u32::from(t.as_slice().iter().any(|&c| c != 0)));
        g
    });
    Ds1 {
        version: ds1.version,
        width: 1,
        height: 1,
        act: ds1.act,
        tag_type: ds1.tag_type,
        files: ds1.files.clone(),
        walls: ds1
            .walls
            .iter()
            .map(|_| WallLayer {
                tiles: Grid::new(1, 1),
                orientations: Grid::new(1, 1),
            })
            .collect(),
        floors: ds1.floors.iter().map(|_| Grid::new(1, 1)).collect(),
        shadow: Grid::new(1, 1),
        tag,
        objects: ds1.objects.clone(),
        groups: ds1.groups.clone(),
        npc_paths: ds1.npc_paths.clone(),
        preserved: Preserved {
            unknown_v9_13: ds1.preserved.unknown_v9_13,
            pre_group_dword: ds1.preserved.pre_group_dword,
            ..Preserved::default()
        },
    }
}

/// Whether upgrading the skeleton to `version` succeeds and `change` then
/// passes its own gates and leaves a writable model.
fn fits(base: &Ds1, change: &Change, version: i32) -> bool {
    let mut m = base.clone();
    if version > m.version && !upgrade(&mut m, version) {
        return false;
    }
    // `upgrade_version` checks its result with the writer too.
    if m.to_bytes().is_err() {
        return false;
    }
    let Ok(missing) = layer_ops::missing_layers(&m, &change.layers) else {
        return false;
    };
    if layer_ops::version_needed(&m, &missing).is_some() {
        return false;
    }
    for layer in missing {
        match layer {
            LayerRef::Wall(_) => m.walls.push(WallLayer {
                tiles: Grid::new(1, 1),
                orientations: Grid::new(1, 1),
            }),
            LayerRef::Floor(_) => m.floors.push(Grid::new(1, 1)),
            LayerRef::Tag => {
                if !(1..=2).contains(&m.tag_type) {
                    m.tag_type = 1;
                }
                m.tag = Some(Grid::new(1, 1));
            }
            LayerRef::Shadow => {}
        }
    }
    m.objects.extend(change.objects.iter().copied());
    if let Some(points) = &change.path {
        m.npc_paths.push(NpcPath {
            x: 0,
            y: 0,
            points: points.clone(),
        });
    }
    m.to_bytes().is_ok()
}

/// What `Document::upgrade_version` does to the model's layout; `false` when
/// it would refuse.
fn upgrade(m: &mut Ds1, version: i32) -> bool {
    if !(9..=13).contains(&version) {
        m.preserved.unknown_v9_13 = [0, 0];
    }
    if let Some(tag) = &m.tag {
        let all_zero = tag.as_slice().iter().all(|&c| c == 0);
        match tag_fate(version, m.tag_type, all_zero) {
            TagFate::Keep => {}
            TagFate::Drop => m.tag = None,
            TagFate::Retype => m.tag_type = 1,
            TagFate::Blocked => return false,
        }
    }
    m.version = version;
    true
}
