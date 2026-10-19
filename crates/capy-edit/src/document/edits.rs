//! Cell, layer, size and version edits.

use std::collections::BTreeMap;

use d2_formats::ds1::DIR_LOOKUP;

use super::{Document, MoveMode, MoveReport};
use crate::error::{EditError, Result, invalid};
use crate::layer::{CellValue, LayerRef};
use crate::layer_ops;
use crate::patch::{CellEdit, Header, LayerData, Patch, Step};
use crate::selection::Selection;
use crate::version::{self, TagFate};

/// Upper bound on `width * height` for `resize` (1024 x 1024 cells).
const MAX_CELLS: u64 = 1 << 20;

const HIDDEN_BIT: u32 = 1 << 31;
const UNWALKABLE_BIT: u32 = 1 << 17;

impl Document {
    /// Sets the given cells of `layer`. The orientation of a value is dropped
    /// for layers that are not walls. Later entries win over earlier ones for
    /// the same cell. Any coordinate outside the map is an error and nothing
    /// is painted.
    pub fn paint(&mut self, layer: LayerRef, cells: &[(u32, u32, CellValue)]) -> Result<()> {
        if !layer.exists_in(&self.ds1) {
            return Err(EditError::NoSuchLayer(layer));
        }
        let mut merged: BTreeMap<(u32, u32), (CellValue, CellValue)> = BTreeMap::new();
        for &(x, y, value) in cells {
            let before = layer.get(&self.ds1, x, y).ok_or(EditError::OutOfBounds {
                x: i64::from(x),
                y: i64::from(y),
                width: self.ds1.width,
                height: self.ds1.height,
            })?;
            let after = layer.normalize(value);
            merged
                .entry((x, y))
                .and_modify(|e| e.1 = after)
                .or_insert((before, after));
        }
        let edits = merged
            .into_iter()
            .filter(|(_, (before, after))| before != after)
            .map(|((x, y), (before, after))| CellEdit {
                x,
                y,
                before,
                after,
            })
            .collect();
        let mut patch = Patch::new("Paint");
        patch.push_cells(layer, edits);
        self.commit(patch)
    }

    /// Sets the selected cells to empty. Selected cells outside the map are ignored.
    pub fn erase(&mut self, selection: &Selection) -> Result<()> {
        let mut patch = Patch::new("Erase");
        for (layer, edits) in self.selected_edits(selection, |_, v| {
            (v != CellValue::EMPTY).then_some(CellValue::EMPTY)
        }) {
            patch.push_cells(layer, edits);
        }
        self.commit(patch)
    }

    /// Sets (`Some(true)`), clears (`Some(false)`) or keeps (`None`) the hidden
    /// and unwalkable bits of the selected non-empty floor, wall and shadow cells.
    pub fn set_cell_flags(
        &mut self,
        selection: &Selection,
        hidden: Option<bool>,
        unwalkable: Option<bool>,
    ) -> Result<()> {
        let mut patch = Patch::new("Set cell flags");
        for (layer, edits) in self.selected_edits(selection, |layer, v| {
            if layer == LayerRef::Tag || v.is_empty() {
                return None;
            }
            let mut cell = v.cell;
            for (flag, bit) in [(hidden, HIDDEN_BIT), (unwalkable, UNWALKABLE_BIT)] {
                match flag {
                    Some(true) => cell |= bit,
                    Some(false) => cell &= !bit,
                    None => {}
                }
            }
            (cell != v.cell).then_some(CellValue { cell, ..v })
        }) {
            patch.push_cells(layer, edits);
        }
        self.commit(patch)
    }

    /// Per layer, the edits `change` asks for on the selected cells that exist.
    /// `change` returns the new value, or `None` to leave the cell alone.
    fn selected_edits(
        &self,
        selection: &Selection,
        change: impl Fn(LayerRef, CellValue) -> Option<CellValue>,
    ) -> Vec<(LayerRef, Vec<CellEdit>)> {
        let mut out: Vec<(LayerRef, Vec<CellEdit>)> = Vec::new();
        for (layer, x, y) in selection.iter() {
            let Some(before) = layer.get(&self.ds1, x, y) else {
                continue;
            };
            let Some(after) = change(layer, before) else {
                continue;
            };
            let edit = CellEdit {
                x,
                y,
                before,
                after,
            };
            match out.last_mut() {
                Some((l, edits)) if *l == layer => edits.push(edit),
                _ => out.push((layer, vec![edit])),
            }
        }
        out
    }

    /// Move the selected cells of `from` to the same `(x, y)` in `to`, adding
    /// `to` (and any lower-numbered layers of its kind) when the map lacks it.
    /// Sources become empty. Moves between a wall and any other layer zero the
    /// orientation; wall to wall keeps it. Empty selected cells are not moved.
    /// `NeedsVersion` / `LayerLimit` when `to` cannot be added; the tag layer
    /// cannot take part in a move.
    pub fn move_to_layer(
        &mut self,
        selection: &Selection,
        from: LayerRef,
        to: LayerRef,
        mode: MoveMode,
    ) -> Result<MoveReport> {
        if !from.exists_in(&self.ds1) {
            return Err(EditError::NoSuchLayer(from));
        }
        if from == to {
            return Ok(MoveReport::default());
        }
        if from == LayerRef::Tag || to == LayerRef::Tag {
            return Err(EditError::LayerLimit(
                "the tag layer cannot be a move source or target".to_string(),
            ));
        }
        let add = layer_ops::ensure_layers(&self.ds1, &[to])?;
        let to_exists = add.is_empty();

        let mut report = MoveReport::default();
        let mut source_edits = Vec::new();
        let mut target_edits = Vec::new();
        for (layer, x, y) in selection.iter() {
            if layer != from {
                continue;
            }
            let Some(source) = from.get(&self.ds1, x, y) else {
                continue;
            };
            if source.is_empty() {
                continue;
            }
            let existing = if to_exists {
                to.get(&self.ds1, x, y).unwrap_or(CellValue::EMPTY)
            } else {
                CellValue::EMPTY
            };
            if mode == MoveMode::SkipOccupied && !existing.is_empty() {
                report.skipped_occupied += 1;
                continue;
            }
            let incoming = to.normalize(CellValue {
                cell: source.cell,
                orientation: if matches!(from, LayerRef::Wall(_)) {
                    source.orientation
                } else {
                    0
                },
            });
            source_edits.push(CellEdit {
                x,
                y,
                before: source,
                after: CellValue::EMPTY,
            });
            if incoming != existing {
                target_edits.push(CellEdit {
                    x,
                    y,
                    before: existing,
                    after: incoming,
                });
            }
            report.moved += 1;
        }
        if report.moved == 0 {
            return Ok(report);
        }

        let mut patch = Patch::new("Move to layer");
        for step in add {
            patch.push(step);
        }
        patch.push_cells(from, source_edits);
        patch.push_cells(to, target_edits);
        self.commit(patch)?;
        if !to_exists {
            report.layer_added = Some(to);
        }
        Ok(report)
    }

    /// Adds `layer` and every lower-numbered layer of its kind that is missing.
    /// `NeedsVersion` when the file version cannot store it, `LayerLimit` past
    /// 4 walls / 2 floors; a no-op when it already exists. Adding the tag layer
    /// to a map without one sets `tag_type` to 1.
    pub fn add_layer(&mut self, layer: LayerRef) -> Result<()> {
        layer_ops::check_limit(layer)?;
        let steps = layer_ops::ensure_layers(&self.ds1, &[layer])?;
        let mut patch = Patch::new("Add layer");
        for step in steps {
            patch.push(step);
        }
        self.commit(patch)
    }

    /// Removes `layer`; higher-numbered layers of its kind shift down and undo
    /// restores everything. The shadow layer, the only wall / floor of a file
    /// below version 4 / 16, and the tag layer of a file below version 4 cannot
    /// be removed. Removing the tag layer sets `tag_type` to 0 and drops the
    /// groups (kept for undo).
    pub fn remove_layer(&mut self, layer: LayerRef) -> Result<()> {
        if !layer.exists_in(&self.ds1) {
            return Err(EditError::NoSuchLayer(layer));
        }
        let version = self.ds1.version;
        let cannot = |why: &str| Err(EditError::LayerLimit(why.to_string()));
        match layer {
            LayerRef::Shadow => return cannot("the shadow layer cannot be removed"),
            LayerRef::Wall(_) if version < 4 => {
                return cannot("files below version 4 need exactly one wall layer");
            }
            LayerRef::Floor(_) if version < 16 => {
                return cannot("files below version 16 need exactly one floor layer");
            }
            LayerRef::Tag if version < 4 => {
                return cannot("files below version 4 always store a tag layer");
            }
            _ => {}
        }
        let Some(data) = LayerData::of(&self.ds1, layer) else {
            return Err(EditError::NoSuchLayer(layer));
        };
        let mut patch = Patch::new("Remove layer");
        patch.push(Step::RemoveLayer { layer, data });
        if layer == LayerRef::Tag {
            let before = Header::of(&self.ds1);
            let mut after = before.clone();
            after.tag_type = 0;
            after.groups.clear();
            after.pre_group_dword = 0;
            if before != after {
                patch.push(Step::Header { before, after });
            }
        }
        self.commit(patch)
    }

    /// Changes the map size, keeping the top-left. Objects, groups and paths are
    /// left as they are. Sizes from 1x1 up to 1,048,576 cells are accepted.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(EditError::LayerLimit(
                "a map is at least 1x1 cells".to_string(),
            ));
        }
        if u64::from(width) * u64::from(height) > MAX_CELLS {
            return Err(EditError::LayerLimit(format!(
                "a map has at most {MAX_CELLS} cells"
            )));
        }
        let (old_w, old_h) = (self.ds1.width, self.ds1.height);
        if (width, height) == (old_w, old_h) {
            return Ok(());
        }
        let mut cut = Vec::new();
        if width < old_w || height < old_h {
            for layer in LayerRef::all_in(&self.ds1) {
                for y in 0..old_h {
                    for x in 0..old_w {
                        if x < width && y < height {
                            continue;
                        }
                        if let Some(v) = layer.get(&self.ds1, x, y)
                            && v != CellValue::EMPTY
                        {
                            cut.push((layer, x, y, v));
                        }
                    }
                }
            }
        }
        let mut patch = Patch::new("Resize map");
        patch.push(Step::Resize {
            old: (old_w, old_h),
            new: (width, height),
            cut,
        });
        self.commit(patch)
    }

    /// Raises the file version (`version >= current`). A tag layer that the new
    /// version has no room for is dropped when it is all zero; a non-empty one
    /// needs version 10 or later (`NeedsVersion`), where `tag_type` becomes 1
    /// to keep it. Crossing version 7 remaps wall orientations through
    /// `DIR_LOOKUP` so the map keeps its meaning. Version 9..=13 leftovers
    /// the new version cannot store are zeroed.
    pub fn upgrade_version(&mut self, version: i32) -> Result<()> {
        let current = self.ds1.version;
        if version < current {
            return Err(invalid(format!(
                "cannot lower the version from {current} to {version}"
            )));
        }
        if version == current {
            return Ok(());
        }
        let mut patch = Patch::new("Upgrade version");
        let mut header = Header::of(&self.ds1);
        header.version = version;
        if !(9..=13).contains(&version) {
            header.unknown_v9_13 = [0, 0];
        }

        if current < 7 && version >= 7 {
            for (i, wall) in self.ds1.walls.iter().enumerate() {
                let Ok(index) = u8::try_from(i) else { continue };
                let layer = LayerRef::Wall(index);
                let mut edits = Vec::new();
                for y in 0..self.ds1.height {
                    for x in 0..self.ds1.width {
                        let (Some(tile), Some(raw)) =
                            (wall.tiles.get(x, y), wall.orientations.get(x, y))
                        else {
                            continue;
                        };
                        let Some(&mapped) = DIR_LOOKUP.get((raw & 0xFF) as usize) else {
                            continue;
                        };
                        let new = (raw & !0xFF) | u32::from(mapped);
                        if new != raw {
                            edits.push(CellEdit {
                                x,
                                y,
                                before: CellValue {
                                    cell: tile.0,
                                    orientation: raw,
                                },
                                after: CellValue {
                                    cell: tile.0,
                                    orientation: new,
                                },
                            });
                        }
                    }
                }
                patch.push_cells(layer, edits);
            }
        }

        if let Some(tag) = &self.ds1.tag {
            let all_zero = tag.as_slice().iter().all(|&c| c == 0);
            match version::tag_fate(version, header.tag_type, all_zero) {
                TagFate::Keep => {}
                TagFate::Drop => patch.push(Step::RemoveLayer {
                    layer: LayerRef::Tag,
                    data: LayerData::Tag(tag.clone()),
                }),
                TagFate::Retype => header.tag_type = 1,
                TagFate::Blocked => {
                    return Err(version::needs(
                        &self.ds1,
                        &version::Change::default(),
                        version,
                    ));
                }
            }
        }
        patch.push(Step::Header {
            before: Header::of(&self.ds1),
            after: header,
        });

        // The new layout must really be writable; check on the result and back out if not.
        patch.apply(&mut self.ds1)?;
        if let Err(e) = self.ds1.to_bytes() {
            let _ = patch.revert(&mut self.ds1);
            return Err(e.into());
        }
        self.record(patch);
        Ok(())
    }
}
