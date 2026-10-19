//! The sprite editor's state machine on top of `capy_paint::PaintSession`:
//! tools, mouse gestures (in frame-pixel coordinates), the selection, the
//! internal clipboard, frames and the DCC cell check. No egui drawing here;
//! `crate::ui::sprites::editor` turns pointer events into the calls below.
//!
//! Every pixel stays a palette index: colours are only ever chosen from the
//! palette, and index 0 (the transparent key) is never a drawing colour.

use std::collections::HashMap;

use bevy_egui::egui;
use capy_paint::{
    Brush, BrushShape, CellViolation, Dither, FillMode, FrameRef, GradientColors, GradientShape,
    Mask, MaskOp, OutlineColor, OutlineSide, Paint, PaintSession, ShapeStyle, dcc_cell_violations,
    reduce_dcc_cells,
};
use capy_sprite::{Sprite, SpriteFormat};
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use super::SpriteSource;

pub type Pt = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaintTool {
    Pencil,
    Eraser,
    Line,
    Rect,
    Ellipse,
    Fill,
    Replace,
    Picker,
    Gradient,
    Shade,
    SelectRect,
    SelectEllipse,
    Lasso,
    Wand,
    Move,
    Offset,
}

impl PaintTool {
    pub const ALL: [PaintTool; 16] = [
        PaintTool::Pencil,
        PaintTool::Eraser,
        PaintTool::Line,
        PaintTool::Rect,
        PaintTool::Ellipse,
        PaintTool::Fill,
        PaintTool::Replace,
        PaintTool::Picker,
        PaintTool::Gradient,
        PaintTool::Shade,
        PaintTool::SelectRect,
        PaintTool::SelectEllipse,
        PaintTool::Lasso,
        PaintTool::Wand,
        PaintTool::Move,
        PaintTool::Offset,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaintTool::Pencil => "Pencil",
            PaintTool::Eraser => "Eraser",
            PaintTool::Line => "Line",
            PaintTool::Rect => "Rectangle",
            PaintTool::Ellipse => "Ellipse",
            PaintTool::Fill => "Fill",
            PaintTool::Replace => "Replace colour",
            PaintTool::Picker => "Picker",
            PaintTool::Gradient => "Gradient",
            PaintTool::Shade => "Shade",
            PaintTool::SelectRect => "Select rectangle",
            PaintTool::SelectEllipse => "Select ellipse",
            PaintTool::Lasso => "Lasso",
            PaintTool::Wand => "Magic wand",
            PaintTool::Move => "Move selection",
            PaintTool::Offset => "Move offset",
        }
    }

    /// The single-key shortcut.
    pub fn key(self) -> char {
        match self {
            PaintTool::Pencil => 'B',
            PaintTool::Eraser => 'E',
            PaintTool::Line => 'L',
            PaintTool::Rect => 'R',
            PaintTool::Ellipse => 'C',
            PaintTool::Fill => 'F',
            PaintTool::Replace => 'K',
            PaintTool::Picker => 'I',
            PaintTool::Gradient => 'G',
            PaintTool::Shade => 'H',
            PaintTool::SelectRect => 'M',
            PaintTool::SelectEllipse => 'U',
            PaintTool::Lasso => 'Q',
            PaintTool::Wand => 'W',
            PaintTool::Move => 'V',
            PaintTool::Offset => 'O',
        }
    }

    pub fn from_key(c: char) -> Option<PaintTool> {
        PaintTool::ALL
            .into_iter()
            .find(|t| t.key().eq_ignore_ascii_case(&c))
    }

    pub fn is_selection(self) -> bool {
        matches!(
            self,
            PaintTool::SelectRect | PaintTool::SelectEllipse | PaintTool::Lasso | PaintTool::Wand
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientMode {
    /// Primary to secondary colour.
    Two,
    /// Along the primary colour's ramp, dark to light.
    Ramp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientSettings {
    pub shape: GradientShape,
    pub mode: GradientMode,
    pub dither: Dither,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineSettings {
    /// Darker shade of the pixel's own ramp instead of a fixed colour.
    pub auto: bool,
    /// Ramp steps for the automatic colour (negative = darker).
    pub steps: i32,
    pub side: OutlineSide,
    pub thickness: u32,
    pub diagonals: bool,
}

/// Modifier keys at the start of a gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
}

impl Mods {
    /// Selection combination: Shift adds, Ctrl subtracts, both intersect.
    pub fn mask_op(self) -> MaskOp {
        match (self.shift, self.ctrl) {
            (true, true) => MaskOp::Intersect,
            (true, false) => MaskOp::Add,
            (false, true) => MaskOp::Subtract,
            (false, false) => MaskOp::Replace,
        }
    }
}

/// A gesture in progress, in frame-pixel coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    /// Pencil, eraser or shade stroke; the session's undo group is open.
    Stroke {
        last: Pt,
        secondary: bool,
    },
    Shape {
        start: Pt,
        end: Pt,
        secondary: bool,
    },
    Gradient {
        start: (f32, f32),
        end: (f32, f32),
    },
    Select {
        start: Pt,
        end: Pt,
        op: MaskOp,
    },
    Lasso {
        points: Vec<(f32, f32)>,
        op: MaskOp,
    },
    /// Dragging the selected pixels (applied on release).
    Move {
        start: Pt,
        end: Pt,
    },
    /// Dragging the frame against the anchor (applied on release).
    Offset {
        start: Pt,
        end: Pt,
    },
}

pub struct EditorState {
    pub session: PaintSession,
    pub format: SpriteFormat,
    pub source: SpriteSource,
    pub cur: FrameRef,
    pub tool: PaintTool,
    pub brush: Brush,
    primary: u8,
    secondary: u8,
    pub fill_mode: FillMode,
    pub shape_style: ShapeStyle,
    /// Replace colour in every frame instead of the current one.
    pub replace_everywhere: bool,
    pub gradient: GradientSettings,
    pub shade_steps: i32,
    pub outline: OutlineSettings,
    /// Outline / clean strays / offset drags apply to every frame of the
    /// direction.
    pub apply_all_frames: bool,
    /// Gradients only paint pixels that are already opaque.
    pub lock_alpha: bool,
    pub selection: Option<Mask>,
    pub clip: Option<IndexedImage>,
    pub onion: bool,
    pub onion_fade: f32,
    pub grid: bool,
    pub checker: bool,
    pub show_anchor: bool,
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub gesture: Option<Gesture>,
    /// Direction "Mirror direction" writes to.
    pub mirror_target: usize,
    violations: Option<(u64, Vec<CellViolation>)>,
    /// Texture cache of the canvas: (key, texture, canvas origin in frame px).
    pub canvas_tex: Option<(u64, egui::TextureHandle, (i32, i32))>,
    pub thumbs: HashMap<(usize, usize), (u64, egui::TextureHandle)>,
    /// Size typed in the "Canvas" box.
    pub canvas_size: (u32, u32),
}

fn first_opaque(palette: &Palette, prefer: [u8; 3]) -> u8 {
    let mut best = (u32::MAX, 1u8);
    for (i, c) in palette.colors.iter().enumerate().skip(1) {
        let d: u32 = c
            .iter()
            .zip(&prefer)
            .map(|(&a, &b)| u32::from(a.abs_diff(b)).pow(2))
            .sum();
        if d < best.0 {
            best = (d, i as u8);
        }
    }
    best.1
}

impl EditorState {
    pub fn new(
        sprite: Sprite,
        format: SpriteFormat,
        source: SpriteSource,
        palette: Palette,
    ) -> Result<EditorState, String> {
        if sprite.directions.is_empty() || sprite.frames_per_direction() == 0 {
            return Err("the sprite has no frames".to_owned());
        }
        let primary = first_opaque(&palette, [255, 255, 255]);
        let secondary = first_opaque(&palette, [16, 16, 16]);
        let largest = sprite
            .directions
            .iter()
            .flatten()
            .map(|f| f.width.max(f.height))
            .max()
            .unwrap_or(1)
            .max(1);
        // About 400 points for the largest frame.
        let zoom = (400 / largest).clamp(1, 12) as f32;
        let session = PaintSession::new(sprite.directions, palette).map_err(|e| e.to_string())?;
        Ok(EditorState {
            session,
            format,
            source,
            cur: FrameRef { dir: 0, frame: 0 },
            tool: PaintTool::Pencil,
            brush: Brush {
                size: 1,
                shape: BrushShape::Square,
            },
            primary,
            secondary,
            fill_mode: FillMode::Contiguous,
            shape_style: ShapeStyle::Outline,
            replace_everywhere: false,
            gradient: GradientSettings {
                shape: GradientShape::Linear,
                mode: GradientMode::Two,
                dither: Dither::Bayer4,
            },
            shade_steps: 1,
            outline: OutlineSettings {
                auto: true,
                steps: -3,
                side: OutlineSide::Outside,
                thickness: 1,
                diagonals: false,
            },
            apply_all_frames: false,
            lock_alpha: true,
            selection: None,
            clip: None,
            onion: false,
            onion_fade: 0.35,
            grid: true,
            checker: true,
            show_anchor: true,
            zoom,
            pan: egui::Vec2::ZERO,
            gesture: None,
            mirror_target: 0,
            violations: None,
            canvas_tex: None,
            thumbs: HashMap::new(),
            canvas_size: (0, 0),
        })
    }

    // ───────── colours ─────────

    pub fn primary(&self) -> u8 {
        self.primary
    }

    pub fn secondary(&self) -> u8 {
        self.secondary
    }

    /// Index 0 is the transparent key and never a drawing colour.
    pub fn set_primary(&mut self, index: u8) {
        if index != 0 {
            self.primary = index;
        }
    }

    pub fn set_secondary(&mut self, index: u8) {
        if index != 0 {
            self.secondary = index;
        }
    }

    pub fn swap_colors(&mut self) {
        std::mem::swap(&mut self.primary, &mut self.secondary);
    }

    fn paint(&self, secondary: bool) -> Paint {
        Paint::Index(if secondary {
            self.secondary
        } else {
            self.primary
        })
    }

    // ───────── frames ─────────

    pub fn directions(&self) -> usize {
        self.session.frames().len()
    }

    pub fn frames(&self) -> usize {
        self.session.frames().first().map_or(0, Vec::len)
    }

    pub fn frame(&self) -> Option<&IndexedImage> {
        self.session.frame(self.cur)
    }

    pub fn to_sprite(&self) -> Sprite {
        Sprite {
            directions: self.session.frames().to_vec(),
        }
    }

    /// Selects a frame; the selection survives when the size matches.
    pub fn go_to(&mut self, dir: usize, frame: usize) {
        self.cancel_gesture();
        self.canvas_size = (0, 0);
        self.cur = FrameRef {
            dir: dir.min(self.directions().saturating_sub(1)),
            frame: frame.min(self.frames().saturating_sub(1)),
        };
        self.fix_selection();
    }

    pub fn step_frame(&mut self, delta: i64) {
        let n = self.frames() as i64;
        if n > 0 {
            let f = (self.cur.frame as i64 + delta).rem_euclid(n) as usize;
            self.go_to(self.cur.dir, f);
        }
    }

    /// Drops the selection when it no longer fits the current frame.
    fn fix_selection(&mut self) {
        let size = self.frame().map(|f| (f.width, f.height));
        if let Some(m) = &self.selection
            && (Some((m.width, m.height)) != size || m.is_empty())
        {
            self.selection = None;
        }
    }

    /// The selection, when there is one for the current frame.
    pub fn mask(&self) -> Option<&Mask> {
        let f = self.frame()?;
        self.selection
            .as_ref()
            .filter(|m| m.width == f.width && m.height == f.height && !m.is_empty())
    }

    fn after_change(&mut self) {
        self.cur = FrameRef {
            dir: self.cur.dir.min(self.directions().saturating_sub(1)),
            frame: self.cur.frame.min(self.frames().saturating_sub(1)),
        };
        self.fix_selection();
    }

    fn run<T>(&mut self, r: capy_paint::Result<T>) -> Result<T, String> {
        let out = r.map_err(|e| e.to_string());
        self.after_change();
        out
    }

    pub fn insert_frame(&mut self, duplicate: bool) -> Result<(), String> {
        let at = self.cur.frame + 1;
        let copy = duplicate.then_some(self.cur.frame);
        let r = self.session.insert_frame(at, copy);
        self.run(r)?;
        self.go_to(self.cur.dir, at);
        Ok(())
    }

    pub fn delete_frame(&mut self) -> Result<(), String> {
        let r = self.session.delete_frame(self.cur.frame);
        self.run(r)
    }

    pub fn move_frame(&mut self, delta: i64) -> Result<(), String> {
        let to = self.cur.frame as i64 + delta;
        if to < 0 || to >= self.frames() as i64 {
            return Ok(());
        }
        let r = self.session.move_frame(self.cur.frame, to as usize);
        self.run(r)?;
        self.go_to(self.cur.dir, to as usize);
        Ok(())
    }

    pub fn mirror_direction(&mut self) -> Result<(), String> {
        if self.mirror_target == self.cur.dir {
            return Err("pick another direction to mirror into".to_owned());
        }
        let r = self
            .session
            .mirror_direction(self.cur.dir, self.mirror_target);
        self.run(r)
    }

    // ───────── history ─────────

    pub fn undo(&mut self) -> bool {
        self.cancel_gesture();
        let done = self.session.undo();
        self.after_change();
        done
    }

    pub fn redo(&mut self) -> bool {
        self.cancel_gesture();
        let done = self.session.redo();
        self.after_change();
        done
    }

    pub fn is_dirty(&self) -> bool {
        self.session.is_dirty()
    }

    /// Shows the sprite under another palette (indices are kept). Not an
    /// edit: a clean sprite stays clean.
    pub fn set_palette(&mut self, palette: Palette) {
        if *self.session.palette() == palette {
            return;
        }
        let was_dirty = self.session.is_dirty();
        self.cancel_gesture();
        self.session.set_palette(palette);
        if !was_dirty {
            self.session.mark_saved();
        }
    }

    // ───────── gestures ─────────

    /// Abandons a gesture; a half-done stroke is kept as its own step.
    pub fn cancel_gesture(&mut self) {
        if let Some(Gesture::Stroke { .. }) = self.gesture {
            self.session.end_group();
        }
        self.gesture = None;
    }

    fn mask_clone(&self) -> Option<Mask> {
        self.mask().cloned()
    }

    /// Mask for gradients: the selection, limited to opaque pixels when
    /// `lock_alpha` is on.
    fn gradient_mask(&self) -> Option<Mask> {
        let f = self.frame()?;
        if !self.lock_alpha {
            return self.mask_clone();
        }
        let mut opaque = Mask::none(f.width, f.height);
        for (i, &a) in f.alpha.iter().enumerate() {
            if a != 0 {
                opaque.bits[i] = true;
            }
        }
        if let Some(sel) = self.mask() {
            opaque.combine(sel, MaskOp::Intersect);
        }
        Some(opaque)
    }

    fn gradient_colors(&self) -> GradientColors {
        match self.gradient.mode {
            GradientMode::Ramp => match self.session.ramps().ramp_of(self.primary) {
                Some((id, _)) => GradientColors::Ramp(id),
                None => GradientColors::Two(self.primary, self.secondary),
            },
            GradientMode::Two => GradientColors::Two(self.primary, self.secondary),
        }
    }

    fn shade_steps(&self, secondary: bool) -> i32 {
        if secondary {
            -self.shade_steps
        } else {
            self.shade_steps
        }
    }

    /// Mouse button down at frame pixel `p` (fractional).
    pub fn press(&mut self, p: (f32, f32), secondary: bool, mods: Mods) -> Result<(), String> {
        self.cancel_gesture();
        let pt = (p.0.floor() as i32, p.1.floor() as i32);
        let f = self.cur;
        let mask = self.mask_clone();
        match self.tool {
            PaintTool::Pencil | PaintTool::Eraser => {
                let paint = if self.tool == PaintTool::Eraser {
                    Paint::Transparent
                } else {
                    self.paint(secondary)
                };
                self.session.begin_group(self.tool.label());
                self.gesture = Some(Gesture::Stroke {
                    last: pt,
                    secondary,
                });
                let r = self
                    .session
                    .pencil(f, &[pt], self.brush, paint, mask.as_ref());
                self.run(r)
            }
            PaintTool::Shade => {
                self.session.begin_group("Shade");
                self.gesture = Some(Gesture::Stroke {
                    last: pt,
                    secondary,
                });
                let steps = self.shade_steps(secondary);
                let r = self
                    .session
                    .shade(f, &[pt], self.brush, steps, mask.as_ref());
                self.run(r)
            }
            PaintTool::Line | PaintTool::Rect | PaintTool::Ellipse => {
                self.gesture = Some(Gesture::Shape {
                    start: pt,
                    end: pt,
                    secondary,
                });
                Ok(())
            }
            PaintTool::Gradient => {
                self.gesture = Some(Gesture::Gradient { start: p, end: p });
                Ok(())
            }
            PaintTool::Fill => {
                let paint = self.paint(secondary);
                let r = self
                    .session
                    .fill(f, pt, paint, self.fill_mode, mask.as_ref());
                self.run(r)
            }
            PaintTool::Replace => {
                let Some(from) = self.session.pick(f, pt) else {
                    return Ok(());
                };
                let target = (!self.replace_everywhere).then_some(f);
                let to = self.paint(secondary);
                let r = self.session.replace_color(target, from, to, mask.as_ref());
                self.run(r)
            }
            PaintTool::Picker => {
                if let Some(Paint::Index(i)) = self.session.pick(f, pt) {
                    if secondary {
                        self.set_secondary(i);
                    } else {
                        self.set_primary(i);
                    }
                }
                Ok(())
            }
            PaintTool::SelectRect | PaintTool::SelectEllipse => {
                self.gesture = Some(Gesture::Select {
                    start: pt,
                    end: pt,
                    op: mods.mask_op(),
                });
                Ok(())
            }
            PaintTool::Lasso => {
                self.gesture = Some(Gesture::Lasso {
                    points: vec![p],
                    op: mods.mask_op(),
                });
                Ok(())
            }
            PaintTool::Wand => {
                let m = self.session.magic_wand(f, pt, self.fill_mode);
                self.combine_selection(m, mods.mask_op());
                Ok(())
            }
            PaintTool::Move => {
                if self.mask().is_some_and(|m| m.get(pt.0, pt.1)) {
                    self.gesture = Some(Gesture::Move { start: pt, end: pt });
                    Ok(())
                } else {
                    Err("press inside the selection to move it".to_owned())
                }
            }
            PaintTool::Offset => {
                self.gesture = Some(Gesture::Offset { start: pt, end: pt });
                Ok(())
            }
        }
    }

    /// Mouse moved with the button down.
    pub fn drag(&mut self, p: (f32, f32)) -> Result<(), String> {
        let pt = (p.0.floor() as i32, p.1.floor() as i32);
        let f = self.cur;
        let mask = self.mask_clone();
        let tool = self.tool;
        let brush = self.brush;
        let Some(gesture) = self.gesture.as_mut() else {
            return Ok(());
        };
        match gesture {
            Gesture::Stroke { last, secondary } => {
                if *last == pt {
                    return Ok(());
                }
                let (from, secondary) = (*last, *secondary);
                *last = pt;
                let r = match tool {
                    PaintTool::Shade => {
                        let pts: Vec<Pt> = line_points(from, pt).into_iter().skip(1).collect();
                        let steps = self.shade_steps(secondary);
                        self.session.shade(f, &pts, brush, steps, mask.as_ref())
                    }
                    PaintTool::Eraser => {
                        self.session
                            .line(f, from, pt, brush, Paint::Transparent, mask.as_ref())
                    }
                    _ => {
                        let paint = self.paint(secondary);
                        self.session.line(f, from, pt, brush, paint, mask.as_ref())
                    }
                };
                self.run(r)
            }
            Gesture::Shape { end, .. }
            | Gesture::Select { end, .. }
            | Gesture::Move { end, .. }
            | Gesture::Offset { end, .. } => {
                *end = pt;
                Ok(())
            }
            Gesture::Gradient { end, .. } => {
                *end = p;
                Ok(())
            }
            Gesture::Lasso { points, .. } => {
                if points
                    .last()
                    .is_none_or(|l| (l.0 - p.0).abs() + (l.1 - p.1).abs() >= 0.5)
                {
                    points.push(p);
                }
                Ok(())
            }
        }
    }

    /// Mouse button released: finishes the gesture.
    pub fn release(&mut self, p: (f32, f32)) -> Result<(), String> {
        self.drag(p)?;
        let Some(gesture) = self.gesture.take() else {
            return Ok(());
        };
        let f = self.cur;
        let mask = self.mask_clone();
        let (w, h) = self.frame().map_or((0, 0), |i| (i.width, i.height));
        match gesture {
            Gesture::Stroke { .. } => {
                self.session.end_group();
                Ok(())
            }
            Gesture::Shape {
                start,
                end,
                secondary,
            } => {
                let paint = self.paint(secondary);
                let r = match self.tool {
                    PaintTool::Line => {
                        self.session
                            .line(f, start, end, self.brush, paint, mask.as_ref())
                    }
                    PaintTool::Ellipse => self.session.ellipse(
                        f,
                        start,
                        end,
                        self.shape_style,
                        self.brush,
                        paint,
                        mask.as_ref(),
                    ),
                    _ => self.session.rectangle(
                        f,
                        start,
                        end,
                        self.shape_style,
                        self.brush,
                        paint,
                        mask.as_ref(),
                    ),
                };
                self.run(r)
            }
            Gesture::Gradient { start, end } => self.apply_gradient(start, end),
            Gesture::Select { start, end, op } => {
                if start == end && op == MaskOp::Replace {
                    self.selection = None;
                    return Ok(());
                }
                let (x0, y0, x1, y1) = (start.0, start.1, end.0, end.1);
                let m = if self.tool == PaintTool::SelectEllipse {
                    Mask::ellipse(w, h, x0, y0, x1, y1)
                } else {
                    Mask::rect(w, h, x0, y0, x1, y1)
                };
                self.combine_selection(m, op);
                Ok(())
            }
            Gesture::Lasso { points, op } => {
                if points.len() < 3 {
                    if op == MaskOp::Replace {
                        self.selection = None;
                    }
                    return Ok(());
                }
                let m = Mask::lasso(w, h, &points);
                self.combine_selection(m, op);
                Ok(())
            }
            Gesture::Move { start, end } => {
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                self.move_selection(dx, dy)
            }
            Gesture::Offset { start, end } => {
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                self.shift_offset(dx, dy)
            }
        }
    }

    /// Applies a gradient from `start` to `end` (frame pixels).
    pub fn apply_gradient(&mut self, start: (f32, f32), end: (f32, f32)) -> Result<(), String> {
        let colors = self.gradient_colors();
        let mask = self.gradient_mask();
        if mask.as_ref().is_some_and(Mask::is_empty) {
            return Err(
                "nothing to paint: the frame has no opaque pixels (turn off Lock alpha)".to_owned(),
            );
        }
        let r = self.session.gradient(
            self.cur,
            start,
            end,
            self.gradient.shape,
            colors,
            self.gradient.dither,
            mask.as_ref(),
        );
        self.run(r)
    }

    fn combine_selection(&mut self, m: Mask, op: MaskOp) {
        let mut base = match (op, self.mask_clone()) {
            (MaskOp::Replace, _) | (_, None) => Mask::none(m.width, m.height),
            (_, Some(cur)) => cur,
        };
        if op == MaskOp::Replace {
            base = m;
        } else {
            base.combine(&m, op);
        }
        self.selection = (!base.is_empty()).then_some(base);
    }

    // ───────── actions ─────────

    fn frames_to_touch(&self) -> Vec<FrameRef> {
        if self.apply_all_frames {
            (0..self.frames())
                .map(|frame| FrameRef {
                    dir: self.cur.dir,
                    frame,
                })
                .collect()
        } else {
            vec![self.cur]
        }
    }

    /// Outline (fixed or auto-darker colour) on the current frame, or every
    /// frame of the direction.
    pub fn apply_outline(&mut self, fixed: Option<u8>) -> Result<(), String> {
        let o = self.outline;
        let color = match fixed {
            Some(i) => OutlineColor::Fixed(i),
            None if o.auto => OutlineColor::AutoDarker(o.steps),
            None => OutlineColor::Fixed(self.primary),
        };
        let mask = self.mask_clone();
        let frames = self.frames_to_touch();
        self.session.begin_group("Outline");
        let mut result = Ok(());
        for f in frames {
            let m = if f == self.cur { mask.as_ref() } else { None };
            if let Err(e) =
                self.session
                    .outline(f, color, o.side, o.thickness.clamp(1, 8), o.diagonals, m)
            {
                result = Err(e.to_string());
                break;
            }
        }
        self.session.end_group();
        self.after_change();
        result
    }

    pub fn clean_strays(&mut self) -> Result<(), String> {
        let mask = self.mask_clone();
        let frames = self.frames_to_touch();
        self.session.begin_group("Clean strays");
        let mut result = Ok(());
        for f in frames {
            let m = if f == self.cur { mask.as_ref() } else { None };
            if let Err(e) = self.session.clean_strays(f, m) {
                result = Err(e.to_string());
                break;
            }
        }
        self.session.end_group();
        self.after_change();
        result
    }

    fn shift_offset(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        if (dx, dy) == (0, 0) {
            return Ok(());
        }
        let frames = self.frames_to_touch();
        self.session.begin_group("Move offset");
        let mut result = Ok(());
        for f in frames {
            let Some((ox, oy)) = self.session.frame(f).map(|i| (i.offset_x, i.offset_y)) else {
                continue;
            };
            if let Err(e) = self.session.set_offset(f, ox + dx, oy + dy) {
                result = Err(e.to_string());
                break;
            }
        }
        self.session.end_group();
        self.after_change();
        result
    }

    pub fn move_selection(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let Some(mask) = self.mask_clone() else {
            return Err("nothing is selected".to_owned());
        };
        if (dx, dy) == (0, 0) {
            return Ok(());
        }
        let r = self.session.move_selection(self.cur, &mask, dx, dy);
        let moved = self.run(r)?;
        self.selection = (!moved.is_empty()).then_some(moved);
        Ok(())
    }

    pub fn flip(&mut self, horizontal: bool) -> Result<(), String> {
        let mask = self.mask_clone();
        let r = self.session.flip(self.cur, mask.as_ref(), horizontal);
        self.run(r)
    }

    pub fn rotate(&mut self, clockwise: bool) -> Result<(), String> {
        let mask = self.mask_clone();
        let r = self.session.rotate90(self.cur, mask.as_ref(), clockwise);
        self.run(r)?;
        // The rotated content no longer matches the old outline.
        if mask.is_some() {
            self.selection = None;
        }
        Ok(())
    }

    pub fn select_all(&mut self) {
        if let Some(f) = self.frame() {
            self.selection = Some(Mask::all(f.width, f.height));
        }
    }

    pub fn deselect(&mut self) {
        self.selection = None;
    }

    pub fn invert_selection(&mut self) {
        let Some((w, h)) = self.frame().map(|f| (f.width, f.height)) else {
            return;
        };
        let mut m = self.mask_clone().unwrap_or_else(|| Mask::none(w, h));
        m.invert();
        self.selection = (!m.is_empty()).then_some(m);
    }

    pub fn grow_selection(&mut self, px: i32) {
        if let Some(m) = self.mask_clone() {
            let out = if px >= 0 {
                m.grow(px as u32)
            } else {
                m.shrink(px.unsigned_abs())
            };
            self.selection = (!out.is_empty()).then_some(out);
        }
    }

    /// Copies the selection (or the whole frame) into the internal clipboard.
    pub fn copy(&mut self) -> Option<&IndexedImage> {
        let mask = match self.mask_clone() {
            Some(m) => m,
            None => {
                let f = self.frame()?;
                Mask::all(f.width, f.height)
            }
        };
        self.clip = Some(self.session.copy(self.cur, &mask));
        self.clip.as_ref()
    }

    pub fn cut(&mut self) -> Result<(), String> {
        let Some(mask) = self.mask_clone() else {
            return Err("select something to cut".to_owned());
        };
        let r = self.session.cut(self.cur, &mask);
        self.clip = Some(self.run(r)?);
        Ok(())
    }

    /// Pastes the internal clipboard where it was copied from and selects it.
    pub fn paste(&mut self) -> Result<(), String> {
        let Some(clip) = self.clip.clone() else {
            return Err("nothing was copied".to_owned());
        };
        let at = self.paste_position(clip.offset_x, clip.offset_y, clip.width, clip.height);
        let r = self.session.paste(self.cur, &clip, at);
        let m = self.run(r)?;
        self.selection = (!m.is_empty()).then_some(m);
        self.tool = PaintTool::Move;
        Ok(())
    }

    /// Pastes an RGBA image (OS clipboard), mapped to the palette.
    pub fn paste_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        dither: Dither,
    ) -> Result<(), String> {
        let at = self.paste_position(0, 0, width, height);
        let r = self
            .session
            .paste_rgba(self.cur, width, height, rgba, at, dither);
        let m = self.run(r)?;
        self.selection = (!m.is_empty()).then_some(m);
        self.tool = PaintTool::Move;
        Ok(())
    }

    /// Keeps a paste on the frame: its own position when it fits, else the
    /// top-left corner.
    fn paste_position(&self, x: i32, y: i32, w: u32, h: u32) -> Pt {
        let Some(f) = self.frame() else { return (0, 0) };
        let fits = x >= 0
            && y >= 0
            && i64::from(x) + i64::from(w) <= i64::from(f.width)
            && i64::from(y) + i64::from(h) <= i64::from(f.height);
        if fits { (x, y) } else { (0, 0) }
    }

    /// Deletes the selected pixels (makes them transparent).
    pub fn clear_selection_pixels(&mut self) -> Result<(), String> {
        let Some(mask) = self.mask_clone() else {
            return Ok(());
        };
        let r = self.session.cut(self.cur, &mask);
        self.run(r).map(|_| ())
    }

    /// Resizes the current frame (or every frame of the direction); the
    /// bottom-centre stays in place, so the anchor does not move.
    pub fn resize_canvas(&mut self, w: u32, h: u32) -> Result<(), String> {
        let frames = self.frames_to_touch();
        self.session.begin_group("Resize canvas");
        let mut result = Ok(());
        for f in frames {
            if let Err(e) = self.session.resize_canvas(f, w, h, (0, 1)) {
                result = Err(e.to_string());
                break;
            }
        }
        self.session.end_group();
        self.after_change();
        result
    }

    // ───────── DCC check ─────────

    /// Cells holding more than 4 values, recomputed when the sprite changed.
    pub fn violations(&mut self) -> &[CellViolation] {
        let rev = self.session.revision();
        if self.violations.as_ref().is_none_or(|(r, _)| *r != rev) {
            self.violations = Some((rev, dcc_cell_violations(self.session.frames())));
        }
        self.violations.as_ref().map_or(&[], |(_, v)| v.as_slice())
    }

    pub fn reduce_all(&mut self) -> Result<usize, String> {
        let r = reduce_dcc_cells(&mut self.session);
        self.run(r)
    }

    /// Frame-pixel rectangle `(x0, y0, x1, y1)` (end exclusive) of a cell.
    pub fn cell_rect(&self, v: &CellViolation) -> Option<(i32, i32, i32, i32)> {
        let frames = self.session.frames().get(v.frame.dir)?;
        let img = frames.get(v.frame.frame)?;
        let (mut ox, mut oy) = (i32::MAX, i32::MAX);
        for f in frames {
            if let Some((x0, y0)) = opaque_min(f) {
                ox = ox.min(x0);
                oy = oy.min(y0);
            }
        }
        if ox == i32::MAX {
            return None;
        }
        let x0 = ox + v.cell_x as i32 * 4 - img.offset_x;
        let y0 = oy + v.cell_y as i32 * 4 - img.offset_y;
        Some((x0.max(0), y0.max(0), x0 + 4, y0 + 4))
    }
}

/// Top-left opaque pixel in anchor space.
fn opaque_min(img: &IndexedImage) -> Option<(i32, i32)> {
    let w = img.width as usize;
    if w == 0 {
        return None;
    }
    let (mut x0, mut y0) = (usize::MAX, usize::MAX);
    for (i, &a) in img.alpha.iter().enumerate() {
        if a != 0 {
            x0 = x0.min(i % w);
            y0 = y0.min(i / w);
        }
    }
    (x0 != usize::MAX).then(|| (img.offset_x + x0 as i32, img.offset_y + y0 as i32))
}

/// The pixels of a line from `a` to `b`, both ends included (Bresenham).
pub fn line_points(a: Pt, b: Pt) -> Vec<Pt> {
    let (mut x, mut y) = a;
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::with_capacity((dx - dy) as usize + 1);
    loop {
        out.push((x, y));
        if (x, y) == b || out.len() > 1 << 16 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8, (i as u8).wrapping_mul(3), 255 - i as u8];
        }
        Palette { colors }
    }

    fn editor(w: u32, h: u32, frames: usize) -> EditorState {
        let mut img = IndexedImage::new(w, h, -(w as i32) / 2, -(h as i32));
        // an opaque 4x4 block in the middle
        for y in 2..6.min(h) {
            for x in 2..6.min(w) {
                let i = (y * w + x) as usize;
                img.pixels[i] = 40;
                img.alpha[i] = 255;
            }
        }
        let sprite = Sprite {
            directions: vec![vec![img; frames]; 2],
        };
        EditorState::new(
            sprite,
            SpriteFormat::Dcc,
            SpriteSource::Imported("t".into()),
            palette(),
        )
        .expect("editor")
    }

    fn opaque_index_zero(e: &EditorState) -> bool {
        e.session.frames().iter().flatten().any(|f| {
            f.pixels
                .iter()
                .zip(&f.alpha)
                .any(|(&p, &a)| a != 0 && p == 0)
        })
    }

    #[test]
    fn index_zero_is_never_a_colour() {
        let mut e = editor(8, 8, 1);
        e.set_primary(0);
        e.set_secondary(0);
        assert_ne!(e.primary(), 0);
        assert_ne!(e.secondary(), 0);
        e.set_primary(7);
        e.swap_colors();
        assert_eq!(e.secondary(), 7);
    }

    #[test]
    fn a_stroke_is_one_undo_step() {
        let mut e = editor(8, 8, 1);
        e.set_primary(9);
        e.press((0.5, 0.5), false, Mods::default()).expect("press");
        e.drag((3.5, 0.5)).expect("drag");
        e.drag((3.5, 3.5)).expect("drag");
        e.release((3.5, 3.5)).expect("release");
        let f = e.frame().expect("frame");
        assert_eq!(f.pixels[0], 9);
        assert_eq!(f.pixels[3], 9);
        assert!(e.is_dirty());
        assert_eq!(e.session.undo_label(), Some("Pencil"));
        assert!(e.undo());
        assert!(!e.session.can_undo(), "one step");
        assert_eq!(e.frame().expect("frame").alpha[0], 0);
        assert!(!e.is_dirty());
    }

    #[test]
    fn selections_combine_and_restrict_tools() {
        let mut e = editor(8, 8, 1);
        e.tool = PaintTool::SelectRect;
        e.press((0.0, 0.0), false, Mods::default()).expect("press");
        e.release((1.0, 1.0)).expect("release");
        assert_eq!(e.mask().map(Mask::count), Some(4));
        e.press(
            (6.0, 6.0),
            false,
            Mods {
                shift: true,
                ctrl: false,
            },
        )
        .expect("press");
        e.release((7.0, 7.0)).expect("release");
        assert_eq!(e.mask().map(Mask::count), Some(8), "added");
        e.press(
            (0.0, 0.0),
            false,
            Mods {
                shift: false,
                ctrl: true,
            },
        )
        .expect("press");
        e.release((0.0, 1.0)).expect("release");
        assert_eq!(e.mask().map(Mask::count), Some(6), "subtracted");
        // A fill restricted to the selection.
        e.tool = PaintTool::Fill;
        e.fill_mode = FillMode::Global;
        e.set_primary(5);
        e.press((7.0, 7.0), false, Mods::default()).expect("fill");
        let f = e.frame().expect("frame");
        assert_eq!(f.alpha.iter().filter(|&&a| a != 0).count(), 16 + 6);
        // A click without a drag deselects.
        e.tool = PaintTool::SelectRect;
        e.press((3.0, 3.0), false, Mods::default()).expect("press");
        e.release((3.0, 3.0)).expect("release");
        assert!(e.mask().is_none());
        e.invert_selection();
        assert_eq!(e.mask().map(Mask::count), Some(64));
    }

    #[test]
    fn gradient_and_outline_stay_palette_locked() {
        let mut e = editor(10, 10, 2);
        e.gradient.mode = GradientMode::Ramp;
        e.apply_gradient((0.0, 0.0), (9.0, 9.0)).expect("gradient");
        // Lock alpha: only the 16 opaque pixels changed, none became index 0.
        let f = e.frame().expect("frame");
        assert_eq!(f.alpha.iter().filter(|&&a| a != 0).count(), 16);
        e.apply_all_frames = true;
        e.outline.auto = true;
        e.apply_outline(None).expect("outline");
        for img in &e.session.frames()[0] {
            assert!(img.alpha.iter().filter(|&&a| a != 0).count() > 16);
        }
        assert!(!opaque_index_zero(&e));
        assert_eq!(e.session.undo_label(), Some("Outline"));
        // An all-transparent frame has nothing to lock onto.
        let mut empty = editor(4, 4, 1);
        empty.select_all();
        empty.clear_selection_pixels().expect("clear");
        assert!(empty.apply_gradient((0.0, 0.0), (3.0, 3.0)).is_err());
    }

    #[test]
    fn move_and_offset_apply_on_release() {
        let mut e = editor(8, 8, 1);
        e.tool = PaintTool::SelectRect;
        e.press((2.0, 2.0), false, Mods::default()).expect("press");
        e.release((5.0, 5.0)).expect("release");
        e.tool = PaintTool::Move;
        assert!(
            e.press((0.0, 0.0), false, Mods::default()).is_err(),
            "outside"
        );
        e.press((3.0, 3.0), false, Mods::default()).expect("press");
        e.drag((4.0, 3.0)).expect("drag");
        assert_eq!(
            e.frame().expect("frame").alpha[2 * 8 + 2],
            255,
            "not yet moved"
        );
        e.release((4.0, 3.0)).expect("release");
        let f = e.frame().expect("frame");
        assert_eq!(f.alpha[2 * 8 + 2], 0);
        assert_eq!(f.alpha[2 * 8 + 6], 255);
        let (ox, oy) = (f.offset_x, f.offset_y);
        e.tool = PaintTool::Offset;
        e.press((1.0, 1.0), false, Mods::default()).expect("press");
        e.release((3.0, 0.0)).expect("release");
        let f = e.frame().expect("frame");
        assert_eq!((f.offset_x, f.offset_y), (ox + 2, oy - 1));
    }

    #[test]
    fn frames_insert_duplicate_delete_and_reorder() {
        let mut e = editor(8, 8, 2);
        e.insert_frame(true).expect("dup");
        assert_eq!(e.frames(), 3);
        assert_eq!(e.cur.frame, 1);
        e.insert_frame(false).expect("blank");
        assert_eq!(e.frames(), 4);
        assert!(e.frame().expect("frame").alpha.iter().all(|&a| a == 0));
        e.move_frame(-1).expect("move");
        assert_eq!(e.cur.frame, 1);
        e.delete_frame().expect("delete");
        assert_eq!(e.frames(), 3);
        e.mirror_target = 0;
        assert!(e.mirror_direction().is_err(), "same direction");
        e.mirror_target = 1;
        e.mirror_direction().expect("mirror");
        e.go_to(9, 9);
        assert_eq!(e.cur, FrameRef { dir: 1, frame: 2 });
    }

    #[test]
    fn copy_paste_and_dcc_check() {
        let mut e = editor(8, 8, 1);
        e.tool = PaintTool::SelectRect;
        e.press((2.0, 2.0), false, Mods::default()).expect("press");
        e.release((3.0, 3.0)).expect("release");
        assert_eq!(e.copy().map(|c| (c.width, c.height)), Some((2, 2)));
        e.paste().expect("paste");
        assert_eq!(e.tool, PaintTool::Move);
        assert_eq!(e.mask().map(Mask::count), Some(4));
        // Five different colours in one 4x4 cell.
        e.deselect();
        e.tool = PaintTool::Pencil;
        for (i, x) in (2..6).enumerate() {
            e.set_primary(100 + i as u8);
            e.press((x as f32, 2.0), false, Mods::default()).expect("p");
            e.release((x as f32, 2.0)).expect("r");
        }
        assert!(!e.violations().is_empty());
        let v = e.violations()[0].clone();
        assert!(e.cell_rect(&v).is_some());
        assert!(e.reduce_all().expect("reduce") > 0);
        assert!(e.violations().is_empty());
    }

    #[test]
    fn line_points_cover_both_ends() {
        assert_eq!(line_points((0, 0), (3, 0)).len(), 4);
        assert_eq!(line_points((0, 0), (0, 0)), vec![(0, 0)]);
        let diag = line_points((0, 0), (-3, 3));
        assert_eq!(diag.first(), Some(&(0, 0)));
        assert_eq!(diag.last(), Some(&(-3, 3)));
    }

    #[test]
    fn tool_keys_round_trip() {
        for t in PaintTool::ALL {
            assert_eq!(PaintTool::from_key(t.key()), Some(t), "{t:?}");
        }
        let mut keys: Vec<char> = PaintTool::ALL.iter().map(|t| t.key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), PaintTool::ALL.len(), "keys are unique");
        assert!(PaintTool::Wand.is_selection());
    }
}
