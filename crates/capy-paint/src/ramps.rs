//! Colour ramps: families of palette entries ordered dark to light.
//!
//! Every palette index (except 0, the transparent key) is classified by hue
//! and saturation into a family; each family is sorted by perceived luminance.
//! All arithmetic is integer, so the result is identical on every platform.

use d2_formats::palette::Palette;
use std::collections::BTreeMap;

/// Identifies one ramp of a [`Ramps`] table. Ids are dense, starting at 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RampId(pub u16);

/// Families smaller than this are merged into their nearest neighbour.
const MIN_RAMP_LEN: usize = 3;
/// Saturation (per mille) below which a colour is grey.
const GREY_SAT: i32 = 120;
/// Brightness (0..=255) below which hue is noise and the colour counts as grey.
const GREY_VALUE: i32 = 20;
/// Saturation (per mille) separating the muted from the vivid band.
const VIVID_SAT: i32 = 450;
/// Hue sector width in tenths of a degree (30 degrees, red-centred).
const SECTOR: i32 = 300;

/// Colour families of a palette: indices grouped by hue/saturation, each ramp
/// sorted dark to light by perceived luminance. Index 0 belongs to no ramp.
#[derive(Clone, Debug)]
pub struct Ramps {
    colors: [[u8; 3]; 256],
    ramps: Vec<Vec<u8>>,
    of: [Option<(u16, u16)>; 256],
}

/// Rec. 709 luma scaled by 10000 (exact integer arithmetic).
pub(crate) fn luma(c: [u8; 3]) -> u32 {
    2126 * u32::from(c[0]) + 7152 * u32::from(c[1]) + 722 * u32::from(c[2])
}

/// (hue in tenths of a degree 0..3600, saturation per mille, value 0..=255).
fn hsv(c: [u8; 3]) -> (i32, i32, i32) {
    let [r, g, b] = c.map(i32::from);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;
    if chroma == 0 || max == 0 {
        return (0, 0, max);
    }
    let h = if max == r {
        600 * (g - b) / chroma
    } else if max == g {
        1200 + 600 * (b - r) / chroma
    } else {
        2400 + 600 * (r - g) / chroma
    };
    (h.rem_euclid(3600), 1000 * chroma / max, max)
}

/// Family key: `(0, 0)` is grey, otherwise `(1 + hue sector, saturation band)`.
type Key = (u8, u8);

fn key_of(c: [u8; 3]) -> Key {
    let (h, s, v) = hsv(c);
    if s < GREY_SAT || v < GREY_VALUE {
        return (0, 0);
    }
    // Sector 0 is centred on red (hue 0).
    let sector = ((h + SECTOR / 2) % 3600) / SECTOR;
    let band = u8::from(s >= VIVID_SAT);
    (1 + sector as u8, band)
}

/// Distance between two family keys (used only to merge tiny families).
fn key_distance(a: Key, b: Key) -> i32 {
    if a.0 == 0 || b.0 == 0 {
        // Grey to anything: closer to the muted band.
        let other = if a.0 == 0 { b } else { a };
        return if a.0 == b.0 {
            0
        } else {
            3 + i32::from(other.1)
        };
    }
    let diff = (i32::from(a.0) - i32::from(b.0)).abs();
    let circ = diff.min(12 - diff);
    circ * 2 + (i32::from(a.1) - i32::from(b.1)).abs()
}

impl Ramps {
    pub fn build(palette: &Palette) -> Ramps {
        let colors = palette.colors;
        let mut families: BTreeMap<Key, Vec<u8>> = BTreeMap::new();
        for index in 1..=255u8 {
            families
                .entry(key_of(colors[usize::from(index)]))
                .or_default()
                .push(index);
        }
        // Merge tiny families into their nearest neighbour, smallest first.
        while families.len() >= 2 {
            let Some((&small, _)) = families
                .iter()
                .filter(|(_, m)| m.len() < MIN_RAMP_LEN)
                .min_by_key(|(k, m)| (m.len(), **k))
            else {
                break;
            };
            let Some(&target) = families
                .iter()
                .filter(|(k, _)| **k != small)
                .min_by_key(|(k, m)| (key_distance(small, **k), usize::MAX - m.len(), **k))
                .map(|(k, _)| k)
            else {
                break;
            };
            let moved = families.remove(&small).unwrap_or_default();
            families.entry(target).or_default().extend(moved);
        }
        let mut ramps: Vec<Vec<u8>> = families.into_values().collect();
        let mut of = [None; 256];
        for (id, ramp) in ramps.iter_mut().enumerate() {
            ramp.sort_by_key(|&i| (luma(colors[usize::from(i)]), i));
            for (pos, &i) in ramp.iter().enumerate() {
                of[usize::from(i)] = Some((id as u16, pos as u16));
            }
        }
        Ramps { colors, ramps, of }
    }

    /// The ramp holding `index` and its position (0 = darkest). `None` for 0.
    pub fn ramp_of(&self, index: u8) -> Option<(RampId, usize)> {
        self.of[usize::from(index)].map(|(id, pos)| (RampId(id), usize::from(pos)))
    }

    /// The indices of a ramp, dark to light. Empty for an unknown id.
    pub fn ramp(&self, id: RampId) -> &[u8] {
        self.ramps.get(usize::from(id.0)).map_or(&[], Vec::as_slice)
    }

    pub fn all(&self) -> impl Iterator<Item = (RampId, &[u8])> {
        self.ramps
            .iter()
            .enumerate()
            .map(|(i, r)| (RampId(i as u16), r.as_slice()))
    }

    /// Number of ramps.
    pub fn len(&self) -> usize {
        self.ramps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ramps.is_empty()
    }

    /// Step along the index's own ramp (negative = darker), clamped at the
    /// ends. An index without a ramp (0) is returned unchanged.
    pub fn shift(&self, index: u8, steps: i32) -> u8 {
        let Some((id, pos)) = self.ramp_of(index) else {
            return index;
        };
        let ramp = self.ramp(id);
        let last = ramp.len().saturating_sub(1) as i64;
        let target = (pos as i64 + i64::from(steps)).clamp(0, last);
        ramp.get(target as usize).copied().unwrap_or(index)
    }

    /// Palette colour of an index.
    pub fn color(&self, index: u8) -> [u8; 3] {
        self.colors[usize::from(index)]
    }

    /// Nearest palette index to an RGB colour, optionally restricted to a
    /// set; never 0 unless `allow_zero`. Ties go to the lowest index. If the
    /// restriction leaves no candidate, the whole palette is searched.
    pub fn nearest(&self, rgb: [u8; 3], within: Option<&[u8]>, allow_zero: bool) -> u8 {
        let mut best: Option<(u64, u8)> = None;
        let consider = |best: &mut Option<(u64, u8)>, i: u8| {
            if i == 0 && !allow_zero {
                return;
            }
            let d = distance(rgb, self.colors[usize::from(i)]);
            if best.is_none_or(|(bd, bi)| (d, i) < (bd, bi)) {
                *best = Some((d, i));
            }
        };
        if let Some(set) = within {
            for &i in set {
                consider(&mut best, i);
            }
        }
        if best.is_none() {
            for i in 0..=255u8 {
                consider(&mut best, i);
            }
        }
        best.map_or(1, |(_, i)| i)
    }
}

/// "Redmean" weighted squared distance, integer arithmetic.
pub(crate) fn distance(a: [u8; 3], b: [u8; 3]) -> u64 {
    let rbar = (i64::from(a[0]) + i64::from(b[0])) / 2;
    let dr = i64::from(a[0]) - i64::from(b[0]);
    let dg = i64::from(a[1]) - i64::from(b[1]);
    let db = i64::from(a[2]) - i64::from(b[2]);
    let d = (512 + rbar) * dr * dr + 1024 * dg * dg + (767 - rbar) * db * db;
    d as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reds 1..=8 (dark to light), greens 9..=16, everything else white.
    fn test_palette() -> Palette {
        let mut colors = [[255u8; 3]; 256];
        colors[0] = [0, 0, 0];
        for i in 0..8u8 {
            colors[1 + usize::from(i)] = [60 + 24 * i, 10, 10];
            colors[9 + usize::from(i)] = [10, 60 + 24 * i, 10];
        }
        Palette { colors }
    }

    #[test]
    fn families_are_sorted_and_cover_everything() {
        let ramps = Ramps::build(&test_palette());
        assert_eq!(ramps.ramp_of(0), None);
        let mut seen = [false; 256];
        for (_, ramp) in ramps.all() {
            assert!(ramp.len() >= MIN_RAMP_LEN || ramps.len() == 1);
            for &i in ramp {
                assert!(!seen[usize::from(i)]);
                seen[usize::from(i)] = true;
            }
        }
        assert!(seen[1..].iter().all(|&s| s));
        let (red, _) = ramps.ramp_of(1).unwrap();
        assert_eq!(ramps.ramp(red), &[1, 2, 3, 4, 5, 6, 7, 8]);
        let (green, _) = ramps.ramp_of(9).unwrap();
        assert_ne!(red, green);
        assert_eq!(ramps.ramp(green), &[9, 10, 11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn shift_clamps_at_both_ends() {
        let ramps = Ramps::build(&test_palette());
        assert_eq!(ramps.shift(4, 2), 6);
        assert_eq!(ramps.shift(4, -2), 2);
        assert_eq!(ramps.shift(7, 100), 8);
        assert_eq!(ramps.shift(2, -100), 1);
        assert_eq!(ramps.shift(8, i32::MAX), 8);
        assert_eq!(ramps.shift(1, i32::MIN), 1);
        assert_eq!(ramps.shift(0, 3), 0);
    }

    #[test]
    fn nearest_respects_restriction_and_zero() {
        let ramps = Ramps::build(&test_palette());
        assert_eq!(ramps.nearest([60, 10, 10], None, false), 1);
        assert_eq!(ramps.nearest([0, 0, 0], None, false), 1);
        assert_eq!(ramps.nearest([0, 0, 0], None, true), 0);
        assert_eq!(ramps.nearest([250, 10, 10], Some(&[1, 2, 3]), false), 3);
        assert_eq!(ramps.nearest([250, 10, 10], None, false), 8);
        // Empty / zero-only restriction falls back to the whole palette.
        assert_eq!(ramps.nearest([60, 10, 10], Some(&[]), false), 1);
        assert_eq!(ramps.nearest([60, 10, 10], Some(&[0]), false), 1);
    }

    #[test]
    fn build_is_deterministic() {
        let a = Ramps::build(&test_palette());
        let b = Ramps::build(&test_palette());
        let av: Vec<_> = a.all().map(|(i, r)| (i, r.to_vec())).collect();
        let bv: Vec<_> = b.all().map(|(i, r)| (i, r.to_vec())).collect();
        assert_eq!(av, bv);
    }

    /// Prints the Act 1 ramp table when the private corpus palette exists.
    #[test]
    fn act1_ramp_table() {
        let rel = "testdata/private/extracted/vanilla/data/global/palette/act1/pal.dat";
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // The corpus lives in the main checkout, not in agent worktrees.
        let candidates = [
            manifest.join("../..").join(rel),
            std::path::PathBuf::from(
                r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor\testdata\private\extracted\vanilla\data\global\palette\act1\pal.dat",
            ),
        ];
        let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
            eprintln!("act1 pal.dat not present, skipping");
            return;
        };
        let palette = Palette::parse_dat(&bytes).unwrap();
        let ramps = Ramps::build(&palette);
        println!("Act 1: {} ramps", ramps.len());
        for (id, ramp) in ramps.all() {
            let lo = palette.colors[usize::from(ramp[0])];
            let hi = palette.colors[usize::from(ramp[ramp.len() - 1])];
            println!(
                "ramp {:2}: {:3} colours, dark {:?} -> light {:?}  {:?}",
                id.0,
                ramp.len(),
                lo,
                hi,
                ramp
            );
        }
        let total: usize = ramps.all().map(|(_, r)| r.len()).sum();
        assert_eq!(total, 255);
        for (_, ramp) in ramps.all() {
            let lumas: Vec<u32> = ramp
                .iter()
                .map(|&i| luma(palette.colors[usize::from(i)]))
                .collect();
            assert!(lumas.windows(2).all(|w| w[0] <= w[1]));
        }
    }
}
