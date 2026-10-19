//! The "Convert to palette" settings as the window edits them and as named
//! presets store them in `config.toml`, and their translation into the
//! planner's `RemapSettings`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use capy_paint::remap::{ACT_SPECIFIC, Adjust, ColorDistance, RemapSettings, TargetSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distance {
    #[default]
    OkLab,
    Redmean,
    Rgb,
}

impl Distance {
    pub const ALL: [Distance; 3] = [Distance::OkLab, Distance::Redmean, Distance::Rgb];

    pub fn label(self) -> &'static str {
        match self {
            Distance::OkLab => "OkLab (perceptual)",
            Distance::Redmean => "Redmean",
            Distance::Rgb => "RGB",
        }
    }

    pub fn parse(text: &str) -> Option<Distance> {
        match text.trim().to_ascii_lowercase().as_str() {
            "oklab" | "ok" => Some(Distance::OkLab),
            "redmean" => Some(Distance::Redmean),
            "rgb" => Some(Distance::Rgb),
            _ => None,
        }
    }

    fn lib(self) -> ColorDistance {
        match self {
            Distance::OkLab => ColorDistance::OkLab,
            Distance::Redmean => ColorDistance::Redmean,
            Distance::Rgb => ColorDistance::Rgb,
        }
    }
}

/// Which target indices may be picked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Allowed {
    #[default]
    All,
    /// Skip the act-specific indices 225..=254.
    Shared,
    /// Only `from..=to` (0 is never picked).
    Range { from: u8, to: u8 },
}

impl Allowed {
    pub fn label(self) -> String {
        match self {
            Allowed::All => "All colours".to_owned(),
            Allowed::Shared => "Shared act colours (skip 225-254)".to_owned(),
            Allowed::Range { from, to } => format!("Indices {from}-{to}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertSettings {
    pub distance: Distance,
    pub allowed: Allowed,
    pub keep_exact: bool,
    pub lightness_weight: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub hue: f32,
    /// `[source, target]` pairs; the last one for a source wins.
    pub overrides: Vec<[u8; 2]>,
}

impl Default for ConvertSettings {
    fn default() -> Self {
        ConvertSettings {
            distance: Distance::OkLab,
            allowed: Allowed::All,
            keep_exact: true,
            lightness_weight: 1.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            hue: 0.0,
            overrides: Vec::new(),
        }
    }
}

/// Slider ranges (lead decision: the planner accepts +-1, +-0.3 is the
/// useful range; lightness weight is an advanced slider).
pub const ADJUST_RANGE: std::ops::RangeInclusive<f32> = -0.3..=0.3;
pub const HUE_RANGE: std::ops::RangeInclusive<f32> = -180.0..=180.0;
pub const LIGHTNESS_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

impl ConvertSettings {
    /// Overrides as a map (0 on either side is dropped).
    pub fn override_map(&self) -> BTreeMap<u8, u8> {
        self.overrides
            .iter()
            .filter(|[s, t]| *s != 0 && *t != 0)
            .map(|[s, t]| (*s, *t))
            .collect()
    }

    pub fn override_for(&self, source: u8) -> Option<u8> {
        self.override_map().get(&source).copied()
    }

    /// Sets (or with `None` clears) the override of one source index.
    pub fn set_override(&mut self, source: u8, target: Option<u8>) {
        self.overrides.retain(|[s, _]| *s != source);
        if let Some(t) = target
            && source != 0
            && t != 0
        {
            self.overrides.push([source, t]);
            self.overrides.sort_unstable();
        }
    }

    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    /// Back to the defaults, keeping the overrides (they have their own
    /// "Clear all").
    pub fn reset_keep_overrides(&mut self) {
        let overrides = std::mem::take(&mut self.overrides);
        *self = ConvertSettings {
            overrides,
            ..ConvertSettings::default()
        };
    }

    /// Values clamped into the slider ranges (presets are hand-editable).
    pub fn clamped(mut self) -> ConvertSettings {
        let fix = |v: f32, r: &std::ops::RangeInclusive<f32>| {
            if v.is_finite() {
                v.clamp(*r.start(), *r.end())
            } else {
                0.0
            }
        };
        self.brightness = fix(self.brightness, &ADJUST_RANGE);
        self.contrast = fix(self.contrast, &ADJUST_RANGE);
        self.saturation = fix(self.saturation, &ADJUST_RANGE);
        self.hue = fix(self.hue, &HUE_RANGE);
        self.lightness_weight = if self.lightness_weight.is_finite() {
            self.lightness_weight
                .clamp(*LIGHTNESS_RANGE.start(), *LIGHTNESS_RANGE.end())
        } else {
            1.0
        };
        if let Allowed::Range { from, to } = &mut self.allowed {
            if *from > *to {
                std::mem::swap(from, to);
            }
            *from = (*from).max(1);
            *to = (*to).max(1);
        }
        self
    }

    /// The planner's settings. `shared_only` (target "All acts") skips the
    /// act-specific indices on top of `allowed`.
    pub fn to_remap(&self, shared_only: bool) -> RemapSettings {
        let act = ACT_SPECIFIC;
        let targets = match (self.allowed, shared_only) {
            (Allowed::All, false) => TargetSet::All,
            (Allowed::All | Allowed::Shared, _) => TargetSet::SharedActColors,
            (Allowed::Range { from, to }, shared) => TargetSet::Only(
                (from.max(1)..=to.max(from.max(1)))
                    .filter(|i| !(shared && act.contains(i)))
                    .collect(),
            ),
        };
        RemapSettings {
            distance: self.distance.lib(),
            targets,
            keep_exact: self.keep_exact,
            lightness_weight: self.lightness_weight,
            adjust: Adjust {
                brightness: self.brightness,
                contrast: self.contrast,
                saturation: self.saturation,
                hue_degrees: self.hue,
            },
            overrides: self.override_map(),
        }
    }

    /// The target indices the settings allow (for the override picker grid).
    pub fn allows(&self, index: u8, shared_only: bool) -> bool {
        if index == 0 {
            return false;
        }
        let act = ACT_SPECIFIC;
        let shared = shared_only || self.allowed == Allowed::Shared;
        match self.allowed {
            Allowed::Range { from, to } if !(from..=to).contains(&index) => false,
            _ => !(shared && act.contains(&index)),
        }
    }
}

/// A named preset in `config.toml` (`[[palette_presets]]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PalettePreset {
    pub name: String,
    #[serde(default)]
    pub settings: ConvertSettings,
}

/// Adds or replaces the preset `name` (case-insensitive); returns false for
/// an empty name.
pub fn save_preset(
    presets: &mut Vec<PalettePreset>,
    name: &str,
    settings: &ConvertSettings,
) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let preset = PalettePreset {
        name: name.to_owned(),
        settings: settings.clone(),
    };
    match presets
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(name))
    {
        Some(p) => *p = preset,
        None => presets.push(preset),
    }
    presets.sort_by_key(|p| p.name.to_ascii_lowercase());
    true
}

pub fn delete_preset(presets: &mut Vec<PalettePreset>, name: &str) -> bool {
    let before = presets.len();
    presets.retain(|p| !p.name.eq_ignore_ascii_case(name.trim()));
    presets.len() != before
}

pub fn find_preset<'a>(presets: &'a [PalettePreset], name: &str) -> Option<&'a PalettePreset> {
    presets
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_set_replace_and_clear() {
        let mut s = ConvertSettings::default();
        s.set_override(230, Some(40));
        s.set_override(231, Some(41));
        s.set_override(230, Some(42));
        assert_eq!(s.override_for(230), Some(42));
        assert_eq!(s.overrides, vec![[230, 42], [231, 41]]);
        s.set_override(0, Some(5));
        s.set_override(5, Some(0));
        assert_eq!(s.overrides.len(), 2, "0 is never an override");
        s.set_override(231, None);
        assert_eq!(s.override_map().len(), 1);
        s.brightness = 0.5;
        s.reset_keep_overrides();
        assert_eq!(s.brightness, 0.0);
        assert_eq!(s.override_for(230), Some(42), "reset keeps overrides");
        s.clear_overrides();
        assert!(s.overrides.is_empty());
    }

    #[test]
    fn targets_follow_allowed_and_all_acts() {
        let mut s = ConvertSettings::default();
        assert_eq!(s.to_remap(false).targets, TargetSet::All);
        assert_eq!(s.to_remap(true).targets, TargetSet::SharedActColors);
        s.allowed = Allowed::Shared;
        assert_eq!(s.to_remap(false).targets, TargetSet::SharedActColors);
        s.allowed = Allowed::Range { from: 220, to: 230 };
        assert_eq!(
            s.to_remap(true).targets,
            TargetSet::Only((220..=224).collect())
        );
        assert!(s.allows(222, true) && !s.allows(226, true) && s.allows(226, false));
        assert!(!s.allows(0, false) && !s.allows(10, false));
        let r = ConvertSettings::default().to_remap(false);
        assert!(r.keep_exact);
        assert_eq!(r.lightness_weight, 1.0);
    }

    #[test]
    fn presets_round_trip_through_toml() {
        let mut presets = Vec::new();
        let mut s = ConvertSettings {
            distance: Distance::Redmean,
            allowed: Allowed::Range { from: 1, to: 224 },
            keep_exact: false,
            lightness_weight: 2.0,
            brightness: 0.1,
            contrast: -0.2,
            saturation: 0.25,
            hue: 15.0,
            overrides: Vec::new(),
        };
        s.set_override(230, Some(41));
        assert!(save_preset(&mut presets, "Act 5 to 1", &s));
        assert!(save_preset(&mut presets, "ui", &ConvertSettings::default()));
        assert!(!save_preset(&mut presets, "  ", &s));
        assert!(save_preset(&mut presets, "UI", &s), "same name replaces");
        assert_eq!(presets.len(), 2);
        #[derive(Serialize, Deserialize)]
        struct Wrap {
            palette_presets: Vec<PalettePreset>,
        }
        let text = toml::to_string(&Wrap {
            palette_presets: presets.clone(),
        })
        .expect("toml");
        let back: Wrap = toml::from_str(&text).expect("parse");
        assert_eq!(back.palette_presets, presets);
        assert_eq!(
            find_preset(&presets, "act 5 TO 1").map(|p| &p.settings),
            Some(&s)
        );
        // A hand-written preset with missing fields gets the defaults.
        let partial: Wrap = toml::from_str(
            "[[palette_presets]]\nname = \"x\"\n[palette_presets.settings]\nhue = 400.0\n",
        )
        .expect("partial");
        let p = partial.palette_presets[0].settings.clone().clamped();
        assert_eq!(p.hue, 180.0);
        assert!(p.keep_exact);
        assert!(delete_preset(&mut presets, "ui"));
        assert!(!delete_preset(&mut presets, "ui"));
    }
}
