//! Palette-indexed images shared by the sprite and tile decoders.

/// Palette-indexed image. `alpha[i] == 0` is transparent, 255 opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    /// Position of pixel (0,0) relative to the tile/sprite anchor.
    pub offset_x: i32,
    pub offset_y: i32,
    pub pixels: Vec<u8>,
    pub alpha: Vec<u8>,
}

impl IndexedImage {
    /// A fully transparent image of the given size.
    pub fn new(width: u32, height: u32, offset_x: i32, offset_y: i32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            offset_x,
            offset_y,
            pixels: vec![0; len],
            alpha: vec![0; len],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_transparent() {
        let img = IndexedImage::new(4, 3, -2, 5);
        assert_eq!(img.pixels.len(), 12);
        assert!(img.alpha.iter().all(|&a| a == 0));
        assert_eq!((img.offset_x, img.offset_y), (-2, 5));
    }
}
