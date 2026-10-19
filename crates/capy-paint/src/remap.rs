//! Palette conversion: plans how the colours of one palette are carried into
//! another (P7). Nothing here is automatic; the app shows the [`RemapPlan`],
//! lets the user adjust the [`RemapSettings`], and applies the table only when
//! asked.
//!
//! # How a source index is mapped
//!
//! For every source index `s` in `1..=255` (used or not, so the table is
//! total), in this order:
//!
//! 1. **Override**: `settings.overrides[s] = t` with `t != 0` wins over
//!    everything, including the allowed target set.
//! 2. **Kept**: with `keep_exact`, `s` maps to itself when `dst[s] == src[s]`
//!    (the raw colours, before any [`Adjust`]) and `s` is an allowed target.
//! 3. **Nearest**: the allowed target closest to the (adjusted) source
//!    colour under `settings.distance`; ties go to the lowest index.
//!
//! Index 0 always maps to 0 and is never a target for anything else, so the
//! table is a valid `d2_formats::remap::IndexTable`.
//!
//! # Colour maths
//!
//! sRGB bytes are decoded to linear light and converted to OkLab
//! (Björn Ottosson's matrices). Both palettes are converted once per call.
//! [`Adjust`] works in OkLab on the source colour: contrast scales L around
//! 0.5, then brightness is added to L (L is clamped to `0..=1`); hue rotates
//! the a/b plane, saturation scales chroma. A colour that leaves the sRGB
//! gamut keeps its L and hue and loses chroma until it fits (binary search).
//! [`RemapEntry::error`] is always the plain OkLab distance x 100 between the
//! *unadjusted* source colour and the chosen target, whatever the distance
//! metric, weight and adjustments, so reports stay comparable.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use crate::color::{LinearTable, linear_to_srgb, nth_root};

/// Indices whose colour differs between the act palettes (and units/sky).
/// Every other index is identical in act1-act5, units and sky.
pub const ACT_SPECIFIC: RangeInclusive<u8> = 225..=254;

/// How "closest colour" is measured when matching.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ColorDistance {
    /// Euclidean distance in OkLab, L scaled by `lightness_weight`.
    #[default]
    OkLab,
    /// The "redmean" weighted sRGB distance (cheap perceptual approximation).
    Redmean,
    /// Plain Euclidean distance on sRGB bytes.
    Rgb,
}

/// Which target indices the matcher may choose (overrides are exempt).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum TargetSet {
    /// Any target index except 0.
    #[default]
    All,
    /// Skip ACT_SPECIFIC: the result looks the same under every act palette.
    SharedActColors,
    /// Only these target indices (0 is ignored). When no usable index is
    /// left (empty, or only 0) the matcher falls back to [`TargetSet::All`]
    /// so the plan is still a valid table; [`TargetSet::effective`] shows
    /// the set actually used.
    Only(Vec<u8>),
}

impl TargetSet {
    /// The target indices actually used, ascending and without duplicates.
    /// Never empty and never contains 0 (`Only` without a usable index falls
    /// back to `1..=255`).
    pub fn effective(&self) -> Vec<u8> {
        let all = || (1..=255u8).collect::<Vec<u8>>();
        match self {
            TargetSet::All => all(),
            TargetSet::SharedActColors => {
                (1..=255u8).filter(|i| !ACT_SPECIFIC.contains(i)).collect()
            }
            TargetSet::Only(list) => {
                let mut mask = [false; 256];
                for &i in list {
                    mask[usize::from(i)] = true;
                }
                let set: Vec<u8> = (1..=255u8).filter(|&i| mask[usize::from(i)]).collect();
                if set.is_empty() { all() } else { set }
            }
        }
    }
}

/// Applied to the source colours before matching; all zero = unchanged.
/// Values outside the documented ranges are clamped; non-finite values count
/// as 0.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Adjust {
    /// -1..=1, added to OkLab L.
    pub brightness: f32,
    /// -1..=1, scales L around 0.5 by `1 + contrast`.
    pub contrast: f32,
    /// -1..=1, scales OkLab chroma by `1 + saturation` (-1 = grey).
    pub saturation: f32,
    /// Rotates the OkLab hue (a/b angle), any value.
    pub hue_degrees: f32,
}

/// Everything the user can tune about a conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct RemapSettings {
    pub distance: ColorDistance,
    pub targets: TargetSet,
    /// Keep index i when the target palette has the same colour at i and i
    /// is allowed (default true; act1<->act5 then only touches 30 indices).
    pub keep_exact: bool,
    /// Values above 1 favour matching lightness over hue/chroma (OkLab L
    /// weight); default 1.0. Ignored by Rgb/Redmean. Negative or non-finite
    /// counts as 1.0.
    pub lightness_weight: f32,
    pub adjust: Adjust,
    /// Source index -> target index, wins over everything. Entries with 0 on
    /// either side are ignored.
    pub overrides: BTreeMap<u8, u8>,
}

impl Default for RemapSettings {
    fn default() -> Self {
        RemapSettings {
            distance: ColorDistance::OkLab,
            targets: TargetSet::All,
            keep_exact: true,
            lightness_weight: 1.0,
            adjust: Adjust::default(),
            overrides: BTreeMap::new(),
        }
    }
}

/// Why a source index got its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    /// Same colour at the same index in both palettes (`keep_exact`).
    Kept,
    /// Closest allowed colour.
    Nearest,
    /// From `RemapSettings::overrides`.
    Override,
}

/// One used source colour in the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct RemapEntry {
    pub source: u8,
    pub target: u8,
    pub source_rgb: [u8; 3],
    pub target_rgb: [u8; 3],
    /// OkLab distance x 100 between the (unadjusted) source colour and the
    /// chosen target colour: ~0 identical, ~2 barely visible, > 10 obvious.
    pub error: f32,
    pub pixels: u64,
    pub how: MatchKind,
}

/// The result of [`plan`].
#[derive(Clone, Debug, PartialEq)]
pub struct RemapPlan {
    /// Valid d2_formats::remap::IndexTable: [0] = 0, nothing else maps to 0;
    /// unused indices map by the same rules.
    pub table: [u8; 256],
    /// Used source indices only (pixels > 0, excluding 0), worst error first
    /// (ties: lower source index first).
    pub entries: Vec<RemapEntry>,
    /// Pixel-weighted mean error and the worst error over `entries` (0 when
    /// there are none).
    pub mean_error: f32,
    pub worst_error: f32,
}

/// Plans the conversion of every index of `src` into `dst`. `used[i]` is the
/// pixel count of index i (`used[0]` is ignored). Deterministic; ties go to
/// the lowest target index.
pub fn plan(
    src: &Palette,
    dst: &Palette,
    used: &[u64; 256],
    settings: &RemapSettings,
) -> RemapPlan {
    let m = Matcher::new(src, dst, settings);
    let mut table = [0u8; 256];
    let mut entries = Vec::new();
    for s in 1..=255u8 {
        let (target, how) = m.decide(s, settings);
        table[usize::from(s)] = target;
        let pixels = used[usize::from(s)];
        if pixels > 0 {
            entries.push(RemapEntry {
                source: s,
                target,
                source_rgb: src.colors[usize::from(s)],
                target_rgb: dst.colors[usize::from(target)],
                error: m.error(s, target),
                pixels,
                how,
            });
        }
    }
    entries.sort_by(|a, b| {
        b.error
            .total_cmp(&a.error)
            .then_with(|| a.source.cmp(&b.source))
    });
    let (mut weighted, mut total) = (0.0f64, 0.0f64);
    let mut worst = 0.0f32;
    for e in &entries {
        weighted += f64::from(e.error) * e.pixels as f64;
        total += e.pixels as f64;
        worst = worst.max(e.error);
    }
    let mean_error = if total > 0.0 {
        (weighted / total) as f32
    } else {
        0.0
    };
    RemapPlan {
        table,
        entries,
        mean_error,
        worst_error: worst,
    }
}

/// The `k` best allowed targets for one source index, best first, with
/// their errors (for the override picker). Ranked by the settings' distance,
/// adjustments and lightness weight (overrides and `keep_exact` play no
/// part); the error is the same report value as [`RemapEntry::error`]. Ties
/// go to the lowest index. Source 0 has no candidates.
pub fn candidates(
    src: &Palette,
    dst: &Palette,
    source: u8,
    settings: &RemapSettings,
    k: usize,
) -> Vec<(u8, f32)> {
    if source == 0 || k == 0 {
        return Vec::new();
    }
    let m = Matcher::new(src, dst, settings);
    let mut ranked: Vec<(f64, u8)> = m
        .allowed
        .iter()
        .map(|&t| (m.match_distance(source, t), t))
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .take(k)
        .map(|(_, t)| (t, m.error(source, t)))
        .collect()
}

/// Maps the pixels of an image (alpha unchanged), for previews.
pub fn apply_to_image(img: &IndexedImage, table: &[u8; 256]) -> IndexedImage {
    let mut out = img.clone();
    for p in &mut out.pixels {
        *p = table[usize::from(*p)];
    }
    out
}

// ---------------------------------------------------------------------------
// colour maths

#[derive(Clone, Copy, Debug, PartialEq)]
struct Lab {
    l: f64,
    a: f64,
    b: f64,
}

/// Linear sRGB (components >= 0) to OkLab.
fn linear_to_oklab([r, g, b]: [f64; 3]) -> Lab {
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
    let (l, m, s) = (cbrt(l), cbrt(m), cbrt(s));
    Lab {
        l: 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        a: 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        b: 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    }
}

/// OkLab to linear sRGB (may be outside `0..=1`).
fn oklab_to_linear(c: Lab) -> [f64; 3] {
    let l = c.l + 0.396_337_777_4 * c.a + 0.215_803_757_3 * c.b;
    let m = c.l - 0.105_561_345_8 * c.a - 0.063_854_172_8 * c.b;
    let s = c.l - 0.089_484_177_5 * c.a - 1.291_485_548_0 * c.b;
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// Cube root of a non-negative value (negatives clamp to 0), platform-exact.
fn cbrt(x: f64) -> f64 {
    nth_root(x.max(0.0), 3)
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    const EPS: f64 = 1e-7;
    rgb.iter().all(|&c| (-EPS..=1.0 + EPS).contains(&c))
}

fn lab_distance(x: Lab, y: Lab) -> f64 {
    let (dl, da, db) = (x.l - y.l, x.a - y.a, x.b - y.b);
    (dl * dl + da * da + db * db).sqrt()
}

/// A colour ready for every distance metric.
#[derive(Clone, Copy, Debug)]
struct Prepared {
    lab: Lab,
    /// sRGB, 0..=255 (fractional after an adjustment).
    rgb: [f64; 3],
}

fn prepare(rgb: [u8; 3], table: &LinearTable) -> Prepared {
    Prepared {
        lab: linear_to_oklab(table.to_linear(rgb)),
        rgb: rgb.map(f64::from),
    }
}

/// `v` clamped to -1..=1, non-finite = 0.
fn unit(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Sanitised adjustment values: (brightness, contrast, saturation, hue rad),
/// or None when nothing changes.
fn adjust_params(adj: &Adjust) -> Option<(f64, f64, f64, f64)> {
    let hue = if adj.hue_degrees.is_finite() {
        f64::from(adj.hue_degrees) % 360.0
    } else {
        0.0
    };
    let p = (
        unit(adj.brightness),
        unit(adj.contrast),
        unit(adj.saturation),
        hue.to_radians(),
    );
    (p != (0.0, 0.0, 0.0, 0.0)).then_some(p)
}

/// Applies the adjustment to an OkLab colour and brings it back into the
/// sRGB gamut: L is clamped, then chroma is reduced along the same hue.
fn adjust_color(c: Lab, (bright, contrast, sat, hue): (f64, f64, f64, f64)) -> Prepared {
    let l = (0.5 + (c.l - 0.5) * (1.0 + contrast) + bright).clamp(0.0, 1.0);
    let (sin, cos) = hue.sin_cos();
    let k = 1.0 + sat;
    let a = (c.a * cos - c.b * sin) * k;
    let b = (c.a * sin + c.b * cos) * k;
    let with_chroma = |scale: f64| Lab {
        l,
        a: a * scale,
        b: b * scale,
    };
    let mut lin = oklab_to_linear(with_chroma(1.0));
    if !in_gamut(lin) {
        // a grey of L in 0..=1 is always in gamut: search the largest scale
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if in_gamut(oklab_to_linear(with_chroma(mid))) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lin = oklab_to_linear(with_chroma(lo));
    }
    let lin = lin.map(|v| v.clamp(0.0, 1.0));
    Prepared {
        lab: linear_to_oklab(lin),
        rgb: lin.map(|v| linear_to_srgb(v) * 255.0),
    }
}

/// Both palettes prepared once, plus the allowed target set.
struct Matcher {
    distance: ColorDistance,
    weight: f64,
    /// Unadjusted source colours (for the reported error).
    src_raw: [Lab; 256],
    /// Source colours after `Adjust` (for matching).
    src: [Prepared; 256],
    dst: [Prepared; 256],
    src_bytes: [[u8; 3]; 256],
    dst_bytes: [[u8; 3]; 256],
    allowed: Vec<u8>,
    allowed_mask: [bool; 256],
}

impl Matcher {
    fn new(src: &Palette, dst: &Palette, settings: &RemapSettings) -> Matcher {
        let table = LinearTable::new();
        let raw = src.colors.map(|c| prepare(c, &table));
        let adjusted = match adjust_params(&settings.adjust) {
            Some(p) => raw.map(|c| adjust_color(c.lab, p)),
            None => raw,
        };
        let w = settings.lightness_weight;
        let weight = if w.is_finite() && w >= 0.0 {
            f64::from(w)
        } else {
            1.0
        };
        let allowed = settings.targets.effective();
        let mut allowed_mask = [false; 256];
        for &t in &allowed {
            allowed_mask[usize::from(t)] = true;
        }
        Matcher {
            distance: settings.distance,
            weight,
            src_raw: raw.map(|c| c.lab),
            src: adjusted,
            dst: dst.colors.map(|c| prepare(c, &table)),
            src_bytes: src.colors,
            dst_bytes: dst.colors,
            allowed,
            allowed_mask,
        }
    }

    /// Matching distance (squared; only the order matters).
    fn match_distance(&self, s: u8, t: u8) -> f64 {
        let x = &self.src[usize::from(s)];
        let y = &self.dst[usize::from(t)];
        match self.distance {
            ColorDistance::OkLab => {
                let dl = (x.lab.l - y.lab.l) * self.weight;
                let (da, db) = (x.lab.a - y.lab.a, x.lab.b - y.lab.b);
                dl * dl + da * da + db * db
            }
            ColorDistance::Redmean => {
                let rmean = 0.5 * (x.rgb[0] + y.rgb[0]);
                let [dr, dg, db] = [0, 1, 2].map(|i| x.rgb[i] - y.rgb[i]);
                (2.0 + rmean / 256.0) * dr * dr
                    + 4.0 * dg * dg
                    + (2.0 + (255.0 - rmean) / 256.0) * db * db
            }
            ColorDistance::Rgb => {
                let [dr, dg, db] = [0, 1, 2].map(|i| x.rgb[i] - y.rgb[i]);
                dr * dr + dg * dg + db * db
            }
        }
    }

    /// Reported error: OkLab distance x 100, unadjusted source.
    fn error(&self, s: u8, t: u8) -> f32 {
        (lab_distance(self.src_raw[usize::from(s)], self.dst[usize::from(t)].lab) * 100.0) as f32
    }

    /// Closest allowed target; the first (lowest) index wins ties.
    fn nearest(&self, s: u8) -> u8 {
        let mut best: Option<(f64, u8)> = None;
        for &t in &self.allowed {
            let d = self.match_distance(s, t);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, t));
            }
        }
        // `allowed` is never empty (TargetSet::effective); 1 is a valid fallback
        best.map_or(1, |(_, t)| t)
    }

    fn decide(&self, s: u8, settings: &RemapSettings) -> (u8, MatchKind) {
        if let Some(&t) = settings.overrides.get(&s)
            && t != 0
        {
            return (t, MatchKind::Override);
        }
        let i = usize::from(s);
        if settings.keep_exact && self.allowed_mask[i] && self.src_bytes[i] == self.dst_bytes[i] {
            return (s, MatchKind::Kept);
        }
        (self.nearest(s), MatchKind::Nearest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Greys: index i = (i, i, i) for i >= 1; index 0 is black like index 1.
    fn grey() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8; 3];
        }
        Palette { colors }
    }

    fn flat(rgb: [u8; 3]) -> Palette {
        Palette { colors: [rgb; 256] }
    }

    fn all_used() -> [u64; 256] {
        [1; 256]
    }

    fn assert_valid(t: &[u8; 256]) {
        assert_eq!(t[0], 0);
        assert!(t[1..].iter().all(|&x| x != 0));
    }

    #[test]
    fn oklab_reference_values() {
        let t = LinearTable::new();
        let white = linear_to_oklab(t.to_linear([255, 255, 255]));
        assert!((white.l - 1.0).abs() < 1e-4 && white.a.abs() < 1e-4 && white.b.abs() < 1e-4);
        let black = linear_to_oklab(t.to_linear([0, 0, 0]));
        assert!(black.l.abs() < 1e-12);
        // sRGB red: L 0.62796, a 0.22486, b 0.12585 (Ottosson)
        let red = linear_to_oklab(t.to_linear([255, 0, 0]));
        assert!((red.l - 0.627_96).abs() < 1e-3);
        assert!((red.a - 0.224_86).abs() < 1e-3);
        assert!((red.b - 0.125_85).abs() < 1e-3);
        // round trip
        let back = oklab_to_linear(red);
        assert!((back[0] - 1.0).abs() < 1e-6 && back[1].abs() < 1e-6 && back[2].abs() < 1e-6);
    }

    #[test]
    fn identity_on_same_palette() {
        let p = grey();
        let r = plan(&p, &p, &all_used(), &RemapSettings::default());
        let id: Vec<u8> = (0..=255u8).collect();
        assert_eq!(r.table.to_vec(), id);
        assert_eq!(r.entries.len(), 255);
        assert!(
            r.entries
                .iter()
                .all(|e| e.how == MatchKind::Kept && e.error == 0.0)
        );
        assert_eq!((r.mean_error, r.worst_error), (0.0, 0.0));
    }

    #[test]
    fn ties_go_to_lowest_index_and_zero_is_never_a_target() {
        // every target colour identical: everything maps to 1, never 0
        let src = grey();
        let dst = flat([7, 7, 7]);
        let r = plan(&src, &dst, &all_used(), &RemapSettings::default());
        assert_valid(&r.table);
        // index 7 has the same colour in both: kept
        assert!(
            r.table[1..]
                .iter()
                .enumerate()
                .all(|(i, &t)| t == 1 || (i == 6 && t == 7))
        );
        assert_eq!(r.table[7], 7);
        // keep_exact keeps every index (same colour at the same index)
        let r = plan(
            &flat([7, 7, 7]),
            &dst,
            &all_used(),
            &RemapSettings::default(),
        );
        assert!(
            r.table[1..]
                .iter()
                .enumerate()
                .all(|(i, &t)| t == i as u8 + 1)
        );
        let no_keep = RemapSettings {
            keep_exact: false,
            ..RemapSettings::default()
        };
        let r = plan(&flat([7, 7, 7]), &dst, &all_used(), &no_keep);
        assert!(r.table[1..].iter().all(|&t| t == 1));
        assert!(r.entries.iter().all(|e| e.how == MatchKind::Nearest));
    }

    #[test]
    fn keep_exact_prefers_same_index_over_lower_duplicate() {
        let mut dst = grey();
        dst.colors[3] = [10, 10, 10]; // duplicate of index 10 at a lower index
        let src = grey();
        let r = plan(&src, &dst, &all_used(), &RemapSettings::default());
        assert_eq!(r.table[10], 10);
        let no_keep = RemapSettings {
            keep_exact: false,
            ..RemapSettings::default()
        };
        assert_eq!(plan(&src, &dst, &all_used(), &no_keep).table[10], 3);
    }

    #[test]
    fn overrides_win_and_zero_overrides_are_ignored() {
        let p = grey();
        let mut s = RemapSettings {
            targets: TargetSet::Only(vec![5]),
            ..RemapSettings::default()
        };
        s.overrides.insert(10, 200); // outside the allowed set: still wins
        s.overrides.insert(11, 0); // ignored
        s.overrides.insert(0, 9); // ignored
        let r = plan(&p, &p, &all_used(), &s);
        assert_valid(&r.table);
        assert_eq!(r.table[10], 200);
        assert_eq!(r.table[11], 5);
        assert_eq!(r.table[5], 5);
        assert!(
            r.table[1..]
                .iter()
                .enumerate()
                .all(|(i, &t)| t == 5 || i + 1 == 10)
        );
        let e = r.entries.iter().find(|e| e.source == 10).unwrap();
        assert_eq!(e.how, MatchKind::Override);
        assert_eq!(e.target_rgb, [200; 3]);
    }

    #[test]
    fn target_sets() {
        assert_eq!(TargetSet::All.effective().len(), 255);
        let shared = TargetSet::SharedActColors.effective();
        assert_eq!(shared.len(), 225);
        assert!(shared.iter().all(|i| !ACT_SPECIFIC.contains(i) && *i != 0));
        assert_eq!(TargetSet::Only(vec![9, 0, 3, 9]).effective(), vec![3, 9]);
        // no usable index: falls back to All
        assert_eq!(TargetSet::Only(vec![]).effective().len(), 255);
        assert_eq!(TargetSet::Only(vec![0, 0]).effective().len(), 255);
        let p = grey();
        let s = RemapSettings {
            targets: TargetSet::Only(vec![0]),
            ..RemapSettings::default()
        };
        let r = plan(&p, &p, &all_used(), &s);
        assert_eq!(r.table.to_vec(), (0..=255u8).collect::<Vec<_>>());
    }

    #[test]
    fn shared_act_colours_never_outputs_act_specific() {
        let p = grey();
        let s = RemapSettings {
            targets: TargetSet::SharedActColors,
            ..RemapSettings::default()
        };
        let r = plan(&p, &p, &all_used(), &s);
        assert_valid(&r.table);
        for i in 1..=255u8 {
            let t = r.table[usize::from(i)];
            assert!(!ACT_SPECIFIC.contains(&t));
            if !ACT_SPECIFIC.contains(&i) {
                assert_eq!(t, i);
            }
        }
        // greys 225..=254 go to the nearest allowed grey: 224 or 255
        assert_eq!(r.table[230], 224);
        assert_eq!(r.table[250], 255);
        assert_eq!(r.table[226], 224);
        assert_eq!(r.table[253], 255);
    }

    #[test]
    fn lightness_weight_changes_the_choice() {
        // source mid grey; target 1 = same lightness but tinted, target 2 =
        // neutral but darker
        let mut src = flat([128, 128, 128]);
        src.colors[0] = [0, 0, 0];
        let mut dst = flat([255, 255, 255]);
        dst.colors[1] = [150, 115, 115]; // reddish, similar L
        dst.colors[2] = [105, 105, 105]; // grey, darker
        let s = |w: f32| RemapSettings {
            keep_exact: false,
            targets: TargetSet::Only(vec![1, 2]),
            lightness_weight: w,
            ..RemapSettings::default()
        };
        let pick = |w| candidates(&src, &dst, 5, &s(w), 1)[0].0;
        assert_eq!(pick(0.2), 2);
        assert_eq!(pick(4.0), 1);
        // error is the same report value whatever the weight
        let e1 = candidates(&src, &dst, 5, &s(0.2), 2);
        let e2 = candidates(&src, &dst, 5, &s(4.0), 2);
        let err = |v: &[(u8, f32)], t| v.iter().find(|c| c.0 == t).unwrap().1;
        assert_eq!(err(&e1, 1), err(&e2, 1));
        assert_eq!(err(&e1, 2), err(&e2, 2));
    }

    #[test]
    fn adjust_moves_the_match_but_not_the_error_basis() {
        let src = grey();
        let dst = grey();
        let bright = RemapSettings {
            keep_exact: false,
            adjust: Adjust {
                brightness: 0.2,
                ..Adjust::default()
            },
            ..RemapSettings::default()
        };
        let r = plan(&src, &dst, &all_used(), &bright);
        assert_valid(&r.table);
        assert!(r.table[100] > 100);
        let e = r.entries.iter().find(|e| e.source == 100).unwrap();
        let expected = candidates(&src, &dst, 100, &RemapSettings::default(), 255)
            .into_iter()
            .find(|c| c.0 == e.target)
            .unwrap()
            .1;
        assert_eq!(e.error, expected);
        assert!(e.error > 5.0);
        // darker
        let dark = RemapSettings {
            adjust: Adjust {
                brightness: -0.2,
                ..Adjust::default()
            },
            ..bright.clone()
        };
        assert!(plan(&src, &dst, &all_used(), &dark).table[100] < 100);
        // keep_exact ignores the adjustment (compares raw colours)
        let kept = RemapSettings {
            keep_exact: true,
            ..bright
        };
        assert_eq!(plan(&src, &dst, &all_used(), &kept).table[100], 100);
    }

    #[test]
    fn saturation_and_hue() {
        // source: saturated red; targets: a grey, a red, a cyan
        let mut src = flat([0, 0, 0]);
        src.colors[1] = [200, 30, 30];
        let mut dst = flat([0, 0, 0]);
        dst.colors[1] = [92, 92, 92];
        dst.colors[2] = [200, 30, 30];
        dst.colors[3] = [0, 140, 140];
        let s = |adjust| RemapSettings {
            keep_exact: false,
            targets: TargetSet::Only(vec![1, 2, 3]),
            adjust,
            ..RemapSettings::default()
        };
        let pick = |a| candidates(&src, &dst, 1, &s(a), 1)[0].0;
        assert_eq!(pick(Adjust::default()), 2);
        let grey_out = Adjust {
            saturation: -1.0,
            ..Adjust::default()
        };
        assert_eq!(pick(grey_out), 1);
        let opposite = Adjust {
            hue_degrees: 180.0,
            ..Adjust::default()
        };
        assert_eq!(pick(opposite), 3);
        // contrast -1 squashes every L to 0.5; saturation +1 out of gamut is clamped
        let c = adjust_color(
            linear_to_oklab(LinearTable::new().to_linear([255, 0, 0])),
            adjust_params(&Adjust {
                saturation: 1.0,
                contrast: 1.0,
                brightness: 1.0,
                hue_degrees: 33.0,
            })
            .unwrap(),
        );
        assert!(c.rgb.iter().all(|v| (0.0..=255.0).contains(v)));
        assert!(in_gamut(oklab_to_linear(c.lab)));
    }

    #[test]
    fn nonsense_settings_are_sanitised() {
        let p = grey();
        let s = RemapSettings {
            lightness_weight: f32::NAN,
            adjust: Adjust {
                brightness: f32::INFINITY,
                contrast: f32::NAN,
                saturation: -7.0,
                hue_degrees: f32::NEG_INFINITY,
            },
            keep_exact: false,
            ..RemapSettings::default()
        };
        let r = plan(&p, &p, &all_used(), &s);
        assert_valid(&r.table);
        assert!(r.entries.iter().all(|e| e.error.is_finite()));
        assert!(r.mean_error.is_finite());
    }

    #[test]
    fn distances_rank_sensibly() {
        for distance in [
            ColorDistance::OkLab,
            ColorDistance::Redmean,
            ColorDistance::Rgb,
        ] {
            let s = RemapSettings {
                distance,
                keep_exact: false,
                ..RemapSettings::default()
            };
            let c = candidates(&grey(), &grey(), 100, &s, 3);
            assert_eq!(c[0], (100, 0.0));
            assert_eq!(c.len(), 3);
            assert!(c[1].0 == 99 || c[1].0 == 101);
            assert!(c[2].0 == 99 || c[2].0 == 101);
        }
        // 99 and 101 tie exactly in RGB: lowest first
        let rgb = RemapSettings {
            distance: ColorDistance::Rgb,
            ..RemapSettings::default()
        };
        assert_eq!(candidates(&grey(), &grey(), 100, &rgb, 3)[1].0, 99);
        assert!(candidates(&grey(), &grey(), 0, &RemapSettings::default(), 5).is_empty());
        assert!(candidates(&grey(), &grey(), 5, &RemapSettings::default(), 0).is_empty());
    }

    #[test]
    fn entries_sorted_and_stats() {
        let src = grey();
        let mut dst = grey();
        dst.colors[50] = [60, 60, 60];
        dst.colors[51] = [60, 60, 60];
        let mut used = [0u64; 256];
        used[0] = 99; // ignored
        used[50] = 3;
        used[200] = 1;
        let r = plan(&src, &dst, &used, &RemapSettings::default());
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].source, 50);
        assert_eq!(r.entries[0].target, 49);
        assert_eq!(r.entries[1].source, 200);
        assert!(r.entries[0].error > 0.0);
        assert_eq!(r.worst_error, r.entries[0].error);
        assert!((r.mean_error - r.entries[0].error * 0.75).abs() < 1e-6);
        // unused 51 is still mapped
        assert_ne!(r.table[51], 51);
    }

    #[test]
    fn apply_to_image_maps_pixels_keeps_alpha() {
        let mut table = [0u8; 256];
        for (i, t) in table.iter_mut().enumerate().skip(1) {
            *t = 255 - i as u8 + 1;
        }
        let img = IndexedImage {
            width: 2,
            height: 2,
            offset_x: 3,
            offset_y: -4,
            pixels: vec![0, 1, 2, 255],
            alpha: vec![0, 255, 255, 255],
        };
        let out = apply_to_image(&img, &table);
        assert_eq!(out.pixels, vec![0, 255, 254, 1]);
        assert_eq!(out.alpha, img.alpha);
        assert_eq!((out.offset_x, out.offset_y, out.width), (3, -4, 2));
    }
}
