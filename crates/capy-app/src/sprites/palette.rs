//! The palette a sprite is shown, quantized and edited with: one of the five
//! act palettes, any other `pal.dat` the profile's overlay holds (units,
//! menus, ...), or a built-in grey ramp when no profile is open.
//!
//! The *raw* palette is what pixels are matched against (import, editing,
//! export); gamma only changes what is drawn on screen.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use d2_assets::{GamePath, Overlay};
use d2_formats::palette::{Palette, act_palette_path};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PaletteChoice {
    /// 0-based act (0 = Act 1).
    Act(u8),
    /// Another `pal.dat` in the overlay.
    Game(GamePath),
    /// The built-in grey ramp (no profile open, or the file is unreadable).
    Grey,
}

impl PaletteChoice {
    pub fn label(&self) -> String {
        match self {
            PaletteChoice::Act(a) => format!("Act {}", a + 1),
            PaletteChoice::Game(p) => {
                // data\global\palette\units\pal.dat -> "units"
                let s = p.as_str();
                let trimmed = s.strip_suffix("\\pal.dat").unwrap_or(s);
                let short = trimmed
                    .strip_prefix("data\\global\\palette\\")
                    .unwrap_or(trimmed);
                short.to_owned()
            }
            PaletteChoice::Grey => "Grey (built in)".to_owned(),
        }
    }

    pub fn game_path(&self) -> Option<GamePath> {
        match self {
            PaletteChoice::Act(a) => Some(GamePath::new(act_palette_path(*a))),
            PaletteChoice::Game(p) => Some(p.clone()),
            PaletteChoice::Grey => None,
        }
    }

    /// The palette's folder name, as palette memory and `sprite.json` store
    /// it: `act1`..`act5`, `units`, `menu0`, ..., or `grey`.
    pub fn memory_label(&self) -> String {
        match self {
            PaletteChoice::Act(a) => format!("act{}", a + 1),
            PaletteChoice::Game(_) => self.label().to_ascii_lowercase(),
            PaletteChoice::Grey => "grey".to_owned(),
        }
    }

    /// The choice a stored label names. Accepts memory labels (`act5`,
    /// `units`), display labels (`Act 5`, `Grey (built in)`) and palette
    /// game paths; `None` for empty text.
    pub fn from_label(label: &str) -> Option<PaletteChoice> {
        let t = label.trim().replace('/', "\\").to_ascii_lowercase();
        if t.is_empty() {
            return None;
        }
        let t = t
            .strip_prefix("data\\global\\palette\\")
            .unwrap_or(&t)
            .trim_end_matches("\\pal.dat")
            .trim_end_matches("\\pal.pl2")
            .to_owned();
        if t.starts_with("grey") || t.starts_with("gray") {
            return Some(PaletteChoice::Grey);
        }
        let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(n) = compact.strip_prefix("act")
            && let Ok(n) = n.parse::<u8>()
            && (1..=5).contains(&n)
        {
            return Some(PaletteChoice::Act(n - 1));
        }
        if compact.contains('\\') || compact.contains(':') || compact.contains("..") {
            return None;
        }
        Some(PaletteChoice::Game(GamePath::new(&format!(
            "data\\global\\palette\\{compact}\\pal.dat"
        ))))
    }

    /// File-name friendly label (for exports).
    pub fn slug(&self) -> String {
        self.label()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }
}

/// A grey ramp: index 0 black (transparent key), 255 white.
pub fn grey_palette() -> Palette {
    let mut colors = [[0u8; 3]; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = [i as u8; 3];
    }
    Palette { colors }
}

/// The choices offered: the five acts, then every other `pal.dat` listed
/// in the profile (act palettes are not repeated).
pub fn choices(extra: &[GamePath]) -> Vec<PaletteChoice> {
    let mut out: Vec<PaletteChoice> = (0..5).map(PaletteChoice::Act).collect();
    let acts: Vec<GamePath> = (0..5).map(|a| GamePath::new(act_palette_path(a))).collect();
    for p in extra {
        if !acts.contains(p) {
            out.push(PaletteChoice::Game(p.clone()));
        }
    }
    out
}

pub struct PaletteStore {
    pub choice: PaletteChoice,
    pub gamma: f32,
    /// Raw palettes read for `epoch`; `None` = unreadable.
    cache: HashMap<PaletteChoice, Option<Palette>>,
    epoch: Option<u64>,
    display: Option<(u64, Palette)>,
}

impl Default for PaletteStore {
    fn default() -> Self {
        PaletteStore {
            choice: PaletteChoice::Act(0),
            gamma: crate::config::DEFAULT_GAMMA,
            cache: HashMap::new(),
            epoch: None,
            display: None,
        }
    }
}

impl PaletteStore {
    /// Reads (once per session) a palette through the overlay.
    fn load(
        &mut self,
        choice: &PaletteChoice,
        overlay: Option<&Overlay>,
        epoch: u64,
    ) -> Option<Palette> {
        if self.epoch != Some(epoch) {
            self.cache.clear();
            self.display = None;
            self.epoch = Some(epoch);
        }
        if let Some(found) = self.cache.get(choice) {
            return found.clone();
        }
        let loaded = match (choice.game_path(), overlay) {
            (Some(path), Some(overlay)) => overlay
                .read_bytes(&path)
                .ok()
                .flatten()
                .and_then(|b| Palette::parse_dat(&b).ok()),
            (None, _) => Some(grey_palette()),
            (Some(_), None) => None,
        };
        self.cache.insert(choice.clone(), loaded.clone());
        loaded
    }

    /// The raw palette of the current choice; the grey ramp when it cannot
    /// be read (the flag is false then).
    pub fn raw(&mut self, overlay: Option<&Overlay>, epoch: u64) -> (Palette, bool) {
        let choice = self.choice.clone();
        match self.load(&choice, overlay, epoch) {
            Some(p) => (p, true),
            None => (grey_palette(), choice == PaletteChoice::Grey),
        }
    }

    /// The raw palette of any choice (`None` when it cannot be read).
    pub fn raw_of(
        &mut self,
        choice: &PaletteChoice,
        overlay: Option<&Overlay>,
        epoch: u64,
    ) -> Option<Palette> {
        self.load(choice, overlay, epoch)
    }

    /// The raw palette of an act (for previews that follow a map's act).
    pub fn act(&mut self, act: u8, overlay: Option<&Overlay>, epoch: u64) -> Palette {
        self.load(&PaletteChoice::Act(act.min(4)), overlay, epoch)
            .unwrap_or_else(grey_palette)
    }

    /// Identity of (choice, gamma, session) for texture caches.
    pub fn key(&self, epoch: u64) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.choice.hash(&mut h);
        self.gamma.to_bits().hash(&mut h);
        epoch.hash(&mut h);
        h.finish()
    }

    /// The palette to draw with: the raw palette with gamma applied.
    pub fn display(&mut self, overlay: Option<&Overlay>, epoch: u64) -> Palette {
        let key = self.key(epoch);
        if let Some((k, p)) = &self.display
            && *k == key
            && self.epoch == Some(epoch)
        {
            return p.clone();
        }
        let (raw, _) = self.raw(overlay, epoch);
        let shown = raw.with_gamma(self.gamma);
        self.display = Some((key, shown.clone()));
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_list_acts_first_without_repeats() {
        let extra = vec![
            GamePath::new("data\\global\\palette\\act2\\pal.dat"),
            GamePath::new("data\\global\\palette\\units\\pal.dat"),
        ];
        let c = choices(&extra);
        assert_eq!(c.len(), 6);
        assert_eq!(c[0], PaletteChoice::Act(0));
        assert_eq!(c[5].label(), "units");
        assert_eq!(PaletteChoice::Act(4).label(), "Act 5");
    }

    #[test]
    fn memory_labels_round_trip() {
        let units = PaletteChoice::Game(GamePath::new("data\\global\\palette\\units\\pal.dat"));
        for c in [
            PaletteChoice::Act(0),
            PaletteChoice::Act(4),
            units.clone(),
            PaletteChoice::Grey,
        ] {
            assert_eq!(
                PaletteChoice::from_label(&c.memory_label()),
                Some(c.clone())
            );
            assert_eq!(PaletteChoice::from_label(&c.label()), Some(c.clone()));
        }
        assert_eq!(PaletteChoice::Act(4).memory_label(), "act5");
        assert_eq!(units.memory_label(), "units");
        assert_eq!(
            PaletteChoice::from_label("ACT2"),
            Some(PaletteChoice::Act(1))
        );
        assert_eq!(
            PaletteChoice::from_label("data\\global\\palette\\act3\\pal.dat"),
            Some(PaletteChoice::Act(2))
        );
        assert_eq!(PaletteChoice::from_label(" "), None);
        assert_eq!(PaletteChoice::from_label("..\\x"), None);
        assert_eq!(
            PaletteChoice::from_label("act9"),
            Some(PaletteChoice::Game(GamePath::new(
                "data\\global\\palette\\act9\\pal.dat"
            )))
        );
    }

    #[test]
    fn no_profile_falls_back_to_grey() {
        let mut store = PaletteStore::default();
        let (p, ok) = store.raw(None, 1);
        assert!(!ok, "Act 1 cannot be read without a profile");
        assert_eq!(p, grey_palette());
        store.choice = PaletteChoice::Grey;
        assert!(store.raw(None, 1).1);
        let k1 = store.key(1);
        store.gamma = 2.0;
        assert_ne!(k1, store.key(1), "gamma changes the texture key");
        assert_ne!(store.display(None, 1), grey_palette());
    }
}
