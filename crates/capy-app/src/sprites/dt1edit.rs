//! A DT1 as an editor document: every tile is one frame (`Tile::render`,
//! all blocks composed, offsets in block space), pixels outside every
//! block's footprint are locked, and saving cuts the edited frames back into
//! their blocks. Only blocks whose pixels changed are re-encoded, so the rest
//! of the file keeps its bytes.
//!
//! A tile can also be edited in a *shape* (`capy_mod::tiles_create`): a new
//! tile from the New tile dialog, or an existing one after "Shape...". Its
//! frame is then the template's box, every pixel of the template is
//! editable (not only the existing blocks), and saving rebuilds the tile
//! with `build_tile`. Which shape a frame has is the paint session's lock tag
//! (`PaintSession::relock_frame`), so undo and redo carry it.

use capy_mod::tiles_create::{BuildOptions, BuiltTile, TileShape, build_tile, template};
use capy_paint::Mask;
use d2_formats::IndexedImage;
use d2_formats::dt1::{Block, BlockFormat, Dt1, Tile};

use super::dt1view::{Dt1View, TileInfo};

/// The block-space rectangle a block can draw in; `bits` marks the pixels
/// that belong to it (the diamond of an isometric block, the whole bounding
/// box of an RLE block).
#[derive(Clone, Debug)]
struct Footprint {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    bits: Vec<bool>,
}

impl Footprint {
    fn empty() -> Footprint {
        Footprint {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            bits: Vec::new(),
        }
    }

    fn of(block: &Block) -> Footprint {
        match block.format {
            BlockFormat::Isometric => {
                // A block that is not exactly 256 bytes draws nothing.
                if block.data.len() != 256 {
                    return Footprint::empty();
                }
                let mut full = block.clone();
                full.data = vec![1; 256];
                Footprint::from_render(&full, true)
            }
            BlockFormat::Rle => Footprint::from_render(block, false),
        }
    }

    fn from_render(block: &Block, by_alpha: bool) -> Footprint {
        match block.render() {
            Ok(img) if img.width > 0 && img.height > 0 => Footprint {
                x: img.offset_x,
                y: img.offset_y,
                w: img.width,
                h: img.height,
                bits: if by_alpha {
                    img.alpha.iter().map(|&a| a != 0).collect()
                } else {
                    vec![true; img.pixels.len()]
                },
            },
            _ => Footprint::empty(),
        }
    }

    fn contains(&self, x: i64, y: i64) -> bool {
        let (c, r) = (x - i64::from(self.x), y - i64::from(self.y));
        if c < 0 || r < 0 || c >= i64::from(self.w) || r >= i64::from(self.h) {
            return false;
        }
        self.bits
            .get(r as usize * self.w as usize + c as usize)
            .copied()
            .unwrap_or(false)
    }
}

/// What a new-tiles document (from the New tile dialog) goes into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewTilesTarget {
    /// The open map's own DT1 ("Add to this map").
    Map,
    /// A library open in the Tile libraries window (its game path).
    Library(String),
}

/// The DT1 a document saves, and what happened on the way.
pub struct Dt1Saved {
    pub bytes: Vec<u8>,
    /// Tiles that differ from the file as opened.
    pub changed: usize,
    /// Pixels a tile's shape could not hold (from `build_tile`).
    pub warnings: Vec<String>,
}

pub struct Dt1Doc {
    /// The file as opened (or the created tiles): what a tile with tag 0 is
    /// cut back into.
    dt1: Dt1,
    /// Each tile's frame when the document opened.
    base: Vec<IndexedImage>,
    /// Each tile of `dt1` rendered (the frame of tag 0).
    rendered: Vec<IndexedImage>,
    pub info: Vec<TileInfo>,
    footprints: Vec<Vec<Footprint>>,
    /// Shapes tiles took in this document: lock tag `n > 0` is
    /// `shapes[n - 1]`. A tile with a shape is edited inside its template
    /// and saved through `build_tile`; tag 0 is the tile's own blocks.
    shapes: Vec<TileShape>,
    /// Each tile's lock tag when the document opened.
    tags: Vec<u64>,
    /// Set for tiles made in the New tile dialog.
    pub new_tiles: Option<NewTilesTarget>,
}

impl Dt1Doc {
    pub fn from_view(view: &Dt1View) -> Dt1Doc {
        Dt1Doc::build(view.dt1.clone(), view.images.clone(), view.tiles.clone())
    }

    fn build(dt1: Dt1, rendered: Vec<IndexedImage>, info: Vec<TileInfo>) -> Dt1Doc {
        let footprints = dt1
            .tiles
            .iter()
            .map(|t| t.blocks.iter().map(Footprint::of).collect())
            .collect();
        Dt1Doc {
            tags: vec![0; rendered.len()],
            base: rendered.clone(),
            rendered,
            dt1,
            info,
            footprints,
            shapes: Vec::new(),
            new_tiles: None,
        }
    }

    /// Created tiles as a document: each one is edited inside the template
    /// of its shape (`shapes[i]` for `tiles[i]`), blank or not.
    pub fn new_tiles(
        tiles: Vec<Tile>,
        shapes: &[TileShape],
        target: NewTilesTarget,
    ) -> Result<Dt1Doc, String> {
        if tiles.is_empty() || tiles.len() != shapes.len() {
            return Err("no tile to edit".to_owned());
        }
        let dt1 = Dt1 {
            version: (7, 6),
            tiles,
        };
        let bytes = dt1.to_bytes().map_err(|e| e.to_string())?;
        let view = Dt1View::load(&bytes)?;
        let mut doc = Dt1Doc::build(view.dt1, view.images, view.tiles);
        for (i, shape) in shapes.iter().enumerate() {
            let picture = doc.rendered[i].clone();
            let (frame, _, _) = shaped_frame(&picture, *shape)?;
            doc.base[i] = frame;
            doc.tags[i] = doc.register_shape(*shape);
        }
        doc.new_tiles = Some(target);
        Ok(doc)
    }

    /// The lock tag of `shape` (registered on first use).
    pub fn register_shape(&mut self, shape: TileShape) -> u64 {
        let shape = shape.normalized().unwrap_or(shape);
        let i = match self.shapes.iter().position(|s| *s == shape) {
            Some(i) => i,
            None => {
                self.shapes.push(shape);
                self.shapes.len() - 1
            }
        };
        i as u64 + 1
    }

    /// The shape a tile is edited in: its registered shape for a non-zero
    /// tag, else the smallest shape holding its blocks.
    pub fn shape_of(&self, i: usize, tag: u64) -> Option<TileShape> {
        match tag {
            0 => self.dt1.tiles.get(i).and_then(TileShape::of_tile),
            n => self.shapes.get(n as usize - 1).copied(),
        }
    }

    /// Each tile's lock tag when the document opened.
    pub fn tags(&self) -> Vec<u64> {
        self.tags.clone()
    }

    /// "tile 12: orientation 0, main 3, sub 1 (#0, rarity 1)".
    pub fn label(&self, i: usize, tag: u64) -> String {
        let Some(t) = self.info.get(i) else {
            return format!("tile {i}");
        };
        let shaped = (tag != 0)
            .then(|| self.shape_of(i, tag))
            .flatten()
            .filter(|s| i32::from(s.orientation) != t.orientation || self.new_tiles.is_none());
        match shaped {
            Some(s) => format!(
                "tile {i}: orientation {} (was {}), main {}, sub {} (#{}, rarity {}), new shape",
                s.orientation, t.orientation, t.main, t.sub, t.n, t.rarity
            ),
            None => format!(
                "tile {i}: orientation {}, main {}, sub {} (#{}, rarity {})",
                t.orientation, t.main, t.sub, t.n, t.rarity
            ),
        }
    }

    /// The frames as the editor's single direction.
    pub fn frames(&self) -> Vec<Vec<IndexedImage>> {
        vec![self.base.clone()]
    }

    /// Per tile, the pixels it can hold: some block's footprint (tag 0) or
    /// its shape's template.
    pub fn editable(&self) -> Vec<Mask> {
        self.base
            .iter()
            .enumerate()
            .map(|(i, img)| match self.shape_of(i, self.tags[i]) {
                Some(shape) if self.tags[i] != 0 => template_mask(img, shape),
                _ => self.footprint_mask(i, img),
            })
            .collect()
    }

    fn footprint_mask(&self, i: usize, img: &IndexedImage) -> Mask {
        let blocks = self.footprints.get(i).map_or(&[][..], Vec::as_slice);
        let mut m = Mask::none(img.width, img.height);
        for y in 0..img.height {
            for x in 0..img.width {
                let (bx, by) = (
                    i64::from(img.offset_x) + i64::from(x),
                    i64::from(img.offset_y) + i64::from(y),
                );
                if blocks.iter().any(|f| f.contains(bx, by)) {
                    m.bits[y as usize * img.width as usize + x as usize] = true;
                }
            }
        }
        m
    }

    /// Tile `i`'s current `picture` moved into `shape`: the frame (the
    /// template's box, anchor space), its editable pixels, the lock tag and
    /// how many opaque pixels the shape cannot hold.
    pub fn reshape_frame(
        &mut self,
        picture: &IndexedImage,
        shape: TileShape,
    ) -> Result<(IndexedImage, Mask, u64, usize), String> {
        let (frame, mask, dropped) = shaped_frame(picture, shape)?;
        let tag = self.register_shape(shape);
        Ok((frame, mask, tag, dropped))
    }

    /// The tiles with the edited `frames` (one direction, one frame per
    /// tile) written in, each tile's lock tag from `tags`.
    pub fn tiles(
        &self,
        frames: &[Vec<IndexedImage>],
        tags: &[u64],
    ) -> Result<(Vec<Tile>, usize, Vec<String>), String> {
        let frames = match frames {
            [one] if one.len() == self.rendered.len() => one,
            _ => return Err("the tile set no longer matches the file".to_owned()),
        };
        let mut tiles = Vec::with_capacity(frames.len());
        let mut changed = 0;
        let mut warnings = Vec::new();
        for (t, edited) in frames.iter().enumerate() {
            let orig = &self.dt1.tiles[t];
            let tag = tags.get(t).copied().unwrap_or(0);
            let tile = if tag == 0 {
                let old = &self.rendered[t];
                if edited == old {
                    orig.clone()
                } else {
                    if (
                        edited.width,
                        edited.height,
                        edited.offset_x,
                        edited.offset_y,
                    ) != (old.width, old.height, old.offset_x, old.offset_y)
                    {
                        return Err(format!("tile {t} changed size"));
                    }
                    let mut tile = orig.clone();
                    self.write_tile(&mut tile, t, edited, old)?;
                    tile
                }
            } else {
                let shape = self
                    .shape_of(t, tag)
                    .ok_or_else(|| format!("tile {t}: its shape is unknown"))?;
                let built = rebuild(orig, edited, shape)?;
                if let Some(w) = built.warning() {
                    warnings.push(format!("tile {t}: {w}"));
                }
                built.tile
            };
            if tile != *orig {
                changed += 1;
            }
            tiles.push(tile);
        }
        Ok((tiles, changed, warnings))
    }

    /// The DT1 file with the edited `frames` written back (see
    /// [`Dt1Doc::tiles`]). Untouched tiles keep their bytes.
    pub fn save(&self, frames: &[Vec<IndexedImage>], tags: &[u64]) -> Result<Dt1Saved, String> {
        let (tiles, changed, warnings) = self.tiles(frames, tags)?;
        let dt1 = Dt1 {
            version: self.dt1.version,
            tiles,
        };
        let bytes = dt1.to_bytes().map_err(|e| e.to_string())?;
        Ok(Dt1Saved {
            bytes,
            changed,
            warnings,
        })
    }

    fn write_tile(
        &self,
        tile: &mut Tile,
        t: usize,
        edited: &IndexedImage,
        old: &IndexedImage,
    ) -> Result<(), String> {
        let foots = self.footprints.get(t).map_or(&[][..], Vec::as_slice);
        // One canvas per block that gets a changed pixel, seeded from its
        // current pixels.
        let mut canvases: Vec<Option<IndexedImage>> = vec![None; tile.blocks.len()];
        let w = edited.width as usize;
        for i in 0..edited.pixels.len() {
            if (edited.pixels[i], edited.alpha[i]) == (old.pixels[i], old.alpha[i]) {
                continue;
            }
            let (bx, by) = (
                i64::from(edited.offset_x) + (i % w) as i64,
                i64::from(edited.offset_y) + (i / w) as i64,
            );
            // Blocks drawn later cover earlier ones, so the last wins.
            let Some(b) = foots.iter().rposition(|f| f.contains(bx, by)) else {
                return Err(format!(
                    "tile {t}: a changed pixel lies outside every block"
                ));
            };
            let foot = &foots[b];
            if canvases[b].is_none() {
                let mut c = IndexedImage::new(foot.w, foot.h, foot.x, foot.y);
                let cur = tile.blocks[b].render().map_err(|e| e.to_string())?;
                paste(&mut c, &cur);
                canvases[b] = Some(c);
            }
            if let Some(c) = canvases[b].as_mut() {
                let (cx, cy) = (
                    (bx - i64::from(foot.x)) as usize,
                    (by - i64::from(foot.y)) as usize,
                );
                let k = cy * foot.w as usize + cx;
                let opaque = edited.alpha[i] != 0 && edited.pixels[i] != 0;
                c.pixels[k] = if opaque { edited.pixels[i] } else { 0 };
                c.alpha[k] = if opaque { 255 } else { 0 };
            }
        }
        for (b, canvas) in canvases.into_iter().enumerate() {
            if let Some(c) = canvas {
                tile.blocks[b] = tile.blocks[b]
                    .with_pixels(&c)
                    .map_err(|e| format!("tile {t}, block {b}: {e}"))?;
            }
        }
        // What the game reads must be what the canvas shows.
        let back = tile.render().map_err(|e| e.to_string())?;
        if !same_pixels(&back, edited) {
            return Err(format!(
                "tile {t}: the blocks cannot hold these pixels (the result would differ)"
            ));
        }
        Ok(())
    }
}

/// `orig` rebuilt from `picture` in `shape` (`build_tile`): number, rarity,
/// sound, animation and preserved bytes kept; flags too when the
/// orientation stays.
fn rebuild(orig: &Tile, picture: &IndexedImage, shape: TileShape) -> Result<BuiltTile, String> {
    let same = i32::from(shape.orientation) == orig.orientation;
    let opts = BuildOptions {
        flags: same.then_some(orig.subtile_flags),
        rarity: orig.rarity,
        sound: orig.sound,
        animated: orig.animated,
    };
    let mut built = build_tile(picture, shape, orig.main_index, orig.sub_index, &opts)
        .map_err(|e| e.to_string())?;
    built.tile.preserved = orig.preserved.clone();
    Ok(built)
}

/// `picture` (anchor space) cut to `shape`'s template box: the frame, its
/// editable pixels (the template) and the opaque pixels left out.
pub fn shaped_frame(
    picture: &IndexedImage,
    shape: TileShape,
) -> Result<(IndexedImage, Mask, usize), String> {
    let t = template(shape).map_err(|e| e.to_string())?;
    let mut frame = t.to_anchor(&t.to_plain(picture));
    let mask = template_mask(&frame, shape);
    for (i, free) in mask.bits.iter().enumerate() {
        if !free {
            frame.pixels[i] = 0;
            frame.alpha[i] = 0;
        }
    }
    let dropped = build_tile(picture, shape, 0, 0, &BuildOptions::default())
        .map_err(|e| e.to_string())?
        .dropped;
    Ok((frame, mask, dropped))
}

/// The pixels of `frame` (anchor space) inside `shape`'s template.
fn template_mask(frame: &IndexedImage, shape: TileShape) -> Mask {
    let mut m = Mask::none(frame.width, frame.height);
    let Ok(t) = template(shape) else { return m };
    for y in 0..frame.height {
        for x in 0..frame.width {
            let (ax, ay) = (
                frame.offset_x.saturating_add(x as i32),
                frame.offset_y.saturating_add(y as i32),
            );
            if t.contains(ax, ay) {
                m.bits[y as usize * frame.width as usize + x as usize] = true;
            }
        }
    }
    m
}

/// Copies `src`'s pixels into `dst` where they overlap (block space).
fn paste(dst: &mut IndexedImage, src: &IndexedImage) {
    for y in 0..src.height as i64 {
        for x in 0..src.width as i64 {
            let (dx, dy) = (
                x + i64::from(src.offset_x) - i64::from(dst.offset_x),
                y + i64::from(src.offset_y) - i64::from(dst.offset_y),
            );
            if dx < 0 || dy < 0 || dx >= i64::from(dst.width) || dy >= i64::from(dst.height) {
                continue;
            }
            let (s, d) = (
                y as usize * src.width as usize + x as usize,
                dy as usize * dst.width as usize + dx as usize,
            );
            dst.pixels[d] = src.pixels[s];
            dst.alpha[d] = src.alpha[s];
        }
    }
}

/// Opaque pixels equal over the same area (block space).
fn same_pixels(a: &IndexedImage, b: &IndexedImage) -> bool {
    let get = |img: &IndexedImage, x: i64, y: i64| -> Option<u8> {
        let (c, r) = (x - i64::from(img.offset_x), y - i64::from(img.offset_y));
        if c < 0 || r < 0 || c >= i64::from(img.width) || r >= i64::from(img.height) {
            return None;
        }
        let i = r as usize * img.width as usize + c as usize;
        (*img.alpha.get(i)? != 0).then(|| img.pixels.get(i).copied())?
    };
    let x0 = i64::from(a.offset_x.min(b.offset_x));
    let y0 = i64::from(a.offset_y.min(b.offset_y));
    let x1 = (i64::from(a.offset_x) + i64::from(a.width))
        .max(i64::from(b.offset_x) + i64::from(b.width));
    let y1 = (i64::from(a.offset_y) + i64::from(a.height))
        .max(i64::from(b.offset_y) + i64::from(b.height));
    (y0..y1).all(|y| (x0..x1).all(|x| get(a, x, y) == get(b, x, y)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use d2_formats::dt1::Tile;

    fn tile(orientation: i32, main: i32, blocks: Vec<Block>) -> Tile {
        Tile {
            orientation,
            main_index: main,
            height: -80,
            width: 160,
            blocks,
            ..Tile::default()
        }
    }

    /// Tile 0: one isometric floor block; tile 1: an RLE block; tile 2: both
    /// kinds (an isometric block and an RLE one beside it).
    fn sample() -> Vec<u8> {
        let mut full = IndexedImage::new(32, 15, 0, 0);
        full.pixels.fill(40);
        full.alpha.fill(255);
        let iso = |x: i16, y: i16| Block::isometric(x, y, 0, 0, &full);
        let mut art = IndexedImage::new(6, 4, 0, 0);
        for (i, (p, a)) in art.pixels.iter_mut().zip(&mut art.alpha).enumerate() {
            if i % 5 != 0 {
                *p = 60 + i as u8;
                *a = 255;
            }
        }
        let rle = |x: i16, y: i16| {
            let mut at = art.clone();
            (at.offset_x, at.offset_y) = (i32::from(x), i32::from(y));
            Block::rle(x, y, 0, 0, 0x2005, &at, 0).unwrap()
        };
        let dt1 = Dt1 {
            version: (7, 6),
            tiles: vec![
                tile(0, 0, vec![iso(0, 0)]),
                tile(1, 2, vec![rle(0, 0)]),
                tile(0, 1, vec![iso(0, 0), rle(40, 3)]),
            ],
        };
        dt1.to_bytes().unwrap()
    }

    fn doc(bytes: &[u8]) -> Dt1Doc {
        Dt1Doc::from_view(&Dt1View::load(bytes).unwrap())
    }

    fn set(frames: &mut [Vec<IndexedImage>], t: usize, x: u32, y: u32, p: u8) {
        let img = &mut frames[0][t];
        let i = (y * img.width + x) as usize;
        img.pixels[i] = p;
        img.alpha[i] = if p == 0 { 0 } else { 255 };
    }

    #[test]
    fn untouched_save_is_byte_identical() {
        let bytes = sample();
        let d = doc(&bytes);
        let saved = d.save(&d.frames(), &d.tags()).unwrap();
        let (out, changed) = (saved.bytes, saved.changed);
        assert_eq!((out, changed), (bytes, 0));
    }

    #[test]
    fn the_diamond_and_block_boxes_are_editable() {
        let d = doc(&sample());
        let m = d.editable();
        // Tile 0 is the 32x15 diamond: its corners are outside.
        assert_eq!((m[0].width, m[0].height), (32, 15));
        assert!(!m[0].get(0, 0) && !m[0].get(31, 14));
        assert!(m[0].get(15, 7));
        // The RLE tile: its whole box (even the transparent holes).
        assert!(m[1].get(0, 0) && m[1].count() == 24);
        // Tile 2: the gap between the diamond and the RLE block is locked.
        let img = &d.base[2];
        assert_eq!(
            d.label(2, 0),
            "tile 2: orientation 0, main 1, sub 0 (#0, rarity 0)"
        );
        assert!(m[2].count() < (img.width * img.height) as usize);
    }

    #[test]
    fn edits_land_in_their_block_and_nothing_else_changes() {
        let bytes = sample();
        let original = Dt1::parse(&bytes).unwrap();
        let d = doc(&bytes);
        let mut frames = d.frames();
        set(&mut frames, 0, 15, 7, 77);
        set(&mut frames, 2, 15, 7, 78);
        // A transparent hole of the RLE tile (index 0 in the box) gets paint.
        set(&mut frames, 1, 0, 0, 90);
        let saved = d.save(&frames, &d.tags()).unwrap();
        let (out, changed) = (saved.bytes, saved.changed);
        assert_eq!(changed, 3);
        let back = Dt1::parse(&out).unwrap();
        let r = |dt: &Dt1, t: usize| dt.tiles[t].render().unwrap();
        assert_eq!(r(&back, 0).pixels[7 * 32 + 15], 77);
        assert_eq!(r(&back, 1).pixels[0], 90);
        let t2 = r(&back, 2);
        assert_eq!(t2.pixels[7 * t2.width as usize + 15], 78);
        // Every other pixel of every tile is as it was.
        for t in 0..3 {
            let (a, b) = (r(&original, t), r(&back, t));
            let diff = a
                .pixels
                .iter()
                .zip(&b.pixels)
                .filter(|(x, y)| x != y)
                .count();
            assert_eq!(diff, [1, 1, 1][t], "tile {t}");
        }
        // The second block of tile 2 is untouched, byte for byte.
        assert_eq!(back.tiles[2].blocks[1], original.tiles[2].blocks[1]);
        assert_eq!(back.tiles[0].blocks[0].x, 0);
    }

    #[test]
    fn a_locked_pixel_is_refused_by_the_session() {
        let d = doc(&sample());
        let mut s =
            capy_paint::PaintSession::new(d.frames(), crate::sprites::palette::grey_palette())
                .unwrap();
        s.lock_structure(d.editable());
        let f = capy_paint::FrameRef { dir: 0, frame: 0 };
        let brush = capy_paint::Brush {
            size: 1,
            shape: capy_paint::BrushShape::Square,
        };
        let paint = capy_paint::Paint::Index(9);
        // (0, 0) is outside the diamond.
        assert!(s.pencil(f, &[(0, 0)], brush, paint, None).is_err());
        assert!(!s.is_dirty());
        assert!(s.pencil(f, &[(15, 7)], brush, paint, None).is_ok());
        assert!(s.is_dirty());
        // A fill never leaks past the diamond.
        s.fill(f, (15, 7), paint, capy_paint::FillMode::Global, None)
            .unwrap();
        let img = s.frame(f).unwrap();
        assert_eq!(img.pixels[0], 0);
        assert_eq!(img.pixels[7 * 32 + 15], 9);
        // The structure is fixed.
        assert!(s.insert_frame(1, None).is_err());
        assert!(s.delete_frame(0).is_err());
        assert!(s.resize_canvas(f, 40, 20, (0, 0)).is_err());
        assert!(s.set_offset(f, 5, 5).is_err());
    }

    fn blank_floor_doc() -> Dt1Doc {
        let built = build_tile(
            &IndexedImage::new(0, 0, 0, 0),
            TileShape::floor(),
            5,
            2,
            &BuildOptions::default(),
        )
        .unwrap();
        Dt1Doc::new_tiles(vec![built.tile], &[TileShape::floor()], NewTilesTarget::Map).unwrap()
    }

    #[test]
    fn a_blank_new_tile_is_editable_across_its_whole_template() {
        let d = blank_floor_doc();
        let tags = d.tags();
        assert!(tags[0] != 0);
        let frame = &d.frames()[0][0];
        assert_eq!(
            (frame.width, frame.height, frame.offset_x, frame.offset_y),
            (160, 79, 0, 0)
        );
        // the whole diamond is free (no blocks yet), its corners are not
        let m = &d.editable()[0];
        let t = template(TileShape::floor()).unwrap();
        let want = t.mask().alpha.iter().filter(|&&a| a != 0).count();
        assert_eq!(m.count(), want);
        assert!(!m.get(0, 0) && m.get(80, 39));
        // painting the centre makes a block there; the number stays
        let mut frames = d.frames();
        let i = 39 * 160 + 80;
        frames[0][0].pixels[i] = 120;
        frames[0][0].alpha[i] = 255;
        let saved = d.save(&frames, &tags).unwrap();
        assert_eq!(saved.changed, 1);
        assert!(saved.warnings.is_empty());
        let back = Dt1::parse(&saved.bytes).unwrap();
        let tile = &back.tiles[0];
        assert_eq!(
            (tile.orientation, tile.main_index, tile.sub_index),
            (0, 5, 2)
        );
        assert_eq!(tile.blocks.len(), 1);
        assert_eq!(
            tile.render()
                .unwrap()
                .pixels
                .iter()
                .filter(|&&p| p == 120)
                .count(),
            1
        );
    }

    #[test]
    fn reshaping_a_tile_in_the_session_saves_the_new_shape_and_undoes() {
        // a 64-pixel left wall filled with colour 50
        let shape = TileShape::wall(1, 64);
        let t = template(shape).unwrap();
        let mut art = t.mask();
        art.pixels.fill(50);
        let wall = build_tile(&art, shape, 1, 0, &BuildOptions::default())
            .unwrap()
            .tile;
        let bytes = Dt1 {
            version: (7, 6),
            tiles: vec![wall],
        }
        .to_bytes()
        .unwrap();
        let mut d = doc(&bytes);
        let mut s =
            capy_paint::PaintSession::new(d.frames(), crate::sprites::palette::grey_palette())
                .unwrap();
        s.lock_structure(d.editable());
        s.set_lock_tags(d.tags());
        let f = capy_paint::FrameRef { dir: 0, frame: 0 };

        // taller: 128 pixels, every old pixel kept
        let picture = s.frame(f).unwrap().clone();
        let (frame, mask, tag, dropped) =
            d.reshape_frame(&picture, TileShape::wall(1, 100)).unwrap();
        assert_eq!(dropped, 0);
        assert_eq!((frame.height, frame.offset_y), (128, -128));
        s.relock_frame(f, frame, mask, tag).unwrap();
        // the new top rows are paintable
        s.pencil(
            f,
            &[(10, 2)],
            capy_paint::Brush {
                size: 1,
                shape: capy_paint::BrushShape::Square,
            },
            capy_paint::Paint::Index(9),
            None,
        )
        .unwrap();
        let tags: Vec<u64> = (0..1).map(|i| s.lock_tag(i)).collect();
        let saved = d.save(s.frames(), &tags).unwrap();
        let back = Dt1::parse(&saved.bytes).unwrap();
        assert_eq!(back.tiles[0].height, -(128 + 32));
        // the pixel painted in the new top rows is stored: (10, 2) of the
        // frame is anchor (10, -126)
        let r = back.tiles[0].render().unwrap();
        let (cx, cy) = (10 - r.offset_x, -126 - r.offset_y);
        assert!(cx >= 0 && cy >= 0);
        assert_eq!(r.pixels[(cy * r.width as i32 + cx) as usize], 9);
        assert!(d.label(0, tags[0]).contains("new shape"));

        // a floor holds none of a wall's pixels: they are counted
        let (_, _, _, dropped) = d.reshape_frame(&picture, TileShape::floor()).unwrap();
        assert_eq!(dropped, 160 * 64);

        // undo both steps: tag 0 again, and the file is as it was
        s.undo();
        s.undo();
        assert_eq!(s.lock_tag(0), 0);
        let saved = d.save(s.frames(), &[0]).unwrap();
        assert_eq!((saved.bytes, saved.changed), (bytes, 0));
    }
}
