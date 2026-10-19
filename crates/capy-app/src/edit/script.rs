//! The `CAPY_SCRIPT` language: one command per line, run once the profile is
//! ready, used for scripted end-to-end checks. This file is the parser; the
//! runner lives in [`crate::script_run`].
//!
//! ```text
//! # comments and blank lines are ignored
//! open data\global\tiles\act1\town\towne1.ds1
//! select-box wall0 0 0 39 39          # layer: floor0 floor1 wall0..wall3 shadow tag
//! move-layer wall0 wall2 overwrite    # overwrite | skip
//! paint floor0 3 3 0x00100201 [orientation]
//! copy
//! paste 10 10 skip                    # overwrite | skip | empty
//! undo
//! redo
//! save
//! assert-dirty false
//! screenshot out.png
//! ```
//!
//! Extra commands for screenshots and checks: `preview-paste x y mode` (show
//! the floating preview without committing), `move-dialog from to [mode]`
//! (open the layer-move dialog), `zoom <divisor>`, `assert-selection <n>`,
//! `wait <frames>`, `select-object <index>` (Objects tool selection),
//! `sprite-preview` (show the Sprite preview panel).
//!
//! Sprites mode:
//!
//! ```text
//! mode sprites                          # or: mode map
//! sprite-open data\global\monsters\zm\cof\zmnuhth.cof
//! sprite-dir 2
//! sprite-frame 5
//! sprite-play on                        # on | off
//! sprite-import a.png b.png dirs 1 frames 2 dcc   # or one folder with sprite.json
//! sprite-save-game data\global\items\invfoo.dc6  # the editor's sprite when open
//! sprite-export-frames C:\out\frames
//! sprite-edit                           # open the viewer's sprite in the editor
//! sprite-tab converter                  # viewer | editor | converter
//! paint-pencil 3 4 120                  # frame pixel, palette index (never 0)
//! paint-gradient 0 0 20 20 ramp 140 bayer [radial]   # or: two 120 140 none|bayer|fs
//! paint-outline auto outside 1          # or: paint-outline 37 inside 2
//! paint-select 2 2 20 30                # rectangle selection
//! paint-onion on
//! paint-undo
//! ```
//!
//! Paint commands open the viewer's sprite in the editor when none is open.
//!
//! Asset browser (these wait for the asset index; keys contain `#`, so a
//! comment starts only at a `#` that begins the line or follows a space):
//!
//! ```text
//! search color:green type:floor file:whimsyfloor.dt1   # the Tiles browser's query
//! assert-search-top 5 data\global\tiles\...\whimsyfloor.dt1#0/23/0/0
//! brush-from data\global\tiles\...\whimsyfloor.dt1#0/23/0/0
//! assert-brush floor0 0x017000c2 [orientation]
//! paint-brush 3 4                       # paint the brush on its layer (records a use)
//! assert-recent <key>
//! favorite-add <group> <key>
//! alias <key> <text...>
//! ctrl-k <query...>                     # open the Ctrl+K palette with this query
//! ctrl-k-close
//! browser-scope map|profile|sprites
//! thumb-size 96
//! tile-menu <key>                       # open the tile's context menu
//! tiles-tab                             # bring the Tiles panel to the front
//! ```
//!
//! History (the timeline index counts from 0 = newest, as the panel lists it;
//! a name matches a checkpoint name or a label):
//!
//! ```text
//! autosave-now                          # record dirty maps now; waits for the worker
//! checkpoint before wall                # the rest of the line is the name
//! assert-history 2 [autosave|save|checkpoint|before-restore]
//! history-select before wall            # or: history-select 0
//! assert-diff floor0 12                 # changed cells on a layer vs the selection
//! show-snapshot on                      # draw the selected snapshot (read-only)
//! history-jump floor0                   # centre the map on that layer's next change
//! restore before wall                   # BeforeRestore, then one undoable replace
//! milestone first version               # gix commit of capybara\saves\
//! assert-milestones 1
//! prune 1                               # keep_last for this prune
//! assert-changed 0                      # entries in the profile's CHANGED.txt
//! setting gamma 1.6                     # gamma, autosave_minutes, keep_last,
//!                                       # max_age_days, select_scope, layer.<kind>
//! history-tab | settings open|close | add-install [folder] | close-dialogs
//! ```

use std::path::PathBuf;

use capy_edit::{LayerRef, MoveMode, PasteMode};

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Open(String),
    SelectBox {
        layer: LayerRef,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
    },
    MoveLayer {
        from: LayerRef,
        to: LayerRef,
        mode: MoveMode,
    },
    Paint {
        layer: LayerRef,
        x: u32,
        y: u32,
        cell: u32,
        orientation: u32,
    },
    Copy,
    Paste {
        x: i32,
        y: i32,
        mode: PasteMode,
    },
    Undo,
    Redo,
    Save,
    AssertDirty(bool),
    Screenshot(PathBuf),
    PreviewPaste {
        x: i32,
        y: i32,
        mode: PasteMode,
    },
    MoveDialog {
        from: LayerRef,
        to: LayerRef,
        mode: MoveMode,
    },
    Zoom(u32),
    AssertSelection(usize),
    Wait(u32),
    SelectObject(usize),
    SpritePreview,
    /// `true` = Sprites mode.
    Mode(bool),
    SpriteOpen(String),
    SpriteDir(usize),
    SpriteFrame(usize),
    SpritePlay(bool),
    SpriteImport {
        files: Vec<PathBuf>,
        dirs: u32,
        frames: u32,
        dcc: bool,
    },
    SpriteSaveGame(String),
    SpriteExportFrames(PathBuf),
    SpriteEdit,
    SpriteTab(String),
    PaintPencil {
        x: i32,
        y: i32,
        index: u8,
    },
    PaintGradient {
        from: (f32, f32),
        to: (f32, f32),
        colors: GradientSpec,
        dither: capy_paint::Dither,
        radial: bool,
    },
    PaintOutline {
        /// `None` = auto-darker.
        color: Option<u8>,
        inside: bool,
        thickness: u32,
    },
    PaintSelect {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
    PaintOnion(bool),
    PaintUndo,
    Search(String),
    AssertSearchTop {
        n: usize,
        key: String,
    },
    BrushFrom(String),
    AssertBrush {
        layer: LayerRef,
        cell: u32,
        orientation: Option<u32>,
    },
    PaintBrush {
        x: u32,
        y: u32,
    },
    AssertRecent(String),
    FavoriteAdd {
        group: String,
        key: String,
    },
    Alias {
        key: String,
        text: String,
    },
    CtrlK(String),
    CtrlKClose,
    BrowserScope(crate::browser::scope::Scope),
    ThumbSize(f32),
    TileMenu(String),
    TilesTab,
    AutosaveNow,
    Checkpoint(String),
    AssertHistory {
        n: usize,
        kind: Option<String>,
    },
    HistorySelect(String),
    AssertDiff {
        layer: LayerRef,
        count: usize,
    },
    ShowSnapshot(bool),
    /// Centre the map on the next change of a layer (the panel's Jump).
    HistoryJump(LayerRef),
    Restore(String),
    Milestone(String),
    AssertMilestones(usize),
    Prune(usize),
    AssertChanged(usize),
    Setting {
        key: String,
        value: String,
    },
    HistoryTab,
    /// Open (true) or close the Settings window.
    Settings(bool),
    /// Open "Add game install..." (optionally for a folder); writes nothing.
    AddInstall(Option<String>),
    CloseDialogs,
}

/// The kinds `assert-history` accepts.
pub const HISTORY_KINDS: [&str; 4] = ["save", "autosave", "checkpoint", "before-restore"];

/// Colours of `paint-gradient`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientSpec {
    /// The ramp of this palette index.
    Ramp(u8),
    Two(u8, u8),
}

fn parse_index(text: &str) -> Result<u8, String> {
    let n = parse_u32(text, "index")?;
    match u8::try_from(n) {
        Ok(0) => Err("index 0 is the transparent key, not a colour".to_owned()),
        Ok(i) => Ok(i),
        Err(_) => Err(format!("index {n} is not 1..255")),
    }
}

fn parse_dither(text: &str) -> Result<capy_paint::Dither, String> {
    match text.to_ascii_lowercase().as_str() {
        "none" => Ok(capy_paint::Dither::None),
        "bayer" | "bayer4" => Ok(capy_paint::Dither::Bayer4),
        "fs" | "floyd" | "floyd-steinberg" => Ok(capy_paint::Dither::FloydSteinberg),
        _ => Err(format!("dither {text:?} must be none, bayer or fs")),
    }
}

fn parse_on(text: &str) -> Result<bool, String> {
    match text.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        other => parse_bool(other),
    }
}

fn parse_f32(text: &str, what: &str) -> Result<f32, String> {
    text.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what}: {text:?} is not a number"))
}

fn parse_usize(text: &str, what: &str) -> Result<usize, String> {
    parse_u32(text, what).map(|n| n as usize)
}

/// A command and the 1-based line it came from.
pub type Line = (usize, Cmd);

/// `floor0`, `wall(2)`, `Shadow`, `tag` (also `f1`, `w0`); indices are 0-based
/// like [`LayerRef`].
pub fn parse_layer(text: &str) -> Result<LayerRef, String> {
    let t: String = text
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '_' | '-' | ' '))
        .collect::<String>()
        .to_ascii_lowercase();
    let split = t.find(|c: char| c.is_ascii_digit()).unwrap_or(t.len());
    let (name, digits) = t.split_at(split);
    let index = |max: u8| -> Result<u8, String> {
        let n: u8 = digits
            .parse()
            .map_err(|_| format!("layer {text:?} needs an index"))?;
        if n > max {
            return Err(format!("layer {text:?}: the index is at most {max}"));
        }
        Ok(n)
    };
    match name {
        "floor" | "f" => Ok(LayerRef::Floor(index(1)?)),
        "wall" | "w" => Ok(LayerRef::Wall(index(3)?)),
        "shadow" | "s" if digits.is_empty() => Ok(LayerRef::Shadow),
        "tag" | "t" if digits.is_empty() => Ok(LayerRef::Tag),
        _ => Err(format!(
            "unknown layer {text:?} (floor0, floor1, wall0..wall3, shadow, tag)"
        )),
    }
}

fn parse_u32(text: &str, what: &str) -> Result<u32, String> {
    let t = text.trim();
    let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => t.parse(),
    };
    parsed.map_err(|_| format!("{what}: {text:?} is not a number"))
}

fn parse_i32(text: &str, what: &str) -> Result<i32, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{what}: {text:?} is not a whole number"))
}

/// The cell value: hexadecimal, with or without `0x`.
fn parse_cell(text: &str) -> Result<u32, String> {
    let t = text.trim();
    let hex = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    u32::from_str_radix(hex, 16).map_err(|_| format!("cell: {text:?} is not a hex number"))
}

fn parse_move_mode(text: &str) -> Result<MoveMode, String> {
    match text.to_ascii_lowercase().as_str() {
        "overwrite" => Ok(MoveMode::Overwrite),
        "skip" | "skip-occupied" => Ok(MoveMode::SkipOccupied),
        _ => Err(format!("mode {text:?} must be overwrite or skip")),
    }
}

fn parse_paste_mode(text: &str) -> Result<PasteMode, String> {
    match text.to_ascii_lowercase().as_str() {
        "overwrite" => Ok(PasteMode::Overwrite),
        "skip" | "skip-occupied" => Ok(PasteMode::SkipOccupied),
        "empty" | "only-empty" => Ok(PasteMode::OnlyIntoEmptyLayers),
        _ => Err(format!("mode {text:?} must be overwrite, skip or empty")),
    }
}

fn parse_bool(text: &str) -> Result<bool, String> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => Err(format!("{text:?} must be true or false")),
    }
}

/// The line without its comment: a `#` at the start or after whitespace
/// (asset keys hold a `#` right after the file name).
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &line[..i];
        }
    }
    line
}

/// Parses one line. Blank lines and `#` comments give `Ok(None)`.
pub fn parse_line(line: &str) -> Result<Option<Cmd>, String> {
    let line = strip_comment(line).trim();
    if line.is_empty() {
        return Ok(None);
    }
    let (word, rest) = match line.split_once(char::is_whitespace) {
        Some((w, r)) => (w, r.trim()),
        None => (line, ""),
    };
    let args: Vec<&str> = rest.split_whitespace().collect();
    let need = |n: usize, usage: &str| -> Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!("{word}: expected {usage}"))
        }
    };
    let cmd = match word.to_ascii_lowercase().as_str() {
        "open" => {
            if rest.is_empty() {
                return Err("open: expected <game path>".to_owned());
            }
            Cmd::Open(rest.to_owned())
        }
        "select-box" => {
            need(5, "<layer> x0 y0 x1 y1")?;
            Cmd::SelectBox {
                layer: parse_layer(args[0])?,
                x0: parse_u32(args[1], "x0")?,
                y0: parse_u32(args[2], "y0")?,
                x1: parse_u32(args[3], "x1")?,
                y1: parse_u32(args[4], "y1")?,
            }
        }
        "move-layer" => {
            need(3, "<from> <to> overwrite|skip")?;
            Cmd::MoveLayer {
                from: parse_layer(args[0])?,
                to: parse_layer(args[1])?,
                mode: parse_move_mode(args[2])?,
            }
        }
        "paint" => {
            if !(4..=5).contains(&args.len()) {
                return Err("paint: expected <layer> x y <cell hex> [orientation]".to_owned());
            }
            Cmd::Paint {
                layer: parse_layer(args[0])?,
                x: parse_u32(args[1], "x")?,
                y: parse_u32(args[2], "y")?,
                cell: parse_cell(args[3])?,
                orientation: match args.get(4) {
                    Some(o) => parse_u32(o, "orientation")?,
                    None => 0,
                },
            }
        }
        "copy" => {
            need(0, "no arguments")?;
            Cmd::Copy
        }
        "paste" => {
            need(3, "x y overwrite|skip|empty")?;
            Cmd::Paste {
                x: parse_i32(args[0], "x")?,
                y: parse_i32(args[1], "y")?,
                mode: parse_paste_mode(args[2])?,
            }
        }
        "preview-paste" => {
            need(3, "x y overwrite|skip|empty")?;
            Cmd::PreviewPaste {
                x: parse_i32(args[0], "x")?,
                y: parse_i32(args[1], "y")?,
                mode: parse_paste_mode(args[2])?,
            }
        }
        "move-dialog" => {
            if !(2..=3).contains(&args.len()) {
                return Err("move-dialog: expected <from> <to> [overwrite|skip]".to_owned());
            }
            Cmd::MoveDialog {
                from: parse_layer(args[0])?,
                to: parse_layer(args[1])?,
                mode: match args.get(2) {
                    Some(m) => parse_move_mode(m)?,
                    None => MoveMode::SkipOccupied,
                },
            }
        }
        "undo" => {
            need(0, "no arguments")?;
            Cmd::Undo
        }
        "redo" => {
            need(0, "no arguments")?;
            Cmd::Redo
        }
        "save" => {
            need(0, "no arguments")?;
            Cmd::Save
        }
        "assert-dirty" => {
            need(1, "true|false")?;
            Cmd::AssertDirty(parse_bool(args[0])?)
        }
        "assert-selection" => {
            need(1, "<count>")?;
            Cmd::AssertSelection(
                usize::try_from(parse_u32(args[0], "count")?).unwrap_or(usize::MAX),
            )
        }
        "screenshot" => {
            if rest.is_empty() {
                return Err("screenshot: expected <png file>".to_owned());
            }
            Cmd::Screenshot(PathBuf::from(rest))
        }
        "zoom" => {
            need(1, "<divisor>")?;
            Cmd::Zoom(parse_u32(args[0], "divisor")?)
        }
        "wait" => {
            need(1, "<frames>")?;
            Cmd::Wait(parse_u32(args[0], "frames")?)
        }
        "select-object" => {
            need(1, "<object index>")?;
            Cmd::SelectObject(parse_usize(args[0], "index")?)
        }
        "sprite-preview" => {
            need(0, "no arguments")?;
            Cmd::SpritePreview
        }
        "mode" => {
            need(1, "sprites|map")?;
            match args[0].to_ascii_lowercase().as_str() {
                "sprites" | "sprite" => Cmd::Mode(true),
                "map" => Cmd::Mode(false),
                other => return Err(format!("mode {other:?} must be sprites or map")),
            }
        }
        "sprite-open" => {
            if rest.is_empty() {
                return Err("sprite-open: expected <game path>".to_owned());
            }
            Cmd::SpriteOpen(rest.to_owned())
        }
        "sprite-dir" => {
            need(1, "<direction>")?;
            Cmd::SpriteDir(parse_usize(args[0], "direction")?)
        }
        "sprite-frame" => {
            need(1, "<frame>")?;
            Cmd::SpriteFrame(parse_usize(args[0], "frame")?)
        }
        "sprite-play" => {
            need(1, "on|off")?;
            Cmd::SpritePlay(parse_on(args[0])?)
        }
        "sprite-import" => {
            let usage = "sprite-import: expected <image>... dirs N frames M dcc|dc6";
            let pos = args
                .iter()
                .position(|a| a.eq_ignore_ascii_case("dirs"))
                .ok_or(usage)?;
            let (files, tail) = args.split_at(pos);
            if files.is_empty() || tail.len() != 5 || !tail[2].eq_ignore_ascii_case("frames") {
                return Err(usage.to_owned());
            }
            let dcc = match tail[4].to_ascii_lowercase().as_str() {
                "dcc" => true,
                "dc6" => false,
                _ => return Err(usage.to_owned()),
            };
            Cmd::SpriteImport {
                files: files.iter().map(PathBuf::from).collect(),
                dirs: parse_u32(tail[1], "dirs")?,
                frames: parse_u32(tail[3], "frames")?,
                dcc,
            }
        }
        "sprite-save-game" => {
            need(1, "<game path>")?;
            Cmd::SpriteSaveGame(args[0].to_owned())
        }
        "sprite-export-frames" => {
            if rest.is_empty() {
                return Err("sprite-export-frames: expected <folder>".to_owned());
            }
            Cmd::SpriteExportFrames(PathBuf::from(rest))
        }
        "sprite-edit" => {
            need(0, "no arguments")?;
            Cmd::SpriteEdit
        }
        "sprite-tab" => {
            need(1, "viewer|editor|converter")?;
            let t = args[0].to_ascii_lowercase();
            if !["viewer", "editor", "converter"].contains(&t.as_str()) {
                return Err(format!(
                    "sprite-tab {t:?} must be viewer, editor or converter"
                ));
            }
            Cmd::SpriteTab(t)
        }
        "paint-pencil" => {
            need(3, "x y <index>")?;
            Cmd::PaintPencil {
                x: parse_i32(args[0], "x")?,
                y: parse_i32(args[1], "y")?,
                index: parse_index(args[2])?,
            }
        }
        "paint-gradient" => {
            let usage = "paint-gradient: expected x0 y0 x1 y1 ramp <index> <dither> [radial] \
                         or x0 y0 x1 y1 two <a> <b> <dither> [radial]";
            if args.len() < 7 {
                return Err(usage.to_owned());
            }
            let from = (parse_f32(args[0], "x0")?, parse_f32(args[1], "y0")?);
            let to = (parse_f32(args[2], "x1")?, parse_f32(args[3], "y1")?);
            let (colors, next) = match args[4].to_ascii_lowercase().as_str() {
                "ramp" => (GradientSpec::Ramp(parse_index(args[5])?), 6),
                "two" if args.len() >= 8 => (
                    GradientSpec::Two(parse_index(args[5])?, parse_index(args[6])?),
                    7,
                ),
                _ => return Err(usage.to_owned()),
            };
            let dither = parse_dither(args.get(next).ok_or(usage)?)?;
            let radial = match args.get(next + 1) {
                None => false,
                Some(s) if s.eq_ignore_ascii_case("radial") => true,
                Some(s) if s.eq_ignore_ascii_case("linear") => false,
                Some(_) => return Err(usage.to_owned()),
            };
            if args.len() > next + 2 {
                return Err(usage.to_owned());
            }
            Cmd::PaintGradient {
                from,
                to,
                colors,
                dither,
                radial,
            }
        }
        "paint-outline" => {
            need(3, "auto|<index> outside|inside <thickness>")?;
            let color = if args[0].eq_ignore_ascii_case("auto") {
                None
            } else {
                Some(parse_index(args[0])?)
            };
            let inside = match args[1].to_ascii_lowercase().as_str() {
                "inside" => true,
                "outside" => false,
                other => return Err(format!("side {other:?} must be outside or inside")),
            };
            let thickness = parse_u32(args[2], "thickness")?;
            if !(1..=8).contains(&thickness) {
                return Err("thickness must be 1..8".to_owned());
            }
            Cmd::PaintOutline {
                color,
                inside,
                thickness,
            }
        }
        "paint-select" => {
            need(4, "x0 y0 x1 y1")?;
            Cmd::PaintSelect {
                x0: parse_i32(args[0], "x0")?,
                y0: parse_i32(args[1], "y0")?,
                x1: parse_i32(args[2], "x1")?,
                y1: parse_i32(args[3], "y1")?,
            }
        }
        "paint-onion" => {
            need(1, "on|off")?;
            Cmd::PaintOnion(parse_on(args[0])?)
        }
        "paint-undo" => {
            need(0, "no arguments")?;
            Cmd::PaintUndo
        }
        "search" => Cmd::Search(rest.to_owned()),
        "assert-search-top" => {
            need(2, "<n> <asset key>")?;
            Cmd::AssertSearchTop {
                n: parse_usize(args[0], "n")?,
                key: args[1].to_owned(),
            }
        }
        "brush-from" => {
            need(1, "<asset key>")?;
            Cmd::BrushFrom(args[0].to_owned())
        }
        "assert-brush" => {
            if !(2..=3).contains(&args.len()) {
                return Err("assert-brush: expected <layer> <cell hex> [orientation]".to_owned());
            }
            Cmd::AssertBrush {
                layer: parse_layer(args[0])?,
                cell: parse_cell(args[1])?,
                orientation: args
                    .get(2)
                    .map(|o| parse_u32(o, "orientation"))
                    .transpose()?,
            }
        }
        "paint-brush" => {
            need(2, "x y")?;
            Cmd::PaintBrush {
                x: parse_u32(args[0], "x")?,
                y: parse_u32(args[1], "y")?,
            }
        }
        "assert-recent" => {
            need(1, "<asset key>")?;
            Cmd::AssertRecent(args[0].to_owned())
        }
        "favorite-add" => {
            need(2, "<group> <asset key>")?;
            Cmd::FavoriteAdd {
                group: args[0].to_owned(),
                key: args[1].to_owned(),
            }
        }
        "alias" => {
            let Some((key, text)) = rest.split_once(char::is_whitespace) else {
                return Err("alias: expected <asset key> <text>".to_owned());
            };
            Cmd::Alias {
                key: key.to_owned(),
                text: text.trim().to_owned(),
            }
        }
        "ctrl-k" => Cmd::CtrlK(rest.to_owned()),
        "ctrl-k-close" => {
            need(0, "no arguments")?;
            Cmd::CtrlKClose
        }
        "browser-scope" => {
            need(1, "map|profile|sprites")?;
            Cmd::BrowserScope(
                crate::browser::scope::Scope::from_slug(args[0]).ok_or_else(|| {
                    format!("scope {:?} must be map, profile or sprites", args[0])
                })?,
            )
        }
        "thumb-size" => {
            need(1, "<points>")?;
            Cmd::ThumbSize(parse_f32(args[0], "size")?)
        }
        "tile-menu" => {
            need(1, "<asset key>")?;
            Cmd::TileMenu(args[0].to_owned())
        }
        "tiles-tab" => {
            need(0, "no arguments")?;
            Cmd::TilesTab
        }
        "autosave-now" => {
            need(0, "no arguments")?;
            Cmd::AutosaveNow
        }
        "checkpoint" => {
            if rest.is_empty() {
                return Err("checkpoint: expected <name>".to_owned());
            }
            Cmd::Checkpoint(rest.to_owned())
        }
        "assert-history" => {
            if !(1..=2).contains(&args.len()) {
                return Err(format!(
                    "assert-history: expected <n> [{}]",
                    HISTORY_KINDS.join("|")
                ));
            }
            let kind = match args.get(1) {
                Some(k) => {
                    let k = k.to_ascii_lowercase();
                    if !HISTORY_KINDS.contains(&k.as_str()) {
                        return Err(format!(
                            "assert-history: kind {k:?} must be one of {}",
                            HISTORY_KINDS.join(", ")
                        ));
                    }
                    Some(k)
                }
                None => None,
            };
            Cmd::AssertHistory {
                n: parse_usize(args[0], "n")?,
                kind,
            }
        }
        "history-select" => {
            if rest.is_empty() {
                return Err("history-select: expected <index|name>".to_owned());
            }
            Cmd::HistorySelect(rest.to_owned())
        }
        "assert-diff" => {
            need(2, "<layer> <count>")?;
            Cmd::AssertDiff {
                layer: parse_layer(args[0])?,
                count: parse_usize(args[1], "count")?,
            }
        }
        "history-jump" => {
            need(1, "<layer>")?;
            Cmd::HistoryJump(parse_layer(args[0])?)
        }
        "show-snapshot" => {
            need(1, "on|off")?;
            Cmd::ShowSnapshot(parse_on(args[0])?)
        }
        "restore" => {
            if rest.is_empty() {
                return Err("restore: expected <index|name>".to_owned());
            }
            Cmd::Restore(rest.to_owned())
        }
        "milestone" => {
            if rest.is_empty() {
                return Err("milestone: expected <message>".to_owned());
            }
            Cmd::Milestone(rest.to_owned())
        }
        "assert-milestones" => {
            need(1, "<n>")?;
            Cmd::AssertMilestones(parse_usize(args[0], "n")?)
        }
        "prune" => {
            need(1, "<keep_last>")?;
            let keep = parse_usize(args[0], "keep_last")?;
            if keep == 0 {
                return Err("prune: keep_last must be at least 1".to_owned());
            }
            Cmd::Prune(keep)
        }
        "assert-changed" => {
            need(1, "<n>")?;
            Cmd::AssertChanged(parse_usize(args[0], "n")?)
        }
        "setting" => {
            let Some((key, value)) = rest.split_once(char::is_whitespace) else {
                return Err("setting: expected <key> <value>".to_owned());
            };
            Cmd::Setting {
                key: key.to_owned(),
                value: value.trim().to_owned(),
            }
        }
        "history-tab" => {
            need(0, "no arguments")?;
            Cmd::HistoryTab
        }
        "settings" => {
            need(1, "open|close")?;
            match args[0].to_ascii_lowercase().as_str() {
                "open" => Cmd::Settings(true),
                "close" => Cmd::Settings(false),
                other => return Err(format!("settings {other:?} must be open or close")),
            }
        }
        "add-install" => Cmd::AddInstall((!rest.is_empty()).then(|| rest.to_owned())),
        "close-dialogs" => {
            need(0, "no arguments")?;
            Cmd::CloseDialogs
        }
        other => return Err(format!("unknown command {other:?}")),
    };
    Ok(Some(cmd))
}

/// Parses a whole script. The error carries the 1-based line number.
pub fn parse_script(text: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        match parse_line(line) {
            Ok(Some(cmd)) => out.push((i + 1, cmd)),
            Ok(None) => {}
            Err(e) => return Err(format!("line {}: {e}", i + 1)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_parse_in_every_spelling() {
        assert_eq!(parse_layer("wall0"), Ok(LayerRef::Wall(0)));
        assert_eq!(parse_layer("Wall(2)"), Ok(LayerRef::Wall(2)));
        assert_eq!(parse_layer("w3"), Ok(LayerRef::Wall(3)));
        assert_eq!(parse_layer("FLOOR1"), Ok(LayerRef::Floor(1)));
        assert_eq!(parse_layer("shadow"), Ok(LayerRef::Shadow));
        assert_eq!(parse_layer("tag"), Ok(LayerRef::Tag));
        assert!(parse_layer("wall4").is_err(), "walls are 0..3");
        assert!(parse_layer("floor2").is_err(), "floors are 0..1");
        assert!(parse_layer("wall").is_err(), "needs an index");
        assert!(parse_layer("shadow1").is_err());
        assert!(parse_layer("roof0").is_err());
    }

    #[test]
    fn every_command_parses() {
        let text = "\
# a script
open data\\global\\tiles\\act1\\town\\towne1.ds1

select-box wall0 0 1 10 20   # trailing comment
move-layer wall0 wall2 overwrite
paint floor0 3 4 0x00100201
paint wall1 5 6 100201 18
copy
paste 7 8 skip
paste -1 2 empty
undo
redo
save
assert-dirty true
assert-selection 12
screenshot out\\shot one.png
preview-paste 4 4 overwrite
move-dialog wall0 wall2
zoom 4
wait 30
";
        let lines = parse_script(text).expect("parses");
        let cmds: Vec<&Cmd> = lines.iter().map(|(_, c)| c).collect();
        assert_eq!(
            cmds[0],
            &Cmd::Open("data\\global\\tiles\\act1\\town\\towne1.ds1".to_owned())
        );
        assert_eq!(lines[0].0, 2, "line numbers are 1-based and kept");
        assert_eq!(
            cmds[1],
            &Cmd::SelectBox {
                layer: LayerRef::Wall(0),
                x0: 0,
                y0: 1,
                x1: 10,
                y1: 20
            }
        );
        assert_eq!(
            cmds[2],
            &Cmd::MoveLayer {
                from: LayerRef::Wall(0),
                to: LayerRef::Wall(2),
                mode: MoveMode::Overwrite
            }
        );
        assert_eq!(
            cmds[3],
            &Cmd::Paint {
                layer: LayerRef::Floor(0),
                x: 3,
                y: 4,
                cell: 0x0010_0201,
                orientation: 0
            }
        );
        assert_eq!(
            cmds[4],
            &Cmd::Paint {
                layer: LayerRef::Wall(1),
                x: 5,
                y: 6,
                cell: 0x0010_0201,
                orientation: 18
            }
        );
        assert_eq!(cmds[5], &Cmd::Copy);
        assert_eq!(
            cmds[6],
            &Cmd::Paste {
                x: 7,
                y: 8,
                mode: PasteMode::SkipOccupied
            }
        );
        assert_eq!(
            cmds[7],
            &Cmd::Paste {
                x: -1,
                y: 2,
                mode: PasteMode::OnlyIntoEmptyLayers
            }
        );
        assert_eq!(&cmds[8..11], &[&Cmd::Undo, &Cmd::Redo, &Cmd::Save]);
        assert_eq!(cmds[11], &Cmd::AssertDirty(true));
        assert_eq!(cmds[12], &Cmd::AssertSelection(12));
        assert_eq!(
            cmds[13],
            &Cmd::Screenshot(PathBuf::from("out\\shot one.png"))
        );
        assert!(matches!(cmds[14], Cmd::PreviewPaste { x: 4, y: 4, .. }));
        assert!(matches!(
            cmds[15],
            Cmd::MoveDialog {
                mode: MoveMode::SkipOccupied,
                ..
            }
        ));
        assert_eq!(cmds[16], &Cmd::Zoom(4));
        assert_eq!(cmds[17], &Cmd::Wait(30));
        assert_eq!(cmds.len(), 18);
    }

    #[test]
    fn bad_lines_name_the_problem_and_the_line() {
        let err = parse_script("undo\nfrobnicate 1 2\n").expect_err("unknown");
        assert!(
            err.contains("line 2") && err.contains("frobnicate"),
            "{err}"
        );
        for bad in [
            "open",
            "select-box wall0 1 2 3",
            "select-box wall0 a b c d",
            "move-layer wall0 wall2",
            "move-layer wall0 wall2 maybe",
            "paint floor0 1 2",
            "paint floor0 1 2 zz",
            "paste 1 2",
            "paste 1 2 sideways",
            "copy now",
            "undo 3",
            "assert-dirty maybe",
            "screenshot",
            "zoom fast",
        ] {
            assert!(parse_line(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn sprite_commands_parse() {
        let text = r"
mode sprites
sprite-open data\global\monsters\zm\cof\zmnuhth.cof
sprite-dir 2
sprite-frame 5
sprite-import a.png b.png dirs 1 frames 2 dcc
sprite-import out dirs 8 frames 0 dc6
sprite-save-game data\global\items\inv.dc6
sprite-export-frames C:\out dir
paint-pencil 3 4 120
paint-gradient 0 0 20 20 ramp 140 bayer
paint-gradient 0 0 20 20 two 1 2 fs radial
paint-outline auto outside 1
paint-outline 37 inside 2
paint-undo
mode map
";
        let lines = parse_script(text).expect("parses");
        let cmds: Vec<&Cmd> = lines.iter().map(|(_, c)| c).collect();
        assert_eq!(cmds[0], &Cmd::Mode(true));
        assert_eq!(cmds[2], &Cmd::SpriteDir(2));
        assert_eq!(
            cmds[4],
            &Cmd::SpriteImport {
                files: vec![PathBuf::from("a.png"), PathBuf::from("b.png")],
                dirs: 1,
                frames: 2,
                dcc: true
            }
        );
        assert!(matches!(
            cmds[5],
            Cmd::SpriteImport {
                dcc: false,
                dirs: 8,
                ..
            }
        ));
        assert_eq!(
            cmds[7],
            &Cmd::SpriteExportFrames(PathBuf::from(r"C:\out dir"))
        );
        assert_eq!(
            cmds[8],
            &Cmd::PaintPencil {
                x: 3,
                y: 4,
                index: 120
            }
        );
        assert!(matches!(
            cmds[9],
            Cmd::PaintGradient {
                colors: GradientSpec::Ramp(140),
                dither: capy_paint::Dither::Bayer4,
                radial: false,
                ..
            }
        ));
        assert!(matches!(
            cmds[10],
            Cmd::PaintGradient {
                colors: GradientSpec::Two(1, 2),
                radial: true,
                ..
            }
        ));
        assert_eq!(
            cmds[11],
            &Cmd::PaintOutline {
                color: None,
                inside: false,
                thickness: 1
            }
        );
        assert_eq!(
            cmds[12],
            &Cmd::PaintOutline {
                color: Some(37),
                inside: true,
                thickness: 2
            }
        );
        assert_eq!(cmds[13], &Cmd::PaintUndo);
        assert_eq!(cmds[14], &Cmd::Mode(false));
        for bad in [
            "mode paint",
            "sprite-import a.png dc6",
            "sprite-import dirs 1 frames 1 dcc",
            "sprite-import a.png dirs 1 frames 1 png",
            "paint-pencil 1 2 0",
            "paint-pencil 1 2 300",
            "paint-gradient 0 0 1 1 ramp 5",
            "paint-gradient 0 0 1 1 ramp 5 sparkle",
            "paint-outline auto sideways 1",
            "paint-outline auto outside 9",
        ] {
            assert!(parse_line(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn browser_commands_parse_and_keys_keep_their_hash() {
        let key = r"data\global\tiles\mxl\zone\whimsy\whimsyfloor.dt1#0/23/0/0";
        let text = format!(
            "search color:green type:floor file:whimsyfloor.dt1  # comment\n\
             assert-search-top 5 {key}\n\
             brush-from {key}\n\
             assert-brush floor0 0x017000c2\n\
             assert-brush wall1 017000c2 4\n\
             paint-brush 3 4\n\
             assert-recent {key}\n\
             favorite-add Meadow {key}\n\
             alias {key} capy e2e meadow\n\
             ctrl-k type:floor green\n\
             browser-scope map\n\
             thumb-size 96\n\
             tile-menu {key}\n\
             tiles-tab\n\
             ctrl-k-close\n"
        );
        let lines = parse_script(&text).expect("parses");
        let cmds: Vec<&Cmd> = lines.iter().map(|(_, c)| c).collect();
        assert_eq!(
            cmds[0],
            &Cmd::Search("color:green type:floor file:whimsyfloor.dt1".to_owned())
        );
        assert_eq!(
            cmds[1],
            &Cmd::AssertSearchTop {
                n: 5,
                key: key.to_owned()
            }
        );
        assert_eq!(cmds[2], &Cmd::BrushFrom(key.to_owned()));
        assert_eq!(
            cmds[3],
            &Cmd::AssertBrush {
                layer: LayerRef::Floor(0),
                cell: 0x0170_00C2,
                orientation: None
            }
        );
        assert!(matches!(
            cmds[4],
            Cmd::AssertBrush {
                layer: LayerRef::Wall(1),
                orientation: Some(4),
                ..
            }
        ));
        assert_eq!(cmds[5], &Cmd::PaintBrush { x: 3, y: 4 });
        assert_eq!(cmds[6], &Cmd::AssertRecent(key.to_owned()));
        assert_eq!(
            cmds[7],
            &Cmd::FavoriteAdd {
                group: "Meadow".to_owned(),
                key: key.to_owned()
            }
        );
        assert_eq!(
            cmds[8],
            &Cmd::Alias {
                key: key.to_owned(),
                text: "capy e2e meadow".to_owned()
            }
        );
        assert_eq!(cmds[9], &Cmd::CtrlK("type:floor green".to_owned()));
        assert_eq!(
            cmds[10],
            &Cmd::BrowserScope(crate::browser::scope::Scope::MapTileSet)
        );
        assert_eq!(cmds[11], &Cmd::ThumbSize(96.0));
        assert_eq!(cmds[12], &Cmd::TileMenu(key.to_owned()));
        assert_eq!(cmds[13], &Cmd::TilesTab);
        assert_eq!(cmds[14], &Cmd::CtrlKClose);
        for bad in [
            "assert-search-top 5",
            "assert-search-top x k",
            "assert-brush floor0",
            "alias onlykey",
            "browser-scope everywhere",
            "favorite-add g",
        ] {
            assert!(parse_line(bad).is_err(), "{bad:?} must be rejected");
        }
        assert_eq!(parse_line("#x"), Ok(None));
    }

    #[test]
    fn history_commands_parse() {
        let text = "\
autosave-now
checkpoint before wall
assert-history 1 autosave
assert-history 3
history-select before wall
history-select 0
assert-diff floor0 12
show-snapshot on
history-jump wall1
restore before wall
milestone first version
assert-milestones 1
prune 1
assert-changed 0
setting gamma 1.6
history-tab
settings open
add-install C:\\Games\\Diablo II
add-install
close-dialogs
";
        let lines = parse_script(text).expect("parses");
        let cmds: Vec<&Cmd> = lines.iter().map(|(_, c)| c).collect();
        assert_eq!(cmds[0], &Cmd::AutosaveNow);
        assert_eq!(cmds[1], &Cmd::Checkpoint("before wall".to_owned()));
        assert_eq!(
            cmds[2],
            &Cmd::AssertHistory {
                n: 1,
                kind: Some("autosave".to_owned())
            }
        );
        assert_eq!(cmds[3], &Cmd::AssertHistory { n: 3, kind: None });
        assert_eq!(cmds[4], &Cmd::HistorySelect("before wall".to_owned()));
        assert_eq!(cmds[5], &Cmd::HistorySelect("0".to_owned()));
        assert_eq!(
            cmds[6],
            &Cmd::AssertDiff {
                layer: LayerRef::Floor(0),
                count: 12
            }
        );
        assert_eq!(cmds[7], &Cmd::ShowSnapshot(true));
        assert_eq!(cmds[8], &Cmd::HistoryJump(LayerRef::Wall(1)));
        assert_eq!(cmds[9], &Cmd::Restore("before wall".to_owned()));
        assert_eq!(cmds[10], &Cmd::Milestone("first version".to_owned()));
        assert_eq!(cmds[11], &Cmd::AssertMilestones(1));
        assert_eq!(cmds[12], &Cmd::Prune(1));
        assert_eq!(cmds[13], &Cmd::AssertChanged(0));
        assert_eq!(
            cmds[14],
            &Cmd::Setting {
                key: "gamma".to_owned(),
                value: "1.6".to_owned()
            }
        );
        assert_eq!(cmds[15], &Cmd::HistoryTab);
        assert_eq!(cmds[16], &Cmd::Settings(true));
        assert_eq!(
            cmds[17],
            &Cmd::AddInstall(Some("C:\\Games\\Diablo II".to_owned()))
        );
        assert_eq!(cmds[18], &Cmd::AddInstall(None));
        assert_eq!(cmds[19], &Cmd::CloseDialogs);
        for bad in [
            "checkpoint",
            "assert-history",
            "assert-history 1 milestone",
            "assert-history x",
            "assert-diff floor0",
            "assert-diff roof 1",
            "restore",
            "milestone",
            "prune 0",
            "prune",
            "setting gamma",
            "settings maybe",
            "autosave-now please",
        ] {
            assert!(parse_line(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn blank_and_comment_lines_are_nothing() {
        assert_eq!(parse_line(""), Ok(None));
        assert_eq!(parse_line("   "), Ok(None));
        assert_eq!(parse_line("# nothing"), Ok(None));
        assert_eq!(parse_line("UNDO"), Ok(Some(Cmd::Undo)), "case-insensitive");
        assert_eq!(parse_script("").expect("empty"), Vec::new());
    }
}
