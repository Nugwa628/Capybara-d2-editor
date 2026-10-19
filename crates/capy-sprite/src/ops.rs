//! Frame and sprite editing operations.

use d2_formats::IndexedImage;

use crate::error::{Result, SpriteError};
use crate::model::Sprite;

fn opaque_at(img: &IndexedImage, i: usize) -> bool {
    img.alpha.get(i).copied().unwrap_or(0) != 0 && img.pixels.get(i).copied().unwrap_or(0) != 0
}

/// Copies the `w x h` block at `(x0, y0)` of `img` into a new image whose
/// offset is shifted so that every pixel keeps its on-screen position.
fn crop(img: &IndexedImage, x0: u32, y0: u32, w: u32, h: u32) -> IndexedImage {
    let mut out = IndexedImage::new(
        w,
        h,
        img.offset_x.saturating_add_unsigned(x0),
        img.offset_y.saturating_add_unsigned(y0),
    );
    let stride = img.width as usize;
    for y in 0..h as usize {
        let src = (y0 as usize + y) * stride + x0 as usize;
        let dst = y * w as usize;
        let n = w as usize;
        if let (Some(p), Some(a), Some(op), Some(oa)) = (
            img.pixels.get(src..src + n),
            img.alpha.get(src..src + n),
            out.pixels.get_mut(dst..dst + n),
            out.alpha.get_mut(dst..dst + n),
        ) {
            op.copy_from_slice(p);
            oa.copy_from_slice(a);
        }
    }
    out
}

/// Cuts an image into `tile_w x tile_h` tiles, row-major. Tiles on the right
/// and bottom edges are smaller when the size is not a multiple. Each tile's
/// offset is the image offset plus the tile's position, so every pixel keeps
/// its place relative to the anchor. A zero tile size or an empty image
/// returns the image unchanged.
pub fn split_frame(img: &IndexedImage, tile_w: u32, tile_h: u32) -> Vec<IndexedImage> {
    if tile_w == 0 || tile_h == 0 || img.width == 0 || img.height == 0 {
        return vec![img.clone()];
    }
    let mut out = Vec::new();
    let mut y = 0;
    while y < img.height {
        let h = tile_h.min(img.height - y);
        let mut x = 0;
        while x < img.width {
            let w = tile_w.min(img.width - x);
            out.push(crop(img, x, y, w, h));
            x += tile_w;
        }
        y += tile_h;
    }
    out
}

/// Joins tiles laid out row-major in `columns` columns back into one image
/// (the inverse of [`split_frame`]). Column widths come from the widest tile
/// of each column and row heights from the tallest tile of each row; tiles
/// smaller than their cell are padded with transparency at the right and
/// bottom. The result's offset is the first tile's.
pub fn join_frames(frames: &[IndexedImage], columns: u32) -> Result<IndexedImage> {
    let first = frames
        .first()
        .ok_or_else(|| SpriteError::invalid("nothing to join"))?;
    if columns == 0 {
        return Err(SpriteError::invalid("a join needs at least one column"));
    }
    let columns = columns as usize;
    let rows = frames.len().div_ceil(columns);
    let mut col_w = vec![0u32; columns];
    let mut row_h = vec![0u32; rows];
    for (i, f) in frames.iter().enumerate() {
        if let Some(w) = col_w.get_mut(i % columns) {
            *w = (*w).max(f.width);
        }
        if let Some(h) = row_h.get_mut(i / columns) {
            *h = (*h).max(f.height);
        }
    }
    let total_w: u64 = col_w.iter().map(|&w| u64::from(w)).sum();
    let total_h: u64 = row_h.iter().map(|&h| u64::from(h)).sum();
    if total_w > u64::from(u32::MAX >> 4) || total_h > u64::from(u32::MAX >> 4) {
        return Err(SpriteError::invalid("joined image would be too large"));
    }
    if total_w.saturating_mul(total_h) > 1 << 28 {
        return Err(SpriteError::invalid("joined image would be too large"));
    }
    let mut out = IndexedImage::new(
        total_w as u32,
        total_h as u32,
        first.offset_x,
        first.offset_y,
    );
    let width = total_w as usize;
    let mut y0 = 0usize;
    for (r, &rh) in row_h.iter().enumerate() {
        let mut x0 = 0usize;
        for (c, &cw) in col_w.iter().enumerate() {
            if let Some(f) = frames.get(r * columns + c) {
                for y in 0..f.height as usize {
                    let n = f.width as usize;
                    let src = y * n;
                    let dst = (y0 + y) * width + x0;
                    if let (Some(p), Some(a), Some(op), Some(oa)) = (
                        f.pixels.get(src..src + n),
                        f.alpha.get(src..src + n),
                        out.pixels.get_mut(dst..dst + n),
                        out.alpha.get_mut(dst..dst + n),
                    ) {
                        op.copy_from_slice(p);
                        oa.copy_from_slice(a);
                    }
                }
            }
            x0 += cw as usize;
        }
        y0 += rh as usize;
    }
    Ok(out)
}

/// Crops transparent borders (alpha 0 or palette index 0) and moves the
/// offset so the picture does not move on screen. An image with no visible
/// pixel becomes one transparent pixel at the same offset.
pub fn trim_borders(img: &IndexedImage) -> IndexedImage {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if opaque_at(img, y * w + x) {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
    }
    match bounds {
        None => IndexedImage::new(1, 1, img.offset_x, img.offset_y),
        Some((x0, y0, x1, y1)) => crop(
            img,
            x0 as u32,
            y0 as u32,
            (x1 - x0 + 1) as u32,
            (y1 - y0 + 1) as u32,
        ),
    }
}

/// Puts the anchor on the bottom-centre pixel: `offset_x = -(width / 2)`,
/// `offset_y = -(height - 1)`.
pub fn center_offset(img: &mut IndexedImage) {
    img.offset_x = -((img.width / 2) as i32);
    img.offset_y = -(img.height.saturating_sub(1) as i32);
}

/// Moves every frame by `(dx, dy)`.
pub fn offset_all(sprite: &mut Sprite, dx: i32, dy: i32) {
    for img in sprite.directions.iter_mut().flatten() {
        img.offset_x = img.offset_x.saturating_add(dx);
        img.offset_y = img.offset_y.saturating_add(dy);
    }
}

/// Appends a direction. It must have as many frames as the others (any
/// number is fine for a sprite with no directions yet).
pub fn add_direction(sprite: &mut Sprite, frames: Vec<IndexedImage>) -> Result<()> {
    if !sprite.directions.is_empty() && frames.len() != sprite.frames_per_direction() {
        return Err(SpriteError::invalid(format!(
            "the new direction has {} frames, the sprite has {} per direction",
            frames.len(),
            sprite.frames_per_direction()
        )));
    }
    sprite.directions.push(frames);
    Ok(())
}

/// Removes the directions with the given indices (out-of-range and repeated
/// indices are ignored).
pub fn remove_directions(sprite: &mut Sprite, dirs: &[usize]) {
    let mut i = 0usize;
    sprite.directions.retain(|_| {
        let keep = !dirs.contains(&i);
        i += 1;
        keep
    });
}

/// Removes the frames with the given indices from every direction
/// (out-of-range and repeated indices are ignored).
pub fn remove_frames(sprite: &mut Sprite, frames: &[usize]) {
    for dir in &mut sprite.directions {
        let mut i = 0usize;
        dir.retain(|_| {
            let keep = !frames.contains(&i);
            i += 1;
            keep
        });
    }
}
