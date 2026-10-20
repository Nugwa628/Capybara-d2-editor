//! P9 map-editing operations: rectangle / flood paint, random mixes, find &
//! replace, floor re-roll, tile reassignment, edge resize and tag groups.
//! Each is one undoable patch.

use std::collections::{BTreeMap, HashMap};

use d2_formats::Grid;
use d2_formats::ds1::{Ds1, Group, NpcPath, PathPoint};

use super::Document;
use crate::error::{EditError, Result, invalid};
use crate::layer::{CellValue, LayerRef};
use crate::patch::{CellEdit, Header, Patch, Step};
use crate::selection::{CellRect, Selection};
use crate::tools::{
    Brush, FloorChoice, MAX_CELLS, RerollOptions, RerollReport, Rng, SUBTILES, TileKey, painted,
    pick_floor, tile_key,
};

/// `(before, after)` of one cell.
type Change = (CellValue, CellValue);

fn pick(mix: &[Brush], rng: &mut Rng) -> Option<Brush> {
    match mix {
        [] => None,
        [one] => Some(*one),
        _ => mix.get(rng.below(mix.len())).copied(),
    }
}

fn inside(within: Option<&Selection>, layer: LayerRef, x: u32, y: u32) -> bool {
    within.is_none_or(|s| s.contains(layer, x, y))
}

impl Document {
    fn tile_layer(&self, layer: LayerRef) -> Result<()> {
        if !layer.exists_in(&self.ds1) {
            return Err(EditError::NoSuchLayer(layer));
        }
        if layer == LayerRef::Tag {
            return Err(invalid("the tag layer holds no tiles"));
        }
        Ok(())
    }

    /// Commits `after` values (later entries win) as one patch; unchanged cells are dropped.
    fn commit_cells(
        &mut self,
        label: &str,
        changes: impl IntoIterator<Item = (LayerRef, u32, u32, CellValue)>,
    ) -> Result<()> {
        let mut merged: BTreeMap<LayerRef, BTreeMap<(u32, u32), Change>> = BTreeMap::new();
        for (layer, x, y, after) in changes {
            let Some(before) = layer.get(&self.ds1, x, y) else {
                continue;
            };
            merged
                .entry(layer)
                .or_default()
                .entry((y, x))
                .and_modify(|e| e.1 = after)
                .or_insert((before, after));
        }
        let mut patch = Patch::new(label);
        for (layer, cells) in merged {
            let edits = cells
                .into_iter()
                .filter(|(_, (b, a))| b != a)
                .map(|((y, x), (before, after))| CellEdit {
                    x,
                    y,
                    before,
                    after,
                })
                .collect();
            patch.push_cells(layer, edits);
        }
        self.commit(patch)
    }

    fn paint_cells_with(
        &mut self,
        label: &str,
        layer: LayerRef,
        cells: &[(u32, u32)],
        mix: &[Brush],
        rng: &mut Rng,
    ) -> Result<()> {
        self.tile_layer(layer)?;
        let changes: Vec<_> = cells
            .iter()
            .filter_map(|&(x, y)| {
                let cur = layer.get(&self.ds1, x, y)?;
                Some((layer, x, y, painted(layer, cur, pick(mix, rng))))
            })
            .collect();
        self.commit_cells(label, changes)
    }

    // ----- paint / erase / fill ------------------------------------------------

    /// Paints the rectangle (clipped to the map) on a floor, wall or shadow
    /// layer. Each cell takes a tile at random from `mix` (one entry = plain
    /// paint, empty = erase); `rng` makes the mix reproducible. Painting
    /// over a tile keeps its other cell bits.
    pub fn paint_rect(
        &mut self,
        layer: LayerRef,
        rect: CellRect,
        mix: &[Brush],
        rng: &mut Rng,
    ) -> Result<()> {
        let cells = rect_cells(rect, self.ds1.width, self.ds1.height);
        let label = if mix.is_empty() { "Erase" } else { "Paint" };
        self.paint_cells_with(label, layer, &cells, mix, rng)
    }

    /// Paints single cells (a brush stroke); cells outside the map are ignored.
    pub fn paint_mix(
        &mut self,
        layer: LayerRef,
        cells: &[(u32, u32)],
        mix: &[Brush],
        rng: &mut Rng,
    ) -> Result<()> {
        let label = if mix.is_empty() { "Erase" } else { "Paint" };
        self.paint_cells_with(label, layer, cells, mix, rng)
    }

    /// Empties the rectangle (clipped to the map) on `layer`.
    pub fn erase_rect(&mut self, layer: LayerRef, rect: CellRect) -> Result<()> {
        self.paint_rect(layer, rect, &[], &mut Rng::new(0))
    }

    /// The cells connected to `(x, y)` on `layer`, as ds1-studio defines it:
    /// edge neighbours (4-connected) holding the same tile (orientation, main,
    /// sub; other cell bits ignored), or all empty cells when `(x, y)` is
    /// empty, kept inside `within` when given. Empty when `(x, y)` is outside
    /// the map or `within`.
    pub fn flood_region(
        &self,
        layer: LayerRef,
        x: u32,
        y: u32,
        within: Option<&Selection>,
    ) -> Vec<(u32, u32)> {
        let (w, h) = (self.ds1.width, self.ds1.height);
        let key_at = |x: u32, y: u32| layer.get(&self.ds1, x, y).map(|v| tile_key(layer, v));
        let Some(target) = key_at(x, y) else {
            return Vec::new();
        };
        if layer == LayerRef::Tag || !inside(within, layer, x, y) {
            return Vec::new();
        }
        let mut seen = vec![false; w as usize * h as usize];
        let idx = |x: u32, y: u32| y as usize * w as usize + x as usize;
        let mut out = Vec::new();
        let mut stack = vec![(x, y)];
        if let Some(s) = seen.get_mut(idx(x, y)) {
            *s = true;
        }
        while let Some((cx, cy)) = stack.pop() {
            out.push((cx, cy));
            let around = [
                (cx.wrapping_add(1), cy),
                (cx.wrapping_sub(1), cy),
                (cx, cy.wrapping_add(1)),
                (cx, cy.wrapping_sub(1)),
            ];
            for (nx, ny) in around {
                if nx >= w || ny >= h || !inside(within, layer, nx, ny) {
                    continue;
                }
                let Some(s) = seen.get_mut(idx(nx, ny)) else {
                    continue;
                };
                if *s {
                    continue;
                }
                *s = true;
                if key_at(nx, ny) == Some(target) {
                    stack.push((nx, ny));
                }
            }
        }
        out.sort_by_key(|&(x, y)| (y, x));
        out
    }

    /// Flood fill: paints [`Document::flood_region`] with the (random) mix; an
    /// empty mix erases it (flood erase).
    pub fn flood_fill(
        &mut self,
        layer: LayerRef,
        x: u32,
        y: u32,
        mix: &[Brush],
        within: Option<&Selection>,
        rng: &mut Rng,
    ) -> Result<()> {
        self.tile_layer(layer)?;
        let cells = self.flood_region(layer, x, y, within);
        let label = if mix.is_empty() {
            "Flood erase"
        } else {
            "Flood fill"
        };
        self.paint_cells_with(label, layer, &cells, mix, rng)
    }

    // ----- find & replace --------------------------------------------------------

    /// Every use of `from` on `layers` (the tag layer and missing layers are
    /// skipped), restricted to `within` when given.
    pub fn find_tiles(
        &self,
        layers: &[LayerRef],
        from: TileKey,
        within: Option<&Selection>,
    ) -> Vec<(LayerRef, u32, u32)> {
        let mut out = Vec::new();
        for &layer in layers {
            if !layer.exists_in(&self.ds1) || layer == LayerRef::Tag {
                continue;
            }
            for y in 0..self.ds1.height {
                for x in 0..self.ds1.width {
                    if inside(within, layer, x, y)
                        && layer.get(&self.ds1, x, y).and_then(|v| tile_key(layer, v)) == Some(from)
                    {
                        out.push((layer, x, y));
                    }
                }
            }
        }
        out
    }

    /// Replaces every use of `from` with `to` on `layers` (whole map, or
    /// `within`), keeping each cell's other bits. Returns the cells changed.
    pub fn replace_tiles(
        &mut self,
        layers: &[LayerRef],
        from: TileKey,
        to: TileKey,
        within: Option<&Selection>,
    ) -> Result<usize> {
        let found = self.find_tiles(layers, from, within);
        let changes: Vec<_> = found
            .iter()
            .filter_map(|&(layer, x, y)| {
                let cur = layer.get(&self.ds1, x, y)?;
                Some((layer, x, y, painted(layer, cur, Some(to.brush()))))
            })
            .collect();
        let n = changes
            .iter()
            .filter(|&&(l, x, y, after)| l.get(&self.ds1, x, y) != Some(after))
            .count();
        self.commit_cells("Replace tiles", changes)?;
        Ok(n)
    }

    // ----- floor re-roll -----------------------------------------------------------

    /// Re-picks the tile of every non-empty cell of floor layer `layer` (in
    /// `area`, or the whole layer) among `choices`, deterministic for the
    /// `opts.seed`: it balances use counts (scaled by rarity), avoids repeating
    /// the left and upper neighbour and never picks rarity-0 tiles next to
    /// ones with a rarity. `walk_of(main, sub)` gives the walkability signature
    /// of the tile being replaced (only used with `preserve_walkability`).
    pub fn reroll_floors(
        &mut self,
        layer: u8,
        area: Option<&Selection>,
        choices: &[FloorChoice],
        opts: &RerollOptions,
        walk_of: &dyn Fn(u8, u8) -> Option<u32>,
    ) -> Result<RerollReport> {
        let lref = LayerRef::Floor(layer);
        self.tile_layer(lref)?;
        let mut report = RerollReport::default();
        let mut unique: Vec<&FloorChoice> = Vec::new();
        for c in choices {
            if !unique.iter().any(|u| (u.main, u.sub) == (c.main, c.sub)) {
                unique.push(c);
            }
        }
        if unique.is_empty() {
            return Ok(report);
        }
        let (w, h) = (self.ds1.width, self.ds1.height);
        let cells = u64::from(w) * u64::from(h);
        let mut counts: HashMap<(u8, u8), u32> = HashMap::new();
        let mut assigned: HashMap<(u32, u32), (u8, u8)> = HashMap::new();
        let mut changes = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if !inside(area, lref, x, y) {
                    continue;
                }
                let Some(cur) = lref.get(&self.ds1, x, y) else {
                    continue;
                };
                let Some(key) = tile_key(lref, cur) else {
                    continue;
                };
                let eligible: Vec<&FloorChoice> = if opts.preserve_walkability {
                    match walk_of(key.main, key.sub) {
                        Some(sig) => unique.iter().copied().filter(|c| c.walk == sig).collect(),
                        None => Vec::new(),
                    }
                } else {
                    unique.clone()
                };
                let key_at = |nx: u32, ny: u32| {
                    assigned.get(&(nx, ny)).copied().or_else(|| {
                        let v = lref.get(&self.ds1, nx, ny)?;
                        tile_key(lref, v).map(|k| (k.main, k.sub))
                    })
                };
                let neighbors = [
                    x.checked_sub(1).and_then(|nx| key_at(nx, y)),
                    y.checked_sub(1).and_then(|ny| key_at(x, ny)),
                ];
                let Some(chosen) = pick_floor(&eligible, neighbors, &counts, opts, (x, y), cells)
                else {
                    report.skipped += 1;
                    continue;
                };
                let k = (chosen.main, chosen.sub);
                *counts.entry(k).or_insert(0) += 1;
                assigned.insert((x, y), k);
                let after = painted(lref, cur, Some(Brush::floor(chosen.main, chosen.sub)));
                if after != cur {
                    report.changed += 1;
                }
                changes.push((lref, x, y, after));
            }
        }
        self.commit_cells("Re-roll floors", changes)?;
        Ok(report)
    }

    // ----- tile reassignment -----------------------------------------------------------

    /// Moves the map's cells from old tile numbers to new ones (after the
    /// caller renumbered DT1 tiles): floors, walls (orientation follows the
    /// new key) and shadows. Returns the cells changed.
    pub fn reassign_tiles(&mut self, moves: &BTreeMap<TileKey, TileKey>) -> Result<usize> {
        let mut changes = Vec::new();
        if !moves.is_empty() {
            for layer in LayerRef::all_in(&self.ds1) {
                if layer == LayerRef::Tag {
                    continue;
                }
                for y in 0..self.ds1.height {
                    for x in 0..self.ds1.width {
                        let Some(cur) = layer.get(&self.ds1, x, y) else {
                            continue;
                        };
                        if let Some(to) = tile_key(layer, cur).and_then(|k| moves.get(&k)) {
                            changes.push((layer, x, y, painted(layer, cur, Some(to.brush()))));
                        }
                    }
                }
            }
        }
        let n = changes.len();
        self.commit_cells("Reassign tiles", changes)?;
        Ok(n)
    }

    // ----- resize on any edge --------------------------------------------------------------

    /// Grows (positive) or shrinks (negative) the map on each side; the
    /// existing `resize` only moves the right and bottom edge. Cells keep
    /// their place relative to the content; objects, NPC paths and groups
    /// move with it and anything pushed off the map is dropped (groups are
    /// clipped). One undo step (a whole-map patch).
    pub fn resize_edges(&mut self, left: i32, top: i32, right: i32, bottom: i32) -> Result<()> {
        if (left, top, right, bottom) == (0, 0, 0, 0) {
            return Ok(());
        }
        let width = i64::from(self.ds1.width) + i64::from(left) + i64::from(right);
        let height = i64::from(self.ds1.height) + i64::from(top) + i64::from(bottom);
        if width < 1 || height < 1 || (width * height) as u64 > MAX_CELLS {
            return Err(EditError::LayerLimit(format!(
                "map would be {width}x{height}"
            )));
        }
        let (nw, nh) = (width as u32, height as u32);
        let old = &self.ds1;
        let mut ds = old.clone();
        ds.width = nw;
        ds.height = nh;
        let (dx, dy) = (i64::from(left), i64::from(top));
        fn remap<T: Copy + Default>(g: &Grid<T>, nw: u32, nh: u32, dx: i64, dy: i64) -> Grid<T> {
            let mut out = Grid::new(nw, nh);
            for y in 0..nh {
                for x in 0..nw {
                    let (ox, oy) = (i64::from(x) - dx, i64::from(y) - dy);
                    if ox >= 0
                        && oy >= 0
                        && let Some(v) = g.get(ox as u32, oy as u32)
                    {
                        out.set(x, y, v);
                    }
                }
            }
            out
        }
        for (new, wall) in ds.walls.iter_mut().zip(&old.walls) {
            new.tiles = remap(&wall.tiles, nw, nh, dx, dy);
            new.orientations = remap(&wall.orientations, nw, nh, dx, dy);
        }
        for (new, floor) in ds.floors.iter_mut().zip(&old.floors) {
            *new = remap(floor, nw, nh, dx, dy);
        }
        ds.shadow = remap(&old.shadow, nw, nh, dx, dy);
        ds.tag = old.tag.as_ref().map(|t| remap(t, nw, nh, dx, dy));
        let (sx, sy) = (dx * SUBTILES, dy * SUBTILES);
        let (maxx, maxy) = (width * SUBTILES, height * SUBTILES);
        let keep = |x: i64, y: i64| x >= 0 && y >= 0 && x < maxx && y < maxy;
        let shift = |v: i32, d: i64| i32::try_from(i64::from(v) + d).ok();
        ds.objects = old
            .objects
            .iter()
            .filter_map(|o| {
                let (x, y) = (shift(o.x, sx)?, shift(o.y, sy)?);
                keep(i64::from(x), i64::from(y)).then_some(d2_formats::ds1::Object { x, y, ..*o })
            })
            .collect();
        ds.npc_paths = old
            .npc_paths
            .iter()
            .filter_map(|p| {
                let (x, y) = (shift(p.x, sx)?, shift(p.y, sy)?);
                if !keep(i64::from(x), i64::from(y)) {
                    return None;
                }
                let points = p
                    .points
                    .iter()
                    .filter_map(|q| {
                        let (qx, qy) = (shift(q.x, sx)?, shift(q.y, sy)?);
                        keep(i64::from(qx), i64::from(qy)).then_some(PathPoint {
                            x: qx,
                            y: qy,
                            ..*q
                        })
                    })
                    .collect();
                Some(NpcPath { x, y, points })
            })
            .collect();
        ds.groups = old
            .groups
            .iter()
            .filter_map(|g| {
                let x0 = (i64::from(g.x) + dx).max(0);
                let y0 = (i64::from(g.y) + dy).max(0);
                let x1 = (i64::from(g.x) + dx + i64::from(g.width)).min(width);
                let y1 = (i64::from(g.y) + dy + i64::from(g.height)).min(height);
                (x1 > x0 && y1 > y0).then_some(Group {
                    x: i32::try_from(x0).ok()?,
                    y: i32::try_from(y0).ok()?,
                    width: i32::try_from(x1 - x0).ok()?,
                    height: i32::try_from(y1 - y0).ok()?,
                    ..*g
                })
            })
            .collect();
        let patch = Patch::replace("Resize map", self.ds1.clone(), ds);
        self.commit(patch)
    }

    // ----- tag groups --------------------------------------------------------------------------

    fn commit_groups(&mut self, label: &str, groups: Vec<Group>) -> Result<()> {
        let ds: &Ds1 = &self.ds1;
        if !groups.is_empty() || !ds.groups.is_empty() {
            if ds.version < 12 {
                return Err(EditError::NeedsVersion {
                    current: ds.version,
                    required: 12,
                });
            }
            if !matches!(ds.tag_type, 1 | 2) {
                return Err(invalid("groups need the tag layer (tag_type 1 or 2)"));
            }
        }
        for g in groups.iter().filter(|g| !ds.groups.contains(g)) {
            if g.width < 1 || g.height < 1 || g.x < 0 || g.y < 0 {
                return Err(invalid("a group needs x, y >= 0 and width, height >= 1"));
            }
            if g.unknown != 0 && ds.version < 13 {
                return Err(EditError::NeedsVersion {
                    current: ds.version,
                    required: 13,
                });
            }
        }
        let before = Header::of(ds);
        let mut after = before.clone();
        after.groups = groups;
        let mut patch = Patch::new(label);
        if before != after {
            patch.push(Step::Header { before, after });
        }
        self.commit(patch)
    }

    fn group_index(&self, index: usize) -> Result<()> {
        if index < self.ds1.groups.len() {
            Ok(())
        } else {
            Err(invalid(format!("no group {index}")))
        }
    }

    /// Appends a group (map cells; needs v12+ and a tag layer). Returns its index.
    pub fn add_group(&mut self, group: Group) -> Result<usize> {
        let mut groups = self.ds1.groups.clone();
        groups.push(group);
        let index = groups.len() - 1;
        self.commit_groups("Add group", groups)?;
        Ok(index)
    }

    /// Replaces all fields of a group.
    pub fn set_group(&mut self, index: usize, group: Group) -> Result<()> {
        self.group_index(index)?;
        let mut groups = self.ds1.groups.clone();
        if let Some(g) = groups.get_mut(index) {
            *g = group;
        }
        self.commit_groups("Edit group", groups)
    }

    pub fn move_group(&mut self, index: usize, x: i32, y: i32) -> Result<()> {
        self.group_index(index)?;
        let mut groups = self.ds1.groups.clone();
        if let Some(g) = groups.get_mut(index) {
            g.x = x;
            g.y = y;
        }
        self.commit_groups("Move group", groups)
    }

    pub fn resize_group(&mut self, index: usize, width: i32, height: i32) -> Result<()> {
        self.group_index(index)?;
        let mut groups = self.ds1.groups.clone();
        if let Some(g) = groups.get_mut(index) {
            g.width = width;
            g.height = height;
        }
        self.commit_groups("Resize group", groups)
    }

    pub fn delete_group(&mut self, index: usize) -> Result<()> {
        self.group_index(index)?;
        let mut groups = self.ds1.groups.clone();
        groups.remove(index);
        self.commit_groups("Delete group", groups)
    }
}

fn rect_cells(r: CellRect, w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in r.y0..=r.y1.min(h.saturating_sub(1)) {
        for x in r.x0..=r.x1.min(w.saturating_sub(1)) {
            out.push((x, y));
        }
    }
    out
}
