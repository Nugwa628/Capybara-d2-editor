//! An open map: the parsed DS1 plus view and visibility state.

use std::collections::BTreeSet;
use std::sync::Arc;

use d2_assets::GamePath;
use d2_formats::ds1::Ds1;

use crate::config::LayerVisibility;
use crate::iso::View;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Floor,
    Wall,
    Shadow,
    Tag,
}

/// One drawable layer of a map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LayerKey {
    Floor(usize),
    Wall(usize),
    Shadow,
    Tag,
}

impl LayerKey {
    pub fn kind(self) -> LayerKind {
        match self {
            LayerKey::Floor(_) => LayerKind::Floor,
            LayerKey::Wall(_) => LayerKind::Wall,
            LayerKey::Shadow => LayerKind::Shadow,
            LayerKey::Tag => LayerKind::Tag,
        }
    }

    pub fn label(self) -> String {
        match self {
            LayerKey::Floor(i) => format!("Floor {}", i + 1),
            LayerKey::Wall(i) => format!("Wall {}", i + 1),
            LayerKey::Shadow => "Shadow".to_owned(),
            LayerKey::Tag => "Tag".to_owned(),
        }
    }
}

/// Every layer present in a map, in draw order (floors, shadow, walls, tag).
pub fn layer_keys(ds1: &Ds1) -> Vec<LayerKey> {
    let mut keys: Vec<LayerKey> = (0..ds1.floors.len()).map(LayerKey::Floor).collect();
    keys.push(LayerKey::Shadow);
    keys.extend((0..ds1.walls.len()).map(LayerKey::Wall));
    if ds1.tag.is_some() {
        keys.push(LayerKey::Tag);
    }
    keys
}

/// Where a file was found, for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceCategory {
    Saves,
    Mod,
    Base,
}

impl SourceCategory {
    pub fn label(self) -> &'static str {
        match self {
            SourceCategory::Saves => "saves",
            SourceCategory::Mod => "mod",
            SourceCategory::Base => "base",
        }
    }
}

/// Classify an overlay layer by its source name: the profile's saves folder is
/// "saves", Diablo II's own archives are "base", everything else is "mod".
pub fn categorize_source(name: &str, is_folder: bool) -> SourceCategory {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("saves\\") || lower == "saves" {
        return SourceCategory::Saves;
    }
    if is_folder {
        return SourceCategory::Mod;
    }
    let stem = lower.strip_suffix(".mpq").unwrap_or(&lower);
    match stem {
        "d2data" | "d2exp" | "d2char" | "patch_d2" | "d2sfx" | "d2music" | "d2speech"
        | "d2video" | "d2xmusic" | "d2xtalk" | "d2xvideo" => SourceCategory::Base,
        _ => SourceCategory::Mod,
    }
}

#[derive(Clone, Debug)]
pub struct SourceInfo {
    pub category: SourceCategory,
    pub layer_name: String,
}

pub struct MapDoc {
    pub id: u64,
    pub profile_id: String,
    pub path: GamePath,
    pub source: SourceInfo,
    pub ds1: Arc<Ds1>,
    /// Bumped whenever the geometry must be rebuilt.
    pub rev: u64,
    pub hidden: BTreeSet<LayerKey>,
    pub objects_visible: bool,
    pub view: View,
    /// False until the view has been fitted to the viewport once.
    pub fitted: bool,
    /// Placeholder until capy-edit/capy-history land.
    pub dirty: bool,
}

impl MapDoc {
    pub fn new(
        id: u64,
        profile_id: String,
        path: GamePath,
        source: SourceInfo,
        ds1: Ds1,
        defaults: &LayerVisibility,
    ) -> MapDoc {
        let mut hidden = BTreeSet::new();
        for key in layer_keys(&ds1) {
            let visible = match key.kind() {
                LayerKind::Floor => defaults.floor,
                LayerKind::Wall => defaults.wall,
                LayerKind::Shadow => defaults.shadow,
                LayerKind::Tag => defaults.tag,
            };
            if !visible {
                hidden.insert(key);
            }
        }
        MapDoc {
            id,
            profile_id,
            path,
            source,
            ds1: Arc::new(ds1),
            rev: 1,
            hidden,
            objects_visible: defaults.objects,
            view: View::default(),
            fitted: false,
            dirty: false,
        }
    }

    pub fn keys(&self) -> Vec<LayerKey> {
        layer_keys(&self.ds1)
    }

    pub fn is_visible(&self, key: LayerKey) -> bool {
        !self.hidden.contains(&key)
    }

    pub fn set_visible(&mut self, key: LayerKey, visible: bool) {
        if visible {
            self.hidden.remove(&key);
        } else {
            self.hidden.insert(key);
        }
    }

    /// File name for the tab, with the dirty marker.
    pub fn title(&self) -> String {
        let name = self.path.file_name();
        if self.dirty {
            format!("\u{25CF} {name}")
        } else {
            name.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        assert_eq!(categorize_source("saves\\mxl", true), SourceCategory::Saves);
        assert_eq!(categorize_source("d2data.mpq", false), SourceCategory::Base);
        assert_eq!(
            categorize_source("Patch_D2.mpq", false),
            SourceCategory::Base
        );
        assert_eq!(
            categorize_source("medianxl-dGlsZXM.mpq", false),
            SourceCategory::Mod
        );
        assert_eq!(categorize_source("C:\\mods\\x", true), SourceCategory::Mod);
    }

    #[test]
    fn keys_follow_the_map() {
        let mut ds1 = Ds1::new(4, 4, 0);
        let keys = layer_keys(&ds1);
        assert_eq!(
            keys,
            vec![LayerKey::Floor(0), LayerKey::Shadow, LayerKey::Wall(0)]
        );
        assert!(ds1.add_wall_layer().is_ok());
        assert_eq!(layer_keys(&ds1).len(), 4);
    }

    #[test]
    fn default_visibility_applies_per_kind() {
        let defaults = LayerVisibility {
            shadow: false,
            ..LayerVisibility::default()
        };
        let doc = MapDoc::new(
            1,
            "p".into(),
            GamePath::new("data\\a.ds1"),
            SourceInfo {
                category: SourceCategory::Base,
                layer_name: "d2data.mpq".into(),
            },
            Ds1::new(2, 2, 0),
            &defaults,
        );
        assert!(doc.is_visible(LayerKey::Floor(0)));
        assert!(!doc.is_visible(LayerKey::Shadow));
        assert_eq!(doc.title(), "a.ds1");
    }
}
