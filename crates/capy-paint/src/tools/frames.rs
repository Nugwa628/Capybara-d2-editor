//! Frame structure: canvas size, offsets, and the frame list shared by every
//! direction.

use super::{at, pixel, put_xy};
use crate::error::{PaintError, Result, invalid};
use crate::session::{FrameRef, MAX_SIDE, PaintSession};
use d2_formats::IndexedImage;

impl PaintSession {
    fn frames_per_direction(&self) -> usize {
        self.frames.first().map_or(0, Vec::len)
    }

    fn require_directions(&self) -> Result<()> {
        self.require_free_structure()?;
        if self.frames.is_empty() {
            return invalid("the sprite has no directions");
        }
        Ok(())
    }

    /// Change the canvas size of one frame. `anchor` says which side keeps its
    /// place on each axis: negative = left / top, 0 = centre, positive = right
    /// / bottom. New area is transparent, cropped area is lost, and the frame
    /// offset is adjusted so the remaining pixels stay where they were.
    /// Sizes must be 1..=8192.
    pub fn resize_canvas(
        &mut self,
        f: FrameRef,
        width: u32,
        height: u32,
        anchor: (i32, i32),
    ) -> Result<()> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return invalid(format!(
                "canvas size must be 1..={MAX_SIDE}, got {width}x{height}"
            ));
        }
        let img = self.img(f)?;
        let place = |new: u32, old: u32, a: i32| -> i32 {
            let diff = new as i32 - old as i32;
            match a.signum() {
                -1 => 0,
                0 => diff / 2,
                _ => diff,
            }
        };
        let shift_x = place(width, img.width, anchor.0);
        let shift_y = place(height, img.height, anchor.1);
        let mut out = IndexedImage::new(
            width,
            height,
            img.offset_x.saturating_sub(shift_x),
            img.offset_y.saturating_sub(shift_y),
        );
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let (sx, sy) = (x - shift_x, y - shift_y);
                if sx >= 0 && sy >= 0 && sx < img.width as i32 && sy < img.height as i32 {
                    put_xy(&mut out, x, y, pixel(img, at(img, sx, sy)));
                }
            }
        }
        self.step("resize canvas", |s| s.commit_image(f, out))
    }

    /// Set the frame's offset (position of pixel (0, 0) relative to the anchor).
    pub fn set_offset(&mut self, f: FrameRef, x: i32, y: i32) -> Result<()> {
        let mut img = self.img(f)?.clone();
        img.offset_x = x;
        img.offset_y = y;
        self.step("set offset", |s| s.commit_image(f, img))
    }

    /// Insert a frame at index `at` (0..=count) in every direction. Without
    /// `copy_from` the new frames are transparent with the size and offset of
    /// the frame before (or, at index 0, after) them; with it they are copies
    /// of frame `copy_from`.
    pub fn insert_frame(&mut self, at: usize, copy_from: Option<usize>) -> Result<()> {
        self.require_directions()?;
        let count = self.frames_per_direction();
        if at > count {
            return Err(PaintError::NoFrame(FrameRef { dir: 0, frame: at }));
        }
        if let Some(src) = copy_from
            && src >= count
        {
            return Err(PaintError::NoFrame(FrameRef { dir: 0, frame: src }));
        }
        let new: Vec<IndexedImage> = self
            .frames
            .iter()
            .map(|dir| match copy_from {
                Some(src) => dir[src].clone(),
                None => {
                    let template = at
                        .checked_sub(1)
                        .and_then(|i| dir.get(i))
                        .or_else(|| dir.get(at));
                    match template {
                        Some(t) => IndexedImage::new(t.width, t.height, t.offset_x, t.offset_y),
                        None => IndexedImage::new(1, 1, 0, 0),
                    }
                }
            })
            .collect();
        self.step("insert frame", |s| {
            s.commit_insert(at, new);
            Ok(())
        })
    }

    /// Delete frame `index` from every direction. The last frame cannot be
    /// deleted.
    pub fn delete_frame(&mut self, index: usize) -> Result<()> {
        self.require_directions()?;
        let count = self.frames_per_direction();
        if index >= count {
            return Err(PaintError::NoFrame(FrameRef {
                dir: 0,
                frame: index,
            }));
        }
        if count == 1 {
            return invalid("cannot delete the only frame");
        }
        let removed: Vec<IndexedImage> = self.frames.iter().map(|d| d[index].clone()).collect();
        self.step("delete frame", |s| {
            s.commit_delete(index, removed);
            Ok(())
        })
    }

    /// Move frame `from` so that it ends up at index `to`, in every direction.
    pub fn move_frame(&mut self, from: usize, to: usize) -> Result<()> {
        self.require_directions()?;
        let count = self.frames_per_direction();
        for i in [from, to] {
            if i >= count {
                return Err(PaintError::NoFrame(FrameRef { dir: 0, frame: i }));
            }
        }
        if from == to {
            return Ok(());
        }
        self.step("move frame", |s| {
            s.commit_move(from, to);
            Ok(())
        })
    }

    /// Replace direction `dst` with a horizontally mirrored copy of `src`.
    /// Each frame is flipped left to right and its `offset_x` becomes
    /// `-(offset_x + width)`, i.e. the mirror is taken about the anchor.
    pub fn mirror_direction(&mut self, src: usize, dst: usize) -> Result<()> {
        self.require_free_structure()?;
        for dir in [src, dst] {
            if dir >= self.frames.len() {
                return Err(PaintError::NoFrame(FrameRef { dir, frame: 0 }));
            }
        }
        if src == dst {
            return invalid("source and destination direction are the same");
        }
        let mirrored: Vec<IndexedImage> = self.frames[src]
            .iter()
            .map(|img| {
                let mut out = IndexedImage::new(
                    img.width,
                    img.height,
                    img.offset_x
                        .saturating_add(img.width as i32)
                        .saturating_neg(),
                    img.offset_y,
                );
                let w = img.width as i32;
                for y in 0..img.height as i32 {
                    for x in 0..w {
                        put_xy(&mut out, w - 1 - x, y, pixel(img, at(img, x, y)));
                    }
                }
                out
            })
            .collect();
        self.step("mirror direction", |s| {
            s.commit_direction(dst, mirrored);
            Ok(())
        })
    }
}
