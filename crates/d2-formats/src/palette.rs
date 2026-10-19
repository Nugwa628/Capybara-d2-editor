//! Palettes: `pal.dat` (768 bytes) and `pal.pl2` (the transform buffer).
//!
//! # Byte order (settled empirically, 2026-10-19)
//!
//! * **`pal.dat` stores BGR**: byte 0 of each triplet is blue.
//! * **The base palette at the front of `pal.pl2` is RGB + pad**: byte 0 of
//!   each 4-byte entry is red. It is `pal.dat` with each triplet reversed.
//!
//! Evidence, over every `pal.dat`/`pal.pl2` pair in the vanilla and Median XL
//! corpora (17 pairs per profile): `pl2[4i..4i+3] == reverse(dat[3i..3i+3])`
//! and `pl2[4i+3] == 0` for all 256 entries, with no exception. Which of the
//! two is RGB is decided by colour statistics of the Act 1 palette (earth and
//! foliage): reading `pal.dat` as BGR gives 156 warm entries (R > B + 10) and
//! 40 cool ones; reading it as RGB gives 40 warm and 156 cool, which cannot
//! be the Act 1 palette. Index 150 of every act palette is then
//! `RGB(84, 120, 188)`, a blue-grey, not the tan the reference guide assumes
//! (the guide's "blue, green, red" description of the PL2 copy is inverted:
//! it presumed `pal.dat` was RGB). The reference editor reads the PL2 copy as
//! R, G, B (`palette.c:27-29`), which this confirms. Index 0 is black and
//! index 255 is white in every act palette under either reading.
//!
//! # `pal.pl2` layout (443,175 bytes; 442,916 for `loading`)
//!
//! | Offset | Size | Table |
//! |---|---|---|
//! | `0x00000` | 1024 | base palette, 256 x (R, G, B, 0) |
//! | `0x00400` | 32 x 256 | `light_levels`: darkness, one 256-byte remap per level |
//! | `0x02400` | 16 x 256 | `highlight` |
//! | `0x03400` | 256 | `saturation` |
//! | `0x03500` | 256 x 256 | `alpha_blend[0]`: source weighs 25% |
//! | `0x13500` | 256 x 256 | `alpha_blend[1]`: 50% |
//! | `0x23500` | 256 x 256 | `alpha_blend[2]`: source weighs 75% |
//! | `0x33500` | 256 x 256 | `additive` |
//! | `0x43500` | 256 x 256 | `multiply` |
//! | `0x53500` | 128 x 256 | `hue_shift` (hue/lightness/saturation family, rows not individually named) |
//! | `0x5B500` | 256 x 256 | `screen` |
//! | `0x6B500` | 256 | `screen_ramp` |
//! | `0x6B600` | 295 | `tint_colors` (raw small data block) |
//! | `0x6B727` | 12 x 256 | `tints` |
//!
//! Each 256x256 table is indexed `table[source][destination]` (flat offset
//! `source * 256 + destination`). The alpha tables were checked against the
//! base palette: `alpha_blend[0][s][d]` is the palette entry nearest to
//! `0.25 * colour(s) + 0.75 * colour(d)` (277 of 300 random pairs agree
//! exactly, the rest are nearest-colour ties), `[1]` is 50/50 and `[2]` is
//! 75% source; additive and multiply agree on 300 of 300 pairs. These rows
//! are stored raw: the reference overwrites row 0 of the six blend tables
//! with the identity at load (`misc.c:1301-1328`), which callers can do
//! themselves. Files shorter than the full buffer are accepted as long as
//! they hold everything through `multiply`; later tables then keep only the
//! complete rows that exist.

use crate::bytes::slice_at;
use crate::{FormatError, Result};

const FORMAT_DAT: &str = "pal.dat";
const FORMAT_PL2: &str = "pal.pl2";

const OFF_LIGHT: usize = 0x400;
const OFF_HIGHLIGHT: usize = 0x2400;
const OFF_SATURATION: usize = 0x3400;
const OFF_ALPHA: [usize; 3] = [0x3500, 0x13500, 0x23500];
const OFF_ADDITIVE: usize = 0x33500;
const OFF_MULTIPLY: usize = 0x43500;
const OFF_HUE: usize = 0x53500;
const OFF_SCREEN: usize = 0x5B500;
const OFF_SCREEN_RAMP: usize = 0x6B500;
const OFF_TINT_COLORS: usize = 0x6B600;
const OFF_TINTS: usize = 0x6B727;
/// Smallest accepted `pal.pl2`: everything through the multiply table.
const MIN_PL2_LEN: usize = OFF_HUE;

/// 256 RGB colours. Index 0 is the transparent key by convention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub colors: [[u8; 3]; 256],
}

impl Palette {
    /// Parses a `pal.dat`: 256 triplets stored **blue, green, red**. Extra
    /// bytes after the first 768 are ignored; fewer is an error.
    pub fn parse_dat(bytes: &[u8]) -> Result<Palette> {
        let raw = slice_at(bytes, 0, 768).ok_or(FormatError::Truncated {
            format: FORMAT_DAT,
            offset: bytes.len(),
            needed: 768 - bytes.len().min(768),
        })?;
        let mut colors = [[0u8; 3]; 256];
        for (color, &[b, g, r]) in colors.iter_mut().zip(raw.as_chunks::<3>().0) {
            *color = [r, g, b];
        }
        Ok(Palette { colors })
    }

    /// Gamma-corrects every channel: `255 * (c / 255)^(1 / gamma)`, rounded
    /// half up (`(int)(v + 0.5)`), as `misc.c:447-467` does. `gamma` is
    /// first rounded to six decimals so `1.30_f32` means exactly 1.3. A
    /// non-finite or non-positive gamma returns the palette unchanged.
    pub fn with_gamma(&self, gamma: f32) -> Palette {
        let g = (f64::from(gamma) * 1e6).round() / 1e6;
        if !g.is_finite() || g <= 0.0 {
            return self.clone();
        }
        let inv = 1.0 / g;
        let mut table = [0u8; 256];
        for (i, slot) in table.iter_mut().enumerate().skip(1) {
            let v = 255.0 * (i as f64 / 255.0).powf(inv);
            *slot = (v + 0.5).clamp(0.0, 255.0) as u8;
        }
        let mut out = self.clone();
        for color in &mut out.colors {
            for channel in color.iter_mut() {
                *channel = table[usize::from(*channel)];
            }
        }
        out
    }
}

/// `"data\\global\\palette\\act{n}\\pal.dat"` for `act` 0..=4 (0-based: Act 1
/// is 0, the folder is `act1`). Larger values clamp to Act 5.
pub fn act_palette_path(act: u8) -> &'static str {
    match act {
        0 => "data\\global\\palette\\act1\\pal.dat",
        1 => "data\\global\\palette\\act2\\pal.dat",
        2 => "data\\global\\palette\\act3\\pal.dat",
        3 => "data\\global\\palette\\act4\\pal.dat",
        _ => "data\\global\\palette\\act5\\pal.dat",
    }
}

/// The transform buffer of a `pal.pl2`. See the module docs for the layout
/// and the byte order of the base palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pl2 {
    pub base: Palette,
    /// 32 darkness remaps (`light_levels[level][index]`).
    pub light_levels: Vec<[u8; 256]>,
    /// 16 brighten-toward-white remaps.
    pub highlight: Vec<[u8; 256]>,
    /// One saturation-boosted remap (256 bytes; empty if the file lacks it).
    pub saturation: Vec<u8>,
    /// `[25%, 50%, 75%][source][destination]`: the palette index of the
    /// source blended over the destination at that source opacity.
    pub alpha_blend: Vec<Vec<[u8; 256]>>,
    /// `additive[source][destination]`.
    pub additive: Vec<[u8; 256]>,
    /// `multiply[source][destination]`.
    pub multiply: Vec<[u8; 256]>,
    /// 128 rows of the hue/lightness/saturation family (not individually named).
    pub hue_shift: Vec<[u8; 256]>,
    /// `screen[source][destination]`.
    pub screen: Vec<[u8; 256]>,
    /// 256 bytes (empty if the file lacks it).
    pub screen_ramp: Vec<u8>,
    /// The raw 295-byte block before the tints (empty if absent).
    pub tint_colors: Vec<u8>,
    /// 12 colour tints (fewer if the file is short).
    pub tints: Vec<[u8; 256]>,
}

/// Up to `rows` complete 256-byte rows starting at `off`.
fn rows(bytes: &[u8], off: usize, rows: usize) -> Vec<[u8; 256]> {
    let mut out = Vec::new();
    for i in 0..rows {
        let Some(row) = slice_at(bytes, off + i * 256, 256) else {
            break;
        };
        let mut r = [0u8; 256];
        r.copy_from_slice(row);
        out.push(r);
    }
    out
}

impl Pl2 {
    /// Parses a `pal.pl2`. The file must hold at least the base palette and
    /// every table through `multiply` (341,248 bytes); anything shorter is
    /// [`FormatError::Truncated`].
    pub fn parse(bytes: &[u8]) -> Result<Pl2> {
        if bytes.len() < MIN_PL2_LEN {
            return Err(FormatError::Truncated {
                format: FORMAT_PL2,
                offset: bytes.len(),
                needed: MIN_PL2_LEN - bytes.len(),
            });
        }
        let mut colors = [[0u8; 3]; 256];
        for (i, color) in colors.iter_mut().enumerate() {
            if let Some(&[r, g, b, _]) = slice_at(bytes, i * 4, 4) {
                *color = [r, g, b];
            }
        }
        let tail = |off: usize, len: usize| slice_at(bytes, off, len).unwrap_or(&[]).to_vec();
        Ok(Pl2 {
            base: Palette { colors },
            light_levels: rows(bytes, OFF_LIGHT, 32),
            highlight: rows(bytes, OFF_HIGHLIGHT, 16),
            saturation: tail(OFF_SATURATION, 256),
            alpha_blend: OFF_ALPHA.iter().map(|&o| rows(bytes, o, 256)).collect(),
            additive: rows(bytes, OFF_ADDITIVE, 256),
            multiply: rows(bytes, OFF_MULTIPLY, 256),
            hue_shift: rows(bytes, OFF_HUE, 128),
            screen: rows(bytes, OFF_SCREEN, 256),
            screen_ramp: tail(OFF_SCREEN_RAMP, 256),
            tint_colors: tail(OFF_TINT_COLORS, OFF_TINTS - OFF_TINT_COLORS),
            tints: rows(bytes, OFF_TINTS, 12),
        })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
