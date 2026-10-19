//! DCC cell-colour limit: check and auto-reduce.
//!
//! # The rule
//!
//! A DCC frame is stored as 4x4 cells, and each cell carries a table of four
//! pixel values (`docs/specs/dc6-dcc.md` section 2.3, "Stage 1 per frame, per
//! cell", row c: "`pixel_mask` ... 4 from pixelMask ... mask of changed colour
//! slots", and "Bits per pixel": `val[0]==val[1]` gives 0 bits, `val[1]==val[2]`
//! gives 1 bit, otherwise 2 bits, so a pixel selects one of **four** slots;
//! reference `dcc.c:1149-1159`). A cell therefore holds at most 4 distinct
//! pixel values.
//!
//! Transparent pixels are a value too (palette index 0), so a cell with a
//! silhouette edge may hold only three opaque colours.
//!
//! # The grid
//!
//! Cells follow the decoder's own partition (`d2_formats::dcc::cell_spans`):
//! along each axis a partial first cell ends on the direction box's 4-pixel
//! grid, then 4-pixel cells, and a trailing 1-pixel sliver merges into the
//! last cell (so a cell can be 5 pixels wide). A frame's box is the bounding
//! box of its opaque pixels and the direction box the union of those, which is
//! what `Dcc::from_frames` encodes. Using the decoder's partition guarantees
//! that a frame passing this check encodes without further reduction (tested
//! by an encode -> decode round trip). Fully transparent frames have no cells.

use crate::error::Result;
use crate::ramps::distance;
use crate::session::{FrameRef, PaintSession};
use d2_formats::IndexedImage;

/// Most distinct pixel values (transparent included) a 4x4 cell can hold.
const MAX_CELL_VALUES: usize = 4;
const CELL: i64 = 4;

/// A cell holding more than 4 distinct pixel values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellViolation {
    pub frame: FrameRef,
    /// Cell column / row in the direction's 4x4 grid.
    pub cell_x: u32,
    pub cell_y: u32,
    /// Distinct pixel values in the cell (transparent counts as one).
    pub colors: usize,
    /// The cell's pixel rectangle in frame-image coordinates, `(x0, y0, x1,
    /// y1)` with exclusive ends. It is exactly the decoder's cell
    /// (`d2_formats::dcc::cell_spans`), so it can be 1..=5 pixels wide or
    /// tall, and always lies inside the frame image.
    pub rect: (u32, u32, u32, u32),
}

/// Pixel rectangle in frame-image coordinates, `x0..x1` by `y0..y1`.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// Bounding box of the opaque pixels in anchor coordinates (end exclusive).
fn opaque_box(img: &IndexedImage) -> Option<(i64, i64, i64, i64)> {
    let w = img.width as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut any = false;
    for (i, &a) in img.alpha.iter().enumerate() {
        if a != 0 && w > 0 {
            any = true;
            x0 = x0.min(i % w);
            x1 = x1.max(i % w);
            y0 = y0.min(i / w);
            y1 = y1.max(i / w);
        }
    }
    any.then(|| {
        (
            i64::from(img.offset_x) + x0 as i64,
            i64::from(img.offset_y) + y0 as i64,
            i64::from(img.offset_x) + x1 as i64 + 1,
            i64::from(img.offset_y) + y1 as i64 + 1,
        )
    })
}

impl Rect {
    /// Non-negative by construction (the cell lies inside the frame box).
    fn to_u32(self) -> (u32, u32, u32, u32) {
        let c = |v: i32| u32::try_from(v).unwrap_or(0);
        (c(self.x0), c(self.y0), c(self.x1), c(self.y1))
    }
}

/// Index of pixel `(x, y)` in the buffers; `None` for a negative coordinate
/// or an index that overflows.
fn index(img: &IndexedImage, x: i32, y: i32) -> Option<usize> {
    let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
    y.checked_mul(img.width as usize)?.checked_add(x)
}

fn value(img: &IndexedImage, x: i32, y: i32) -> u8 {
    let Some(i) = index(img, x, y) else {
        return 0;
    };
    match (img.alpha.get(i), img.pixels.get(i)) {
        (Some(&a), Some(&p)) if a != 0 => p,
        _ => 0,
    }
}

/// Every cell over the limit, with its frame-image rectangle.
fn scan(frames: &[Vec<IndexedImage>]) -> Vec<(CellViolation, Rect)> {
    let mut out = Vec::new();
    for (dir, images) in frames.iter().enumerate() {
        let boxes: Vec<_> = images.iter().map(opaque_box).collect();
        let Some(ox) = boxes.iter().flatten().map(|b| b.0).min() else {
            continue;
        };
        let oy = boxes.iter().flatten().map(|b| b.1).min().unwrap_or(0);
        for (frame, (img, b)) in images.iter().zip(&boxes).enumerate() {
            let Some(&(bx0, by0, bx1, by1)) = b.as_ref() else {
                continue;
            };
            let cols = d2_formats::dcc::cell_spans(bx0, ox, bx1 - bx0);
            let rows = d2_formats::dcc::cell_spans(by0, oy, by1 - by0);
            for &(rs, rl) in &rows {
                for &(cs, cl) in &cols {
                    let (cx, cy) = (cs.div_euclid(CELL), rs.div_euclid(CELL));
                    // Inside the frame box, so these fit the image (and an i32).
                    let local = |v: i64, offset: i32| (v - i64::from(offset)) as i32;
                    let r = Rect {
                        x0: local(ox + cs, img.offset_x),
                        y0: local(oy + rs, img.offset_y),
                        x1: local(ox + cs + cl, img.offset_x),
                        y1: local(oy + rs + rl, img.offset_y),
                    };
                    let mut seen = [false; 256];
                    for y in r.y0..r.y1 {
                        for x in r.x0..r.x1 {
                            seen[usize::from(value(img, x, y))] = true;
                        }
                    }
                    let colors = seen.iter().filter(|&&s| s).count();
                    if colors > MAX_CELL_VALUES {
                        out.push((
                            CellViolation {
                                frame: FrameRef { dir, frame },
                                cell_x: u32::try_from(cx).unwrap_or(u32::MAX),
                                cell_y: u32::try_from(cy).unwrap_or(u32::MAX),
                                colors,
                                rect: r.to_u32(),
                            },
                            r,
                        ));
                    }
                }
            }
        }
    }
    out
}

/// DCC stores each 4x4 cell with at most 4 distinct pixel values (see
/// `docs/specs/dc6-dcc.md`); lists the cells over the limit, in direction,
/// frame, row, column order.
pub fn dcc_cell_violations(frames: &[Vec<IndexedImage>]) -> Vec<CellViolation> {
    scan(frames).into_iter().map(|(v, _)| v).collect()
}

/// Reduce every offending cell to 4 values by merging the two nearest opaque
/// colours (by RGB distance; the less used one becomes the more used one)
/// until the cell fits. Transparency is never merged into a colour. Returns
/// how many cells changed; the whole reduction is one undo step.
pub fn reduce_dcc_cells(session: &mut PaintSession) -> Result<usize> {
    let found = scan(session.frames());
    if found.is_empty() {
        return Ok(0);
    }
    let colors = session.palette().colors;
    let mut by_frame: Vec<(FrameRef, Vec<Rect>)> = Vec::new();
    for (v, r) in &found {
        match by_frame.last_mut() {
            Some((f, rects)) if *f == v.frame => rects.push(*r),
            _ => by_frame.push((v.frame, vec![*r])),
        }
    }
    let changed = found.len();
    session.step("reduce DCC cells", |s| {
        for (f, rects) in by_frame {
            let mut img = s.img(f)?.clone();
            for r in rects {
                reduce_cell(&mut img, r, &colors);
            }
            s.commit_image(f, img)?;
        }
        Ok(())
    })?;
    Ok(changed)
}

fn reduce_cell(img: &mut IndexedImage, r: Rect, colors: &[[u8; 3]; 256]) {
    let mut count = [0u32; 256];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            count[usize::from(value(img, x, y))] += 1;
        }
    }
    let transparent = count[0] > 0;
    let keep = MAX_CELL_VALUES - usize::from(transparent);
    let mut live: Vec<u8> = (1..=255u8).filter(|&i| count[usize::from(i)] > 0).collect();
    // remap[i] = the colour that i was merged into.
    let mut remap: [u8; 256] = std::array::from_fn(|i| i as u8);
    while live.len() > keep && live.len() >= 2 {
        let mut best: Option<(u64, usize, usize)> = None;
        for a in 0..live.len() {
            for b in a + 1..live.len() {
                let d = distance(colors[usize::from(live[a])], colors[usize::from(live[b])]);
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, a, b));
                }
            }
        }
        let Some((_, a, b)) = best else { break };
        let (ia, ib) = (live[a], live[b]);
        // The less used colour disappears; ties drop the higher index.
        let (stay, gone, gone_pos) = if count[usize::from(ia)] >= count[usize::from(ib)] {
            (ia, ib, b)
        } else {
            (ib, ia, a)
        };
        count[usize::from(stay)] += count[usize::from(gone)];
        count[usize::from(gone)] = 0;
        for m in &mut remap {
            if *m == gone {
                *m = stay;
            }
        }
        live.remove(gone_pos);
    }
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let Some(i) = index(img, x, y) else {
                continue;
            };
            if let (Some(&a), Some(p)) = (img.alpha.get(i), img.pixels.get_mut(i))
                && a != 0
            {
                *p = remap[usize::from(*p)];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::palette::Palette;

    fn palette() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [
                i as u8,
                (i as u8).wrapping_mul(3),
                (i as u8).wrapping_mul(7),
            ];
        }
        Palette { colors }
    }

    fn img(w: u32, h: u32, ox: i32, oy: i32, px: &[u8]) -> IndexedImage {
        let mut im = IndexedImage::new(w, h, ox, oy);
        for (i, &p) in px.iter().enumerate() {
            im.pixels[i] = p;
            im.alpha[i] = if p == 0 { 0 } else { 255 };
        }
        im
    }

    #[test]
    fn counts_distinct_values_with_transparency() {
        // One 4x4 cell: 4 opaque colours -> fine; add transparency -> 5.
        let mut px = vec![1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4];
        let ok = img(4, 4, 0, 0, &px);
        assert!(dcc_cell_violations(&[vec![ok]]).is_empty());
        px[15] = 0;
        let bad = img(4, 4, 0, 0, &px);
        let v = dcc_cell_violations(&[vec![bad]]);
        assert_eq!(
            v,
            [CellViolation {
                frame: FrameRef { dir: 0, frame: 0 },
                cell_x: 0,
                cell_y: 0,
                colors: 5,
                rect: (0, 0, 4, 4),
            }]
        );
    }

    #[test]
    fn grid_follows_direction_box() {
        // Frame 0 starts at x=0, frame 1 at x=2: its cells are phase-shifted,
        // so 4 pixels wide at offset 2 straddle two cells (2 + 2 pixels).
        let a = img(1, 1, 0, 0, &[9]);
        let b = img(4, 1, 2, 0, &[1, 2, 3, 4]);
        // Cell 0 of frame 1 holds x=2,3 (values 1,2); cell 1 holds x=4,5 (3,4).
        assert!(dcc_cell_violations(&[vec![a, b]]).is_empty());
        // A 5-pixel-wide frame box is ONE cell: the decoder merges a trailing
        // 1-pixel sliver into the last cell (`d2_formats::dcc::cell_spans`),
        // so 5 distinct values in one 5x1 row violate the limit.
        let wide = img(
            5,
            4,
            0,
            0,
            &[1, 2, 3, 4, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let v = dcc_cell_violations(&[vec![wide]]);
        assert_eq!(v.len(), 1, "the sliver merges into one 5-wide cell: {v:?}");
        assert_eq!(v[0].colors, 5);
        // A 6-pixel row splits 4 + 2, so 6 values fit (4 and 2 per cell).
        let six = img(6, 1, 0, 0, &[1, 2, 3, 4, 5, 6]);
        assert!(dcc_cell_violations(&[vec![six]]).is_empty());
        let dense = img(
            4,
            4,
            0,
            0,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        );
        assert_eq!(dcc_cell_violations(&[vec![dense]])[0].colors, 16);
    }

    #[test]
    fn violation_rect_is_the_decoder_cell_in_frame_coordinates() {
        // Frame 0 puts the direction box origin at anchor (0, 0). Frame 1 sits
        // at offset (1, 1) with a transparent first column, so its opaque box
        // is anchor x 2..9, y 1..5. `cell_spans` splits x into 2..4 and 4..9
        // (the trailing sliver merges, 5 wide) and keeps y as one 1..5 cell.
        let a = img(1, 1, 0, 0, &[9]);
        let mut px = vec![0u8; 8 * 4];
        for y in 0..4 {
            for x in 1..8 {
                px[y * 8 + x] = (1 + y * 8 + x) as u8;
            }
        }
        let b = img(8, 4, 1, 1, &px);
        let frames = vec![vec![a, b.clone()]];
        let v = dcc_cell_violations(&frames);
        let rects: Vec<_> = v.iter().map(|c| c.rect).collect();
        assert_eq!(rects, [(1, 0, 3, 4), (3, 0, 8, 4)]);
        let cols = d2_formats::dcc::cell_spans(2, 0, 7);
        assert_eq!(cols, [(2, 2), (4, 5)], "matches the decoder partition");
        // Each rect holds exactly the reported distinct values.
        for c in &v {
            let (x0, y0, x1, y1) = c.rect;
            assert!(x1 <= b.width && y1 <= b.height);
            let mut seen = std::collections::BTreeSet::new();
            for y in y0..y1 {
                for x in x0..x1 {
                    seen.insert(value(&b, x as i32, y as i32));
                }
            }
            assert_eq!(seen.len(), c.colors, "{c:?}");
        }
    }

    #[test]
    fn reduce_fixes_every_violation_and_is_one_undo_step() {
        let px: Vec<u8> = (1..=16).collect();
        let frame = img(4, 4, 0, 0, &px);
        let original = vec![vec![frame.clone(), frame]];
        let mut s = PaintSession::new(original.clone(), palette()).unwrap();
        assert_eq!(dcc_cell_violations(s.frames()).len(), 2);
        let changed = reduce_dcc_cells(&mut s).unwrap();
        assert_eq!(changed, 2);
        assert!(dcc_cell_violations(s.frames()).is_empty());
        for f in &s.frames()[0] {
            assert!(f.alpha.iter().all(|&a| a == 255), "silhouette unchanged");
        }
        // Idempotent, no extra history.
        assert_eq!(reduce_dcc_cells(&mut s).unwrap(), 0);
        assert!(s.undo());
        assert_eq!(s.frames(), original.as_slice());
        assert!(!s.undo());
    }

    #[test]
    fn reduce_keeps_transparency_and_merges_nearest() {
        let mut colors = [[0u8; 3]; 256];
        colors[1] = [10, 10, 10];
        colors[2] = [12, 10, 10];
        colors[3] = [200, 0, 0];
        colors[4] = [0, 200, 0];
        colors[5] = [0, 0, 200];
        #[rustfmt::skip]
        let px = [
            1, 2, 3, 4,
            1, 1, 5, 0,
            0, 0, 0, 0,
            0, 0, 0, 1,
        ];
        let mut s =
            PaintSession::new(vec![vec![img(4, 4, 0, 0, &px)]], Palette { colors }).unwrap();
        assert_eq!(dcc_cell_violations(s.frames())[0].colors, 6);
        assert_eq!(reduce_dcc_cells(&mut s).unwrap(), 1);
        let out = &s.frames()[0][0];
        assert_eq!(out.pixels[1], 1, "2 merges into the nearer, more used 1");
        let opaque: std::collections::BTreeSet<u8> = out
            .pixels
            .iter()
            .zip(&out.alpha)
            .filter(|(_, a)| **a != 0)
            .map(|(p, _)| *p)
            .collect();
        assert!(opaque.len() <= 3, "{opaque:?}");
        let alpha: Vec<u8> = px.iter().map(|&p| if p == 0 { 0 } else { 255 }).collect();
        assert_eq!(out.alpha, alpha, "silhouette untouched");
    }

    /// A noisy multi-frame, multi-direction sprite reduces to a clean one, and
    /// the silhouette (alpha) never changes.
    #[test]
    fn reduction_makes_noisy_sprites_encodable() {
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut dirs = Vec::new();
        for _ in 0..2 {
            let mut frames = Vec::new();
            for k in 0..3 {
                let (w, h) = (13 + k, 11 + k);
                let mut im = IndexedImage::new(w, h, -(k as i32) - 3, -5 + k as i32);
                for i in 0..(w * h) as usize {
                    let r = next();
                    if r % 7 != 0 {
                        im.pixels[i] = 1 + (r >> 8) as u8 % 30;
                        im.alpha[i] = 255;
                    }
                }
                frames.push(im);
            }
            dirs.push(frames);
        }
        let alpha: Vec<Vec<Vec<u8>>> = dirs
            .iter()
            .map(|d| d.iter().map(|f| f.alpha.clone()).collect())
            .collect();
        let mut s = PaintSession::new(dirs, palette()).unwrap();
        let bad = dcc_cell_violations(s.frames()).len();
        assert!(bad > 20, "the noise should violate many cells, got {bad}");
        assert_eq!(reduce_dcc_cells(&mut s).unwrap(), bad);
        assert!(dcc_cell_violations(s.frames()).is_empty());
        for (d, dir) in s.frames().iter().enumerate() {
            for (f, im) in dir.iter().enumerate() {
                assert_eq!(im.alpha, alpha[d][f]);
            }
        }
        // The reduced frames survive DCC encode -> decode unchanged. The
        // decoder crops frames to their own box, so compare the opaque pixels
        // in anchor space rather than the rectangles.
        let opaque = |im: &IndexedImage| -> std::collections::BTreeMap<(i64, i64), u8> {
            let mut out = std::collections::BTreeMap::new();
            for y in 0..im.height {
                for x in 0..im.width {
                    let i = (y * im.width + x) as usize;
                    if im.alpha[i] != 0 {
                        let at = (
                            i64::from(im.offset_x) + i64::from(x),
                            i64::from(im.offset_y) + i64::from(y),
                        );
                        out.insert(at, im.pixels[i]);
                    }
                }
            }
            out
        };
        let dcc = d2_formats::dcc::Dcc::from_frames(s.frames()).unwrap();
        let back = d2_formats::dcc::Dcc::parse(&dcc.to_bytes().unwrap()).unwrap();
        for (d, dir) in s.frames().iter().enumerate() {
            for (f, im) in dir.iter().enumerate() {
                let decoded = &back.directions[d].frames[f];
                assert_eq!(opaque(decoded), opaque(im), "dir {d} frame {f}");
            }
        }
    }

    #[test]
    fn extreme_offsets_do_not_overflow() {
        let a = img(4, 4, i32::MIN, i32::MIN, &(1..=16).collect::<Vec<u8>>());
        let b = img(
            4,
            4,
            i32::MAX - 4,
            i32::MAX - 4,
            &(1..=16).collect::<Vec<u8>>(),
        );
        let frames = vec![vec![a, b]];
        assert_eq!(dcc_cell_violations(&frames).len(), 2);
        let mut s = PaintSession::new(frames, palette()).unwrap();
        assert_eq!(reduce_dcc_cells(&mut s).unwrap(), 2);
        assert!(dcc_cell_violations(s.frames()).is_empty());
    }
}
