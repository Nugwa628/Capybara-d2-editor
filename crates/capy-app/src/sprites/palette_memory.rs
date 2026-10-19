//! Palette memory: which palette a sprite or tile file belongs to, so the
//! Sprites tab reopens it with the same palette and warns before art made
//! under one palette is saved into a file that belongs to another.
//!
//! The rules live in the pure functions at the top (easy to refine with
//! real examples); the `AppState` part at the bottom only reads and writes
//! user.db (`sprite_palettes`, keyed by game path or loose file path).
//!
//! - **Remembered**: the palette a file was last viewed with after the user
//!   picked it, imported into, saved, or converted to.
//! - **Folder**: the most recent palette of any file in the same folder; it
//!   is only *suggested* (one click), never applied silently.
//! - Opening a file with a remembered palette switches to it.

use std::collections::HashMap;

use super::palette::PaletteChoice;

/// Where the badge's palette stands for the file on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeSource {
    /// The current palette is the one remembered for this file.
    Remembered,
    /// The file is remembered with another palette.
    RememberedOther,
    /// Nothing for the file; the folder's palette is the current one.
    Folder,
    /// Nothing for the file; the folder uses another palette.
    FolderOther,
    /// Nothing remembered (or the sprite has no file yet).
    None,
}

impl BadgeSource {
    /// The word scripts use (`assert-palette-badge act5 remembered`).
    pub fn word(self) -> &'static str {
        match self {
            BadgeSource::Remembered => "remembered",
            BadgeSource::RememberedOther => "remembered-other",
            BadgeSource::Folder => "folder",
            BadgeSource::FolderOther => "folder-other",
            BadgeSource::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Info,
    Warn,
    Danger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub current: PaletteChoice,
    pub source: BadgeSource,
    /// "Palette: Act 5 — remembered for this file".
    pub text: String,
    pub tone: Tone,
    /// A palette offered with one click ("Use Act 5").
    pub offer: Option<PaletteChoice>,
}

fn same(a: &PaletteChoice, label: &str) -> bool {
    PaletteChoice::from_label(label).as_ref() == Some(a)
}

/// The badge for the file `has_file` (false: an import not saved yet) with
/// the palettes remembered for it and its folder.
pub fn badge(
    current: &PaletteChoice,
    has_file: bool,
    file: Option<&str>,
    folder: Option<&str>,
) -> Badge {
    let name = current.label();
    let parse = |l: &str| PaletteChoice::from_label(l);
    let (source, text, tone, offer) = match (has_file, file, folder) {
        (false, _, _) => (
            BadgeSource::None,
            format!("Palette: {name} \u{2014} not remembered (save the sprite to remember it)"),
            Tone::Warn,
            None,
        ),
        (true, Some(f), _) if same(current, f) => (
            BadgeSource::Remembered,
            format!("Palette: {name} \u{2014} remembered for this file"),
            Tone::Good,
            None,
        ),
        (true, Some(f), _) => {
            let other = parse(f);
            let other_name = other
                .as_ref()
                .map_or_else(|| f.to_owned(), PaletteChoice::label);
            (
                BadgeSource::RememberedOther,
                format!("Palette: {name} \u{2014} this file was last used with {other_name}!"),
                Tone::Danger,
                other,
            )
        }
        (true, None, Some(d)) if same(current, d) => (
            BadgeSource::Folder,
            format!("Palette: {name} \u{2014} used in this folder"),
            Tone::Info,
            None,
        ),
        (true, None, Some(d)) => {
            let other = parse(d);
            let other_name = other
                .as_ref()
                .map_or_else(|| d.to_owned(), PaletteChoice::label);
            (
                BadgeSource::FolderOther,
                format!("Palette: {name} \u{2014} not remembered; this folder uses {other_name}"),
                Tone::Warn,
                other,
            )
        }
        (true, None, None) => (
            BadgeSource::None,
            format!("Palette: {name} \u{2014} not remembered"),
            Tone::Warn,
            None,
        ),
    };
    Badge {
        current: current.clone(),
        source,
        text,
        tone,
        offer,
    }
}

/// What opening a file does to the palette: switch to its remembered one
/// (`Some`), else nothing (a folder palette is only offered by the badge).
pub fn on_open(current: &PaletteChoice, file: Option<&str>) -> Option<PaletteChoice> {
    let remembered = PaletteChoice::from_label(file?)?;
    (remembered != *current).then_some(remembered)
}

/// The remembered palette when writing into a file with `current` would
/// contradict it (the save/import warning).
pub fn mismatch(current: &PaletteChoice, remembered: Option<&str>) -> Option<PaletteChoice> {
    let r = PaletteChoice::from_label(remembered?)?;
    (r != *current).then_some(r)
}

/// A default palette guessed from a game path when nothing is remembered:
/// tiles under `tiles\actN\` (and `tiles\expansion\` = Act 5).
pub fn guess_from_path(key: &str) -> Option<PaletteChoice> {
    let k = key.replace('/', "\\").to_ascii_lowercase();
    let rest = k.split_once("\\tiles\\")?.1;
    let folder = rest.split('\\').next()?;
    match folder {
        "expansion" => Some(PaletteChoice::Act(4)),
        f => {
            let n: u8 = f.strip_prefix("act")?.parse().ok()?;
            (1..=5).contains(&n).then(|| PaletteChoice::Act(n - 1))
        }
    }
}

/// UI, item and units art looks the same in every act only with the shared
/// colours: the "All acts" target is suggested (not selected) for them.
pub fn suggests_all_acts(key: &str) -> bool {
    let k = key.replace('/', "\\").to_ascii_lowercase();
    ["\\ui\\", "\\items\\", "\\units\\", "\\fonts\\"]
        .iter()
        .any(|p| k.contains(p))
        || k.starts_with("data\\global\\ui")
        || k.starts_with("data\\global\\items")
}

// ───────────────────────── user.db ─────────────────────────

use crate::state::AppState;

/// Lookups of the current profile, cleared on every write.
#[derive(Default)]
pub struct MemoryCache {
    file: HashMap<String, Option<String>>,
    folder: HashMap<String, Option<String>>,
    profile: String,
}

impl AppState {
    /// The profile palette memory is filed under ("" without a profile).
    fn memory_profile(&self) -> String {
        self.session
            .as_ref()
            .map_or_else(String::new, |s| s.profile.id.clone())
    }

    fn memory_cache(&mut self) -> &mut MemoryCache {
        let profile = self.memory_profile();
        let cache = &mut self.sprites.memory;
        if cache.profile != profile {
            *cache = MemoryCache {
                profile,
                ..MemoryCache::default()
            };
        }
        cache
    }

    /// The palette label remembered for `key`.
    pub fn remembered_palette(&mut self, key: &str) -> Option<String> {
        let k = key.to_ascii_lowercase();
        if let Some(v) = self.memory_cache().file.get(&k) {
            return v.clone();
        }
        let profile = self.memory_profile();
        let found = self
            .user_db(|u| u.sprite_palette(&profile, &k))
            .unwrap_or(None);
        self.memory_cache().file.insert(k, found.clone());
        found
    }

    /// The palette label last used in `key`'s folder.
    pub fn folder_palette(&mut self, key: &str) -> Option<String> {
        let k = key.to_ascii_lowercase();
        if let Some(v) = self.memory_cache().folder.get(&k) {
            return v.clone();
        }
        let profile = self.memory_profile();
        let found = self
            .user_db(|u| u.folder_palette(&profile, &k))
            .unwrap_or(None);
        self.memory_cache().folder.insert(k, found.clone());
        found
    }

    /// Remembers `choice` for `key` (no-op when it already is).
    pub fn remember_palette(&mut self, key: &str, choice: &PaletteChoice) {
        let label = choice.memory_label();
        if self.remembered_palette(key).as_deref() == Some(label.as_str()) {
            return;
        }
        let profile = self.memory_profile();
        let k = key.to_ascii_lowercase();
        let result = self.user_db(|u| u.set_sprite_palette(&profile, &k, Some(&label)));
        let cache = self.memory_cache();
        cache.file.clear();
        cache.folder.clear();
        if let Err(e) = result {
            self.warn(format!("Cannot remember the palette of {key}: {e}"));
        }
    }

    /// The badge for a file key (`None`: a sprite without a file yet).
    pub fn palette_badge(&mut self, key: Option<&str>) -> Badge {
        let current = self.sprites.palette.choice.clone();
        match key {
            None => badge(&current, false, None, None),
            Some(k) => {
                let file = self.remembered_palette(k);
                let folder = if file.is_none() {
                    self.folder_palette(k)
                } else {
                    None
                };
                badge(&current, true, file.as_deref(), folder.as_deref())
            }
        }
    }

    /// On opening `key`: switch to its remembered palette.
    pub fn apply_remembered_palette(&mut self, key: Option<&str>) {
        let Some(k) = key else { return };
        let file = self.remembered_palette(k);
        if let Some(choice) = on_open(&self.sprites.palette.choice, file.as_deref()) {
            self.info(format!("Palette {} (remembered for {k})", choice.label()));
            self.sprites.palette.choice = choice;
            self.sync_editor_palette();
        }
    }

    /// The user picked `choice` while `key` was on show.
    pub fn pick_palette(&mut self, choice: PaletteChoice, key: Option<&str>) {
        self.sprites.palette.choice = choice.clone();
        self.sync_editor_palette();
        if let Some(k) = key {
            self.remember_palette(k, &choice);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_states() {
        let act1 = PaletteChoice::Act(0);
        let b = badge(&act1, true, Some("act1"), Some("act5"));
        assert_eq!(
            (b.source, b.tone, b.offer),
            (BadgeSource::Remembered, Tone::Good, None)
        );
        assert!(b.text.contains("Act 1") && b.text.contains("remembered for this file"));
        let b = badge(&act1, true, Some("act5"), None);
        assert_eq!(b.source, BadgeSource::RememberedOther);
        assert_eq!(b.offer, Some(PaletteChoice::Act(4)));
        assert_eq!(b.tone, Tone::Danger);
        let b = badge(&act1, true, None, Some("Act 1"));
        assert_eq!(b.source, BadgeSource::Folder);
        let b = badge(&act1, true, None, Some("units"));
        assert_eq!(b.source, BadgeSource::FolderOther);
        assert_eq!(b.offer.map(|c| c.memory_label()), Some("units".to_owned()));
        assert_eq!(badge(&act1, true, None, None).source, BadgeSource::None);
        assert_eq!(
            badge(&act1, false, Some("act5"), None).source,
            BadgeSource::None
        );
    }

    #[test]
    fn opening_switches_only_to_a_remembered_palette() {
        let act1 = PaletteChoice::Act(0);
        assert_eq!(on_open(&act1, Some("act5")), Some(PaletteChoice::Act(4)));
        assert_eq!(on_open(&act1, Some("act1")), None);
        assert_eq!(on_open(&act1, None), None);
        assert_eq!(mismatch(&act1, Some("ACT5")), Some(PaletteChoice::Act(4)));
        assert_eq!(mismatch(&act1, Some("act1")), None);
        assert_eq!(mismatch(&act1, None), None);
    }

    #[test]
    fn path_guesses() {
        assert_eq!(
            guess_from_path("data\\global\\tiles\\act5\\lava\\lavafloor.dt1"),
            Some(PaletteChoice::Act(4))
        );
        assert_eq!(
            guess_from_path("data\\global\\tiles\\expansion\\siege\\x.dt1"),
            Some(PaletteChoice::Act(4))
        );
        assert_eq!(
            guess_from_path("data\\global\\tiles\\act2\\a.dt1"),
            Some(PaletteChoice::Act(1))
        );
        assert_eq!(guess_from_path("data\\global\\ui\\panel\\x.dc6"), None);
        assert!(suggests_all_acts("data\\global\\ui\\SPELLS\\skillicon.dc6"));
        assert!(suggests_all_acts("data\\global\\items\\invfoo.dc6"));
        assert!(!suggests_all_acts("data\\global\\tiles\\act1\\a.dt1"));
    }

    #[test]
    fn remembering_through_the_state() {
        let tmp = tempfile::tempdir().expect("tmp");
        let paths = crate::paths::Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        let mut s = AppState::new(paths, crate::config::Config::default(), Vec::new());
        let key = "data\\global\\ui\\memtest\\a.dc6";
        let b = s.palette_badge(Some(key));
        assert_eq!(b.source, BadgeSource::None);
        s.pick_palette(PaletteChoice::Act(4), Some(key));
        assert_eq!(s.palette_badge(Some(key)).source, BadgeSource::Remembered);
        // A sibling file sees the folder's palette.
        let sibling = "data\\global\\ui\\memtest\\b.dc6";
        assert_eq!(s.palette_badge(Some(sibling)).source, BadgeSource::Folder);
        s.sprites.palette.choice = PaletteChoice::Act(0);
        assert_eq!(
            s.palette_badge(Some(sibling)).source,
            BadgeSource::FolderOther
        );
        assert_eq!(
            s.palette_badge(Some(key)).source,
            BadgeSource::RememberedOther
        );
        s.apply_remembered_palette(Some(key));
        assert_eq!(s.sprites.palette.choice, PaletteChoice::Act(4));
    }
}
