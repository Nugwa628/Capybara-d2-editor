//! The editable sprite: frames, palette, ramps and unlimited undo/redo.
//!
//! # History
//!
//! Every mutating call is one undo step, or part of an open group
//! ([`PaintSession::begin_group`] / [`PaintSession::end_group`], nestable;
//! only the outermost pair counts). Steps store pixel diffs of the changed
//! rectangle only (before and after), whole frames for structural changes of a
//! frame (canvas resize, rotate), and the frame lists for insert / delete /
//! move / mirror. A call that changes nothing records nothing.
//!
//! `undo`, `redo` and `mark_saved` close an open group first.
//!
//! # Dirty tracking
//!
//! Each recorded step has a unique id; the session is clean when the step on
//! top of the undo stack is the one that was on top at `mark_saved` (so
//! undoing back to the saved state is clean again). `set_palette` is not
//! undoable and makes the session dirty until the next `mark_saved`.
//! `set_display_palette` is the view-only variant: it swaps the palette (and
//! bumps `revision` so caches refresh) but never dirties the session and
//! records no history, for previewing the sprite under another act.

use crate::error::{PaintError, Result, invalid};
use crate::ramps::Ramps;
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

/// Identifies one frame: `frames[dir][frame]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameRef {
    pub dir: usize,
    pub frame: usize,
}

/// What a tool writes: a palette index (never 0) or transparency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    Index(u8),
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushShape {
    Square,
    Round,
}

/// A stamp of `size` x `size` pixels (0 counts as 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brush {
    pub size: u32,
    pub shape: BrushShape,
}

/// Largest accepted frame side (canvas resize, input frames).
pub(crate) const MAX_SIDE: u32 = 8192;

/// A rectangle of before/after pixels.
#[derive(Clone, Debug)]
pub(crate) struct RectDiff {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    before: (Vec<u8>, Vec<u8>),
    after: (Vec<u8>, Vec<u8>),
}

#[derive(Clone, Debug)]
pub(crate) enum Edit {
    Rect {
        f: FrameRef,
        diff: RectDiff,
    },
    Frame {
        f: FrameRef,
        before: Box<IndexedImage>,
        after: Box<IndexedImage>,
    },
    Insert {
        at: usize,
        frames: Vec<IndexedImage>,
    },
    Delete {
        at: usize,
        frames: Vec<IndexedImage>,
    },
    Move {
        from: usize,
        to: usize,
    },
    Direction {
        dir: usize,
        before: Vec<IndexedImage>,
        after: Vec<IndexedImage>,
    },
}

type Frames = Vec<Vec<IndexedImage>>;

impl Edit {
    fn apply(&self, frames: &mut Frames, forward: bool) {
        match self {
            Edit::Rect { f, diff } => {
                let data = if forward { &diff.after } else { &diff.before };
                let Some(img) = frames.get_mut(f.dir).and_then(|d| d.get_mut(f.frame)) else {
                    return;
                };
                let width = img.width as usize;
                for row in 0..diff.h {
                    let dst = (diff.y + row) * width + diff.x;
                    let src = row * diff.w;
                    let (Some(dp), Some(da), Some(sp), Some(sa)) = (
                        img.pixels.get_mut(dst..dst + diff.w),
                        img.alpha.get_mut(dst..dst + diff.w),
                        data.0.get(src..src + diff.w),
                        data.1.get(src..src + diff.w),
                    ) else {
                        return;
                    };
                    dp.copy_from_slice(sp);
                    da.copy_from_slice(sa);
                }
            }
            Edit::Frame { f, before, after } => {
                let src = if forward { after } else { before };
                if let Some(slot) = frames.get_mut(f.dir).and_then(|d| d.get_mut(f.frame)) {
                    *slot = (**src).clone();
                }
            }
            Edit::Insert { at, frames: new } => {
                if forward {
                    insert_all(frames, *at, new);
                } else {
                    remove_all(frames, *at);
                }
            }
            Edit::Delete { at, frames: old } => {
                if forward {
                    remove_all(frames, *at);
                } else {
                    insert_all(frames, *at, old);
                }
            }
            Edit::Move { from, to } => {
                let (a, b) = if forward { (*from, *to) } else { (*to, *from) };
                for dir in frames.iter_mut() {
                    if a < dir.len() {
                        let img = dir.remove(a);
                        dir.insert(b.min(dir.len()), img);
                    }
                }
            }
            Edit::Direction { dir, before, after } => {
                let src = if forward { after } else { before };
                if let Some(slot) = frames.get_mut(*dir) {
                    *slot = src.clone();
                }
            }
        }
    }
}

fn insert_all(frames: &mut Frames, at: usize, new: &[IndexedImage]) {
    for (dir, img) in frames.iter_mut().zip(new) {
        dir.insert(at.min(dir.len()), img.clone());
    }
}

fn remove_all(frames: &mut Frames, at: usize) {
    for dir in frames.iter_mut() {
        if at < dir.len() {
            dir.remove(at);
        }
    }
}

#[derive(Clone, Debug)]
struct Step {
    id: u64,
    label: String,
    edits: Vec<Edit>,
}

#[derive(Debug)]
struct Group {
    depth: usize,
    step: Step,
}

/// An editable sprite with unlimited undo/redo. Every mutating call is one
/// undo step (or part of an open group). No-op calls return `Ok` and record
/// nothing.
///
/// Frames are normalised on construction and kept normalised: alpha is 0 or
/// 255, transparent pixels carry index 0, and opaque pixels never use index 0.
#[derive(Debug)]
pub struct PaintSession {
    pub(crate) frames: Frames,
    palette: Palette,
    ramps: Ramps,
    undo: Vec<Step>,
    redo: Vec<Step>,
    group: Option<Group>,
    revision: u64,
    next_id: u64,
    saved_id: u64,
    palette_dirty: bool,
}

/// Enforce the palette-lock invariants on one image in place.
pub(crate) fn canonicalize(img: &mut IndexedImage) {
    for (p, a) in img.pixels.iter_mut().zip(img.alpha.iter_mut()) {
        if *a != 0 {
            *a = 255;
        }
        if *a == 0 || *p == 0 {
            *a = 0;
            *p = 0;
        }
    }
}

impl PaintSession {
    /// Builds a session. Frames must be consistent (`pixels` and `alpha` hold
    /// `width * height` entries) and every direction must have the same number
    /// of frames. They are normalised: any non-zero alpha becomes 255, and an
    /// opaque pixel with index 0 becomes transparent.
    pub fn new(mut frames: Vec<Vec<IndexedImage>>, palette: Palette) -> Result<PaintSession> {
        let count = frames.first().map_or(0, Vec::len);
        for (d, dir) in frames.iter_mut().enumerate() {
            if dir.len() != count {
                return invalid(format!(
                    "direction {d} has {} frames, direction 0 has {count}",
                    dir.len()
                ));
            }
            for (i, img) in dir.iter_mut().enumerate() {
                let len = (img.width as usize).saturating_mul(img.height as usize);
                if img.pixels.len() != len || img.alpha.len() != len {
                    return invalid(format!(
                        "frame {d}/{i}: pixel buffers do not match its size"
                    ));
                }
                if img.width > MAX_SIDE || img.height > MAX_SIDE {
                    return invalid(format!("frame {d}/{i} is larger than {MAX_SIDE} pixels"));
                }
                canonicalize(img);
            }
        }
        let ramps = Ramps::build(&palette);
        Ok(PaintSession {
            frames,
            palette,
            ramps,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
            revision: 0,
            next_id: 1,
            saved_id: 0,
            palette_dirty: false,
        })
    }

    pub fn frames(&self) -> &[Vec<IndexedImage>] {
        &self.frames
    }

    pub fn frame(&self, f: FrameRef) -> Option<&IndexedImage> {
        self.frames.get(f.dir)?.get(f.frame)
    }

    pub fn into_frames(self) -> Vec<Vec<IndexedImage>> {
        self.frames
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn ramps(&self) -> &Ramps {
        &self.ramps
    }

    /// Swap palette (e.g. another act); pixels keep their indices. Not
    /// undoable; the session is dirty until the next `mark_saved`. Use
    /// [`PaintSession::set_display_palette`] when the swap only changes how
    /// the sprite is shown.
    pub fn set_palette(&mut self, palette: Palette) {
        self.swap_palette(palette);
        self.palette_dirty = true;
    }

    /// Display-only palette swap (e.g. previewing another act): pixels keep
    /// their indices and the ramps follow the new palette, like `set_palette`,
    /// but the dirty state is left as it was and no history is recorded.
    /// `revision` still increases so caches built from the palette refresh.
    /// A pending `set_palette` change stays dirty.
    pub fn set_display_palette(&mut self, palette: Palette) {
        self.swap_palette(palette);
    }

    fn swap_palette(&mut self, palette: Palette) {
        self.ramps = Ramps::build(&palette);
        self.palette = palette;
        self.revision += 1;
    }

    // ---- history ----

    pub fn undo(&mut self) -> bool {
        self.end_all_groups();
        let Some(step) = self.undo.pop() else {
            return false;
        };
        for edit in step.edits.iter().rev() {
            edit.apply(&mut self.frames, false);
        }
        self.redo.push(step);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_all_groups();
        let Some(step) = self.redo.pop() else {
            return false;
        };
        for edit in &step.edits {
            edit.apply(&mut self.frames, true);
        }
        self.undo.push(step);
        self.revision += 1;
        true
    }

    pub fn can_undo(&self) -> bool {
        self.undo.last().is_some()
            || self
                .group
                .as_ref()
                .is_some_and(|g| !g.step.edits.is_empty())
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label of the step `undo` would revert (the group label for groups).
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    /// Label of the step `redo` would re-apply.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Start merging the following calls into one undo step. Nestable.
    pub fn begin_group(&mut self, label: &str) {
        match &mut self.group {
            Some(g) => g.depth += 1,
            None => {
                let id = self.next_id;
                self.next_id += 1;
                self.group = Some(Group {
                    depth: 1,
                    step: Step {
                        id,
                        label: label.to_string(),
                        edits: Vec::new(),
                    },
                });
            }
        }
    }

    /// Close the innermost open group. An unbalanced call is ignored. A group
    /// that recorded nothing leaves no step.
    pub fn end_group(&mut self) {
        let Some(g) = &mut self.group else { return };
        g.depth -= 1;
        if g.depth > 0 {
            return;
        }
        if let Some(Group { step, .. }) = self.group.take()
            && !step.edits.is_empty()
        {
            self.undo.push(step);
            self.redo.clear();
        }
    }

    fn end_all_groups(&mut self) {
        if let Some(g) = &mut self.group {
            g.depth = 1;
            self.end_group();
        }
    }

    /// Increases on every change to the frames or palette, including undo and
    /// redo. Use it to invalidate caches.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// True when the frames or palette differ from the last `mark_saved`
    /// (or from construction).
    pub fn is_dirty(&self) -> bool {
        let pending = self
            .group
            .as_ref()
            .is_some_and(|g| !g.step.edits.is_empty());
        let top = self.undo.last().map_or(0, |s| s.id);
        pending || top != self.saved_id || self.palette_dirty
    }

    pub fn mark_saved(&mut self) {
        self.end_all_groups();
        self.saved_id = self.undo.last().map_or(0, |s| s.id);
        self.palette_dirty = false;
    }

    // ---- crate-internal editing primitives ----

    /// Run `body` as one undo step (or as part of the open group).
    pub(crate) fn step<T>(
        &mut self,
        label: &str,
        body: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.begin_group(label);
        let out = body(self);
        self.end_group();
        out
    }

    /// Apply and record an edit.
    fn push(&mut self, edit: Edit) {
        edit.apply(&mut self.frames, true);
        self.revision += 1;
        if let Some(g) = &mut self.group {
            g.step.edits.push(edit);
        }
    }

    pub(crate) fn img(&self, f: FrameRef) -> Result<&IndexedImage> {
        self.frame(f).ok_or(PaintError::NoFrame(f))
    }

    /// Replace a frame by `new` (after normalising it), recording the smallest
    /// edit: a rectangle diff when size and offset are unchanged, the whole
    /// frame otherwise. Returns without recording when nothing changed.
    pub(crate) fn commit_image(&mut self, f: FrameRef, mut new: IndexedImage) -> Result<()> {
        canonicalize(&mut new);
        let cur = self.img(f)?;
        if *cur == new {
            return Ok(());
        }
        let same_shape = cur.width == new.width
            && cur.height == new.height
            && cur.offset_x == new.offset_x
            && cur.offset_y == new.offset_y
            && cur.pixels.len() == new.pixels.len();
        let edit = if same_shape {
            let w = cur.width as usize;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for i in 0..cur.pixels.len() {
                if cur.pixels[i] != new.pixels[i] || cur.alpha[i] != new.alpha[i] {
                    let (x, y) = (i % w, i / w);
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
            let (rw, rh) = (x1 - x0 + 1, y1 - y0 + 1);
            let take = |img: &IndexedImage| {
                let mut p = Vec::with_capacity(rw * rh);
                let mut a = Vec::with_capacity(rw * rh);
                for y in y0..=y1 {
                    let s = y * w + x0;
                    p.extend_from_slice(&img.pixels[s..s + rw]);
                    a.extend_from_slice(&img.alpha[s..s + rw]);
                }
                (p, a)
            };
            Edit::Rect {
                f,
                diff: RectDiff {
                    x: x0,
                    y: y0,
                    w: rw,
                    h: rh,
                    before: take(cur),
                    after: take(&new),
                },
            }
        } else {
            Edit::Frame {
                f,
                before: Box::new(cur.clone()),
                after: Box::new(new),
            }
        };
        self.push(edit);
        Ok(())
    }

    pub(crate) fn commit_insert(&mut self, at: usize, frames: Vec<IndexedImage>) {
        self.push(Edit::Insert { at, frames });
    }

    pub(crate) fn commit_delete(&mut self, at: usize, frames: Vec<IndexedImage>) {
        self.push(Edit::Delete { at, frames });
    }

    pub(crate) fn commit_move(&mut self, from: usize, to: usize) {
        self.push(Edit::Move { from, to });
    }

    pub(crate) fn commit_direction(&mut self, dir: usize, mut after: Vec<IndexedImage>) {
        for img in &mut after {
            canonicalize(img);
        }
        let Some(before) = self.frames.get(dir).cloned() else {
            return;
        };
        if before == after {
            return;
        }
        self.push(Edit::Direction { dir, before, after });
    }
}
