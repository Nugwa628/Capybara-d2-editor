//! An open map: the parsed DS1 plus view and visibility state.

use std::collections::BTreeSet;
use std::sync::Arc;

use capy_edit::{Document, LayerRef, Selection};
use capy_render::{TileSet, Visibility};
use d2_assets::{GamePath, MapInfo};
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

    /// The editing model's name for this layer.
    pub fn to_ref(self) -> LayerRef {
        match self {
            LayerKey::Floor(i) => LayerRef::Floor(u8::try_from(i).unwrap_or(u8::MAX)),
            LayerKey::Wall(i) => LayerRef::Wall(u8::try_from(i).unwrap_or(u8::MAX)),
            LayerKey::Shadow => LayerRef::Shadow,
            LayerKey::Tag => LayerRef::Tag,
        }
    }

    pub fn from_ref(layer: LayerRef) -> LayerKey {
        match layer {
            LayerRef::Floor(i) => LayerKey::Floor(usize::from(i)),
            LayerRef::Wall(i) => LayerKey::Wall(usize::from(i)),
            LayerRef::Shadow => LayerKey::Shadow,
            LayerRef::Tag => LayerKey::Tag,
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

/// Where the textured rendering of a map stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderState {
    /// The map catalog has not told us the LvlType and DT1 list yet.
    WaitingForContext,
    /// The DT1s, atlas or mesh are being built.
    Loading,
    Ready,
    Failed(String),
    /// The map belongs to a profile other than the one now open.
    ProfileClosed,
}

/// What the last finished build produced, for the Properties panel.
#[derive(Clone, Debug)]
pub struct RenderStatus {
    pub state: RenderState,
    /// Visible non-empty cells (per layer) with no tile.
    pub missing: usize,
    /// The most frequent missing keys, as text.
    pub missing_top: Vec<String>,
    pub tiles_drawn: usize,
    /// `(page size, layers)` of the atlas.
    pub atlas: Option<(u32, u32)>,
    /// DT1s that could not be read.
    pub warnings: Vec<String>,
    pub millis: u64,
}

impl Default for RenderStatus {
    fn default() -> Self {
        RenderStatus {
            state: RenderState::WaitingForContext,
            missing: 0,
            missing_top: Vec::new(),
            tiles_drawn: 0,
            atlas: None,
            warnings: Vec::new(),
            millis: 0,
        }
    }
}

impl RenderStatus {
    pub fn is_ready(&self) -> bool {
        self.state == RenderState::Ready
    }
}

/// Display name of a layer reference ("Wall 3").
pub fn layer_label(layer: LayerRef) -> String {
    LayerKey::from_ref(layer).label()
}

pub struct MapDoc {
    pub id: u64,
    pub profile_id: String,
    pub path: GamePath,
    pub source: SourceInfo,
    /// The editing model: the DS1 plus its undo history.
    pub edit: Document,
    /// A copy of `edit.ds1()` for the render worker, refreshed by
    /// [`MapDoc::sync`] whenever the document's revision moved.
    pub ds1: Arc<Ds1>,
    /// The document revision `ds1` was copied at.
    pub synced_revision: u64,
    /// Bumped whenever the geometry must be rebuilt.
    pub rev: u64,
    pub selection: Selection,
    /// Indices into `ds1.objects` of the selected objects.
    pub object_sel: Vec<usize>,
    pub active_layer: LayerRef,
    /// The parsed DT1 set the map was last drawn with (thumbnails).
    pub tiles: Option<Arc<TileSet>>,
    pub hidden: BTreeSet<LayerKey>,
    pub objects_visible: bool,
    /// Roof tiles (orientation 15), drawn by default.
    pub roofs_visible: bool,
    /// Special tiles (orientation 10 and 11), hidden by default.
    pub special_visible: bool,
    /// How the game would load this map; `None` until the catalog is ready.
    pub info: Option<MapInfo>,
    pub render: RenderStatus,
    pub view: View,
    /// False until the view has been fitted to the viewport once.
    pub fitted: bool,
    /// The document revision last recorded by autosave.
    pub autosaved_revision: Option<u64>,
    /// While set, `ds1` is this history snapshot (shown read-only), not the
    /// edited map.
    pub snapshot_view: Option<capy_history::SnapshotId>,
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
        let edit = Document::new(ds1.clone());
        let synced_revision = edit.revision();
        let active_layer = if ds1.walls.is_empty() {
            LayerRef::Floor(0)
        } else {
            LayerRef::Wall(0)
        };
        MapDoc {
            id,
            profile_id,
            path,
            source,
            edit,
            ds1: Arc::new(ds1),
            synced_revision,
            rev: 1,
            selection: Selection::new(),
            object_sel: Vec::new(),
            active_layer,
            tiles: None,
            hidden,
            objects_visible: defaults.objects,
            roofs_visible: true,
            special_visible: false,
            info: None,
            render: RenderStatus::default(),
            view: View::default(),
            fitted: false,
            autosaved_revision: None,
            snapshot_view: None,
        }
    }

    /// Copy the edited model for the renderer when it changed, and repair
    /// state that named things the edit removed. Cheap when nothing moved.
    pub fn sync(&mut self) {
        let revision = self.edit.revision();
        if revision == self.synced_revision {
            return;
        }
        // An edit ends a snapshot view: the map shows what was edited.
        self.snapshot_view = None;
        self.synced_revision = revision;
        self.ds1 = Arc::new(self.edit.ds1().clone());
        self.rev += 1;
        let keys = layer_keys(&self.ds1);
        self.hidden.retain(|k| keys.contains(k));
        if !self.active_layer.exists_in(&self.ds1) {
            self.active_layer = LayerRef::Floor(0);
        }
        let objects = self.ds1.objects.len();
        self.object_sel.retain(|&i| i < objects);
    }

    /// Show a history snapshot in place of the map (read-only).
    pub fn show_snapshot(&mut self, id: capy_history::SnapshotId, ds1: Arc<Ds1>) {
        if self.snapshot_view == Some(id) {
            return;
        }
        self.snapshot_view = Some(id);
        self.ds1 = ds1;
        self.after_view_change();
    }

    /// Back from a snapshot view to the edited map.
    pub fn show_current(&mut self) {
        if self.snapshot_view.take().is_none() {
            return;
        }
        self.ds1 = Arc::new(self.edit.ds1().clone());
        self.after_view_change();
    }

    fn after_view_change(&mut self) {
        self.rev += 1;
        let keys = layer_keys(&self.ds1);
        self.hidden.retain(|k| keys.contains(k));
        let objects = self.ds1.objects.len();
        self.object_sel.retain(|&i| i < objects);
    }

    /// Tell the document where it came from, for the clipboard's source
    /// record and the act / LvlType warnings of a paste.
    pub fn sync_clip_context(&mut self) {
        let lvl_type = self.info.as_ref().and_then(|i| i.lvl_type);
        self.edit
            .set_clip_context(Some(self.path.to_string()), lvl_type);
    }

    pub fn is_dirty(&self) -> bool {
        self.edit.is_dirty()
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

    /// The layer toggles in the form `capy-render` takes.
    pub fn visibility(&self) -> Visibility {
        let mut vis = Visibility {
            roofs: self.roofs_visible,
            special: self.special_visible,
            objects: self.objects_visible,
            shadow: self.is_visible(LayerKey::Shadow),
            ..Visibility::default()
        };
        for (i, v) in vis.floors.iter_mut().enumerate() {
            *v = self.is_visible(LayerKey::Floor(i));
        }
        for (i, v) in vis.walls.iter_mut().enumerate() {
            *v = self.is_visible(LayerKey::Wall(i));
        }
        vis
    }

    /// File name for the tab, with the dirty marker.
    pub fn title(&self) -> String {
        let name = self.path.file_name();
        if self.is_dirty() {
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
    fn visibility_maps_layer_toggles_for_the_renderer() {
        let mut doc = MapDoc::new(
            1,
            "p".into(),
            GamePath::new("data\\a.ds1"),
            SourceInfo {
                category: SourceCategory::Base,
                layer_name: "d2data.mpq".into(),
            },
            Ds1::new(2, 2, 0),
            &LayerVisibility::default(),
        );
        let vis = doc.visibility();
        assert_eq!(vis, Visibility::default(), "special tiles start hidden");
        doc.set_visible(LayerKey::Floor(1), false);
        doc.set_visible(LayerKey::Wall(2), false);
        doc.set_visible(LayerKey::Shadow, false);
        doc.special_visible = true;
        doc.roofs_visible = false;
        let vis = doc.visibility();
        assert_eq!(vis.floors, [true, false]);
        assert_eq!(vis.walls, [true, true, false, true]);
        assert!(!vis.shadow && !vis.roofs && vis.special);
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
