//! The command registry: every action of the Map Editor with its id, label,
//! icon, default key and "enabled when" rule. The ribbon, the map's
//! right-click menu, the Ctrl+K command finder and the keyboard shortcuts are
//! all built from it.
//!
//! Ids and default keys follow ds1-studio's `src/ui/keybindings.ts`. Its
//! commands whose feature Capybara does not have yet stay in the table
//! (`available: false`) so their keys are reserved and later phases only
//! switch them on. Capybara's own commands (lasso, select by tile ID, select
//! identical, flags, ...) are added after them.
//!
//! Bindings: the user's changes live in `config.toml` (`[ui.keys]`, only the
//! ids that differ from the default; `""` = no key). Binding a key takes it
//! away from any command that had it, so every key does one thing.

use std::collections::BTreeMap;
use std::fmt;

use bevy_egui::egui::{self, Key, Modifiers};
use capy_edit::LayerRef;

use super::theme::icon;
use crate::config::AppMode;
use crate::doc::LayerKey;
use crate::edit::ops::EditOp;
use crate::edit::tools::Tool;
use crate::iso::ZOOM_STEPS;
use crate::objects::ObjectMode;
use crate::state::AppState;

/// One entry of the registry.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    /// The heading it is listed under (Shortcuts dialog, command finder).
    pub group: &'static str,
    pub icon: char,
    /// Default key, ds1-studio's notation (`Ctrl+Shift+Z`); `""` for none.
    pub key: &'static str,
    /// The feature exists in this version. Unavailable commands keep their
    /// default key reserved but never run.
    pub available: bool,
    /// Opens a window outside the app (a file dialog, Explorer). Scripted
    /// "click everything" runs skip these.
    pub external: bool,
}

const fn cmd(
    id: &'static str,
    label: &'static str,
    group: &'static str,
    icon: char,
    key: &'static str,
) -> Command {
    Command {
        id,
        label,
        group,
        icon,
        key,
        available: true,
        external: false,
    }
}

/// A ds1-studio command that Capybara does not have yet.
const fn later(
    id: &'static str,
    label: &'static str,
    group: &'static str,
    key: &'static str,
) -> Command {
    Command {
        id,
        label,
        group,
        icon: icon::NONE,
        key,
        available: false,
        external: false,
    }
}

const fn external(cmd: Command) -> Command {
    Command {
        external: true,
        ..cmd
    }
}

/// The registry, ds1-studio's table first (in its order), then Capybara's.
pub const COMMANDS: &[Command] = &[
    // Tools
    cmd(
        "tool.select",
        "Select tool",
        "Tools",
        icon::MOUSE_POINTER,
        "V",
    ),
    cmd(
        "tool.paint",
        "Paint tool (freehand)",
        "Tools",
        icon::PAINTBRUSH,
        "B",
    ),
    later("tool.rect", "Paint / erase a rectangle", "Tools", "U"),
    later(
        "tool.fill",
        "Flood fill / erase a connected area",
        "Tools",
        "L",
    ),
    cmd("tool.erase", "Erase tool", "Tools", icon::ERASER, "E"),
    cmd("tool.pick", "Pick tool", "Tools", icon::PIPETTE, "I"),
    cmd("tool.object", "Objects tool", "Tools", icon::BOX, "O"),
    cmd(
        "tool.toggleObjects",
        "Toggle tiles / objects editing",
        "Tools",
        icon::BOXES,
        "Shift+O",
    ),
    // Edit
    cmd("edit.undo", "Undo", "Edit", icon::UNDO, "Ctrl+Z"),
    cmd("edit.redo", "Redo", "Edit", icon::REDO, "Ctrl+Y"),
    cmd(
        "edit.redo2",
        "Redo (alternative)",
        "Edit",
        icon::REDO,
        "Ctrl+Shift+Z",
    ),
    cmd("file.save", "Save", "File", icon::SAVE, "Ctrl+S"),
    cmd("edit.copy", "Copy", "Edit", icon::COPY, "Ctrl+C"),
    cmd("edit.cut", "Cut", "Edit", icon::SCISSORS, "Ctrl+X"),
    cmd(
        "edit.paste",
        "Paste",
        "Edit",
        icon::CLIPBOARD_PASTE,
        "Ctrl+V",
    ),
    cmd(
        "edit.selectAll",
        "Select all on the active layer",
        "Edit",
        icon::SQUARE_DASHED_POINTER,
        "Ctrl+A",
    ),
    cmd(
        "edit.cancel",
        "Cancel / drop the paste / deselect",
        "Edit",
        icon::CIRCLE_X,
        "Escape",
    ),
    cmd(
        "edit.delete",
        "Delete (active layer / object)",
        "Edit",
        icon::TRASH,
        "Delete",
    ),
    later(
        "edit.deleteAll",
        "Delete everything (all layers + objects)",
        "Edit",
        "Shift+Delete",
    ),
    later("edit.replace", "Find & replace tiles", "Edit", "Ctrl+H"),
    // View
    later("view.next", "Next view", "View", "Tab"),
    later("view.prev", "Previous view", "View", "Shift+Tab"),
    cmd("view.fit", "Fit map", "View", icon::MAXIMIZE, "F"),
    later(
        "view.game",
        "Game view (what the character sees)",
        "View",
        "Z",
    ),
    cmd(
        "view.zoom100",
        "Zoom to 100% (one game pixel per screen pixel)",
        "View",
        icon::SEARCH,
        "Home",
    ),
    cmd("view.zoomIn", "Zoom in a step", "View", icon::ZOOM_IN, "="),
    cmd(
        "view.zoomOut",
        "Zoom out a step",
        "View",
        icon::ZOOM_OUT,
        "-",
    ),
    cmd("view.grid", "Grid", "View", icon::GRID, "G"),
    later("view.rooms", "Game rooms (8x8)", "View", "R"),
    later("view.walkable", "Walkability view", "View", "W"),
    later("view.automap", "Automap view", "View", "A"),
    cmd("view.markers", "Object markers", "View", icon::EYE, "M"),
    cmd("view.sprites", "Object sprites", "View", icon::BOX, "N"),
    cmd(
        "view.animate",
        "Animate objects",
        "View",
        icon::WAVES,
        "Shift+N",
    ),
    later("view.paths", "NPC paths", "View", "P"),
    cmd("view.minimap", "Minimap", "View", icon::MAP_PINNED, "K"),
    later("view.pops", "Roof hiding view", "View", "H"),
    later(
        "view.popsInside",
        "Show roofs as hidden (as if inside)",
        "View",
        "Shift+H",
    ),
    later("view.light", "Level light view", "View", "Shift+L"),
    cmd(
        "view.focus",
        "Just the map (fold both side panels, or bring them back)",
        "View",
        icon::FULLSCREEN,
        "Shift+F",
    ),
    cmd(
        "app.commands",
        "Find a command by name",
        "View",
        icon::SEARCH,
        "Ctrl+K",
    ),
    later(
        "view.snapshot",
        "Copy the map view as a picture",
        "View",
        "PrintScreen",
    ),
    // Layers (visibility)
    cmd("layer.floor1", "Floor 1", "Layers", icon::LAYERS, "1"),
    cmd("layer.floor2", "Floor 2", "Layers", icon::LAYERS, "2"),
    cmd("layer.wall1", "Wall 1", "Layers", icon::LAYERS, "3"),
    cmd("layer.wall2", "Wall 2", "Layers", icon::LAYERS, "4"),
    cmd("layer.wall3", "Wall 3", "Layers", icon::LAYERS, "5"),
    cmd("layer.wall4", "Wall 4", "Layers", icon::LAYERS, "6"),
    cmd("layer.shadows", "Shadows", "Layers", icon::LAYERS, "7"),
    cmd("layer.roofs", "Roofs", "Layers", icon::LAYERS, "8"),
    later("layer.lowerWalls", "Lower walls", "Layers", "9"),
    cmd(
        "layer.specials",
        "Special tiles",
        "Layers",
        icon::LAYERS,
        "0",
    ),
    cmd(
        "solo.floor1",
        "Show only Floor 1 (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+1",
    ),
    cmd(
        "solo.floor2",
        "Show only Floor 2 (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+2",
    ),
    cmd(
        "solo.wall1",
        "Show only Wall 1 (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+3",
    ),
    cmd(
        "solo.wall2",
        "Show only Wall 2 (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+4",
    ),
    cmd(
        "solo.wall3",
        "Show only Wall 3 (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+5",
    ),
    cmd(
        "solo.wall4",
        "Show only Wall 4 (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+6",
    ),
    cmd(
        "solo.shadows",
        "Show only Shadows (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+7",
    ),
    cmd(
        "solo.roofs",
        "Show only Roofs (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+8",
    ),
    later(
        "solo.lowerWalls",
        "Show only Lower walls (again: back as it was)",
        "Layers",
        "Shift+9",
    ),
    cmd(
        "solo.specials",
        "Show only Special tiles (again: back as it was)",
        "Layers",
        icon::EYE,
        "Shift+0",
    ),
    cmd(
        "layer.showAll",
        "Show every layer",
        "Layers",
        icon::EYE,
        "Backspace",
    ),
    // Active layer
    cmd(
        "active.floor1",
        "Work on Floor 1",
        "Active layer",
        icon::LAYERS,
        "Ctrl+1",
    ),
    cmd(
        "active.floor2",
        "Work on Floor 2",
        "Active layer",
        icon::LAYERS,
        "Ctrl+2",
    ),
    cmd(
        "active.wall1",
        "Work on Wall 1",
        "Active layer",
        icon::LAYERS,
        "Ctrl+3",
    ),
    cmd(
        "active.wall2",
        "Work on Wall 2",
        "Active layer",
        icon::LAYERS,
        "Ctrl+4",
    ),
    cmd(
        "active.wall3",
        "Work on Wall 3",
        "Active layer",
        icon::LAYERS,
        "Ctrl+5",
    ),
    cmd(
        "active.wall4",
        "Work on Wall 4",
        "Active layer",
        icon::LAYERS,
        "Ctrl+6",
    ),
    cmd(
        "active.shadows",
        "Work on Shadows",
        "Active layer",
        icon::LAYERS,
        "Ctrl+7",
    ),
    // Modes
    cmd("mode.tiles", "Tiles mode", "Modes", icon::PAINTBRUSH, "F1"),
    cmd("mode.objects", "Objects mode", "Modes", icon::BOX, "F2"),
    later("mode.walk", "Walkability mode", "Modes", "F3"),
    later("mode.automap", "Automap mode", "Modes", "F4"),
    later("mode.light", "Level light mode", "Modes", "F5"),
    later("mode.roofs", "Roof hiding mode", "Modes", "F6"),
    // ---- Capybara's own ----
    cmd("tool.lasso", "Lasso selection", "Select", icon::LASSO, "Q"),
    cmd(
        "tool.selectById",
        "Select by tile ID",
        "Select",
        icon::HASH,
        "T",
    ),
    cmd(
        "tool.selectIdentical",
        "Select identical",
        "Select",
        icon::COPY_CHECK,
        "S",
    ),
    cmd(
        "select.scopeActive",
        "Selection tools look at the active layer",
        "Select",
        icon::LAYERS,
        "",
    ),
    cmd(
        "select.scopeVisible",
        "Selection tools look at every visible layer",
        "Select",
        icon::LAYERS,
        "",
    ),
    cmd("edit.deselect", "Deselect", "Edit", icon::X, "Ctrl+D"),
    cmd(
        "edit.commitPaste",
        "Place the paste",
        "Edit",
        icon::CHECK,
        "Enter",
    ),
    cmd(
        "edit.moveToLayer",
        "Move selection to layer...",
        "Edit",
        icon::ARROW_RIGHT_LEFT,
        "",
    ),
    cmd(
        "flags.setHidden",
        "Set the hidden flag on the selection",
        "Edit",
        icon::FLAG,
        "",
    ),
    cmd(
        "flags.clearHidden",
        "Clear the hidden flag on the selection",
        "Edit",
        icon::FLAG,
        "",
    ),
    cmd(
        "flags.setUnwalkable",
        "Set the unwalkable flag on the selection",
        "Edit",
        icon::FLAG,
        "",
    ),
    cmd(
        "flags.clearUnwalkable",
        "Clear the unwalkable flag on the selection",
        "Edit",
        icon::FLAG,
        "",
    ),
    cmd(
        "objects.select",
        "Objects: select and move",
        "Tools",
        icon::MOUSE_POINTER,
        "",
    ),
    cmd("objects.add", "Objects: add", "Tools", icon::PLUS, ""),
    cmd(
        "objects.path",
        "Objects: NPC path",
        "Tools",
        icon::ROUTE,
        "",
    ),
    cmd("file.close", "Close map", "File", icon::FILE_X, ""),
    external(cmd(
        "file.openSaves",
        "Open saves folder",
        "File",
        icon::FOLDER_OPEN,
        "",
    )),
    cmd(
        "file.markPacked",
        "Mark as packed (empty CHANGED.txt)",
        "File",
        icon::PACKAGE_CHECK,
        "",
    ),
    external(cmd(
        "file.importIni",
        "Import DS1Edit ini mappings...",
        "File",
        icon::FILE_INPUT,
        "",
    )),
    cmd(
        "file.preferences",
        "Preferences...",
        "File",
        icon::SETTINGS,
        "",
    ),
    cmd(
        "file.addInstall",
        "Add game install...",
        "File",
        icon::PLUS,
        "",
    ),
    cmd("app.quit", "Quit", "File", icon::LOG_OUT, ""),
    cmd("map.resize", "Resize map...", "Map", icon::EXPAND, ""),
    cmd("map.info", "Map info", "Map", icon::INFO, ""),
    cmd("map.addFloor", "Add floor layer", "Map", icon::PLUS, ""),
    cmd("map.addWall", "Add wall layer", "Map", icon::PLUS, ""),
    cmd("map.addTag", "Add tag layer", "Map", icon::PLUS, ""),
    cmd(
        "view.foldLeft",
        "Fold the map list",
        "View",
        icon::PANEL_LEFT,
        "",
    ),
    cmd(
        "view.foldRight",
        "Fold the tiles column",
        "View",
        icon::PANEL_RIGHT,
        "",
    ),
    cmd("view.log", "Log window", "View", icon::SCROLL_TEXT, ""),
    cmd(
        "view.objectsWindow",
        "Objects window",
        "View",
        icon::BOX,
        "",
    ),
    cmd(
        "view.spritePreview",
        "Sprite preview window",
        "View",
        icon::IMAGE,
        "",
    ),
    cmd("view.history", "History", "View", icon::HISTORY, ""),
    cmd(
        "app.sprites",
        "Switch to Sprites mode",
        "View",
        icon::IMAGE,
        "",
    ),
    cmd(
        "app.assetSearch",
        "Search every asset...",
        "View",
        icon::SCAN_SEARCH,
        "",
    ),
    cmd("help.shortcuts", "Shortcuts", "Help", icon::KEYBOARD, ""),
    cmd("help.about", "About", "Help", icon::INFO, ""),
    // ---- Tile libraries, presets, game data, diagnostics, packages ----
    cmd(
        "tiles.dt1Manager",
        "DT1 libraries (duplicate, rename, recolour)...",
        "Tile libraries",
        icon::FOLDER,
        "",
    ),
    cmd(
        "tiles.settings",
        "Tile settings (.ini)...",
        "Tile libraries",
        icon::SETTINGS,
        "",
    ),
    cmd(
        "tiles.collision",
        "Collision flags of the 5x5 sub-tiles...",
        "Tile libraries",
        icon::FLAG,
        "",
    ),
    cmd(
        "tiles.paint",
        "Paint tile pixels...",
        "Tile libraries",
        icon::PAINTBRUSH,
        "",
    ),
    cmd(
        "tiles.actSafe",
        "Make a library act-safe...",
        "Tile libraries",
        icon::PALETTE,
        "",
    ),
    cmd(
        "tiles.review",
        "Review the map's libraries (duplicates, own tiles)...",
        "Tile libraries",
        icon::COPY_CHECK,
        "",
    ),
    cmd(
        "presets.save",
        "Save the selection as a preset...",
        "Presets",
        icon::PLUS,
        "",
    ),
    cmd(
        "presets.builder",
        "Preset builder (20x20)...",
        "Presets",
        icon::BOXES,
        "",
    ),
    cmd(
        "presets.suggest",
        "Suggest presets from this map...",
        "Presets",
        icon::SCAN_SEARCH,
        "",
    ),
    cmd(
        "game.addToGame",
        "Add the map to the game (Levels, LvlPrest, LvlTypes)...",
        "Game",
        icon::PLUS,
        "",
    ),
    cmd(
        "game.changeType",
        "Change the level type...",
        "Game",
        icon::ARROW_RIGHT_LEFT,
        "",
    ),
    cmd(
        "game.typePackages",
        "Type packages...",
        "Game",
        icon::PACKAGE_CHECK,
        "",
    ),
    cmd(
        "game.entryText",
        "Entering text...",
        "Game",
        icon::SCROLL_TEXT,
        "",
    ),
    cmd("game.cube", "Cube recipe...", "Game", icon::BOX, ""),
    cmd(
        "game.customObject",
        "Custom object...",
        "Game",
        icon::IMAGE,
        "",
    ),
    cmd(
        "game.automap",
        "Automap editor...",
        "Game",
        icon::MAP_PINNED,
        "",
    ),
    cmd("game.tables", "Tables...", "Game", icon::LIST_TREE, ""),
    cmd(
        "diag.unusedDt1",
        "Unused DT1s...",
        "Diagnostics",
        icon::SCAN_SEARCH,
        "",
    ),
    cmd(
        "diag.restore",
        "Restore archived assets...",
        "Diagnostics",
        icon::HISTORY,
        "",
    ),
    cmd(
        "diag.compat",
        "Compatibility check...",
        "Diagnostics",
        icon::CHECK,
        "",
    ),
    cmd(
        "diag.crashLog",
        "Crash log reader...",
        "Diagnostics",
        icon::SCROLL_TEXT,
        "",
    ),
    cmd(
        "file.exportPackage",
        "Export: map package (.zip)...",
        "File",
        icon::PACKAGE_CHECK,
        "",
    ),
    cmd(
        "file.importPackage",
        "Import: map package...",
        "File",
        icon::FILE_INPUT,
        "",
    ),
    cmd(
        "file.importDs1Dt1",
        "Import: DS1 with its DT1s...",
        "File",
        icon::FILE_INPUT,
        "",
    ),
];

/// The registry entry of `id`.
pub fn find(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.id == id)
}

// ---- key combos -----------------------------------------------------------

/// A key with modifiers, written like ds1-studio: `Ctrl+Alt+Shift+Key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Combo {
    pub fn parse(text: &str) -> Option<Combo> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut parts: Vec<&str> = text.split('+').collect();
        // "Ctrl++" has an empty last part: the key is "+".
        let last = match parts.pop()? {
            "" => "+",
            k => k,
        };
        let mut combo = Combo {
            ctrl: false,
            alt: false,
            shift: false,
            key: key_from_label(last)?,
        };
        for m in parts.iter().filter(|p| !p.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "control" => combo.ctrl = true,
                "alt" => combo.alt = true,
                "shift" => combo.shift = true,
                _ => return None,
            }
        }
        Some(combo)
    }

    /// The combo of a key event: Shift+digit is matched by the physical key
    /// (Shift+1 types "!"), everything else by the logical one.
    pub fn of_event(key: Key, physical: Option<Key>, m: Modifiers) -> Combo {
        let key = match physical {
            Some(p) if m.shift && is_digit(p) => p,
            _ => key,
        };
        Combo {
            ctrl: m.ctrl || m.command,
            alt: m.alt,
            shift: m.shift,
            key,
        }
    }

    pub fn modifiers(self) -> Modifiers {
        Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: false,
            command: self.ctrl,
        }
    }
}

impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(key_label(self.key))
    }
}

fn is_digit(k: Key) -> bool {
    matches!(
        k,
        Key::Num0
            | Key::Num1
            | Key::Num2
            | Key::Num3
            | Key::Num4
            | Key::Num5
            | Key::Num6
            | Key::Num7
            | Key::Num8
            | Key::Num9
    )
}

/// ds1-studio's name of a key: digits and `=` / `-` as typed, letters in
/// capitals, the rest by egui's name.
pub fn key_label(k: Key) -> &'static str {
    match k {
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Plus => "+",
        _ => k.name(),
    }
}

fn key_from_label(text: &str) -> Option<Key> {
    if text.chars().count() == 1 {
        let upper = text.to_ascii_uppercase();
        return Key::from_name(&upper);
    }
    Key::from_name(text)
}

/// Keys never offered as shortcuts (modifier-only or text-editing keys
/// egui reports as keys).
pub fn bindable(k: Key) -> bool {
    !matches!(k, Key::Copy | Key::Cut | Key::Paste)
}

// ---- bindings ---------------------------------------------------------------

/// The key of every command: the defaults with the user's changes on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
    by_id: BTreeMap<&'static str, Option<Combo>>,
}

impl Bindings {
    /// Defaults plus `changes` (id -> key text, `""` = none). Unknown ids and
    /// unreadable keys are ignored.
    pub fn new(changes: &BTreeMap<String, String>) -> Bindings {
        let mut by_id: BTreeMap<&'static str, Option<Combo>> = COMMANDS
            .iter()
            .map(|c| (c.id, Combo::parse(c.key)))
            .collect();
        for (id, text) in changes {
            if let Some(c) = find(id) {
                by_id.insert(c.id, Combo::parse(text));
            }
        }
        Bindings { by_id }
    }

    pub fn get(&self, id: &str) -> Option<Combo> {
        self.by_id.get(id).copied().flatten()
    }

    /// The key text of `id` for tooltips and menus (`""` when none).
    pub fn label(&self, id: &str) -> String {
        self.get(id).map(|c| c.to_string()).unwrap_or_default()
    }

    /// The command `combo` runs, among the available ones.
    pub fn command_for(&self, combo: Combo) -> Option<&'static str> {
        self.by_id
            .iter()
            .filter(|(_, c)| **c == Some(combo))
            .map(|(id, _)| *id)
            .find(|id| find(id).is_some_and(|c| c.available))
    }

    /// Bind `combo` to `id`; a command that had it loses it.
    pub fn bind(&mut self, id: &'static str, combo: Option<Combo>) {
        if combo.is_some() {
            for c in self.by_id.values_mut() {
                if *c == combo {
                    *c = None;
                }
            }
        }
        self.by_id.insert(id, combo);
    }

    /// The bindings that differ from the defaults, as `config.toml` keeps them.
    pub fn changes(&self) -> BTreeMap<String, String> {
        COMMANDS
            .iter()
            .filter_map(|c| {
                let now = self.get(c.id);
                (now != Combo::parse(c.key)).then(|| {
                    (
                        c.id.to_owned(),
                        now.map(|k| k.to_string()).unwrap_or_default(),
                    )
                })
            })
            .collect()
    }
}

// ---- running ----------------------------------------------------------------

/// The map layer a ds1-studio layer name stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerName {
    Key(LayerKey),
    Roofs,
    Specials,
}

fn layer_name(name: &str) -> Option<LayerName> {
    Some(match name {
        "floor1" => LayerName::Key(LayerKey::Floor(0)),
        "floor2" => LayerName::Key(LayerKey::Floor(1)),
        "wall1" => LayerName::Key(LayerKey::Wall(0)),
        "wall2" => LayerName::Key(LayerKey::Wall(1)),
        "wall3" => LayerName::Key(LayerKey::Wall(2)),
        "wall4" => LayerName::Key(LayerKey::Wall(3)),
        "shadows" => LayerName::Key(LayerKey::Shadow),
        "roofs" => LayerName::Roofs,
        "specials" => LayerName::Specials,
        _ => return None,
    })
}

fn has_layer(state: &AppState, layer: LayerName) -> bool {
    match (state.active_doc(), layer) {
        (Some(doc), LayerName::Key(k)) => doc.keys().contains(&k),
        (Some(_), _) => true,
        (None, _) => false,
    }
}

/// Is `layer` shown on the active map?
pub fn layer_shown(state: &AppState, layer: LayerName) -> bool {
    match (state.active_doc(), layer) {
        (Some(doc), LayerName::Key(k)) => doc.is_visible(k),
        (Some(doc), LayerName::Roofs) => doc.roofs_visible,
        (Some(doc), LayerName::Specials) => doc.special_visible,
        (None, _) => false,
    }
}

pub fn set_layer_shown(state: &mut AppState, layer: LayerName, on: bool) {
    let Some(doc) = state.active_doc_mut() else {
        return;
    };
    match layer {
        LayerName::Key(k) => doc.set_visible(k, on),
        LayerName::Roofs => doc.roofs_visible = on,
        LayerName::Specials => doc.special_visible = on,
    }
}

/// Show only `layer` (tile layers; objects stay as they are), or, when it is
/// already soloed, put the visibility back as it was.
pub fn solo(state: &mut AppState, layer: LayerName) {
    let Some(doc) = state.active_doc() else {
        return;
    };
    let doc_id = doc.id;
    if let Some(saved) = state.shell.solo.take()
        && saved.doc == doc_id
        && saved.layer == layer
    {
        if let Some(doc) = state.active_doc_mut() {
            doc.hidden = saved.hidden;
            doc.roofs_visible = saved.roofs;
            doc.special_visible = saved.specials;
        }
        return;
    }
    let Some(doc) = state.active_doc_mut() else {
        return;
    };
    let saved = super::shell::Solo {
        doc: doc_id,
        layer,
        hidden: doc.hidden.clone(),
        roofs: doc.roofs_visible,
        specials: doc.special_visible,
    };
    for k in doc.keys() {
        doc.set_visible(k, layer == LayerName::Key(k));
    }
    doc.roofs_visible = layer == LayerName::Roofs;
    doc.special_visible = layer == LayerName::Specials;
    state.shell.solo = Some(saved);
}

fn layer_ref(layer: LayerName) -> Option<LayerRef> {
    match layer {
        LayerName::Key(k) => Some(k.to_ref()),
        _ => None,
    }
}

fn has_selection(state: &AppState) -> bool {
    state.active_doc().is_some_and(|d| !d.selection.is_empty())
}

fn has_any_selection(state: &AppState) -> bool {
    state
        .active_doc()
        .is_some_and(|d| !d.selection.is_empty() || !d.object_sel.is_empty())
}

fn map_facts(state: &AppState) -> Option<(usize, usize, bool)> {
    state
        .active_doc()
        .map(|d| (d.ds1.floors.len(), d.ds1.walls.len(), d.ds1.tag.is_some()))
}

/// Can `id` run now? Unknown and unavailable commands cannot.
pub fn enabled(state: &AppState, id: &str) -> bool {
    if !find(id).is_some_and(|c| c.available) {
        return false;
    }
    if let Some(on) = super::tools::enabled(state, id) {
        return on;
    }
    let suffix = |p: &str| id.strip_prefix(p).and_then(layer_name);
    if let Some(layer) = suffix("layer.").or_else(|| suffix("solo.")) {
        return has_layer(state, layer);
    }
    if let Some(layer) = suffix("active.") {
        return has_layer(state, layer);
    }
    let doc = state.active_doc();
    match id {
        "edit.undo" | "edit.redo2" | "edit.redo" => doc.is_some_and(|d| {
            if id == "edit.undo" {
                d.edit.can_undo()
            } else {
                d.edit.can_redo()
            }
        }),
        "edit.copy" | "edit.cut" | "edit.moveToLayer" => has_selection(state),
        "flags.setHidden"
        | "flags.clearHidden"
        | "flags.setUnwalkable"
        | "flags.clearUnwalkable" => has_selection(state),
        "edit.delete" => {
            has_any_selection(state)
                || (state.edit.tool == Tool::Objects && state.objects.mode == ObjectMode::Path)
        }
        "edit.deselect" => has_any_selection(state),
        "edit.commitPaste" => state.edit.paste.is_some(),
        "edit.cancel"
        | "app.commands"
        | "view.focus"
        | "view.foldLeft"
        | "view.foldRight"
        | "view.log"
        | "view.objectsWindow"
        | "view.spritePreview"
        | "app.sprites"
        | "file.preferences"
        | "file.addInstall"
        | "app.quit"
        | "help.shortcuts"
        | "help.about"
        | "view.markers"
        | "view.animate"
        | "view.minimap"
        | "view.grid"
        | "select.scopeActive"
        | "select.scopeVisible" => true,
        "app.assetSearch" => state.session.is_some(),
        "file.openSaves" | "file.importIni" => state.active_profile_id().is_some(),
        "file.markPacked" => state.active_profile_id().is_some() && state.edit.changed > 0,
        "map.addFloor" => map_facts(state).is_some_and(|(f, _, _)| f < 2),
        "map.addWall" => map_facts(state).is_some_and(|(_, w, _)| w < 4),
        "map.addTag" => map_facts(state).is_some_and(|(_, _, t)| !t),
        "view.zoomIn" => doc.is_some_and(|d| d.view.zoom > 0),
        "view.zoomOut" => doc.is_some_and(|d| d.view.zoom + 1 < ZOOM_STEPS.len()),
        _ => doc.is_some(),
    }
}

/// Is the toggle or tool `id` on (drawn highlighted)?
pub fn active(state: &AppState, id: &str) -> bool {
    if let Some(on) = super::tools::active(state, id) {
        return on;
    }
    let tool = state.edit.tool;
    let objects = |mode| tool == Tool::Objects && state.objects.mode == mode;
    let suffix = |p: &str| id.strip_prefix(p).and_then(layer_name);
    if let Some(layer) = suffix("layer.") {
        return layer_shown(state, layer);
    }
    if let Some(layer) = suffix("active.") {
        return layer_ref(layer)
            .is_some_and(|l| state.active_doc().is_some_and(|d| d.active_layer == l));
    }
    match id {
        "tool.select" => tool == Tool::SelectBox,
        "tool.paint" => tool == Tool::Paint,
        "tool.erase" => tool == Tool::Erase,
        "tool.pick" => tool == Tool::Eyedropper,
        "tool.object" | "mode.objects" => tool == Tool::Objects,
        "mode.tiles" => tool != Tool::Objects,
        "tool.lasso" => tool == Tool::Lasso,
        "tool.selectById" => tool == Tool::SelectById,
        "tool.selectIdentical" => tool == Tool::SelectIdentical,
        "select.scopeActive" => state.edit.scope == crate::edit::tools::SelectScope::Active,
        "select.scopeVisible" => state.edit.scope == crate::edit::tools::SelectScope::Visible,
        "objects.select" => objects(ObjectMode::Select),
        "objects.add" => objects(ObjectMode::Place),
        "objects.path" => objects(ObjectMode::Path),
        "view.grid" => state.edit.grid,
        "view.markers" => state.config.object_markers,
        "view.animate" => state.config.animate_objects,
        "view.sprites" => state.active_doc().is_some_and(|d| d.objects_visible),
        "view.minimap" => state.config.ui.minimap,
        "view.focus" => state.config.ui.left_folded && state.config.ui.right_folded,
        "view.foldLeft" => state.config.ui.left_folded,
        "view.foldRight" => state.config.ui.right_folded,
        "view.log" => state.shell.log_window,
        "view.objectsWindow" => state.shell.objects_window,
        "view.spritePreview" => state.shell.preview_window,
        "view.history" => state.config.ui.sections.history,
        "map.info" => state.config.ui.sections.map,
        _ => false,
    }
}

/// Actions that change the map, which a history snapshot shown in place of
/// the map (read-only) refuses, like the mouse tools do.
pub fn edits_the_map(id: &str) -> bool {
    matches!(
        id,
        "edit.undo"
            | "edit.redo"
            | "edit.redo2"
            | "edit.cut"
            | "edit.paste"
            | "edit.delete"
            | "edit.commitPaste"
            | "edit.moveToLayer"
            | "flags.setHidden"
            | "flags.clearHidden"
            | "flags.setUnwalkable"
            | "flags.clearUnwalkable"
            | "map.resize"
            | "map.addFloor"
            | "map.addWall"
            | "map.addTag"
    )
}

/// The toast a refused keyboard edit shows.
pub const SNAPSHOT_READ_ONLY: &str = "Viewing a snapshot: turn it off to edit";

/// True (after a toast) when the active map shows a history snapshot.
pub fn refuse_in_snapshot(state: &mut AppState) -> bool {
    if state
        .active_doc()
        .is_some_and(|d| d.snapshot_view.is_some())
    {
        state.notify(SNAPSHOT_READ_ONLY);
        return true;
    }
    false
}

fn set_tool(state: &mut AppState, tool: Tool) {
    super::map_tools::cancel_gesture(state);
    if state.edit.tool != Tool::Objects {
        state.shell.prev_tool = state.edit.tool;
    }
    state.edit.tool = tool;
    if tool == Tool::Objects {
        state.objects.focus = true;
    }
}

fn zoom(state: &mut AppState, zoom_in: bool) {
    if let Some(doc) = state.active_doc_mut() {
        doc.view.zoom_at([0.0, 0.0], zoom_in);
    }
}

fn toggle_config(state: &mut AppState, f: impl FnOnce(&mut crate::config::Config)) {
    f(&mut state.config);
    state.config_dirty = true;
}

fn add_layer(state: &mut AppState, which: fn(usize, usize) -> LayerRef) {
    if let (Some(i), Some((f, w, _))) = (state.active, map_facts(state)) {
        state.exec(i, EditOp::AddLayer(which(f, w)));
    }
}

fn flags(state: &mut AppState, hidden: Option<bool>, unwalkable: Option<bool>) {
    if let Some(i) = state.active {
        state.exec(i, EditOp::SetFlags { hidden, unwalkable });
    }
}

/// Run `id` if it is enabled. Returns whether it ran.
pub fn run(state: &mut AppState, id: &str) -> bool {
    if !enabled(state, id) {
        return false;
    }
    if edits_the_map(id) && refuse_in_snapshot(state) {
        if id == "edit.paste" || id == "edit.commitPaste" {
            state.cancel_paste();
        }
        return false;
    }
    if super::tools::run(state, id) {
        return true;
    }
    let suffix = |p: &str| id.strip_prefix(p).and_then(layer_name);
    if let Some(layer) = suffix("layer.") {
        let on = layer_shown(state, layer);
        set_layer_shown(state, layer, !on);
        return true;
    }
    if let Some(layer) = suffix("solo.") {
        solo(state, layer);
        return true;
    }
    if let Some(layer) = suffix("active.") {
        if let (Some(l), Some(doc)) = (layer_ref(layer), state.active_doc_mut()) {
            doc.active_layer = l;
        }
        return true;
    }
    match id {
        "tool.select" => set_tool(state, Tool::SelectBox),
        "tool.paint" => set_tool(state, Tool::Paint),
        "tool.erase" => set_tool(state, Tool::Erase),
        "tool.pick" => set_tool(state, Tool::Eyedropper),
        "tool.object" | "mode.objects" => set_tool(state, Tool::Objects),
        "tool.lasso" => set_tool(state, Tool::Lasso),
        "tool.selectById" => set_tool(state, Tool::SelectById),
        "tool.selectIdentical" => set_tool(state, Tool::SelectIdentical),
        "tool.toggleObjects" | "mode.tiles" => {
            if state.edit.tool == Tool::Objects {
                let back = state.shell.prev_tool;
                set_tool(state, back);
            } else if id == "tool.toggleObjects" {
                set_tool(state, Tool::Objects);
            }
        }
        "objects.select" => super::objects::set_mode(state, ObjectMode::Select),
        "objects.add" => super::objects::set_mode(state, ObjectMode::Place),
        "objects.path" => super::objects::set_mode(state, ObjectMode::Path),
        "select.scopeActive" => state.edit.scope = crate::edit::tools::SelectScope::Active,
        "select.scopeVisible" => state.edit.scope = crate::edit::tools::SelectScope::Visible,
        "edit.undo" => state.undo(),
        "edit.redo" | "edit.redo2" => state.redo(),
        "file.save" => state.save_active(),
        "edit.copy" => state.copy_selection(),
        "edit.cut" => state.cut_selection(),
        "edit.paste" => state.begin_paste(),
        "edit.selectAll" => select_all(state),
        "edit.deselect" => state.deselect(),
        "edit.delete" => delete(state),
        "edit.commitPaste" => state.commit_paste(),
        "edit.cancel" => cancel(state),
        "edit.moveToLayer" => state.edit.move_dialog = super::dialogs::new_move_dialog(state),
        "flags.setHidden" => flags(state, Some(true), None),
        "flags.clearHidden" => flags(state, Some(false), None),
        "flags.setUnwalkable" => flags(state, None, Some(true)),
        "flags.clearUnwalkable" => flags(state, None, Some(false)),
        "view.fit" => state.request_fit = true,
        "view.zoom100" => {
            if let Some(doc) = state.active_doc_mut() {
                doc.view.zoom = 0;
            }
        }
        "view.zoomIn" => zoom(state, true),
        "view.zoomOut" => zoom(state, false),
        "layer.showAll" => {
            state.shell.solo = None;
            if let Some(doc) = state.active_doc_mut() {
                doc.hidden.clear();
                doc.roofs_visible = true;
                doc.special_visible = true;
            }
        }
        "view.grid" => state.edit.grid = !state.edit.grid,
        "view.markers" => toggle_config(state, |c| c.object_markers = !c.object_markers),
        "view.animate" => toggle_config(state, |c| c.animate_objects = !c.animate_objects),
        "view.sprites" => {
            if let Some(doc) = state.active_doc_mut() {
                doc.objects_visible = !doc.objects_visible;
            }
        }
        "view.minimap" => toggle_config(state, |c| c.ui.minimap = !c.ui.minimap),
        "view.focus" => toggle_config(state, |c| {
            let fold = !(c.ui.left_folded && c.ui.right_folded);
            c.ui.left_folded = fold;
            c.ui.right_folded = fold;
        }),
        "view.foldLeft" => toggle_config(state, |c| c.ui.left_folded = !c.ui.left_folded),
        "view.foldRight" => toggle_config(state, |c| c.ui.right_folded = !c.ui.right_folded),
        "view.log" => state.shell.log_window = !state.shell.log_window,
        "view.objectsWindow" => state.shell.objects_window = !state.shell.objects_window,
        "view.spritePreview" => state.shell.preview_window = !state.shell.preview_window,
        "view.history" => toggle_config(state, |c| c.ui.sections.history = !c.ui.sections.history),
        "map.info" => toggle_config(state, |c| c.ui.sections.map = !c.ui.sections.map),
        "app.commands" => state.shell.finder.toggle(),
        "app.sprites" => toggle_config(state, |c| c.mode = AppMode::Sprites),
        "app.assetSearch" => {
            let query = std::mem::take(&mut state.shell.finder.query);
            state.shell.finder.open = false;
            state.open_palette_with(&query);
        }
        "file.close" => {
            if let Some(i) = state.active {
                state.request_close(i);
            }
        }
        "file.openSaves" => super::menu::open_saves_folder(state),
        "file.markPacked" => state.edit.confirm_packed = true,
        "file.importIni" => state.pick_ini_files(),
        "file.preferences" => state.settings.open = true,
        "file.addInstall" => state.open_add_install(None),
        "app.quit" => state.request_quit(),
        "map.resize" => state.edit.resize_dialog = super::dialogs::new_resize_dialog(state),
        "map.addFloor" => add_layer(state, |f, _| LayerRef::Floor(f as u8)),
        "map.addWall" => add_layer(state, |_, w| LayerRef::Wall(w as u8)),
        "map.addTag" => add_layer(state, |_, _| LayerRef::Tag),
        "help.shortcuts" => state.shell.shortcuts.open = true,
        "help.about" => state.shell.about_open = true,
        _ => return false,
    }
    true
}

/// Escape: drop the paste, the gesture or a dialog; else deselect.
fn cancel(state: &mut AppState) {
    if state.edit.paste.is_some() {
        state.cancel_paste();
    } else if super::map_tools::cancel_gesture(state) {
    } else if state.edit.move_dialog.is_some() || state.edit.resize_dialog.is_some() {
        state.edit.move_dialog = None;
        state.edit.resize_dialog = None;
    } else {
        state.deselect();
    }
}

/// Select every non-empty cell of the active layer.
pub fn select_all(state: &mut AppState) {
    let Some(doc) = state.active_doc_mut() else {
        return;
    };
    let layer: LayerRef = doc.active_layer;
    let rect = capy_edit::CellRect::new(
        (0, 0),
        (
            doc.ds1.width.saturating_sub(1),
            doc.ds1.height.saturating_sub(1),
        ),
    );
    doc.selection.box_select(
        &doc.ds1,
        rect,
        &[layer],
        false,
        capy_edit::SelectOp::Replace,
    );
}

/// Delete: the selected path point (Path tool), else the selected objects,
/// else the selected cells.
pub fn delete(state: &mut AppState) {
    let Some(index) = state.active else { return };
    if state.edit.tool == Tool::Objects && state.objects.mode == ObjectMode::Path {
        let len = state.selected_path().len();
        if let Some(cmd) = state.objects.path.delete(len) {
            state.run_path_command(cmd);
        }
        return;
    }
    let doc = &state.docs[index];
    if !doc.object_sel.is_empty() {
        let indices = doc.object_sel.clone();
        state.exec(index, EditOp::DeleteObjects(indices));
    } else if !doc.selection.is_empty() {
        state.exec(index, EditOp::Erase);
    }
}

/// Runs the commands whose keys were pressed this frame (Map Editor). Skipped
/// while a text field has focus or a modal dialog is open.
pub fn handle_keys(ctx: &egui::Context, state: &mut AppState) {
    if state.edit.modal_open()
        || state.history.modal_open()
        || state.settings.modal_open()
        || state.shell.blocks_keys()
        || ctx.text_edit_focused()
    {
        return;
    }
    let bindings = state.shell.bindings.clone();
    let mut wanted: Vec<&'static str> = Vec::new();
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat: false,
                modifiers,
            } = e
            else {
                return true;
            };
            let combo = Combo::of_event(*key, *physical_key, *modifiers);
            match bindings.command_for(combo) {
                Some(id) if enabled(state, id) => {
                    wanted.push(id);
                    false
                }
                _ => true,
            }
        });
    });
    for id in wanted {
        if run(state, id) {
            state.shell.ran.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use capy_edit::CellValue;
    use d2_assets::GamePath;
    use d2_formats::ds1::{Cell, Ds1};

    use super::*;
    use crate::config::Config;
    use crate::doc::{MapDoc, SourceCategory, SourceInfo};
    use crate::paths::Paths;

    fn state_with_edit() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut state = AppState::new(
            Paths::at(tmp.path().join("capybara")),
            Config::default(),
            Vec::new(),
        );
        let mut ds1 = Ds1::new(4, 4, 0);
        ds1.floors[0].set(1, 1, Cell(1).with_indices(1, 0));
        state.docs.push(MapDoc::new(
            1,
            "p".into(),
            GamePath::new("data\\a.ds1"),
            SourceInfo {
                category: SourceCategory::Base,
                layer_name: "x".into(),
            },
            ds1,
            &Config::default().layer_visibility,
        ));
        state.active = Some(0);
        let value = CellValue {
            cell: Cell(1).with_indices(2, 0).0,
            orientation: 0,
        };
        assert!(state.exec(
            0,
            EditOp::Paint {
                layer: LayerRef::Floor(0),
                cells: vec![(2, 2, value)],
            }
        ));
        (state, tmp)
    }

    #[test]
    fn ids_are_unique_and_default_keys_parse_and_differ() {
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashMap::new();
        for c in COMMANDS {
            assert!(ids.insert(c.id), "{} twice", c.id);
            if c.key.is_empty() {
                continue;
            }
            // PrintScreen has no egui key; it stays reserved for later.
            match Combo::parse(c.key) {
                Some(k) => {
                    assert_eq!(k.to_string(), c.key, "{} round-trips", c.id);
                    assert!(keys.insert(k, c.id).is_none(), "{} reuses {}", c.id, c.key);
                }
                None => assert!(!c.available, "{}: {:?} does not parse", c.id, c.key),
            }
        }
    }

    #[test]
    fn every_available_command_runs() {
        let (mut state, _tmp) = state_with_edit();
        state.docs[0].selection.insert(LayerRef::Floor(0), 1, 1);
        for c in COMMANDS.iter().filter(|c| c.available && !c.external) {
            if matches!(c.id, "app.quit" | "app.sprites" | "file.close") {
                continue; // they end the session the other commands need
            }
            if enabled(&state, c.id) {
                assert!(run(&mut state, c.id), "{} has no action", c.id);
            }
        }
        // Unavailable ones never run.
        assert!(!run(&mut state, "edit.replace"));
        assert!(!enabled(&state, "view.walkable"));
    }

    #[test]
    fn bind_moves_a_key_and_changes_keep_only_differences() {
        let mut b = Bindings::new(&BTreeMap::new());
        assert_eq!(
            b.command_for(Combo::parse("V").expect("V")),
            Some("tool.select")
        );
        assert!(b.changes().is_empty());
        b.bind("tool.paint", Combo::parse("V"));
        assert_eq!(
            b.command_for(Combo::parse("V").expect("V")),
            Some("tool.paint")
        );
        assert_eq!(b.get("tool.select"), None, "the old owner lost it");
        let changes = b.changes();
        assert_eq!(changes.get("tool.paint").map(String::as_str), Some("V"));
        assert_eq!(changes.get("tool.select").map(String::as_str), Some(""));
        assert_eq!(Bindings::new(&changes), b, "reloads the same");
    }

    #[test]
    fn combos_parse_like_ds1_studio() {
        let c = Combo::parse("Ctrl+Shift+Z").expect("parse");
        assert!(c.ctrl && c.shift && !c.alt && c.key == Key::Z);
        assert_eq!(Combo::parse("=").map(|c| c.key), Some(Key::Equals));
        assert_eq!(Combo::parse("-").map(|c| c.key), Some(Key::Minus));
        assert_eq!(Combo::parse("Shift+1").map(|c| c.key), Some(Key::Num1));
        assert_eq!(Combo::parse("v"), Combo::parse("V"));
        assert_eq!(Combo::parse("Hyper+V"), None);
        // Shift+1 arrives as "!" with the physical key 1.
        let m = Modifiers::SHIFT;
        let e = Combo::of_event(Key::Exclamationmark, Some(Key::Num1), m);
        assert_eq!(Some(e), Combo::parse("Shift+1"));
    }

    #[test]
    fn keyboard_edits_are_refused_while_a_snapshot_is_shown() {
        let (mut state, _tmp) = state_with_edit();
        state.docs[0].snapshot_view = Some(capy_history::SnapshotId(1));
        state.docs[0].selection.insert(LayerRef::Floor(0), 1, 1);
        let revision = state.docs[0].edit.revision();
        for id in ["edit.undo", "edit.cut", "edit.delete", "edit.paste"] {
            run(&mut state, id);
        }
        assert_eq!(state.docs[0].edit.revision(), revision, "nothing changed");
        assert!(state.edit.paste.is_none());
        assert!(
            state.toasts.iter().any(|t| t.text == SNAPSHOT_READ_ONLY),
            "the user is told why"
        );
        // Non-editing commands still work: tools, deselect.
        assert!(run(&mut state, "tool.paint"));
        assert_eq!(state.edit.tool, Tool::Paint);
        assert!(run(&mut state, "edit.deselect"));
        assert!(state.docs[0].selection.is_empty());
        // Back on the map, Ctrl+Z works again.
        state.docs[0].show_current();
        assert!(run(&mut state, "edit.undo"));
        assert_ne!(state.docs[0].edit.revision(), revision);
    }

    #[test]
    fn solo_shows_one_layer_and_again_restores() {
        let (mut state, _tmp) = state_with_edit();
        let floor = LayerName::Key(LayerKey::Floor(0));
        assert!(run(&mut state, "solo.floor1"));
        assert!(layer_shown(&state, floor));
        assert!(!layer_shown(&state, LayerName::Key(LayerKey::Shadow)));
        assert!(!layer_shown(&state, LayerName::Roofs));
        assert!(run(&mut state, "solo.floor1"));
        assert!(layer_shown(&state, LayerName::Key(LayerKey::Shadow)));
        assert!(layer_shown(&state, LayerName::Roofs));
        // Layers the map lacks are disabled.
        assert!(!enabled(&state, "layer.wall4"));
        assert!(!enabled(&state, "active.floor2"));
        assert!(run(&mut state, "active.shadows"));
        assert_eq!(state.docs[0].active_layer, LayerRef::Shadow);
    }

    #[test]
    fn toggle_objects_returns_to_the_previous_tool() {
        let (mut state, _tmp) = state_with_edit();
        run(&mut state, "tool.erase");
        run(&mut state, "tool.toggleObjects");
        assert_eq!(state.edit.tool, Tool::Objects);
        run(&mut state, "tool.toggleObjects");
        assert_eq!(state.edit.tool, Tool::Erase);
    }
}
