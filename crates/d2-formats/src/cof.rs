//! COF animation descriptors (`data\global\{monsters,chars,objects}\*\cof\*.cof`).
//!
//! ```text
//! 0   u8 layers (L)   1 u8 frames per direction (F)   2 u8 directions (D)
//! 3..28  25 bytes of header the editors never decode (version 20 in most files)
//! 28  L x 9-byte layer records:
//!       component (HD TR LG RA LA RH LH SH S1..S8 = 0..15), shadow,
//!       selectable, transparent, draw effect, weapon class (4 bytes)
//! then F key-frame bytes
//! then D x F x L priority bytes
//! ```
//!
//! The priority table gives, for every (direction, frame), the order in which
//! the layers are drawn, first to last, as **component indices** (the same
//! values as `CofLayer::component`), so for a well-formed file every row is a
//! permutation of the layers' components. It is indexed by the **file
//! direction**, the same index as the sprite frames (see PLAN.md, "Format
//! decisions"). Duplicate component ids exist in a few boss COFs; every
//! record is kept in order, so the caller decides which duplicate wins.
//!
//! # Leniency
//!
//! Trailing bytes are ignored. A file whose layer records are complete but
//! whose key-frame bytes or priority table run short (three Median XL files
//! do, by 1 to 150 bytes) is accepted: the missing bytes read as zero and
//! [`Cof::missing_bytes`] says how many there were (at most 4096 are
//! tolerated; beyond that, or when even the layer records are cut off, the
//! file is [`FormatError::Truncated`]). The shipped `amblxbow.cof` is not a
//! COF at all (72 bytes declaring 230 layers) and fails that way.

use crate::bytes::slice_at;
use crate::{FormatError, Result};

const FORMAT: &str = "cof";
const HEADER_LEN: usize = 28;
const LAYER_LEN: usize = 9;
/// Zero-filled bytes tolerated past the end of a short file.
const MAX_MISSING: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CofLayer {
    /// Component id 0..=15 (HD TR LG RA LA RH LH SH S1..S8).
    pub component: u8,
    /// Non-zero when the layer casts a projected shadow.
    pub shadow: u8,
    pub selectable: u8,
    /// Non-zero enables the blend named by `draw_effect`.
    pub transparent: u8,
    /// Blend mode index (see `docs/specs/cof.md` section 3).
    pub draw_effect: u8,
    /// Weapon class as stored, usually 3 letters NUL- or space-padded
    /// (`HTH\0`, `1HS\0`).
    pub weapon_class: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cof {
    pub directions: u8,
    pub frames_per_direction: u8,
    pub layers: Vec<CofLayer>,
    /// One flag byte per frame.
    pub keyframes: Vec<u8>,
    /// `[direction][frame][slot]` flattened, `directions * frames_per_direction
    /// * layers.len()` bytes; see [`Cof::draw_order`].
    pub layer_order: Vec<u8>,
    /// Bytes the key-frame and priority tables needed past the end of the
    /// file, read as zero (0 for every well-formed file).
    pub missing_bytes: usize,
}

impl Cof {
    pub fn parse(bytes: &[u8]) -> Result<Cof> {
        let truncated = |offset: usize, needed: usize| FormatError::Truncated {
            format: FORMAT,
            offset,
            needed,
        };
        let header = slice_at(bytes, 0, HEADER_LEN)
            .ok_or_else(|| truncated(bytes.len(), HEADER_LEN - bytes.len()))?;
        let layer_count = usize::from(header.first().copied().unwrap_or(0));
        let frames = usize::from(header.get(1).copied().unwrap_or(0));
        let directions = usize::from(header.get(2).copied().unwrap_or(0));
        let records_end = HEADER_LEN + layer_count * LAYER_LEN;
        let records = slice_at(bytes, HEADER_LEN, layer_count * LAYER_LEN)
            .ok_or_else(|| truncated(bytes.len(), records_end - bytes.len()))?;
        let layers: Vec<CofLayer> = records
            .as_chunks::<LAYER_LEN>()
            .0
            .iter()
            .map(|r| CofLayer {
                component: r.first().copied().unwrap_or(0),
                shadow: r.get(1).copied().unwrap_or(0),
                selectable: r.get(2).copied().unwrap_or(0),
                transparent: r.get(3).copied().unwrap_or(0),
                draw_effect: r.get(4).copied().unwrap_or(0),
                weapon_class: [
                    r.get(5).copied().unwrap_or(0),
                    r.get(6).copied().unwrap_or(0),
                    r.get(7).copied().unwrap_or(0),
                    r.get(8).copied().unwrap_or(0),
                ],
            })
            .collect();

        let order_len = directions * frames * layer_count;
        let want = frames + order_len;
        let have = bytes.len().saturating_sub(records_end).min(want);
        let missing = want - have;
        if missing > MAX_MISSING {
            return Err(truncated(bytes.len(), missing));
        }
        let tail = bytes.get(records_end..records_end + have).unwrap_or(&[]);
        let mut padded = tail.to_vec();
        padded.resize(want, 0);
        let layer_order = padded.split_off(frames);
        Ok(Cof {
            directions: directions as u8,
            frames_per_direction: frames as u8,
            layers,
            keyframes: padded,
            layer_order,
            missing_bytes: missing,
        })
    }

    /// The layers (as component indices) of `(dir, frame)` in drawing order,
    /// first drawn first. `dir` is the file direction. `None` when either
    /// index is out of range.
    pub fn draw_order(&self, dir: u8, frame: u8) -> Option<&[u8]> {
        if dir >= self.directions || frame >= self.frames_per_direction {
            return None;
        }
        let row = usize::from(dir) * usize::from(self.frames_per_direction) + usize::from(frame);
        let width = self.layers.len();
        self.layer_order.get(row * width..(row + 1) * width)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
