//! COF helpers: which sprite files make up a layer, and compositing the
//! layers of one animation frame (`docs/specs/cof.md`).
//!
//! # Compositing rules
//!
//! * Layers are drawn in the order of the COF priority table for the given
//!   (file direction, frame) ([`Cof::draw_order`]); a component with several
//!   sprites uses the **last** one, as a COF with several records of one
//!   component uses its last record ([`Cof::component_layer`]); a component
//!   the caller supplied no sprite for is skipped.
//! * The layer's sprite direction is the COF file direction; a layer with
//!   fewer directions is skipped, a layer with fewer frames shows its last
//!   frame.
//! * A layer is blended when [`CofLayerSprite::transparent`] is set and its
//!   `draw_effect` (COF `trans_b`) is 0 to 4 or 6; other values draw opaque
//!   ([`Blend::for_cof_layer`]).
//! * Shadows are not drawn. The canvas is the union of the drawn frames and
//!   starts fully transparent.
//!
//! The path rules, the last-record rule and the blend mapping live in
//! `d2-formats` (`d2_formats::cof`, `d2_formats::blend`), shared with the map
//! renderer (`capy-render`), which draws each layer as its own scaled bitmap.

use d2_formats::blend::Blend;
use d2_formats::cof::Cof;
use d2_formats::palette::Palette;

use crate::error::{Result, SpriteError};
use crate::model::{RgbaFrame, Sprite};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Object,
    Monster,
    Character,
}

impl UnitKind {
    /// The sprite folder, `data\global\{objects|monsters|chars}`.
    pub fn base_dir(self) -> &'static str {
        match self {
            UnitKind::Object => "data\\global\\objects",
            UnitKind::Monster => "data\\global\\monsters",
            UnitKind::Character => "data\\global\\chars",
        }
    }
}

/// Component codes by COF component index (HD TR LG RA LA RH LH SH S1..S8);
/// the same constant as `d2_formats::cof::COMPONENT_CODES`.
pub use d2_formats::cof::COMPONENT_CODES;

/// Candidate paths of one layer's sprite:
/// `data\global\{objects|monsters|chars}\{token}\{component}\{token}{component}{armor}{mode}{weapon_class}.dcc`
/// (lower case, MPQ-style backslashes), then the same with `.dc6`. The
/// caller opens the first that resolves. `component` is the two-letter code
/// (`"TR"`), `armor` the skin/armour token from obj.txt (`"lit"`, may be
/// empty), `mode` the animation mode (`"nu"`), `weapon_class` the COF
/// layer's class (`"hth"`; trailing NUL and spaces are ignored). The rule is
/// `d2_formats::cof::component_paths`, which the map renderer uses too.
pub fn cof_component_paths(
    kind: UnitKind,
    token: &str,
    mode: &str,
    weapon_class: &str,
    component: &str,
    armor: &str,
) -> Vec<String> {
    d2_formats::cof::component_paths(kind.base_dir(), token, mode, weapon_class, component, armor)
        .into()
}

/// A COF layer together with its decoded sprite.
#[derive(Clone, Debug)]
pub struct CofLayerSprite {
    /// COF component index (0..=15).
    pub component: u8,
    pub sprite: Sprite,
    /// The COF layer's `draw_effect` (`trans_b`).
    pub draw_effect: u8,
    /// The COF layer's `transparent` flag (`trans_a != 0`).
    pub transparent: bool,
}

fn blend_for(layer: &CofLayerSprite) -> Blend {
    Blend::for_cof_layer(layer.transparent, layer.draw_effect)
}

/// Composites one (file direction, frame) of a COF; see the module docs.
/// Fails when `dir`/`frame` are outside the COF. An animation with nothing
/// to draw gives a 0x0 frame.
pub fn compose_cof_frame(
    cof: &Cof,
    layers: &[CofLayerSprite],
    dir: u8,
    frame: u8,
    palette: &Palette,
) -> Result<RgbaFrame> {
    let order = cof.draw_order(dir, frame).ok_or_else(|| {
        SpriteError::invalid(format!(
            "direction {dir} / frame {frame} is outside the COF ({} directions x {} frames)",
            cof.directions, cof.frames_per_direction
        ))
    })?;
    // (layer, frame image) in drawing order
    let mut drawn = Vec::new();
    for &component in order {
        // the last matching sprite wins, like the last COF record does
        let Some(layer) = layers.iter().rev().find(|l| l.component == component) else {
            continue;
        };
        let Some(frames) = layer.sprite.directions.get(usize::from(dir)) else {
            continue;
        };
        let index = usize::from(frame).min(frames.len().saturating_sub(1));
        if let Some(img) = frames.get(index)
            && img.width > 0
            && img.height > 0
        {
            drawn.push((layer, img));
        }
    }
    let Some(first) = drawn.first() else {
        return Ok(RgbaFrame::new(0, 0, 0, 0));
    };
    let (mut x0, mut y0) = (first.1.offset_x, first.1.offset_y);
    let (mut x1, mut y1) = (
        x0.saturating_add_unsigned(first.1.width),
        y0.saturating_add_unsigned(first.1.height),
    );
    for (_, img) in &drawn {
        x0 = x0.min(img.offset_x);
        y0 = y0.min(img.offset_y);
        x1 = x1.max(img.offset_x.saturating_add_unsigned(img.width));
        y1 = y1.max(img.offset_y.saturating_add_unsigned(img.height));
    }
    let (w, h) = (
        (i64::from(x1) - i64::from(x0)) as u64,
        (i64::from(y1) - i64::from(y0)) as u64,
    );
    if w.saturating_mul(h) > 1 << 26 {
        return Err(SpriteError::invalid("the composite would be too large"));
    }
    let mut canvas = RgbaFrame::new(w as u32, h as u32, x0, y0);
    for (layer, img) in drawn {
        let blend = blend_for(layer);
        let (ox, oy) = ((img.offset_x - x0) as u32, (img.offset_y - y0) as u32);
        for y in 0..img.height {
            for x in 0..img.width {
                let i = (y * img.width + x) as usize;
                let (Some(&p), Some(&a)) = (img.pixels.get(i), img.alpha.get(i)) else {
                    continue;
                };
                if a == 0 || p == 0 {
                    continue;
                }
                let dst = canvas.get(ox + x, oy + y).unwrap_or([0, 0, 0, 0]);
                let src = palette.colors[usize::from(p)];
                canvas.put(ox + x, oy + y, blend.apply(dst, src));
            }
        }
    }
    Ok(canvas)
}
