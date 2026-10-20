//! A DT1 as an editor document: every tile is one frame (`Tile::render`,
//! all blocks composed, offsets in block space), pixels outside every
//! block's footprint are locked, and saving cuts the edited frames back into
//! their blocks. Only blocks whose pixels changed are re-encoded, so the rest
//! of the file keeps its bytes.

use capy_paint::Mask;
use d2_formats::IndexedImage;
use d2_formats::dt1::{Block, BlockFormat, Dt1};

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

pub struct Dt1Doc {
    dt1: Dt1,
    /// Each tile's frame as last loaded or saved.
    base: Vec<IndexedImage>,
    pub info: Vec<TileInfo>,
    footprints: Vec<Vec<Footprint>>,
}

impl Dt1Doc {
    pub fn from_view(view: &Dt1View) -> Dt1Doc {
        let footprints = view
            .dt1
            .tiles
            .iter()
            .map(|t| t.blocks.iter().map(Footprint::of).collect())
            .collect();
        Dt1Doc {
            dt1: view.dt1.clone(),
            base: view.images.clone(),
            info: view.tiles.clone(),
            footprints,
        }
    }

    /// "tile 12: orientation 0, main 3, sub 1 (#0, rarity 1)".
    pub fn label(&self, i: usize) -> String {
        match self.info.get(i) {
            Some(t) => format!(
                "tile {i}: orientation {}, main {}, sub {} (#{}, rarity {})",
                t.orientation, t.main, t.sub, t.n, t.rarity
            ),
            None => format!("tile {i}"),
        }
    }

    /// The frames as the editor's single direction.
    pub fn frames(&self) -> Vec<Vec<IndexedImage>> {
        vec![self.base.clone()]
    }

    /// Per tile, the pixels some block can draw.
    pub fn editable(&self) -> Vec<Mask> {
        self.base
            .iter()
            .zip(&self.footprints)
            .map(|(img, blocks)| {
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
            })
            .collect()
    }

    /// Writes the edited `frames` (one direction, one frame per tile) into
    /// the tiles' blocks and returns the file, plus how many tiles changed.
    /// Nothing changes in `self` unless the whole save works.
    pub fn save(&mut self, frames: &[Vec<IndexedImage>]) -> Result<(Vec<u8>, usize), String> {
        let frames = match frames {
            [one] if one.len() == self.base.len() => one,
            _ => return Err("the tile set no longer matches the file".to_owned()),
        };
        let mut dt1 = self.dt1.clone();
        let mut changed = 0;
        for (t, (edited, old)) in frames.iter().zip(&self.base).enumerate() {
            if edited == old {
                continue;
            }
            if (
                edited.width,
                edited.height,
                edited.offset_x,
                edited.offset_y,
            ) != (old.width, old.height, old.offset_x, old.offset_y)
            {
                return Err(format!("tile {t} changed size"));
            }
            self.write_tile(&mut dt1, t, edited, old)?;
            changed += 1;
        }
        let bytes = dt1.to_bytes().map_err(|e| e.to_string())?;
        self.dt1 = dt1;
        self.base = frames.clone();
        Ok((bytes, changed))
    }

    fn write_tile(
        &self,
        dt1: &mut Dt1,
        t: usize,
        edited: &IndexedImage,
        old: &IndexedImage,
    ) -> Result<(), String> {
        let foots = self.footprints.get(t).map_or(&[][..], Vec::as_slice);
        let tile = dt1
            .tiles
            .get_mut(t)
            .ok_or_else(|| format!("tile {t} is gone"))?;
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
        let mut d = doc(&bytes);
        let (out, changed) = d.save(&d.frames()).unwrap();
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
        assert!(m[2].count() < (img.width * img.height) as usize);
    }

    #[test]
    fn edits_land_in_their_block_and_nothing_else_changes() {
        let bytes = sample();
        let original = Dt1::parse(&bytes).unwrap();
        let mut d = doc(&bytes);
        let mut frames = d.frames();
        set(&mut frames, 0, 15, 7, 77);
        set(&mut frames, 2, 15, 7, 78);
        // A transparent hole of the RLE tile (index 0 in the box) gets paint.
        set(&mut frames, 1, 0, 0, 90);
        let (out, changed) = d.save(&frames).unwrap();
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
}
