//! Objects and NPCs: the object tables, the animation clock, the "Add
//! object" picker, the NPC path sub-tool and the legacy ini import. The map
//! drawing of objects is in [`crate::render`]; the egui side in
//! `ui/objects.rs`.

pub mod catalog;
pub mod ini;
pub mod path_tool;
pub mod picker;

use std::path::PathBuf;
use std::sync::Arc;

use d2_assets::objects::{ObjectCatalog, ObjectInfo};
use d2_formats::ds1::{Object, PathPoint};

use self::catalog::{CatalogNews, CatalogSlot};
use self::path_tool::{PathCommand, PathTool};
use self::picker::PickerState;
use crate::edit::ops::EditOp;
use crate::state::{AppState, CatalogState};

/// The editor's animation clock: 25 ticks per second (`main.c:1842`).
pub const TICKS_PER_SECOND: f64 = 25.0;

/// The tick at `seconds` of app time.
pub fn tick_at(seconds: f64) -> u32 {
    let t = (seconds.max(0.0) * TICKS_PER_SECOND).floor();
    if t >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        t as u32
    }
}

/// Which animation tick the map shows: a pinned tick (script
/// `object-frame-tick`) wins, else the clock when "Animate objects" is on,
/// else none (frame 0).
pub fn shown_tick(animate: bool, pinned: Option<u32>, seconds: f64) -> Option<u32> {
    pinned.or_else(|| animate.then(|| tick_at(seconds)))
}

/// What the Objects tool does with a click.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjectMode {
    /// Select, drag, delete.
    #[default]
    Select,
    /// Place the picker's choice where clicked.
    Place,
    /// Edit the selected object's NPC path.
    Path,
}

pub struct ObjectsState {
    pub catalog: CatalogSlot,
    /// Script `object-frame-tick N`: show this animation tick.
    pub frame_tick: Option<u32>,
    pub mode: ObjectMode,
    pub picker: PickerState,
    pub path: PathTool,
    /// `(epoch, act, list)` of `ObjectCatalog::placeable`.
    placeables: Option<(u64, u8, Arc<Vec<ObjectInfo>>)>,
    /// Bring the Objects panel to the front (set when the tool is picked).
    pub focus: bool,
    /// App time in seconds, for the animation clock (set each frame).
    pub clock: f64,
    /// The picker's animated preview of its chosen entry.
    pub preview: Option<PickerPreview>,
    /// The Objects panel's edit buffer for the selected object.
    pub props: Option<PropsEdit>,
    /// "Import DS1Edit ini mappings...": the file dialog's answer.
    ini_picker: Option<std::sync::Mutex<std::sync::mpsc::Receiver<Option<Vec<PathBuf>>>>>,
}

/// The picker preview's frames for one `(epoch, kind, id, act, gamma)`.
pub struct PickerPreview {
    pub key: (u64, i32, i32, u8, u32),
    pub frames: Vec<Option<crate::sprites::viewer::Tex>>,
    /// Frames per 25 Hz tick, times 256.
    pub delta: u32,
    pub error: Option<String>,
}

/// The Objects panel's editable copy of one object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropsEdit {
    pub doc: u64,
    pub index: usize,
    /// The object as it was when the buffer was filled.
    pub original: Object,
    pub edited: Object,
}

impl Default for ObjectsState {
    fn default() -> Self {
        ObjectsState {
            catalog: CatalogSlot::None,
            frame_tick: None,
            mode: ObjectMode::default(),
            picker: PickerState::default(),
            path: PathTool::default(),
            placeables: None,
            focus: false,
            clock: 0.0,
            preview: None,
            props: None,
            ini_picker: None,
        }
    }
}

impl AppState {
    /// Starts the object tables of the open profile and logs when they are
    /// in. Called once per frame.
    pub fn poll_objects(&mut self) {
        self.poll_ini_picker();
        let epoch = self.session_epoch;
        if let Some(session) = &self.session {
            let (overlay, version) = (session.overlay.clone(), session.profile.game_version);
            self.objects.catalog.ensure(epoch, &overlay, version);
        }
        match self.objects.catalog.collect() {
            Some(CatalogNews::Ready { warnings }) => {
                for w in warnings.iter().take(10) {
                    self.warn(format!("Object tables: {w}"));
                }
                self.info(format!(
                    "Object tables ready ({} warning(s)); objects draw as sprites",
                    warnings.len()
                ));
            }
            Some(CatalogNews::Failed(e)) => {
                self.error(format!("Object tables: {e}; objects show as markers only"));
            }
            None => {}
        }
    }

    /// True once `doc` is drawn with its objects, or never will be (no
    /// objects, objects hidden, or the object tables failed to load).
    pub fn objects_settled(&self, doc: &crate::doc::MapDoc) -> bool {
        doc.ds1.objects.is_empty()
            || !doc.objects_visible
            || doc.render.objects_epoch == Some(self.session_epoch)
            || matches!(
                self.objects.catalog.poll(self.session_epoch),
                catalog::CatalogPoll::Failed(_)
            )
    }

    /// The object tables of the open profile, once built.
    pub fn object_catalog(&self) -> Option<Arc<ObjectCatalog>> {
        self.objects.catalog.ready(self.session_epoch)
    }

    /// `ObjectCatalog::placeable(act)`, cached per session and act.
    pub fn placeables(&mut self, act: u8) -> Option<Arc<Vec<ObjectInfo>>> {
        let epoch = self.session_epoch;
        if let Some((e, a, list)) = &self.objects.placeables
            && *e == epoch
            && *a == act
        {
            return Some(list.clone());
        }
        let catalog = self.object_catalog()?;
        let list = Arc::new(catalog.placeable(act));
        self.objects.placeables = Some((epoch, act, list.clone()));
        Some(list)
    }

    /// Places a `(kind, id)` object at sub-tile `(x, y)` of document `index`
    /// and selects it. Errors (an ambiguous NPC path anchor) are reported; an
    /// old DS1 version opens the upgrade question.
    pub fn place_object(&mut self, index: usize, kind: i32, id: i32, x: i32, y: i32) -> bool {
        let object = Object {
            kind,
            id,
            x,
            y,
            flags: 0,
        };
        self.exec(index, EditOp::AddObject(object))
    }

    /// The selected object of the active map when exactly one is selected.
    pub fn single_object(&self) -> Option<(usize, usize)> {
        let index = self.active?;
        match self.docs.get(index)?.object_sel.as_slice() {
            [object] => Some((index, *object)),
            _ => None,
        }
    }

    /// Runs a path command of the Path sub-tool on the selected object.
    pub fn run_path_command(&mut self, command: PathCommand) -> bool {
        let Some((index, object)) = self.single_object() else {
            self.notify("Select one object first");
            return false;
        };
        let op = match command {
            PathCommand::Add { point, at } => EditOp::AddPathPoint { object, point, at },
            PathCommand::Move { index: i, x, y } => EditOp::MovePathPoint {
                object,
                index: i,
                x,
                y,
            },
            PathCommand::Delete { index: i } => EditOp::DeletePathPoints {
                object,
                indices: vec![i],
            },
        };
        self.exec(index, op)
    }

    /// Points of the selected object's NPC path (empty when it has none).
    pub fn selected_path(&self) -> Vec<PathPoint> {
        self.single_object()
            .and_then(|(i, o)| self.docs[i].edit.path_of(o))
            .map(|p| p.points.clone())
            .unwrap_or_default()
    }

    /// File > "Import DS1Edit ini mappings...": a native multi-file picker
    /// on a worker thread; [`AppState::poll_ini_picker`] imports the answer.
    pub fn pick_ini_files(&mut self) {
        if self.objects.ini_picker.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("ini-picker".to_owned())
            .spawn(move || {
                let picked = rfd::FileDialog::new()
                    .set_title("Import DS1Edit ini mappings")
                    .add_filter("DS1Edit ini", &["ini", "txt"])
                    .pick_files();
                let _ = tx.send(picked);
            });
        if spawned.is_ok() {
            self.objects.ini_picker = Some(std::sync::Mutex::new(rx));
        }
    }

    fn poll_ini_picker(&mut self) {
        let answer = match &self.objects.ini_picker {
            Some(rx) => match rx.lock().map(|r| r.try_recv()) {
                Ok(Ok(answer)) => answer,
                Ok(Err(std::sync::mpsc::TryRecvError::Empty)) => return,
                _ => None,
            },
            None => return,
        };
        self.objects.ini_picker = None;
        if let Some(files) = answer.filter(|f| !f.is_empty())
            && let Err(e) = self.import_ini_files(&files)
        {
            self.error(format!("Import DS1Edit ini mappings: {e}"));
        }
    }

    /// File > "Import DS1Edit ini mappings...": imports the files into the
    /// map catalog and `config.toml` of the open profile.
    pub fn import_ini_files(&mut self, files: &[PathBuf]) -> Result<ini::IniImport, String> {
        let profile = self
            .session
            .as_ref()
            .map(|s| s.profile.id.clone())
            .ok_or("no profile is open")?;
        let mut texts = Vec::new();
        let mut unreadable = Vec::new();
        for f in files {
            let name = f
                .file_name()
                .map_or_else(|| f.display().to_string(), |n| n.to_string_lossy().into_owned());
            match std::fs::read(f) {
                // DS1Edit's files are plain ASCII; anything else is read leniently.
                Ok(bytes) => texts.push((name, String::from_utf8_lossy(&bytes).into_owned())),
                Err(e) => unreadable.push(format!("{}: {e}", f.display())),
            }
        }
        let CatalogState::Ready(catalog) = &mut self.catalog else {
            return Err("the map catalog is not ready yet".to_owned());
        };
        let mut report = ini::import(&mut self.config, catalog, &profile, &texts);
        report.files = files.len();
        report.warnings.extend(unreadable);
        self.config_dirty = true;
        for w in &report.warnings {
            self.warn(format!("ini import: {w}"));
        }
        for path in report.maps.clone() {
            self.refresh_infos_for(&path, &profile);
        }
        self.notify(report.summary());
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_runs_at_25_hz_and_off_means_frame_0() {
        assert_eq!(tick_at(0.0), 0);
        assert_eq!(tick_at(0.039), 0);
        assert_eq!(tick_at(0.04), 1);
        assert_eq!(tick_at(2.0), 50);
        assert_eq!(tick_at(-3.0), 0);
        assert_eq!(tick_at(1e12), u32::MAX);
        assert_eq!(shown_tick(false, None, 9.0), None);
        assert_eq!(shown_tick(true, None, 2.0), Some(50));
        assert_eq!(shown_tick(false, Some(7), 2.0), Some(7), "a pinned tick wins");
        assert_eq!(shown_tick(true, Some(7), 2.0), Some(7));
    }
}
