//! Copy, cut and paste.

use std::collections::BTreeMap;

use d2_formats::ds1::{Ds1, Object};

use super::Document;
use super::objects::{removal_steps, version_needed};
use crate::clipboard::{ClipSource, Clipboard, PasteConflict, PasteMode, PastePreview};
use crate::error::{EditError, Result};
use crate::layer::{CellValue, LayerRef};
use crate::layer_ops;
use crate::patch::{CellEdit, Patch, Step};
use crate::selection::{CellRect, Selection};

/// Sub-tiles per cell along one axis.
const SUBTILES: i64 = 5;

/// The cells of `selection` that exist in `ds1`, with their current values.
fn selected_cells(ds1: &Ds1, selection: &Selection) -> Vec<(LayerRef, u32, u32, CellValue)> {
    selection
        .iter()
        .filter_map(|(layer, x, y)| layer.get(ds1, x, y).map(|v| (layer, x, y, v)))
        .collect()
}

fn bounds_of(cells: &[(LayerRef, u32, u32, CellValue)]) -> Option<CellRect> {
    let first = cells.first()?;
    let mut r = CellRect {
        x0: first.1,
        y0: first.2,
        x1: first.1,
        y1: first.2,
    };
    for &(_, x, y, _) in cells {
        r.x0 = r.x0.min(x);
        r.y0 = r.y0.min(y);
        r.x1 = r.x1.max(x);
        r.y1 = r.y1.max(y);
    }
    Some(r)
}

/// Indices of the objects whose sub-tile lies inside `rect`.
fn objects_in(ds1: &Ds1, rect: CellRect) -> Vec<usize> {
    let x0 = i64::from(rect.x0) * SUBTILES;
    let y0 = i64::from(rect.y0) * SUBTILES;
    let x1 = (i64::from(rect.x1) + 1) * SUBTILES;
    let y1 = (i64::from(rect.y1) + 1) * SUBTILES;
    ds1.objects
        .iter()
        .enumerate()
        .filter(|(_, o)| (x0..x1).contains(&i64::from(o.x)) && (y0..y1).contains(&i64::from(o.y)))
        .map(|(i, _)| i)
        .collect()
}

fn layer_is_empty(ds1: &Ds1, layer: LayerRef) -> bool {
    match layer {
        LayerRef::Floor(i) => ds1
            .floors
            .get(usize::from(i))
            .is_none_or(|g| g.as_slice().iter().all(|c| c.is_empty())),
        LayerRef::Wall(i) => ds1
            .walls
            .get(usize::from(i))
            .is_none_or(|w| w.tiles.as_slice().iter().all(|c| c.is_empty())),
        LayerRef::Shadow => ds1.shadow.as_slice().iter().all(|c| c.is_empty()),
        LayerRef::Tag => ds1
            .tag
            .as_ref()
            .is_none_or(|g| g.as_slice().iter().all(|&c| c == 0)),
    }
}

struct PastePlan {
    preview: PastePreview,
    edits: Vec<(LayerRef, Vec<CellEdit>)>,
    objects: Vec<Object>,
}

impl Document {
    /// Copies the selected cells (those that exist) and the objects whose
    /// sub-tile lies inside the selection's bounding rectangle.
    pub fn copy(&self, selection: &Selection) -> Clipboard {
        let ds1 = &self.ds1;
        let source = ClipSource {
            game_path: self.game_path.clone(),
            act: ds1.act,
            lvl_type: self.lvl_type,
            version: ds1.version,
        };
        let cells = selected_cells(ds1, selection);
        let Some(rect) = bounds_of(&cells) else {
            return Clipboard {
                width: 0,
                height: 0,
                cells: Vec::new(),
                objects: Vec::new(),
                source,
            };
        };
        let (ox, oy) = (i64::from(rect.x0) * SUBTILES, i64::from(rect.y0) * SUBTILES);
        let objects = objects_in(ds1, rect)
            .into_iter()
            .filter_map(|i| ds1.objects.get(i))
            .filter_map(|o| {
                Some(Object {
                    x: i32::try_from(i64::from(o.x) - ox).ok()?,
                    y: i32::try_from(i64::from(o.y) - oy).ok()?,
                    ..*o
                })
            })
            .collect();
        Clipboard {
            width: rect.x1 - rect.x0 + 1,
            height: rect.y1 - rect.y0 + 1,
            cells: cells
                .into_iter()
                .map(|(l, x, y, v)| (l, x - rect.x0, y - rect.y0, v))
                .collect(),
            objects,
            source,
        }
    }

    /// `copy`, then empties the selected cells and deletes the objects inside
    /// the selection's bounds (with the NPC paths anchored only on them).
    pub fn cut(&mut self, selection: &Selection) -> Result<Clipboard> {
        let clip = self.copy(selection);
        let cells = selected_cells(&self.ds1, selection);
        let mut patch = Patch::new("Cut");
        let mut run: Option<(LayerRef, Vec<CellEdit>)> = None;
        for (layer, x, y, before) in cells.iter().copied() {
            if before == CellValue::EMPTY {
                continue;
            }
            let edit = CellEdit {
                x,
                y,
                before,
                after: CellValue::EMPTY,
            };
            match &mut run {
                Some((l, edits)) if *l == layer => edits.push(edit),
                _ => {
                    if let Some((l, edits)) = run.take() {
                        patch.push_cells(l, edits);
                    }
                    run = Some((layer, vec![edit]));
                }
            }
        }
        if let Some((l, edits)) = run {
            patch.push_cells(l, edits);
        }
        if let Some(rect) = bounds_of(&cells) {
            for step in removal_steps(&self.ds1, &objects_in(&self.ds1, rect)) {
                patch.push(step);
            }
        }
        self.commit(patch)?;
        Ok(clip)
    }

    /// What `paste` would do, without changing anything.
    pub fn preview_paste(&self, clip: &Clipboard, at: (i32, i32), mode: PasteMode) -> PastePreview {
        self.plan_paste(clip, at, mode).preview
    }

    /// Pastes `clip` with its top-left cell at `at`, adding the layers it needs.
    /// Cells outside the map are dropped (counted in `out_of_bounds`).
    /// `NeedsVersion` when the clip needs layers or objects the file version
    /// cannot store; `Empty` when nothing would change.
    pub fn paste(
        &mut self,
        clip: &Clipboard,
        at: (i32, i32),
        mode: PasteMode,
    ) -> Result<PastePreview> {
        let plan = self.plan_paste(clip, at, mode);
        if let Some(required) = plan.preview.needs_version {
            return Err(EditError::NeedsVersion {
                current: self.ds1.version,
                required,
            });
        }
        let mut patch = Patch::new("Paste");
        for step in layer_ops::add_steps(&self.ds1, &plan.preview.layers_to_add) {
            patch.push(step);
        }
        for (layer, edits) in plan.edits {
            patch.push_cells(layer, edits);
        }
        if !plan.objects.is_empty() {
            let len = self.ds1.objects.len();
            patch.push(Step::ObjectsInsert {
                items: plan
                    .objects
                    .into_iter()
                    .enumerate()
                    .map(|(k, o)| (len + k, o))
                    .collect(),
            });
        }
        self.commit(patch)?;
        Ok(plan.preview)
    }

    fn plan_paste(&self, clip: &Clipboard, at: (i32, i32), mode: PasteMode) -> PastePlan {
        let ds1 = &self.ds1;
        let mut preview = PastePreview::default();

        if clip.source.act != ds1.act {
            preview.warnings.push(format!(
                "the clipboard is from act {} but this map is act {}: its tiles may not exist in this map's DT1 set",
                i64::from(clip.source.act) + 1,
                i64::from(ds1.act) + 1
            ));
        }
        if let (Some(a), Some(b)) = (clip.source.lvl_type, self.lvl_type)
            && a != b
        {
            preview.warnings.push(format!(
                "the clipboard is from LvlType {a} but this map is LvlType {b}: its tiles may not exist in this map's DT1 set"
            ));
        }

        // Group by target layer; a cell listed twice keeps its last value.
        let mut per_layer: BTreeMap<LayerRef, BTreeMap<(u32, u32), CellValue>> = BTreeMap::new();
        let mut unusable = 0usize;
        for &(layer, cx, cy, value) in &clip.cells {
            if layer_ops::check_limit(layer).is_err() {
                unusable += 1;
                continue;
            }
            let tx = i64::from(at.0) + i64::from(cx);
            let ty = i64::from(at.1) + i64::from(cy);
            match (u32::try_from(tx), u32::try_from(ty)) {
                (Ok(x), Ok(y)) if x < ds1.width && y < ds1.height => {
                    per_layer
                        .entry(layer)
                        .or_default()
                        .insert((x, y), layer.normalize(value));
                }
                _ => preview.out_of_bounds += 1,
            }
        }
        if unusable > 0 {
            preview.warnings.push(format!(
                "{unusable} clipboard cell(s) are on layers a map cannot have and were ignored"
            ));
        }

        let mut edits = Vec::new();
        let mut wanted = Vec::new();
        for (layer, cells) in per_layer {
            let exists = layer.exists_in(ds1);
            let layer_empty = layer_is_empty(ds1, layer);
            let mut layer_edits = Vec::new();
            for ((x, y), incoming) in cells {
                let existing = layer.get(ds1, x, y).unwrap_or(CellValue::EMPTY);
                let occupied = !layer.is_empty_value(existing);
                if occupied && existing != incoming {
                    preview.conflicts.push(PasteConflict {
                        layer,
                        x,
                        y,
                        existing,
                        incoming,
                    });
                }
                let allowed = match mode {
                    PasteMode::Overwrite => true,
                    PasteMode::SkipOccupied => !occupied,
                    PasteMode::OnlyIntoEmptyLayers => layer_empty,
                };
                if allowed && existing != incoming {
                    layer_edits.push(CellEdit {
                        x,
                        y,
                        before: existing,
                        after: incoming,
                    });
                }
            }
            if !layer_edits.is_empty() {
                preview.writes += layer_edits.len();
                if !exists {
                    wanted.push(layer);
                }
                edits.push((layer, layer_edits));
            }
        }
        let missing = layer_ops::missing_layers(ds1, &wanted).unwrap_or_default();
        let mut needs = layer_ops::version_needed(ds1, &missing);

        let mut objects = Vec::new();
        let mut dropped = 0usize;
        let (max_x, max_y) = (
            i64::from(ds1.width) * SUBTILES,
            i64::from(ds1.height) * SUBTILES,
        );
        for o in &clip.objects {
            let x = i64::from(at.0) * SUBTILES + i64::from(o.x);
            let y = i64::from(at.1) * SUBTILES + i64::from(o.y);
            match (i32::try_from(x), i32::try_from(y)) {
                (Ok(x), Ok(y))
                    if (0..max_x).contains(&i64::from(x)) && (0..max_y).contains(&i64::from(y)) =>
                {
                    objects.push(Object { x, y, ..*o });
                }
                _ => dropped += 1,
            }
        }
        if dropped > 0 {
            preview.warnings.push(format!(
                "{dropped} clipboard object(s) fall outside the map and were dropped"
            ));
        }
        if let Some(required) = version_needed(ds1, &objects) {
            needs = needs.max(Some(required));
        }

        preview.layers_to_add = missing;
        preview.needs_version = needs;
        PastePlan {
            preview,
            edits,
            objects,
        }
    }
}
