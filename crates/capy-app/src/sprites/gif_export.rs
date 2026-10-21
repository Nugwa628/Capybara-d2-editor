//! The "Export GIF" dialog's logic: which frames and directions go out, how
//! a COF is composed, and the encoded files. No egui here; the window is in
//! `crate::ui::sprites::gif_dialog`.

use std::path::{Path, PathBuf};

use bevy_egui::egui;
use capy_paint::{GifOptions, animated_gif_with, delay_for_fps, rgba_to_indexed};
use capy_sprite::{Sprite, UnitKind};
use d2_assets::{GamePath, Overlay};
use d2_formats::IndexedImage;
use d2_formats::cof::Cof;
use d2_formats::palette::Palette;
use d2_formats::player_seq::{self, SeqStep};
use d2_formats::txt::TxtTable;

use super::cof::CofView;
use super::viewer::ViewerState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirChoice {
    Current,
    All,
    Ticked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// One file per direction (`name_dNN.gif`).
    PerDirection,
    /// One file playing the directions one after the other.
    Sequence,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GifSettings {
    /// First and last frame (inclusive).
    pub start: usize,
    pub end: usize,
    pub dirs: DirChoice,
    pub ticked: Vec<bool>,
    pub output: OutputMode,
    pub fps: f32,
    pub looping: bool,
    /// 1..=4.
    pub scale: u32,
    /// `None` keeps the background transparent.
    pub solid: Option<[u8; 3]>,
    /// A COF layer to export alone; `None` exports the composite.
    pub part: Option<usize>,
    /// A skill's `seqnum`: a character COF plays that sequence instead of
    /// `start..=end` of its own frames (which then index the sequence).
    pub seqnum: Option<u32>,
}

impl GifSettings {
    pub fn new(frames: usize, dirs: usize, fps: f32) -> GifSettings {
        GifSettings {
            start: 0,
            end: frames.saturating_sub(1),
            dirs: DirChoice::Current,
            ticked: vec![false; dirs],
            output: OutputMode::PerDirection,
            fps,
            looping: true,
            scale: 1,
            solid: None,
            part: None,
            seqnum: None,
        }
    }

    pub fn range(&self, frames: usize) -> (usize, usize) {
        clamp_range(self.start, self.end, frames)
    }

    pub fn options(&self) -> GifOptions {
        GifOptions {
            delay_cs: delay_for_fps(self.fps),
            looping: self.looping,
            scale: self.scale.clamp(1, 4),
            background: self.solid,
        }
    }
}

/// An inclusive range inside `0..frames`, swapped if reversed. `(0, 0)` when
/// there are no frames.
pub fn clamp_range(start: usize, end: usize, frames: usize) -> (usize, usize) {
    let last = frames.saturating_sub(1);
    let (a, b) = (start.min(last), end.min(last));
    (a.min(b), a.max(b))
}

/// Parses "3-7", "3 - 7", "5" (one frame) into 0-based inclusive bounds from
/// 1-based text.
pub fn parse_range(text: &str, frames: usize) -> Option<(usize, usize)> {
    let t = text.trim();
    let (a, b) = match t.split_once('-') {
        Some((a, b)) => (
            a.trim().parse::<usize>().ok()?,
            b.trim().parse::<usize>().ok()?,
        ),
        None => {
            let n = t.parse::<usize>().ok()?;
            (n, n)
        }
    };
    if a == 0 || b == 0 || a > frames || b > frames {
        return None;
    }
    Some(clamp_range(a - 1, b - 1, frames))
}

/// The directions to export, ascending and inside `0..total`.
pub fn selected_dirs(s: &GifSettings, current: usize, total: usize) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    match s.dirs {
        DirChoice::Current => vec![current.min(total - 1)],
        DirChoice::All => (0..total).collect(),
        DirChoice::Ticked => (0..total)
            .filter(|&d| s.ticked.get(d).copied().unwrap_or(false))
            .collect(),
    }
}

/// What the dialog draws from.
#[derive(Clone, Copy)]
pub enum GifSource<'a> {
    Sprite(&'a Sprite),
    Cof(&'a CofView),
    /// A character COF played in a skill's sequence order.
    Seq(&'a SeqPlan),
}

impl GifSource<'_> {
    pub fn directions(&self) -> usize {
        match self {
            GifSource::Sprite(s) => s.directions.len(),
            GifSource::Cof(c) => c.directions(),
            GifSource::Seq(p) => p.directions(),
        }
    }

    pub fn frames(&self) -> usize {
        match self {
            GifSource::Sprite(s) => s.frames_per_direction(),
            GifSource::Cof(c) => c.frames(),
            GifSource::Seq(p) => p.steps.len(),
        }
    }

    /// The frames `start..=end` of one direction as indexed images (a COF
    /// frame is composed with its blend modes, then mapped to the palette).
    pub fn frames_of(
        &self,
        dir: usize,
        (start, end): (usize, usize),
        palette: &Palette,
    ) -> Result<Vec<IndexedImage>, String> {
        match self {
            GifSource::Sprite(s) => {
                let frames = s
                    .directions
                    .get(dir)
                    .ok_or_else(|| format!("no direction {dir}"))?;
                Ok((start..=end)
                    .filter_map(|f| frames.get(f).cloned())
                    .collect())
            }
            GifSource::Cof(c) => (start..=end)
                .map(|f| indexed_frame(c, dir, f, palette))
                .collect(),
            GifSource::Seq(p) => (start..=end)
                .map(|i| {
                    let &(view, frame) = p
                        .steps
                        .get(i)
                        .ok_or_else(|| format!("no sequence step {}", i + 1))?;
                    indexed_frame(&p.views[view], dir, frame, palette)
                })
                .collect(),
        }
    }
}

/// One composed COF frame mapped to the palette.
fn indexed_frame(
    c: &CofView,
    dir: usize,
    frame: usize,
    palette: &Palette,
) -> Result<IndexedImage, String> {
    let r = c.compose(dir, frame, palette)?;
    Ok(rgba_to_indexed(
        r.width, r.height, r.offset_x, r.offset_y, &r.pixels, palette,
    ))
}

/// A step list resolved against the frame counts of the mode COFs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// (mode, frame) in play order, frames inside their COF.
    pub order: Vec<(&'static str, usize)>,
    /// Modes whose COF is missing (each once, in first-use order).
    pub missing: Vec<&'static str>,
    /// Steps that named a frame past the end of their COF.
    pub clamped: usize,
}

/// Orders a sequence's steps for playback. `frames_in(mode)` is the frame
/// count of that mode's COF, `None` when there is none. A step past the last
/// frame shows the last frame (the game's behaviour is not known; the
/// shipped data has a handful of these). Steps of a missing mode are left
/// out and reported.
pub fn resolve_steps(steps: &[SeqStep], frames_in: impl Fn(&str) -> Option<usize>) -> Resolved {
    let mut out = Resolved::default();
    for s in steps {
        match frames_in(s.mode) {
            None | Some(0) => {
                if !out.missing.contains(&s.mode) {
                    out.missing.push(s.mode);
                }
            }
            Some(n) => {
                let frame = usize::from(s.frame);
                out.clamped += usize::from(frame >= n);
                out.order.push((s.mode, frame.min(n - 1)));
            }
        }
    }
    out
}

/// A character COF's frames in a skill's sequence order: one loaded COF per
/// mode the sequence uses (same token, weapon class and armour as the
/// Viewer's) and the (COF, frame) list to play.
pub struct SeqPlan {
    pub seqnum: u32,
    /// The Viewer revision it was built from.
    pub revision: u64,
    views: Vec<CofView>,
    steps: Vec<(usize, usize)>,
    /// Why it cannot be exported (shown in orange).
    pub problems: Vec<String>,
    /// Worth knowing, but it still exports.
    pub notes: Vec<String>,
}

impl SeqPlan {
    fn directions(&self) -> usize {
        self.views
            .iter()
            .map(CofView::directions)
            .min()
            .unwrap_or(0)
    }

    pub fn playable(&self) -> bool {
        self.problems.is_empty() && !self.steps.is_empty()
    }

    /// Resolves sequence `seqnum` for `base`'s weapon class, loading the COF
    /// of every mode it uses through `overlay`.
    pub fn build(
        seqnum: u32,
        base: &CofView,
        revision: u64,
        overlay: Option<&Overlay>,
        names: &[GamePath],
    ) -> SeqPlan {
        let mut plan = SeqPlan {
            seqnum,
            revision,
            views: Vec::new(),
            steps: Vec::new(),
            problems: Vec::new(),
            notes: Vec::new(),
        };
        let class = base.name.class.as_str();
        let Some(seq) = player_seq::sequence(seqnum) else {
            plan.problems.push(format!("No sequence {seqnum}"));
            return plan;
        };
        let Some(steps) = seq.steps(class) else {
            plan.problems.push(format!(
                "{} has no sequence for the {} weapon class",
                seq.name,
                class.to_ascii_uppercase()
            ));
            return plan;
        };
        let prefer: Vec<(String, String)> = base
            .layers
            .iter()
            .map(|l| (l.code.to_owned(), l.armor.clone()))
            .collect();
        let mut modes: Vec<&'static str> = Vec::new();
        for s in steps {
            if !modes.contains(&s.mode) {
                modes.push(s.mode);
            }
        }
        let mut loaded: Vec<Option<usize>> = Vec::new();
        for &mode in &modes {
            let mut name = base.name.clone();
            name.mode = mode.to_owned();
            let path = name.game_path(base.kind);
            let bytes = overlay.and_then(|o| o.read_bytes(&path).ok().flatten());
            let view = match bytes.map(|b| Cof::parse(&b)) {
                None => {
                    plan.problems.push(format!(
                        "Missing COF for mode {}: {}",
                        mode.to_uppercase(),
                        path
                    ));
                    None
                }
                Some(Err(e)) => {
                    plan.problems.push(format!("Cannot read {path}: {e}"));
                    None
                }
                Some(Ok(cof)) => {
                    let mut v = CofView::load(cof, name, Some(path), overlay, names, &prefer);
                    // Keep the layers the Viewer has hidden hidden.
                    for (i, l) in v.layers.clone().iter().enumerate() {
                        let hidden = base
                            .layers
                            .iter()
                            .any(|b| b.component == l.component && !b.visible);
                        if hidden {
                            v.set_visible(i, false);
                        }
                    }
                    plan.views.push(v);
                    Some(plan.views.len() - 1)
                }
            };
            loaded.push(view);
        }
        let index_of = |mode: &str| {
            let at = modes.iter().position(|m| *m == mode)?;
            loaded[at]
        };
        let resolved = resolve_steps(steps, |m| index_of(m).map(|i| plan.views[i].frames()));
        plan.steps = resolved
            .order
            .iter()
            .filter_map(|&(m, f)| Some((index_of(m)?, f)))
            .collect();
        if resolved.clamped > 0 {
            plan.notes.push(format!(
                "{} step(s) name a frame past the end of their COF and show its last frame",
                resolved.clamped
            ));
        }
        plan
    }
}

/// One choice of the "Skill sequence" list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqChoice {
    pub seqnum: u32,
    /// "Leap (seq 13)".
    pub label: String,
}

/// The skills of `token`'s class that play a player sequence, from the
/// profile's `skills.txt`; every sequence by name when there is none (a
/// compiled-only profile, or a token that is not a player class).
pub fn seq_choices(overlay: Option<&Overlay>, token: &str) -> Vec<SeqChoice> {
    let table = overlay
        .and_then(|o| {
            o.read_bytes(&GamePath::new("data\\global\\excel\\skills.txt"))
                .ok()
                .flatten()
        })
        .and_then(|b| TxtTable::parse(&b).ok());
    let class = player_seq::class_of_token(token);
    let mut out: Vec<SeqChoice> = table
        .map(|t| player_seq::skills_using(&t))
        .unwrap_or_default()
        .into_iter()
        .filter(|s| class.is_none_or(|c| s.class == c))
        .map(|s| SeqChoice {
            seqnum: s.seqnum,
            label: format!("{} (seq {})", s.name, s.seqnum),
        })
        .collect();
    if out.is_empty() {
        out = player_seq::PLAYER_SEQUENCES
            .iter()
            .map(|s| SeqChoice {
                seqnum: u32::from(s.seqnum),
                label: format!("{} (seq {})", s.name, s.seqnum),
            })
            .collect();
    }
    out
}

/// One encoded GIF.
pub struct GifFile {
    /// The direction, or `None` for a sequence of several.
    pub dir: Option<usize>,
    pub frames: usize,
    pub bytes: Vec<u8>,
}

/// Encodes what `s` selects: one file per direction, or one sequence.
pub fn build(
    source: GifSource<'_>,
    s: &GifSettings,
    current_dir: usize,
    palette: &Palette,
) -> Result<Vec<GifFile>, String> {
    let range = s.range(source.frames());
    let dirs = selected_dirs(s, current_dir, source.directions());
    if dirs.is_empty() {
        return Err("tick at least one direction".to_owned());
    }
    let opts = s.options();
    let encode = |frames: &[IndexedImage]| {
        animated_gif_with(frames, palette, &opts).map_err(|e| e.to_string())
    };
    if s.output == OutputMode::Sequence || dirs.len() == 1 {
        let mut all = Vec::new();
        for &d in &dirs {
            all.extend(source.frames_of(d, range, palette)?);
        }
        return Ok(vec![GifFile {
            dir: (dirs.len() == 1).then(|| dirs[0]),
            frames: all.len(),
            bytes: encode(&all)?,
        }]);
    }
    dirs.iter()
        .map(|&d| {
            let frames = source.frames_of(d, range, palette)?;
            Ok(GifFile {
                dir: Some(d),
                frames: frames.len(),
                bytes: encode(&frames)?,
            })
        })
        .collect()
}

/// Where each file goes: the picked path for a single file, else
/// `<stem>_dNN.gif` beside it.
pub fn output_paths(picked: &Path, files: &[GifFile]) -> Vec<PathBuf> {
    let single = picked.with_extension("gif");
    if files.len() == 1 {
        return vec![single];
    }
    let stem = picked
        .file_stem()
        .map_or_else(|| "sprite".to_owned(), |s| s.to_string_lossy().into_owned());
    files
        .iter()
        .map(|f| single.with_file_name(format!("{stem}_d{:02}.gif", f.dir.unwrap_or(0))))
        .collect()
}

/// Writes the files; the message lists them.
pub fn write(picked: &Path, files: &[GifFile]) -> Result<String, String> {
    let paths = output_paths(picked, files);
    for (p, f) in paths.iter().zip(files) {
        std::fs::write(p, &f.bytes).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    }
    let frames: usize = files.iter().map(|f| f.frames).sum();
    Ok(match paths.as_slice() {
        [p] => format!("Exported {frames} frames to {}", p.display()),
        [first, .., last] => format!(
            "Exported {} GIFs ({frames} frames) {} .. {}",
            paths.len(),
            first.display(),
            last.display()
        ),
        [] => "Nothing to export".to_owned(),
    })
}

/// The dialog's state while it is open.
pub struct GifDialog {
    /// Which panel's sprite: the Viewer's (a COF is read live) or a copy of
    /// the Editor's.
    pub from_editor: bool,
    /// The direction on show when the dialog opened.
    pub current_dir: usize,
    /// A copy of the sprite (always for the Editor; for the Viewer only
    /// when it is not a COF).
    pub sprite: Option<Sprite>,
    pub stem: String,
    pub settings: GifSettings,
    /// The text of the range box.
    pub range_text: String,
    /// Thumbnails of the frames of `thumb_dir`.
    pub thumbs: Vec<Option<ThumbTex>>,
    pub thumb_key: u64,
    /// Preview playback position and clock.
    pub preview_frame: usize,
    pub preview_accum: f64,
    pub preview_clock: Option<f64>,
    /// Shown in red when the last export failed.
    pub error: Option<String>,
    pub open: bool,
    /// Frames per direction of the source.
    pub frame_count: usize,
    /// The skills offered for a character COF (`None` until first needed).
    pub seq_choices: Option<Vec<SeqChoice>>,
    /// The chosen skill's sequence, loaded.
    pub seq_plan: Option<SeqPlan>,
}

#[derive(Clone)]
pub struct ThumbTex {
    pub handle: egui::TextureHandle,
    pub offset: (i32, i32),
    pub size: (u32, u32),
}

impl GifDialog {
    pub fn new(
        from_editor: bool,
        current_dir: usize,
        sprite: Option<Sprite>,
        stem: String,
        settings: GifSettings,
    ) -> GifDialog {
        let range_text = range_label(&settings);
        let frame_count = settings.end + 1;
        GifDialog {
            from_editor,
            current_dir,
            sprite,
            stem,
            settings,
            range_text,
            thumbs: Vec::new(),
            thumb_key: u64::MAX,
            preview_frame: 0,
            preview_accum: 0.0,
            preview_clock: None,
            error: None,
            open: true,
            frame_count,
            seq_choices: None,
            seq_plan: None,
        }
    }

    /// Why the chosen skill sequence cannot be exported, if it cannot.
    pub fn sequence_problem(&self) -> Option<String> {
        let want = self.settings.seqnum?;
        match &self.seq_plan {
            Some(p) if p.seqnum == want && p.problems.is_empty() && p.steps.is_empty() => {
                Some("The sequence has no frames".to_owned())
            }
            Some(p) if p.seqnum == want => p.problems.first().cloned(),
            _ => Some("The sequence is still loading".to_owned()),
        }
    }
}

/// Brings the dialog's skill list and loaded sequence up to date with the
/// Viewer's COF and the chosen skill; resets the frame range when the
/// sequence changes. Call before drawing or exporting.
pub fn sync_sequence(
    dlg: &mut GifDialog,
    viewer: &ViewerState,
    overlay: Option<&Overlay>,
    names: &[GamePath],
) {
    let base = viewer
        .cof()
        .filter(|c| !dlg.from_editor && c.kind == UnitKind::Character);
    let Some(base) = base else {
        // Nothing to sequence (the Viewer moved on to something else).
        if dlg.settings.seqnum.take().is_some() || dlg.seq_plan.take().is_some() {
            reset_range(dlg, viewer);
        }
        return;
    };
    if dlg.seq_choices.is_none() {
        dlg.seq_choices = Some(seq_choices(overlay, &base.name.token));
    }
    let have = dlg.seq_plan.as_ref().map(|p| (p.seqnum, p.revision));
    let want = dlg.settings.seqnum.map(|n| (n, viewer.revision));
    if have == want {
        return;
    }
    dlg.seq_plan = want.map(|(n, rev)| SeqPlan::build(n, base, rev, overlay, names));
    reset_range(dlg, viewer);
}

/// The whole of the current source as the frame range.
fn reset_range(dlg: &mut GifDialog, viewer: &ViewerState) {
    let frames = source_of(viewer, dlg).map_or(0, |s| s.frames());
    dlg.frame_count = frames;
    dlg.settings.start = 0;
    dlg.settings.end = frames.saturating_sub(1);
    dlg.range_text = range_label(&dlg.settings);
    dlg.thumb_key = u64::MAX;
    dlg.preview_frame = 0;
}

pub fn range_label(s: &GifSettings) -> String {
    format!("{}-{}", s.start + 1, s.end + 1)
}

/// What `dlg` exports from: a single COF layer, the dialog's copy of a
/// sprite, or the Viewer's COF composite.
pub fn source_of<'a>(viewer: &'a ViewerState, dlg: &'a GifDialog) -> Option<GifSource<'a>> {
    if let (Some(n), Some(p)) = (dlg.settings.seqnum, &dlg.seq_plan)
        && p.seqnum == n
        && p.playable()
        && !dlg.from_editor
    {
        return Some(GifSource::Seq(p));
    }
    if let (Some(i), Some(c)) = (dlg.settings.part, viewer.cof())
        && !dlg.from_editor
    {
        return c
            .layers
            .get(i)
            .and_then(|l| l.sprite.as_ref())
            .map(GifSource::Sprite);
    }
    match &dlg.sprite {
        Some(s) => Some(GifSource::Sprite(s)),
        None => viewer.cof().map(GifSource::Cof),
    }
}

/// Applies one `key=value` setting (scripts and tests):
/// `range=3-7` (1-based), `dirs=cur|all|0,2`, `output=per|seq`, `fps=N`,
/// `loop=on|off`, `scale=N`, `bg=none|RRGGBB`, `part=none|N`,
/// `seq=none|N` (a skill's seqnum; the range then covers the sequence, so set
/// `range` in a later command).
pub fn apply_setting(
    s: &mut GifSettings,
    key: &str,
    value: &str,
    frames: usize,
    dirs: usize,
) -> Result<(), String> {
    let bad = || format!("sprite-gif-set: bad value {value:?} for {key}");
    match key {
        "range" => {
            let (a, b) = parse_range(value, frames).ok_or_else(bad)?;
            (s.start, s.end) = (a, b);
        }
        "dirs" => match value {
            "cur" | "current" => s.dirs = DirChoice::Current,
            "all" => s.dirs = DirChoice::All,
            list => {
                s.ticked = vec![false; dirs];
                for n in list.split(',') {
                    let d: usize = n.trim().parse().map_err(|_| bad())?;
                    *s.ticked.get_mut(d).ok_or_else(bad)? = true;
                }
                s.dirs = DirChoice::Ticked;
            }
        },
        "output" => {
            s.output = match value {
                "per" => OutputMode::PerDirection,
                "seq" => OutputMode::Sequence,
                _ => return Err(bad()),
            }
        }
        "fps" => s.fps = value.parse().map_err(|_| bad())?,
        "loop" => s.looping = matches!(value, "on" | "true" | "1"),
        "scale" => s.scale = value.parse::<u32>().map_err(|_| bad())?.clamp(1, 4),
        "bg" => {
            s.solid = if value == "none" {
                None
            } else {
                let v =
                    u32::from_str_radix(value.trim_start_matches('#'), 16).map_err(|_| bad())?;
                Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
            }
        }
        "part" => {
            s.part = if value == "none" {
                None
            } else {
                Some(value.parse().map_err(|_| bad())?)
            }
        }
        "seq" => {
            s.seqnum = if value == "none" {
                None
            } else {
                Some(value.parse().map_err(|_| bad())?)
            }
        }
        _ => return Err(format!("sprite-gif-set: unknown setting {key:?}")),
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn sprite(dirs: usize, frames: usize) -> Sprite {
        let mut directions = Vec::new();
        for d in 0..dirs {
            let mut fs = Vec::new();
            for f in 0..frames {
                let mut img = IndexedImage::new(2, 2, 0, 0);
                img.pixels.fill((d * 10 + f + 1) as u8);
                img.alpha.fill(255);
                fs.push(img);
            }
            directions.push(fs);
        }
        Sprite { directions }
    }

    fn pal() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8, 255 - i as u8, 7];
        }
        Palette { colors }
    }

    fn decode(bytes: &[u8]) -> Vec<gif::Frame<'static>> {
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::Indexed);
        let mut d = o.read_info(std::io::Cursor::new(bytes)).unwrap();
        let mut v = Vec::new();
        while let Some(f) = d.read_next_frame().unwrap() {
            v.push(f.clone());
        }
        v
    }

    #[test]
    fn ranges_clamp_swap_and_parse() {
        assert_eq!(clamp_range(2, 5, 10), (2, 5));
        assert_eq!(clamp_range(7, 3, 10), (3, 7));
        assert_eq!(clamp_range(0, 99, 4), (0, 3));
        assert_eq!(clamp_range(5, 6, 0), (0, 0));
        assert_eq!(parse_range("3-7", 10), Some((2, 6)));
        assert_eq!(parse_range(" 7 - 3 ", 10), Some((2, 6)));
        assert_eq!(parse_range("4", 10), Some((3, 3)));
        assert_eq!(parse_range("0-3", 10), None);
        assert_eq!(parse_range("2-11", 10), None);
        assert_eq!(parse_range("x", 10), None);
    }

    #[test]
    fn direction_selection() {
        let mut s = GifSettings::new(4, 8, 25.0);
        assert_eq!(selected_dirs(&s, 3, 8), vec![3]);
        assert_eq!(selected_dirs(&s, 99, 8), vec![7], "current is clamped");
        s.dirs = DirChoice::All;
        assert_eq!(selected_dirs(&s, 0, 8), (0..8).collect::<Vec<_>>());
        s.dirs = DirChoice::Ticked;
        assert!(selected_dirs(&s, 0, 8).is_empty());
        s.ticked[6] = true;
        s.ticked[1] = true;
        assert_eq!(selected_dirs(&s, 0, 8), vec![1, 6], "ascending");
        assert!(selected_dirs(&s, 0, 0).is_empty());
    }

    #[test]
    fn options_follow_the_settings() {
        let mut s = GifSettings::new(4, 1, 12.5);
        let o = s.options();
        assert_eq!(
            (o.delay_cs, o.looping, o.scale, o.background),
            (8, true, 1, None)
        );
        s.fps = 100.0;
        s.looping = false;
        s.scale = 9;
        s.solid = Some([1, 2, 3]);
        let o = s.options();
        assert_eq!(o.delay_cs, 2, "never below 2 cs");
        assert!(!o.looping);
        assert_eq!(o.scale, 4);
        assert_eq!(o.background, Some([1, 2, 3]));
        assert_eq!(delay_for_fps(f32::NAN), 4);
    }

    #[test]
    fn per_direction_and_sequence_outputs() {
        let sp = sprite(3, 5);
        let p = pal();
        let mut s = GifSettings::new(5, 3, 25.0);
        s.start = 1;
        s.end = 3;
        s.dirs = DirChoice::All;
        let files = build(GifSource::Sprite(&sp), &s, 0, &p).unwrap();
        assert_eq!(files.len(), 3);
        for (d, f) in files.iter().enumerate() {
            assert_eq!((f.dir, f.frames), (Some(d), 3));
            let frames = decode(&f.bytes);
            assert_eq!(frames.len(), 3);
            for (i, fr) in frames.iter().enumerate() {
                assert_eq!(
                    fr.buffer[0],
                    (d * 10 + 1 + i + 1) as u8,
                    "dir {d} frame {i}"
                );
                assert_eq!(fr.delay, 4);
            }
        }
        s.output = OutputMode::Sequence;
        let files = build(GifSource::Sprite(&sp), &s, 0, &p).unwrap();
        assert_eq!(files.len(), 1);
        let frames = decode(&files[0].bytes);
        let firsts: Vec<u8> = frames.iter().map(|f| f.buffer[0]).collect();
        assert_eq!(firsts, vec![2, 3, 4, 12, 13, 14, 22, 23, 24]);
        s.dirs = DirChoice::Ticked;
        assert!(build(GifSource::Sprite(&sp), &s, 0, &p).is_err());
    }

    #[test]
    fn a_sequence_plays_its_steps_in_order_across_modes() {
        // Fend with a spear: A1 and A2 interleave.
        let fend = player_seq::sequence(9).unwrap();
        let steps = fend.steps("2ht").unwrap();
        let r = resolve_steps(steps, |m| Some(if m == "a1" { 20 } else { 16 }));
        assert_eq!(r.missing, Vec::<&str>::new());
        assert_eq!(r.clamped, 0);
        assert_eq!(
            &r.order[..5],
            &[("a1", 0), ("a1", 1), ("a1", 2), ("a2", 2), ("a1", 6)]
        );
        assert_eq!(r.order.len(), 15, "the padding entry is gone");

        // Whirlwind (one mode): frame 3 is played twice.
        let ww = player_seq::sequence(10).unwrap().steps("hth").unwrap();
        let r = resolve_steps(ww, |_| Some(10));
        let frames: Vec<usize> = r.order.iter().map(|&(_, f)| f).collect();
        assert_eq!(frames, vec![0, 1, 2, 3, 3, 4, 5, 6]);
        assert!(r.order.iter().all(|&(m, _)| m == "a1"));

        // A missing mode is reported once; a short COF clamps.
        let r = resolve_steps(steps, |m| (m == "a1").then_some(4));
        assert_eq!(r.missing, vec!["a2"]);
        assert_eq!(r.clamped, 7);
        assert!(r.order.iter().all(|&(m, f)| m == "a1" && f < 4));
    }

    #[test]
    fn output_names() {
        let mk = |d| GifFile {
            dir: Some(d),
            frames: 1,
            bytes: Vec::new(),
        };
        let one = output_paths(Path::new("out/a.png"), &[mk(3)]);
        assert_eq!(one, vec![PathBuf::from("out/a.gif")]);
        let many = output_paths(Path::new("out/a.gif"), &[mk(0), mk(5)]);
        assert_eq!(
            many,
            vec![
                PathBuf::from("out/a_d00.gif"),
                PathBuf::from("out/a_d05.gif")
            ]
        );
    }
}
