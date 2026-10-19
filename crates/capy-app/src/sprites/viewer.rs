//! The Viewer's state: what is loaded (a DC6/DCC, or a COF with its layers),
//! the direction and frame, playback, zoom and overlays. Drawing lives in
//! `crate::ui::sprites::viewer`.

use std::collections::HashMap;

use bevy_egui::egui;
use capy_sprite::{RgbaFrame, Sprite, SpriteFormat, to_rgba};
use d2_formats::palette::Palette;

use super::SpriteSource;
use super::cof::CofView;

/// The game's animation clock: AnimData speed 256 advances one frame per
/// 25 Hz tick.
pub const DEFAULT_FPS: f32 = 25.0;
pub const MAX_ZOOM: u32 = 16;

/// Frames per second for an AnimData speed (`speed / 256` frames per tick).
pub fn fps_for_speed(speed: u32) -> f32 {
    if speed == 0 {
        DEFAULT_FPS
    } else {
        (speed as f32 * DEFAULT_FPS / 256.0).clamp(0.5, 100.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    Checker,
    Solid([u8; 3]),
}

pub struct SingleSprite {
    pub sprite: Sprite,
    pub format: SpriteFormat,
}

pub enum Loaded {
    Sprite(SingleSprite),
    Cof(Box<CofView>),
}

/// A frame uploaded to the GPU, with where it sits relative to the anchor.
#[derive(Clone)]
pub struct Tex {
    pub handle: egui::TextureHandle,
    pub offset: (i32, i32),
    pub size: (u32, u32),
}

pub struct ViewerState {
    pub loaded: Option<Loaded>,
    pub source: Option<SpriteSource>,
    pub dir: usize,
    pub frame: usize,
    pub playing: bool,
    pub fps: f32,
    /// Where `fps` came from ("AnimData speed 128", "default").
    pub fps_note: String,
    pub zoom: u32,
    pub pan: egui::Vec2,
    pub background: Background,
    pub show_box: bool,
    pub show_anchor: bool,
    /// Last animation clock reading (egui time, seconds).
    clock: Option<f64>,
    accum: f64,
    /// Bumped whenever what the textures show changes.
    pub revision: u64,
    pub tex_key: u64,
    pub textures: HashMap<(usize, usize), Option<Tex>>,
    /// Shown in the panel when the last open failed.
    pub error: Option<String>,
}

impl Default for ViewerState {
    fn default() -> Self {
        ViewerState {
            loaded: None,
            source: None,
            dir: 0,
            frame: 0,
            playing: false,
            fps: DEFAULT_FPS,
            fps_note: "default".to_owned(),
            zoom: 2,
            pan: egui::Vec2::ZERO,
            background: Background::Checker,
            show_box: true,
            show_anchor: true,
            clock: None,
            accum: 0.0,
            revision: 0,
            tex_key: 0,
            textures: HashMap::new(),
            error: None,
        }
    }
}

impl ViewerState {
    pub fn directions(&self) -> usize {
        match &self.loaded {
            Some(Loaded::Sprite(s)) => s.sprite.directions.len(),
            Some(Loaded::Cof(c)) => c.directions(),
            None => 0,
        }
    }

    pub fn frames(&self) -> usize {
        match &self.loaded {
            Some(Loaded::Sprite(s)) => s.sprite.frames_per_direction(),
            Some(Loaded::Cof(c)) => c.frames(),
            None => 0,
        }
    }

    /// Replaces what is shown.
    pub fn set_loaded(&mut self, loaded: Loaded, source: SpriteSource, fps: f32, note: String) {
        self.loaded = Some(loaded);
        self.source = Some(source);
        self.fps = fps;
        self.fps_note = note;
        self.error = None;
        self.pan = egui::Vec2::ZERO;
        self.clamp();
        self.bump();
    }

    /// The content changed: textures must be rebuilt.
    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Keeps direction and frame inside the loaded sprite.
    pub fn clamp(&mut self) {
        self.dir = self.dir.min(self.directions().saturating_sub(1));
        self.frame = self.frame.min(self.frames().saturating_sub(1));
    }

    pub fn set_dir(&mut self, dir: usize) {
        self.dir = dir;
        self.clamp();
    }

    pub fn set_frame(&mut self, frame: usize) {
        self.frame = frame;
        self.clamp();
    }

    pub fn step(&mut self, delta: i64) {
        let n = self.frames() as i64;
        if n > 0 {
            self.frame = (self.frame as i64 + delta).rem_euclid(n) as usize;
        }
    }

    /// Advances playback to time `now` (seconds). Returns whether the frame
    /// changed. Stopped playback only resets the clock.
    pub fn advance(&mut self, now: f64) -> bool {
        let last = self.clock.replace(now);
        if !self.playing || self.frames() < 2 {
            self.accum = 0.0;
            return false;
        }
        let Some(last) = last else { return false };
        self.accum += (now - last).clamp(0.0, 1.0);
        let step = 1.0 / f64::from(self.fps.max(0.1));
        let before = self.frame;
        while self.accum >= step {
            self.accum -= step;
            self.step(1);
        }
        self.frame != before
    }

    /// The RGBA of one frame (`None` when nothing is loaded).
    pub fn rgba(&self, dir: usize, frame: usize, palette: &Palette) -> Option<Result<RgbaFrame, String>> {
        match self.loaded.as_ref()? {
            Loaded::Sprite(s) => {
                let img = s.sprite.directions.get(dir)?.get(frame)?;
                Some(Ok(to_rgba(img, palette)))
            }
            Loaded::Cof(c) => Some(c.compose(dir, frame, palette)),
        }
    }

    /// Union box (anchor space) used to keep the anchor still while playing.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        match self.loaded.as_ref()? {
            Loaded::Sprite(s) => s.sprite.bounds(),
            Loaded::Cof(c) => c.bounds(),
        }
    }

    /// The single sprite on show (not a COF).
    pub fn sprite(&self) -> Option<(&Sprite, SpriteFormat)> {
        match self.loaded.as_ref()? {
            Loaded::Sprite(s) => Some((&s.sprite, s.format)),
            Loaded::Cof(_) => None,
        }
    }

    pub fn cof(&self) -> Option<&CofView> {
        match self.loaded.as_ref()? {
            Loaded::Cof(c) => Some(c),
            Loaded::Sprite(_) => None,
        }
    }

    pub fn cof_mut(&mut self) -> Option<&mut CofView> {
        match self.loaded.as_mut()? {
            Loaded::Cof(c) => Some(c),
            Loaded::Sprite(_) => None,
        }
    }

    pub fn title(&self) -> String {
        self.source
            .as_ref()
            .map_or_else(|| "(nothing open)".to_owned(), SpriteSource::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::IndexedImage;

    fn sprite(dirs: usize, frames: usize) -> Loaded {
        let img = IndexedImage::new(2, 2, 0, -2);
        Loaded::Sprite(SingleSprite {
            sprite: Sprite {
                directions: vec![vec![img; frames]; dirs],
            },
            format: SpriteFormat::Dc6,
        })
    }

    #[test]
    fn speed_maps_to_frames_per_second() {
        assert_eq!(fps_for_speed(256), 25.0);
        assert_eq!(fps_for_speed(128), 12.5);
        assert_eq!(fps_for_speed(0), DEFAULT_FPS);
    }

    #[test]
    fn playback_steps_at_the_frame_rate_and_wraps() {
        let mut v = ViewerState::default();
        v.set_loaded(sprite(2, 4), SpriteSource::Imported("x".into()), 10.0, String::new());
        assert!(!v.advance(0.0), "stopped");
        v.playing = true;
        assert!(!v.advance(0.0));
        assert!(!v.advance(0.05), "half a frame");
        assert!(v.advance(0.101));
        assert_eq!(v.frame, 1);
        v.advance(0.401);
        assert_eq!(v.frame, 0, "wrapped after 4 frames");
        v.playing = false;
        assert!(!v.advance(5.0));
        assert_eq!(v.frame, 0);
    }

    #[test]
    fn direction_and_frame_stay_in_range() {
        let mut v = ViewerState::default();
        v.set_dir(5);
        assert_eq!(v.dir, 0, "nothing loaded");
        v.set_loaded(sprite(8, 3), SpriteSource::Imported("x".into()), 25.0, String::new());
        v.set_dir(9);
        v.set_frame(7);
        assert_eq!((v.dir, v.frame), (7, 2));
        v.step(-3);
        assert_eq!(v.frame, 2);
        v.set_loaded(sprite(1, 1), SpriteSource::Imported("y".into()), 25.0, String::new());
        assert_eq!((v.dir, v.frame), (0, 0), "clamped on load");
        assert!(v.rgba(0, 0, &super::super::palette::grey_palette()).is_some());
        assert!(v.rgba(3, 0, &super::super::palette::grey_palette()).is_none());
    }
}
