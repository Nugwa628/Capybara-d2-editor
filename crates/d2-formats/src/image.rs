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

    /// RGBA8 pixels (`width * height * 4` bytes, row-major): colour from
    /// `palette`, alpha from [`IndexedImage::alpha`] (palette index 0 is not
    /// treated specially here; the decoders already give it alpha 0).
    pub fn to_rgba(&self, palette: &crate::palette::Palette) -> Vec<u8> {
        let len = (self.width as usize).saturating_mul(self.height as usize);
        // the fields are public: size the hint by the pixels that exist, so a
        // caller-built image with a bogus size cannot trip "capacity overflow"
        let mut out = Vec::with_capacity(len.min(self.pixels.len()).saturating_mul(4));
        for i in 0..len {
            let index = self.pixels.get(i).copied().unwrap_or(0);
            let alpha = self.alpha.get(i).copied().unwrap_or(0);
            let [r, g, b] = palette.colors[usize::from(index)];
            out.extend_from_slice(&[r, g, b, alpha]);
        }
        out
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

    #[test]
    fn to_rgba_uses_palette_and_alpha() {
        let mut colors = [[0u8; 3]; 256];
        colors[1] = [10, 20, 30];
        colors[2] = [40, 50, 60];
        let pal = crate::palette::Palette { colors };
        let mut img = IndexedImage::new(2, 2, 0, 0);
        img.pixels = vec![1, 2, 2, 0];
        img.alpha = vec![255, 255, 0, 128];
        let rgba = img.to_rgba(&pal);
        assert_eq!(rgba.len(), 2 * 2 * 4);
        assert_eq!(&rgba[0..4], &[10, 20, 30, 255]);
        assert_eq!(&rgba[4..8], &[40, 50, 60, 255]);
        assert_eq!(&rgba[8..12], &[40, 50, 60, 0], "alpha honoured");
        assert_eq!(&rgba[12..16], &[0, 0, 0, 128]);
    }
}
