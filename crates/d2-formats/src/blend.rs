//! Sprite blend modes and their pixel maths (`docs/specs/cof.md` section 3).
//!
//! [`Blend`] is shared by the map renderer (`capy-render`, which re-exports it
//! as `capy_render::Blend`) and the sprite compositor (`capy-sprite`); the
//! maths reproduces DS1Edit's Allegro blenders (`preview.c:182,291-380`).
//! [`Blend::for_cof_layer`] is the COF `trans_a`/`trans_b` mapping both use.

/// Sprite blend modes keyed on COF `trans_b` (`preview.c:291-380`): 25/50/75%
/// alpha, additive, multiply, and `(ONE, INV_SRC_COLOR)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Alpha25,
    Alpha50,
    Alpha75,
    Additive,
    Multiply,
    Screen,
    /// DS1Edit's projected object shadow (`a5_draw_shadow_sprite`,
    /// `ui/compat.h:151`): the sprite as a black silhouette with coverage
    /// `1 - level/32`, so `out = dst * level / 32`. `level` is D2's darkness
    /// level, 10 for opaque layers and 28 for transparent ones
    /// (`preview.c:2259`). The source colour is ignored.
    Darken(u8),
}

impl Blend {
    /// The blend of a COF layer (`preview.c:2131`): blended only when
    /// `transparent` (`trans_a != 0`) and `draw_effect` (`trans_b`) is 0 to 4
    /// or 6; every other layer draws opaque ([`Blend::Normal`]).
    pub fn for_cof_layer(transparent: bool, draw_effect: u8) -> Blend {
        if !transparent {
            return Blend::Normal;
        }
        match draw_effect {
            0 => Blend::Alpha25,
            1 => Blend::Alpha50,
            2 => Blend::Alpha75,
            3 => Blend::Additive,
            4 => Blend::Multiply,
            6 => Blend::Screen,
            _ => Blend::Normal,
        }
    }

    /// Blends one source colour over a destination colour (both straight RGB,
    /// destination alpha `dst_a`); returns the new `[r, g, b, a]`.
    pub fn apply(self, dst: [u8; 4], src: [u8; 3]) -> [u8; 4] {
        let weight: f32 = match self {
            Blend::Normal => 1.0,
            Blend::Alpha25 => 0.25,
            Blend::Alpha50 => 0.5,
            Blend::Alpha75 => 0.75,
            Blend::Darken(level) => {
                let weight = 1.0 - f32::from(level.min(32)) / 32.0;
                return tinted(dst, [0, 0, 0], weight);
            }
            Blend::Additive => {
                return [
                    dst[0].saturating_add(src[0]),
                    dst[1].saturating_add(src[1]),
                    dst[2].saturating_add(src[2]),
                    255,
                ];
            }
            Blend::Multiply => {
                let m = |s: u8, d: u8| ((u16::from(s) * u16::from(d) + 127) / 255) as u8;
                return [m(src[0], dst[0]), m(src[1], dst[1]), m(src[2], dst[2]), 255];
            }
            Blend::Screen => {
                let m = |s: u8, d: u8| {
                    let inv = 255 - u16::from(s);
                    (u16::from(s) + (u16::from(d) * inv + 127) / 255).min(255) as u8
                };
                return [m(src[0], dst[0]), m(src[1], dst[1]), m(src[2], dst[2]), 255];
            }
        };
        if weight >= 1.0 {
            return [src[0], src[1], src[2], 255];
        }
        tinted(dst, src, weight)
    }
}

/// A draw tinted with `(w, w, w, w)` under the blender `(ALPHA, INVERSE_ALPHA)`.
///
/// The reference tints the sprite with (f, f, f, f) (`preview.c:182`)
/// but its blender is (ALPHA, INVERSE_ALPHA), so the tint scales the
/// colour as well as the coverage: out = d * (1 - f) + s * f * f. That
/// is a straight "over" of colour `s * f` with coverage `f` (measured on
/// the shadow pixels of the oracle: 0.5 d + 0.25 s). Over an opaque
/// pixel the result stays opaque; DS1Edit's own canvas drops to alpha
/// 0.75 there, a blender artefact we do not reproduce.
/// The tint is stored as a byte, so the weight is quantised: 0.5 is 128/255.
fn tinted(dst: [u8; 4], src: [u8; 3], weight: f32) -> [u8; 4] {
    let weight = (weight * 255.0 + 0.5).floor() / 255.0;
    let da = f32::from(dst[3]) / 255.0;
    let out_a = weight + da * (1.0 - weight);
    if out_a <= 0.0 {
        return dst;
    }
    let mix = |s: u8, d: u8| -> u8 {
        let v = (f32::from(s) * weight * weight + f32::from(d) * da * (1.0 - weight)) / out_a;
        v.round().clamp(0.0, 255.0) as u8
    };
    [
        mix(src[0], dst[0]),
        mix(src[1], dst[1]),
        mix(src[2], dst[2]),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn blend_normal_replaces_the_pixel() {
        assert_eq!(
            Blend::Normal.apply([9, 9, 9, 255], [1, 2, 3]),
            [1, 2, 3, 255]
        );
    }

    #[test]
    fn blend_alpha_tint_scales_colour_and_coverage() {
        // Measured on the oracle's shadow pixels: out = d * (127/255) + s * (128/255)^2.
        let out = Blend::Alpha50.apply([161, 146, 105, 255], [11, 10, 11]);
        assert_eq!(out, [83, 75, 55, 255]);
        // 25% and 75% quantise the tint to 64/255 and 191/255.
        let a25 = Blend::Alpha25.apply([200, 200, 200, 255], [255, 0, 0]);
        assert_eq!(a25, [166, 150, 150, 255]);
        let a75 = Blend::Alpha75.apply([200, 200, 200, 255], [255, 0, 0]);
        assert_eq!(a75, [193, 50, 50, 255]);
        // Over nothing the coverage stays in the alpha channel.
        let over_void = Blend::Alpha50.apply([0, 0, 0, 0], [200, 100, 0]);
        assert_eq!(over_void[3], 128);
    }

    #[test]
    fn blend_additive_multiply_screen() {
        assert_eq!(
            Blend::Additive.apply([200, 10, 0, 255], [100, 20, 30]),
            [255, 30, 30, 255]
        );
        assert_eq!(
            Blend::Multiply.apply([255, 128, 0, 255], [100, 100, 100]),
            [100, 50, 0, 255]
        );
        // (ONE, INV_SRC_COLOR): s + d * (1 - s).
        assert_eq!(
            Blend::Screen.apply([100, 100, 100, 255], [255, 0, 128]),
            [255, 100, 178, 255]
        );
    }

    #[test]
    fn darken_scales_the_destination_by_level_over_32() {
        // the source colour is ignored
        let out = Blend::Darken(16).apply([200, 100, 50, 255], [255, 255, 255]);
        assert_eq!(out, Blend::Darken(16).apply([200, 100, 50, 255], [0, 0, 0]));
        assert!(out[0] < 200 && out[0] > 80, "{out:?}");
        // level 32 and above leaves the pixel alone
        assert_eq!(
            Blend::Darken(40).apply([7, 8, 9, 255], [0, 0, 0]),
            [7, 8, 9, 255]
        );
    }

    #[test]
    fn cof_layers_blend_only_when_transparent() {
        for (effect, blend) in [
            (0, Blend::Alpha25),
            (1, Blend::Alpha50),
            (2, Blend::Alpha75),
            (3, Blend::Additive),
            (4, Blend::Multiply),
            (5, Blend::Normal),
            (6, Blend::Screen),
            (7, Blend::Normal),
            (255, Blend::Normal),
        ] {
            assert_eq!(Blend::for_cof_layer(true, effect), blend, "{effect}");
            assert_eq!(Blend::for_cof_layer(false, effect), Blend::Normal);
        }
    }
}
