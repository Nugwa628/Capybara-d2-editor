//! Planning of layer additions, shared by `add_layer`, layer moves and paste.

use std::collections::BTreeSet;

use d2_formats::ds1::{Ds1, MAX_FLOOR_LAYERS, MAX_WALL_LAYERS};

use crate::error::{EditError, Result};
use crate::layer::LayerRef;
use crate::patch::{Header, LayerData, Step};

/// Lowest DS1 version that can store `layer`.
pub(crate) fn min_version_for(layer: LayerRef) -> i32 {
    match layer {
        LayerRef::Wall(_) => 4,
        LayerRef::Floor(_) => 16,
        LayerRef::Tag => 10,
        LayerRef::Shadow => i32::MIN,
    }
}

/// Whether `layer` is a layer a map can ever hold.
pub(crate) fn check_limit(layer: LayerRef) -> Result<()> {
    match layer {
        LayerRef::Wall(i) if usize::from(i) >= MAX_WALL_LAYERS => Err(EditError::LayerLimit(
            format!("a map has at most {MAX_WALL_LAYERS} wall layers"),
        )),
        LayerRef::Floor(i) if usize::from(i) >= MAX_FLOOR_LAYERS => Err(EditError::LayerLimit(
            format!("a map has at most {MAX_FLOOR_LAYERS} floor layers"),
        )),
        _ => Ok(()),
    }
}

/// Every layer that must be created for all of `targets` to exist, in order
/// (floors, walls, tag), including the intermediate indices.
pub(crate) fn missing_layers(ds1: &Ds1, targets: &[LayerRef]) -> Result<Vec<LayerRef>> {
    let mut missing = BTreeSet::new();
    for &target in targets {
        check_limit(target)?;
        match target {
            LayerRef::Floor(i) => {
                for k in ds1.floors.len()..=usize::from(i) {
                    if let Ok(k) = u8::try_from(k) {
                        missing.insert(LayerRef::Floor(k));
                    }
                }
            }
            LayerRef::Wall(i) => {
                for k in ds1.walls.len()..=usize::from(i) {
                    if let Ok(k) = u8::try_from(k) {
                        missing.insert(LayerRef::Wall(k));
                    }
                }
            }
            LayerRef::Tag => {
                if ds1.tag.is_none() {
                    missing.insert(LayerRef::Tag);
                }
            }
            LayerRef::Shadow => {}
        }
    }
    Ok(missing.into_iter().collect())
}

/// The version the document must be upgraded to before `missing` can be
/// added, `None` when the current version already allows it.
pub(crate) fn version_needed(ds1: &Ds1, missing: &[LayerRef]) -> Option<i32> {
    let required = missing.iter().map(|l| min_version_for(*l)).max()?;
    (required > ds1.version).then_some(required)
}

/// The steps that create `missing` (already checked against the version).
pub(crate) fn add_steps(ds1: &Ds1, missing: &[LayerRef]) -> Vec<Step> {
    let mut steps = Vec::new();
    if missing.contains(&LayerRef::Tag) && !(1..=2).contains(&ds1.tag_type) {
        let before = Header::of(ds1);
        let mut after = before.clone();
        after.tag_type = 1;
        steps.push(Step::Header { before, after });
    }
    for &layer in missing {
        if let Some(data) = LayerData::empty(layer, ds1.width, ds1.height) {
            steps.push(Step::InsertLayer { layer, data });
        }
    }
    steps
}

/// Steps creating every layer of `targets` that is missing, or the error the
/// app should react to (`NeedsVersion`, `LayerLimit`).
pub(crate) fn ensure_layers(ds1: &Ds1, targets: &[LayerRef]) -> Result<Vec<Step>> {
    let missing = missing_layers(ds1, targets)?;
    if let Some(required) = version_needed(ds1, &missing) {
        return Err(EditError::NeedsVersion {
            current: ds1.version,
            required,
        });
    }
    Ok(add_steps(ds1, &missing))
}
