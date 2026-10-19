//! Clipboard data. Copy, cut and paste themselves are `Document` methods.

use d2_formats::ds1::Object;

use crate::layer::{CellValue, LayerRef};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipSource {
    pub game_path: Option<String>,
    pub act: i32,
    pub lvl_type: Option<u32>,
    pub version: i32,
}

/// Cells relative to the selection's top-left, plus objects inside the bounds.
/// Serializable so it can cross documents (and the OS clipboard as JSON).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Clipboard {
    pub width: u32,
    pub height: u32,
    pub cells: Vec<(LayerRef, u32, u32, CellValue)>,
    /// `x`/`y` relative to the clip origin, in sub-tiles (5 per cell).
    #[serde(with = "object_list")]
    pub objects: Vec<Object>,
    pub source: ClipSource,
}

impl Clipboard {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// `None` for anything that is not a clipboard written by [`Clipboard::to_json`].
    pub fn from_json(text: &str) -> Option<Clipboard> {
        serde_json::from_str(text).ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    /// Every clipboard cell replaces what is there.
    Overwrite,
    /// Only cells whose target is empty are written.
    SkipOccupied,
    /// Only layers that are entirely empty (or do not exist yet) are written.
    OnlyIntoEmptyLayers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteConflict {
    pub layer: LayerRef,
    pub x: u32,
    pub y: u32,
    pub existing: CellValue,
    pub incoming: CellValue,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PastePreview {
    /// Cells that will change.
    pub writes: usize,
    /// Non-empty targets with different content (reported in every mode,
    /// whether or not the mode overwrites them).
    pub conflicts: Vec<PasteConflict>,
    /// Clip cells that fall outside the map (dropped).
    pub out_of_bounds: usize,
    /// Layers that will be created, including intermediate indices.
    pub layers_to_add: Vec<LayerRef>,
    pub needs_version: Option<i32>,
    /// E.g. different act / LvlType: tiles may not exist in this map's DT1
    /// set. Also one entry per sub-tile where a pasted object would make an
    /// NPC path anchor ambiguous; `paste` refuses those with the same text.
    pub warnings: Vec<String>,
}

/// `Object` lives in `d2-formats` without serde; this mirrors it for the
/// clipboard's JSON.
mod object_list {
    use d2_formats::ds1::Object;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct ObjectDef {
        kind: i32,
        id: i32,
        x: i32,
        y: i32,
        flags: i32,
    }

    pub fn serialize<S: Serializer>(objects: &[Object], s: S) -> Result<S::Ok, S::Error> {
        let defs: Vec<ObjectDef> = objects
            .iter()
            .map(|o| ObjectDef {
                kind: o.kind,
                id: o.id,
                x: o.x,
                y: o.y,
                flags: o.flags,
            })
            .collect();
        defs.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Object>, D::Error> {
        let defs = Vec::<ObjectDef>::deserialize(d)?;
        Ok(defs
            .into_iter()
            .map(|o| Object {
                kind: o.kind,
                id: o.id,
                x: o.x,
                y: o.y,
                flags: o.flags,
            })
            .collect())
    }
}
