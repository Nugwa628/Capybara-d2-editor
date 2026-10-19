//! DT1 tile lookup: the key index, the reference's variant rule, fallbacks and
//! the per-tile bitmap geometry.
//!
//! Reference citations are relative to `reference/d2-ds1-edit/src/` and refer
//! to DS1Edit 2.1.1 (`docs/specs/dt1.md`, `docs/specs/ds1.md`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use d2_assets::{GamePath, Overlay};
use d2_formats::IndexedImage;
use d2_formats::dt1::{Dt1, Tile, TileKey};

use crate::error::Result;

/// Height of a tile cell at 1:1 (`ds1.c:1217`).
pub(crate) const TILE_H: i32 = 80;
/// Width of a tile cell at 1:1 (`ds1.c:1216`).
pub(crate) const TILE_W: i32 = 160;

/// Index of a tile: the DT1 (position in the DT1 list) and the tile inside it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TileRef {
    pub dt1: usize,
    pub tile: usize,
}

/// How a group of tiles sharing one key picks its representative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VariantRule {
    /// The documented game rule (`docs/specs/dt1.md` section 8, PLAN.md): the
    /// highest rarity wins and ties go to the first tile in load order; when
    /// every rarity is 0, the last matching tile of the first DT1 that has
    /// the key.
    #[default]
    Documented,
    /// What DS1Edit actually computes (`misc.c:156-212`): the group head's own
    /// rarity is never compared against the others, so on ties or when the
    /// head is the highest the choice differs. Kept to measure the effect on
    /// oracle parity.
    ReferenceQuirk,
}

/// What a tile is, from its orientation (`misc.c:287-337`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileClass {
    /// Orientation 0.
    Floor,
    /// Orientation 15.
    Roof,
    /// Orientations 1..=14 except 10, 11 and 13.
    UpperWall,
    /// Orientations above 15.
    LowerWall,
    /// Orientation 13.
    Shadow,
    /// Orientations 10 and 11.
    Special,
    /// Anything else (negative orientations); never drawn.
    Other,
}

impl TileClass {
    pub fn of_orientation(orientation: i32) -> TileClass {
        match orientation {
            0 => TileClass::Floor,
            13 => TileClass::Shadow,
            10 | 11 => TileClass::Special,
            15 => TileClass::Roof,
            1..=14 => TileClass::UpperWall,
            16.. => TileClass::LowerWall,
            _ => TileClass::Other,
        }
    }
}

/// Where a tile's bitmap sits relative to its cell, and how big it is
/// (`dt1.c:329-420`, `misc.c:287-337`, `preview.c` draw functions).
///
/// The reference makes one `width x height` bitmap per tile, draws every
/// sub-block at `(x, y_add + y)` and clips to the bitmap. The bitmap's top row
/// is then placed at the cell's top corner `(mx, my)`:
///
/// * floors: `my`;
/// * roofs: `my - roof_y`;
/// * walls, shadows, special tiles: `my + 80 - zero_line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileGeometry {
    pub class: TileClass,
    /// Bitmap size in pixels; either being 0 means the tile draws nothing.
    pub width: u32,
    pub height: u32,
    /// Added to every sub-block's y when drawing into the bitmap.
    pub y_add: i32,
    /// `BLOCK_TABLE_S::zero_line`.
    pub zero_line: i32,
    /// `BLOCK_TABLE_S::roof_y`; 0 except for roofs.
    pub roof_y: i32,
}

impl TileGeometry {
    pub fn of(tile: &Tile) -> TileGeometry {
        let class = TileClass::of_orientation(tile.orientation);
        let orientation = tile.orientation;
        let mut width = tile.width;
        if class == TileClass::Special {
            width = 160; // dt1.c:380
        }
        let mut size_y = tile.height;
        let mut height = -size_y;
        let mut y_add = 96; // dt1.c:387: lower wall by default
        if orientation == 0 || orientation == 15 {
            if size_y != 0 {
                size_y = -80;
                height = 80;
                y_add = 0;
            }
        } else if orientation < 15 && size_y != 0 {
            size_y += 32;
            height -= 32;
            y_add = height;
        }
        let zero_line = match class {
            TileClass::Floor | TileClass::Roof | TileClass::Other => 0,
            TileClass::Shadow | TileClass::Special | TileClass::UpperWall => -size_y,
            TileClass::LowerWall => 96,
        };
        let roof_y = if class == TileClass::Roof {
            i32::from(tile.roof_height)
        } else {
            0
        };
        let (width, height) = match (u32::try_from(width), u32::try_from(height)) {
            (Ok(w), Ok(h)) if w > 0 && h > 0 => (w, h),
            _ => (0, 0),
        };
        TileGeometry {
            class,
            width,
            height,
            y_add,
            zero_line,
            roof_y,
        }
    }

    /// Walls, shadows and special tiles hang off the cell's bottom corner
    /// (`preview.c:1115`: `y1 += tile_h`).
    pub fn wall_like(&self) -> bool {
        matches!(
            self.class,
            TileClass::UpperWall | TileClass::LowerWall | TileClass::Shadow | TileClass::Special
        )
    }

    /// True when the tile draws nothing (the reference has no bitmap).
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Row of the bitmap's top edge relative to the cell's top corner, for a
    /// zoom divisor `div` (integer maths exactly as the reference:
    /// `zero_line * mul / div` truncates toward zero).
    pub fn bitmap_top(&self, div: i32) -> i32 {
        let div = div.max(1);
        let tile_h = TILE_H / div;
        let mut top = -(self.zero_line / div);
        if self.wall_like() {
            top += tile_h;
        }
        if self.class == TileClass::Roof {
            top -= self.roof_y / div; // preview.c:1367
        }
        top
    }
}

/// Loaded DT1s plus the key index.
pub struct TileSet {
    paths: Vec<GamePath>,
    dt1s: Vec<Dt1>,
    index: HashMap<TileKey, Vec<TileRef>>,
    rule: VariantRule,
    images: Mutex<HashMap<TileRef, Arc<IndexedImage>>>,
}

impl std::fmt::Debug for TileSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileSet")
            .field("paths", &self.paths)
            .field("rule", &self.rule)
            .field("keys", &self.index.len())
            .finish()
    }
}

impl TileSet {
    /// Loads and parses each DT1 through the overlay; unreadable ones become
    /// warnings (and an empty DT1 so indices stay aligned with `dt1_files`).
    pub fn load(overlay: &Overlay, dt1_files: &[GamePath]) -> Result<(TileSet, Vec<String>)> {
        let mut warnings = Vec::new();
        let mut dt1s = Vec::with_capacity(dt1_files.len());
        for path in dt1_files {
            let parsed = match overlay.read_bytes(path) {
                Ok(Some(bytes)) => match Dt1::parse(&bytes) {
                    Ok(dt1) => Some(dt1),
                    Err(e) => {
                        warnings.push(format!("{path}: {e}"));
                        None
                    }
                },
                Ok(None) => {
                    warnings.push(format!("{path}: not found"));
                    None
                }
                Err(e) => {
                    warnings.push(format!("{path}: {e}"));
                    None
                }
            };
            dt1s.push(parsed.unwrap_or(Dt1 {
                version: (7, 6),
                tiles: Vec::new(),
            }));
        }
        Ok((TileSet::from_dt1s(dt1_files.to_vec(), dt1s), warnings))
    }

    /// Builds a set from already parsed DT1s (`paths` and `dt1s` are parallel;
    /// extra entries on either side are ignored).
    pub fn from_dt1s(mut paths: Vec<GamePath>, dt1s: Vec<Dt1>) -> TileSet {
        paths.truncate(dt1s.len());
        while paths.len() < dt1s.len() {
            paths.push(GamePath::new(&format!("dt1#{}", paths.len())));
        }
        let mut index: HashMap<TileKey, Vec<TileRef>> = HashMap::new();
        for (d, dt1) in dt1s.iter().enumerate() {
            for (t, tile) in dt1.tiles.iter().enumerate() {
                if let Some(key) = tile.key() {
                    index
                        .entry(key)
                        .or_default()
                        .push(TileRef { dt1: d, tile: t });
                }
            }
        }
        TileSet {
            paths,
            dt1s,
            index,
            rule: VariantRule::default(),
            images: Mutex::new(HashMap::new()),
        }
    }

    /// Selects the variant rule (default [`VariantRule::Documented`]).
    pub fn with_variant_rule(mut self, rule: VariantRule) -> TileSet {
        self.rule = rule;
        self
    }

    pub fn variant_rule(&self) -> VariantRule {
        self.rule
    }

    pub fn dt1_paths(&self) -> &[GamePath] {
        &self.paths
    }

    pub fn tile(&self, r: &TileRef) -> Option<&Tile> {
        self.dt1s.get(r.dt1)?.tiles.get(r.tile)
    }

    /// Every tile with exactly this key, in load order (DT1 order, then file
    /// order).
    pub fn candidates(&self, key: TileKey) -> &[TileRef] {
        self.index.get(&key).map_or(&[], Vec::as_slice)
    }

    fn rarity(&self, r: &TileRef) -> i32 {
        self.tile(r).map_or(0, |t| t.rarity)
    }

    /// The representative of a group of candidates under the active rule.
    fn pick(&self, cands: &[TileRef]) -> Option<TileRef> {
        let first = cands.first()?;
        if cands.iter().all(|r| self.rarity(r) == 0) {
            // misc.c:196-199: the last block of the first DT1 of the group.
            return cands.iter().rfind(|r| r.dt1 == first.dt1).cloned();
        }
        match self.rule {
            VariantRule::Documented => {
                let mut best = first;
                for r in cands {
                    if self.rarity(r) > self.rarity(best) {
                        best = r;
                    }
                }
                Some(best.clone())
            }
            VariantRule::ReferenceQuirk => {
                // misc.c:156-187 on the group sorted by (dt1 slot, rarity,
                // block): the head only seeds `first_block`; the maximum is
                // searched among the later entries.
                let sorted = self.sorted_group(cands);
                let head = sorted.first()?;
                let mut first_block = (self.rarity(head) != 0).then(|| head.clone());
                let mut max_rarity = -1;
                for r in sorted.iter().skip(1) {
                    let rarity = self.rarity(r);
                    if rarity != 0 {
                        if first_block.is_none() {
                            first_block = Some(r.clone());
                        }
                        if rarity > max_rarity {
                            first_block = Some(r.clone());
                            max_rarity = rarity;
                        }
                    }
                }
                first_block
            }
        }
    }

    /// `misc_qsort_helper_block_table_1` order inside one key group:
    /// DT1 slot, then rarity ascending, then block index.
    fn sorted_group(&self, cands: &[TileRef]) -> Vec<TileRef> {
        let mut sorted = cands.to_vec();
        sorted.sort_by_key(|r| (r.dt1, self.rarity(r), r.tile));
        sorted
    }

    /// Resolves a key to one tile: the exact key, then (orientation 18 and 19
    /// only) the other of the pair with the same sub index, then that other
    /// orientation with sub 0 (`ds1.c:153-193`, "trick of O=18 M=3 I=1 --->
    /// O=19 M=3 I=0"). The variant rule picks among tiles sharing the key.
    pub fn resolve(&self, key: TileKey) -> Option<TileRef> {
        if let Some(r) = self.pick(self.candidates(key)) {
            return Some(r);
        }
        let other = match key.orientation {
            18 => 19,
            19 => 18,
            _ => return None,
        };
        let swapped = TileKey {
            orientation: other,
            ..key
        };
        if let Some(r) = self.pick(self.candidates(swapped)) {
            return Some(r);
        }
        self.pick(self.candidates(TileKey { sub: 0, ..swapped }))
    }

    /// The frame of an animated floor the headless oracle shows. DS1Edit
    /// stores animation frames as blocks of one key whose rarity counts up;
    /// it draws `bt_ptr += curr_frame - rarity` from the representative
    /// (`preview.c:935-944`) and `curr_frame` is 0 until the animation timer
    /// runs. Tiles that are not animated floors come back unchanged.
    pub fn animation_frame0(&self, r: &TileRef) -> TileRef {
        let Some(tile) = self.tile(r) else {
            return r.clone();
        };
        if tile.orientation != 0 || tile.animated != 1 || tile.rarity <= 0 {
            return r.clone();
        }
        let Some(key) = tile.key() else {
            return r.clone();
        };
        let sorted = self.sorted_group(self.candidates(key));
        let Some(pos) = sorted.iter().position(|x| x == r) else {
            return r.clone();
        };
        let back = usize::try_from(tile.rarity).unwrap_or(0);
        sorted
            .get(pos.saturating_sub(back))
            .cloned()
            .unwrap_or_else(|| r.clone())
    }

    /// The orientation-4 tile DS1Edit draws together with an orientation-3
    /// wall (`preview.c:1131-1160`, `misc.c:1056-1074`): the first block with
    /// orientation 4 and the same main and sub index in (DT1 slot, rarity,
    /// block) order, regardless of the variant rule.
    pub fn corner_pair(&self, r: &TileRef) -> Option<TileRef> {
        let tile = self.tile(r)?;
        if tile.orientation != 3 {
            return None;
        }
        let key = tile.key()?;
        let partner = TileKey {
            orientation: 4,
            ..key
        };
        self.sorted_group(self.candidates(partner))
            .into_iter()
            .next()
    }

    /// Bitmap geometry of a tile.
    pub fn geometry(&self, r: &TileRef) -> Option<TileGeometry> {
        self.tile(r).map(TileGeometry::of)
    }

    /// The tile as the reference's bitmap sees it: sub-blocks drawn at
    /// `(x, y_add + y)` and clipped to `geometry.width x geometry.height`,
    /// then cropped to the pixels written. `offset_x`/`offset_y` are the crop's
    /// position inside that bitmap (not an anchor offset: add
    /// [`TileGeometry::bitmap_top`] and the cell position for that). Cached.
    pub fn image(&self, r: &TileRef) -> Result<Arc<IndexedImage>> {
        if let Some(hit) = self
            .images
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(r)
        {
            return Ok(Arc::clone(hit));
        }
        let image = match self.tile(r) {
            Some(tile) => Arc::new(build_image(tile)?),
            None => Arc::new(IndexedImage::new(0, 0, 0, 0)),
        };
        self.images
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(r.clone(), Arc::clone(&image));
        Ok(image)
    }
}

/// Renders `tile` into bitmap space and clips it to the bitmap.
fn build_image(tile: &Tile) -> Result<IndexedImage> {
    let geom = TileGeometry::of(tile);
    if geom.is_empty() {
        return Ok(IndexedImage::new(0, 0, 0, 0));
    }
    let raw = tile.render()?;
    let bw = i64::from(geom.width);
    let bh = i64::from(geom.height);
    // Position of the raw image inside the bitmap.
    let rx0 = i64::from(raw.offset_x);
    let ry0 = i64::from(raw.offset_y) + i64::from(geom.y_add);
    let (rw, rh) = (i64::from(raw.width), i64::from(raw.height));
    let x0 = rx0.max(0);
    let y0 = ry0.max(0);
    let x1 = (rx0 + rw).min(bw);
    let y1 = (ry0 + rh).min(bh);
    if x1 <= x0 || y1 <= y0 {
        return Ok(IndexedImage::new(0, 0, 0, 0));
    }
    let (Ok(ox), Ok(oy), Ok(w), Ok(h)) = (
        i32::try_from(x0),
        i32::try_from(y0),
        u32::try_from(x1 - x0),
        u32::try_from(y1 - y0),
    ) else {
        return Ok(IndexedImage::new(0, 0, 0, 0));
    };
    let mut out = IndexedImage::new(w, h, ox, oy);
    for row in 0..(y1 - y0) {
        let src_row = y0 + row - ry0;
        let src = usize::try_from(src_row * rw + (x0 - rx0)).unwrap_or(0);
        let dst = usize::try_from(row * (x1 - x0)).unwrap_or(0);
        let n = usize::try_from(x1 - x0).unwrap_or(0);
        if let (Some(sp), Some(sa), Some(dp), Some(da)) = (
            raw.pixels.get(src..src + n),
            raw.alpha.get(src..src + n),
            out.pixels.get_mut(dst..dst + n),
            out.alpha.get_mut(dst..dst + n),
        ) {
            dp.copy_from_slice(sp);
            da.copy_from_slice(sa);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
