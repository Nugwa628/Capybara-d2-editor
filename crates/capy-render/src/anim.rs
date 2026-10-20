//! Animated floor tiles (water, lava) and the game-view frame helpers.
//!
//! ds1-studio (`GameData.ts` `TileLibrary.frames`/`isAnimation`, `MapView.tsx`
//! clock): a tile is an animation only when every variant of its key is
//! flagged animated, the rarities are distinct and start at 0 (Act 5 snow
//! flags plain random variants as animated with repeated rarities; the game
//! does not animate those). The frames are the variants ordered by rarity.
//! The clock runs in game ticks (25/s); animated floors advance every 2.5
//! ticks, i.e. 10 frames per second.

use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::TileKey;

use crate::tileset::{TileRef, TileSet};

/// Animated floors advance at this rate (2.5 game ticks per frame).
pub const FLOOR_FPS: f64 = 10.0;
/// The game's tick rate.
pub const TICKS_PER_SECOND: f64 = 25.0;

/// The animation frames of a floor/wall key in order, or empty when the key
/// is not a real frame sequence.
pub fn animation_frames(tiles: &TileSet, key: TileKey) -> Vec<TileRef> {
    let list = tiles.candidates(key);
    if list.len() < 2 {
        return Vec::new();
    }
    let mut rar = Vec::with_capacity(list.len());
    for r in list {
        let Some(t) = tiles.tile(r) else {
            return Vec::new();
        };
        if t.animated == 0 {
            return Vec::new();
        }
        rar.push(t.rarity);
    }
    let mut sorted = rar.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != rar.len() || sorted[0] != 0 {
        return Vec::new();
    }
    let mut frames: Vec<(i32, TileRef)> = rar.into_iter().zip(list.iter().cloned()).collect();
    frames.sort_by_key(|(r, _)| *r);
    frames.into_iter().map(|(_, r)| r).collect()
}

/// Which frame of `n` shows after `seconds` (10 fps, looping).
pub fn frame_at(n: usize, seconds: f64) -> usize {
    if n == 0 {
        return 0;
    }
    ((seconds.max(0.0) * FLOOR_FPS).floor() as usize) % n
}

/// A drawn cell whose tile animates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimatedCell {
    pub floor_layer: usize,
    pub x: u32,
    pub y: u32,
    pub frames: Vec<TileRef>,
}

impl AnimatedCell {
    pub fn frame(&self, seconds: f64) -> &TileRef {
        &self.frames[frame_at(self.frames.len(), seconds)]
    }
}

/// Every visible floor cell whose key is an animation (water, lava).
pub fn animated_floor_cells(ds1: &Ds1, tiles: &TileSet) -> Vec<AnimatedCell> {
    let mut cache: std::collections::HashMap<TileKey, Vec<TileRef>> =
        std::collections::HashMap::new();
    let mut out = Vec::new();
    for (layer, g) in ds1.floors.iter().enumerate() {
        for y in 0..ds1.height {
            for x in 0..ds1.width {
                let c = g.get(x, y).unwrap_or(Cell::EMPTY);
                if c.is_empty() || c.hidden() {
                    continue;
                }
                let key = TileKey {
                    orientation: 0,
                    main: c.main_index(),
                    sub: c.sub_index(),
                };
                let frames = cache
                    .entry(key)
                    .or_insert_with(|| animation_frames(tiles, key));
                if frames.len() > 1 {
                    out.push(AnimatedCell {
                        floor_layer: layer,
                        x,
                        y,
                        frames: frames.clone(),
                    });
                }
            }
        }
    }
    out
}

/// The in-game screen size "Game view" frames (default 800x600; valid range
/// 320..=7680 per side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameView {
    pub width: u32,
    pub height: u32,
}

impl Default for GameView {
    fn default() -> Self {
        GameView {
            width: 800,
            height: 600,
        }
    }
}

impl GameView {
    pub fn new(width: u32, height: u32) -> Option<GameView> {
        let ok = |v: u32| (320..=7680).contains(&v);
        (ok(width) && ok(height)).then_some(GameView { width, height })
    }
    /// The world rectangle `(x0, y0, w, h)` the character sees, centred on
    /// `(cx, cy)`: the game draws at 1:1, so it is the screen size itself.
    pub fn world_rect(&self, cx: f64, cy: f64) -> (f64, f64, f64, f64) {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        (cx - w / 2.0, cy - h / 2.0, w, h)
    }
    /// Zoom that fits the game screen plus a 20 px margin per side in a
    /// viewport of `vw` x `vh` device pixels: at most 1.0 (or `max` when
    /// smooth zoom allows the device pixel ratio).
    pub fn fit_zoom(&self, vw: f64, vh: f64, max: f64) -> f64 {
        max.min(vw / (f64::from(self.width) + 40.0))
            .min(vh / (f64::from(self.height) + 40.0))
    }
}
