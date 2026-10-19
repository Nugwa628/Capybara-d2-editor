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
//! The grid is aligned to the *direction* box: `cells = ceil(box / 4)` from the
//! box origin, and each frame's cells are the direction cells clipped to the
//! frame (spec "Cell grid", `dcc.c:632,645`). A frame's box is taken as the
//! bounding box of its opaque pixels, and the direction box as the union of
//! those, which is what an encoder cropping the frames would produce. Fully
//! transparent frames have no cells.

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

fn value(img: &IndexedImage, x: i32, y: i32) -> u8 {
    let i = y as usize * img.width as usize + x as usize;
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
            let cx0 = (bx0 - ox).div_euclid(CELL);
            let cx1 = (bx1 - 1 - ox).div_euclid(CELL);
            let cy0 = (by0 - oy).div_euclid(CELL);
            let cy1 = (by1 - 1 - oy).div_euclid(CELL);
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    // Cell clipped to the frame box, then to image coordinates.
                    // Inside the frame box, so these fit the image (and an i32).
                    let local = |v: i64, offset: i32| (v - i64::from(offset)) as i32;
                    let r = Rect {
                        x0: local((ox + cx * CELL).max(bx0), img.offset_x),
                        y0: local((oy + cy * CELL).max(by0), img.offset_y),
                        x1: local((ox + cx * CELL + CELL).min(bx1), img.offset_x),
                        y1: local((oy + cy * CELL + CELL).min(by1), img.offset_y),
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
            let i = y as usize * img.width as usize + x as usize;
            if img.alpha[i] != 0 {
                img.pixels[i] = remap[usize::from(img.pixels[i])];
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
                colors: 5
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
        // Same pixels in a 5-wide, 1-tall row all land in one cell when aligned.
        let wide = img(
            5,
            4,
            0,
            0,
            &[1, 2, 3, 4, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let v = dcc_cell_violations(&[vec![wide]]);
        assert!(v.is_empty(), "5 values spread over 2 cells: {v:?}");
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
        // TODO: once `d2_formats::dcc::Dcc::from_frames` is merged, encode the
        // reduced frames with `to_bytes`, `parse` them again and compare.
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
