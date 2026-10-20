//! Make act-safe (ds1-studio `act0Palette.ts`, `ActSafeDialog.tsx`): remap the
//! pixels of tiles so they use only colours that look the same in every act
//! (the "Act 0" idea). Builds on `capy_paint::remap::plan` for the nearest
//! shared colour and on [`crate::tiles_dt1::recolor_tiles`] to patch pixels.

use capy_paint::remap::{self, ColorDistance, RemapEntry, RemapSettings, TargetSet};
use d2_formats::dt1::Dt1;
use d2_formats::palette::Palette;

use crate::tiles_dt1::{Dt1ManagerError, TileChange, recolor_tiles};
use crate::tiles_pixels::histogram;

/// Palette slots that are act-specific in the five act palettes (225..=254).
pub use capy_paint::remap::ACT_SPECIFIC;

/// Slots that hold the same colour in every given palette (index 0, the
/// transparent key, is never usable). Equivalent to Gimli's act0 palette
/// when given the five game act palettes. Empty input marks nothing usable.
pub fn shared_act_colors(palettes: &[Palette]) -> [bool; 256] {
    let mut usable = [false; 256];
    let Some(first) = palettes.first() else {
        return usable;
    };
    for (i, u) in usable.iter_mut().enumerate().skip(1) {
        *u = palettes.iter().all(|p| p.colors[i] == first.colors[i]);
    }
    usable
}

/// Which colours count as act-safe.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[allow(clippy::large_enum_variant)]
pub enum Safe {
    /// Everything outside 225..=254 (the P7 `SharedActColors` set).
    #[default]
    OutsideActSlots,
    /// Exactly these slots (see [`shared_act_colors`]).
    Only([bool; 256]),
}

impl Safe {
    pub fn is_safe(&self, index: u8) -> bool {
        match self {
            Safe::OutsideActSlots => !ACT_SPECIFIC.contains(&index),
            Safe::Only(m) => m[usize::from(index)],
        }
    }

    fn target_set(&self) -> TargetSet {
        match self {
            Safe::OutsideActSlots => TargetSet::SharedActColors,
            Safe::Only(m) => TargetSet::Only((1..=255u8).filter(|&i| m[usize::from(i)]).collect()),
        }
    }
}

/// Options for [`make_act_safe`].
#[derive(Clone, Debug, PartialEq)]
pub struct ActSafeOptions {
    pub safe: Safe,
    pub distance: ColorDistance,
}

impl Default for ActSafeOptions {
    fn default() -> Self {
        ActSafeOptions {
            safe: Safe::default(),
            // ds1-studio's community Act 0 conversions use plain RGB distance.
            distance: ColorDistance::Rgb,
        }
    }
}

/// The outcome of [`make_act_safe`].
#[derive(Clone, Debug, PartialEq)]
pub struct ActSafeReport {
    /// The index table that was applied.
    pub table: [u8; 256],
    /// Pixels that were not act-safe before.
    pub unsafe_pixels: u64,
    /// Pixels rewritten.
    pub pixels_changed: u64,
    pub tiles: Vec<TileChange>,
    /// Per source colour used, worst error first.
    pub entries: Vec<RemapEntry>,
    pub mean_error: f32,
    pub worst_error: f32,
}

fn selected<'a>(dt1: &'a Dt1, tiles: Option<&'a [usize]>) -> impl Iterator<Item = usize> + 'a {
    (0..dt1.tiles.len()).filter(move |i| tiles.is_none_or(|t| t.contains(i)))
}

/// Pixel counts per palette index of the chosen tiles (index 0 literals included).
pub fn tile_histogram(dt1: &Dt1, tiles: Option<&[usize]>) -> [u64; 256] {
    let mut counts = [0u64; 256];
    for i in selected(dt1, tiles) {
        if let Some(t) = dt1.tiles.get(i) {
            t.blocks.iter().for_each(|b| histogram(b, &mut counts));
        }
    }
    counts
}

/// How many pixels of the chosen tiles use a colour that is not act-safe.
pub fn count_unsafe_pixels(dt1: &Dt1, tiles: Option<&[usize]>, safe: &Safe) -> u64 {
    tile_histogram(dt1, tiles)
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(i, _)| !safe.is_safe(*i as u8))
        .map(|(_, n)| *n)
        .sum()
}

/// Plans the remap without changing anything (for the preview).
pub fn plan_act_safe(
    dt1: &Dt1,
    home: &Palette,
    tiles: Option<&[usize]>,
    opts: &ActSafeOptions,
) -> remap::RemapPlan {
    let settings = RemapSettings {
        distance: opts.distance,
        targets: opts.safe.target_set(),
        keep_exact: true,
        ..RemapSettings::default()
    };
    remap::plan(home, home, &tile_histogram(dt1, tiles), &settings)
}

/// Remaps the chosen tiles (`None` = all) so they use only act-safe colours,
/// judged by `home`, the palette of the act the tiles were drawn for.
/// Already-safe pixels are untouched; only pixel literals change. A DT1
/// that is already safe yields a report with no tile changes.
pub fn make_act_safe(
    dt1: &mut Dt1,
    home: &Palette,
    tiles: Option<&[usize]>,
    opts: &ActSafeOptions,
) -> Result<ActSafeReport, Dt1ManagerError> {
    let unsafe_pixels = count_unsafe_pixels(dt1, tiles, &opts.safe);
    let plan = plan_act_safe(dt1, home, tiles, opts);
    let mut table = plan.table;
    // Never touch a colour that is already safe, whatever the planner chose.
    for i in 1..=255u8 {
        if opts.safe.is_safe(i) {
            table[usize::from(i)] = i;
        }
    }
    let changes = if unsafe_pixels == 0 {
        Vec::new()
    } else {
        recolor_tiles(dt1, tiles, &table)?
    };
    Ok(ActSafeReport {
        table,
        unsafe_pixels,
        pixels_changed: changes.iter().map(|c| c.pixels).sum(),
        tiles: changes,
        entries: plan.entries,
        mean_error: plan.mean_error,
        worst_error: plan.worst_error,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn shared_colours() {
        let mut a = Palette {
            colors: [[1, 2, 3]; 256],
        };
        let mut b = a.clone();
        b.colors[10] = [9, 9, 9];
        a.colors[0] = [0, 0, 0];
        let m = shared_act_colors(&[a, b]);
        assert!(!m[0]);
        assert!(m[9]);
        assert!(!m[10]);
        assert!(shared_act_colors(&[]).iter().all(|&v| !v));
    }
}
