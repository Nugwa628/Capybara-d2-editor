//! App themes and fonts (ds1-studio's `src/ui/themes.ts`, rebuilt for egui).
//!
//! A theme is five base colours (background, panel, text, accent, line); the
//! in-between shades (hover, borders, faint text...) are worked out from
//! them, so a custom theme needs only five colours and still looks
//! consistent. Built-in themes may pin exact shades instead. Share codes use
//! PD2 Filter Forge's format (`ffthemes:` + base64 of `{"name", "colors"}`),
//! like ds1-studio, so a theme made in one app pastes into the others.
//!
//! Fonts: Segoe UI is read from `C:\Windows\Fonts` at start-up (egui's own
//! font is the fallback), and the Lucide icon font (ISC) is embedded as the
//! `icons` family.

use bevy_egui::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use serde::{Deserialize, Serialize};

/// The default theme's id.
pub const DEFAULT_THEME: &str = "studio";

/// The Lucide icon font (ISC, see THIRD-PARTY-LICENSES.md).
const LUCIDE: &[u8] = include_bytes!("../../../../assets/fonts/lucide.ttf");
/// The family name icons are drawn with.
const ICON_FAMILY: &str = "icons";
/// Segoe UI, read at run time (it is not ours to ship).
const SEGOE_UI: &str = r"C:\Windows\Fonts\segoeui.ttf";

/// An RGB colour.
pub type Rgb = [u8; 3];

const fn rgb(v: u32) -> Rgb {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The five base colours of a theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub bg: Rgb,
    pub panel: Rgb,
    pub text: Rgb,
    pub accent: Rgb,
    pub line: Rgb,
}

/// Shades a built-in theme pins instead of deriving them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exact {
    pub bg2: Option<Rgb>,
    pub panel2: Option<Rgb>,
    pub line2: Option<Rgb>,
    pub heading: Option<Rgb>,
    pub muted: Option<Rgb>,
    pub hover: Option<Rgb>,
    pub accent_text: Option<Rgb>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub colors: ThemeColors,
    pub exact: Exact,
    pub custom: bool,
}

/// A user's theme as `config.toml` stores it (colours as `#rrggbb`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomTheme {
    pub id: String,
    pub name: String,
    pub bg: String,
    pub panel: String,
    pub text: String,
    pub accent: String,
    pub line: String,
}

impl CustomTheme {
    /// `None` when a colour is not `#rrggbb`.
    pub fn to_theme(&self) -> Option<Theme> {
        let colors = ThemeColors {
            bg: parse_hex(&self.bg)?,
            panel: parse_hex(&self.panel)?,
            text: parse_hex(&self.text)?,
            accent: parse_hex(&self.accent)?,
            line: parse_hex(&self.line)?,
        };
        Some(Theme {
            id: self.id.clone(),
            name: self.name.clone(),
            dark: is_dark(colors.bg),
            colors,
            exact: Exact::default(),
            custom: true,
        })
    }
}

fn builtin(
    id: &str,
    name: &str,
    dark: bool,
    [bg, panel, text, accent, line]: [u32; 5],
    exact: Exact,
) -> Theme {
    Theme {
        id: id.to_owned(),
        name: name.to_owned(),
        dark,
        colors: ThemeColors {
            bg: rgb(bg),
            panel: rgb(panel),
            text: rgb(text),
            accent: rgb(accent),
            line: rgb(line),
        },
        exact,
        custom: false,
    }
}

/// The nine built-in themes, in ds1-studio's order.
pub fn builtin_themes() -> Vec<Theme> {
    let e = |bg2, panel2, line2, muted, hover| Exact {
        bg2: Some(rgb(bg2)),
        panel2: Some(rgb(panel2)),
        line2: Some(rgb(line2)),
        muted: Some(rgb(muted)),
        hover: Some(rgb(hover)),
        ..Exact::default()
    };
    vec![
        builtin(
            "studio",
            "DS1 Studio",
            true,
            [0x0b0c0f, 0x13151a, 0xd8dbe2, 0xd4a84f, 0x252932],
            Exact {
                heading: Some(rgb(0xaeb4bf)),
                accent_text: Some(rgb(0xf1d9a6)),
                ..e(0x101216, 0x181b21, 0x3a404c, 0x7d8491, 0x1e222a)
            },
        ),
        builtin(
            "sanctuary",
            "Sanctuary",
            true,
            [0x0f0d10, 0x1a171c, 0xebe4d6, 0xd9a441, 0x2e2830],
            e(0x151216, 0x221e24, 0x3b3440, 0xa1978c, 0x262129),
        ),
        builtin(
            "midnight",
            "Midnight",
            true,
            [0x0b0f14, 0x141a22, 0xe3e9f1, 0x4a9de0, 0x242d39],
            e(0x10151c, 0x1a212b, 0x303b4a, 0x93a1b3, 0x1c2430),
        ),
        builtin(
            "graphite",
            "Graphite",
            true,
            [0x111214, 0x1a1c1f, 0xe6e7e9, 0x9aa7b8, 0x2a2d31],
            Exact::default(),
        ),
        builtin(
            "nord",
            "Nord",
            true,
            [0x1e222a, 0x262b34, 0xe5e9f0, 0x88c0d0, 0x353c48],
            Exact::default(),
        ),
        builtin(
            "ember",
            "Ember",
            true,
            [0x130c0b, 0x1e1412, 0xf1e6e1, 0xe2583e, 0x33221e],
            Exact::default(),
        ),
        builtin(
            "verdant",
            "Verdant",
            true,
            [0x0c1310, 0x142019, 0xe3efe8, 0x5fbf8a, 0x223328],
            Exact::default(),
        ),
        builtin(
            "parchment",
            "Parchment",
            false,
            [0xefe8da, 0xf8f3e8, 0x2b241b, 0xb7791f, 0xd8ccb4],
            e(0xe7dfcf, 0xefe7d7, 0xc8b995, 0x6d604d, 0xebe2cf),
        ),
        builtin(
            "snow",
            "Snow",
            false,
            [0xf3f5f8, 0xffffff, 0x1c2230, 0x3b6fd8, 0xdde2ea],
            Exact::default(),
        ),
    ]
}

/// The theme with `id` among the custom and built-in ones; the default
/// theme when there is none.
pub fn find_theme(id: &str, custom: &[CustomTheme]) -> Theme {
    let builtins = builtin_themes();
    custom
        .iter()
        .filter(|c| c.id == id)
        .find_map(CustomTheme::to_theme)
        .or_else(|| builtins.iter().find(|t| t.id == id).cloned())
        .unwrap_or_else(|| builtins.into_iter().next().unwrap_or_else(fallback_theme))
}

fn fallback_theme() -> Theme {
    builtin(
        DEFAULT_THEME,
        "DS1 Studio",
        true,
        [0x0b0c0f, 0x13151a, 0xd8dbe2, 0xd4a84f, 0x252932],
        Exact::default(),
    )
}

pub fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    let body = s.trim().strip_prefix('#')?;
    if body.len() != 6 || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(body, 16).ok()?;
    Some(rgb(v))
}

/// Relative luminance below one half (ds1-studio's `isDarkColor`).
pub fn is_dark(c: Rgb) -> bool {
    let [r, g, b] = c.map(|v| f32::from(v) / 255.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
}

/// CSS `color-mix(in srgb, a pct%, b)`.
fn mix(a: Rgb, b: Rgb, pct: u32) -> Rgb {
    let p = pct as f32 / 100.0;
    let mut out = [0u8; 3];
    for i in 0..3 {
        let v = f32::from(a[i]) * p + f32::from(b[i]) * (1.0 - p);
        out[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Every shade the UI paints with, worked out from a theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shades {
    pub dark: bool,
    pub bg: Color32,
    pub bg2: Color32,
    pub panel: Color32,
    pub panel2: Color32,
    pub border: Color32,
    pub border2: Color32,
    pub text: Color32,
    pub heading: Color32,
    pub muted: Color32,
    pub hover: Color32,
    pub accent: Color32,
    /// The accent at low opacity: selected rows, active chips.
    pub accent_dim: Color32,
    pub accent_hover: Color32,
    /// Text on a selected row or active tab.
    pub accent_text: Color32,
    /// Text on a solid accent background.
    pub on_accent: Color32,
    pub error: Color32,
    pub warn: Color32,
    pub ok: Color32,
    pub info: Color32,
}

impl Shades {
    /// ds1-studio's `themeVars`.
    pub fn of(t: &Theme) -> Shades {
        let c = t.colors;
        let e = t.exact;
        let a = c.accent;
        let accent_dim = {
            let alpha = if t.dark { 0.14 } else { 0.18 };
            Color32::from_rgba_unmultiplied(a[0], a[1], a[2], (alpha * 255.0_f32).round() as u8)
        };
        let accent_text = match e.accent_text {
            Some(x) => x,
            None => mix(a, c.text, if t.dark { 45 } else { 75 }),
        };
        Shades {
            dark: t.dark,
            bg: c32(c.bg),
            bg2: c32(e.bg2.unwrap_or_else(|| mix(c.panel, c.bg, 45))),
            panel: c32(c.panel),
            panel2: c32(e.panel2.unwrap_or_else(|| mix(c.text, c.panel, 5))),
            border: c32(c.line),
            border2: c32(e.line2.unwrap_or_else(|| mix(c.text, c.line, 12))),
            text: c32(c.text),
            heading: c32(e.heading.unwrap_or_else(|| mix(c.text, c.bg, 78))),
            muted: c32(e.muted.unwrap_or_else(|| mix(c.text, c.bg, 58))),
            hover: c32(e.hover.unwrap_or_else(|| mix(c.text, c.panel, 7))),
            accent: c32(a),
            accent_dim,
            accent_hover: c32(mix(a, if t.dark { [255; 3] } else { [0; 3] }, 85)),
            accent_text: c32(accent_text),
            on_accent: if is_dark(a) {
                Color32::WHITE
            } else {
                Color32::from_rgb(0x1a, 0x14, 0x06)
            },
            error: c32(if t.dark { rgb(0xff5a6e) } else { rgb(0xc53030) }),
            warn: c32(if t.dark { rgb(0xf0b54a) } else { rgb(0xb7791f) }),
            ok: c32(if t.dark { rgb(0x7fd38a) } else { rgb(0x2f855a) }),
            info: c32(if t.dark { rgb(0x7cc8ff) } else { rgb(0x2b6cb0) }),
        }
    }

    /// egui's visuals for these shades.
    pub fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let radius = CornerRadius::same(5);
        v.override_text_color = None;
        v.weak_text_color = Some(self.muted);
        v.hyperlink_color = self.accent;
        v.faint_bg_color = self.bg2;
        v.extreme_bg_color = self.bg;
        v.text_edit_bg_color = Some(self.bg);
        v.code_bg_color = self.panel2;
        v.warn_fg_color = self.warn;
        v.error_fg_color = self.error;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.border2);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(6);
        v.panel_fill = self.panel;
        v.selection.bg_fill = self.accent_dim;
        v.selection.stroke = Stroke::new(1.0, self.accent_text);
        v.text_cursor.stroke = Stroke::new(2.0, self.accent);
        v.striped = false;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = self.panel2;
        w.inactive.weak_bg_fill = self.panel2;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;

        w.hovered.bg_fill = self.hover;
        w.hovered.weak_bg_fill = self.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, self.border2);
        w.hovered.fg_stroke = Stroke::new(1.5, self.text);
        w.hovered.corner_radius = radius;

        w.active.bg_fill = self.accent_dim;
        w.active.weak_bg_fill = self.accent_dim;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(1.5, self.accent_text);
        w.active.corner_radius = radius;

        w.open.bg_fill = self.hover;
        w.open.weak_bg_fill = self.hover;
        w.open.bg_stroke = Stroke::new(1.0, self.border2);
        w.open.fg_stroke = Stroke::new(1.0, self.text);
        w.open.corner_radius = radius;
        v
    }
}

/// Puts `theme` on the context (cheap; call when it changed).
pub fn apply(ctx: &egui::Context, theme: &Theme) {
    let shades = Shades::of(theme);
    let visuals = shades.visuals();
    ctx.all_styles_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(7.0, 3.0);
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::proportional(17.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
    });
}

/// The fonts: Segoe UI first when Windows has it, egui's fonts behind it,
/// and the icon family. Returns whether Segoe UI was found.
pub fn install_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    let segoe = std::fs::read(SEGOE_UI).ok();
    let found = segoe.is_some();
    if let Some(bytes) = segoe {
        fonts
            .font_data
            .insert("segoe_ui".to_owned(), FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe_ui".to_owned());
    }
    fonts
        .font_data
        .insert("lucide".to_owned(), FontData::from_static(LUCIDE).into());
    // egui's own font behind the icons, for the replacement glyph.
    let mut icons = vec!["lucide".to_owned()];
    icons.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .into_iter()
            .flatten()
            .filter(|name| name.as_str() != "segoe_ui")
            .take(1)
            .cloned(),
    );
    fonts
        .families
        .insert(FontFamily::Name(ICON_FAMILY.into()), icons);
    ctx.set_fonts(fonts);
    found
}

/// The icon font at `size`; the proportional font until the icon family is
/// installed (egui applies new fonts one frame later and panics on an
/// unknown family).
pub fn icon_font(size: f32, ready: bool) -> FontId {
    if ready {
        FontId::new(size, FontFamily::Name(ICON_FAMILY.into()))
    } else {
        FontId::proportional(size)
    }
}

/// Lucide code points of the icons the UI uses (from `lucide-static`'s
/// `font/lucide.css`, 1.59.0).
pub mod icon {
    pub const NONE: char = '\0';
    pub const ARROW_RIGHT_LEFT: char = '\u{e417}';
    pub const BOX: char = '\u{e061}';
    pub const BOXES: char = '\u{e2d0}';
    pub const CHECK: char = '\u{e06c}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    pub const CHEVRON_LEFT: char = '\u{e06e}';
    pub const CHEVRON_RIGHT: char = '\u{e06f}';
    pub const CIRCLE_X: char = '\u{e084}';
    pub const CLIPBOARD_PASTE: char = '\u{e3e8}';
    pub const CLOCK: char = '\u{e087}';
    pub const COPY: char = '\u{e09e}';
    pub const COPY_CHECK: char = '\u{e3fb}';
    pub const ERASER: char = '\u{e28f}';
    pub const EXPAND: char = '\u{e21a}';
    pub const EYE: char = '\u{e0ba}';
    pub const FILE_INPUT: char = '\u{e0c5}';
    pub const FILE_X: char = '\u{e0cd}';
    pub const FLAG: char = '\u{e0d1}';
    pub const FOLDER: char = '\u{e0d7}';
    pub const FOLDER_OPEN: char = '\u{e247}';
    pub const FULLSCREEN: char = '\u{e534}';
    pub const GRID: char = '\u{e0e9}';
    pub const HASH: char = '\u{e0ef}';
    pub const HISTORY: char = '\u{e1f5}';
    pub const IMAGE: char = '\u{e0f6}';
    pub const INFO: char = '\u{e0f9}';
    pub const KEYBOARD: char = '\u{e284}';
    pub const LASSO: char = '\u{e1ce}';
    pub const LAYERS: char = '\u{e529}';
    pub const LIST_TREE: char = '\u{e408}';
    pub const LOG_OUT: char = '\u{e10e}';
    pub const MAP_PINNED: char = '\u{e53d}';
    pub const MAXIMIZE: char = '\u{e112}';
    pub const MOUSE_POINTER: char = '\u{e1c3}';
    pub const PACKAGE_CHECK: char = '\u{e266}';
    pub const PAINTBRUSH: char = '\u{e2e7}';
    pub const PALETTE: char = '\u{e1dd}';
    pub const PANEL_LEFT: char = '\u{e12a}';
    pub const PANEL_RIGHT: char = '\u{e431}';
    pub const PIPETTE: char = '\u{e13b}';
    pub const PLUS: char = '\u{e13d}';
    pub const REDO: char = '\u{e2a0}';
    pub const ROUTE: char = '\u{e53e}';
    pub const SAVE: char = '\u{e14d}';
    pub const SCAN_SEARCH: char = '\u{e537}';
    pub const SCISSORS: char = '\u{e14e}';
    pub const SCROLL_TEXT: char = '\u{e45f}';
    pub const SEARCH: char = '\u{e151}';
    pub const SETTINGS: char = '\u{e154}';
    pub const SQUARE_DASHED_POINTER: char = '\u{e509}';
    pub const TRASH: char = '\u{e18e}';
    pub const UNDO: char = '\u{e2a1}';
    pub const WAVES: char = '\u{e283}';
    pub const X: char = '\u{e1b2}';
    pub const ZOOM_IN: char = '\u{e1b6}';
    pub const ZOOM_OUT: char = '\u{e1b7}';
}

// ---- share codes -----------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.trim().bytes().filter(|b| !b.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[derive(Serialize, Deserialize)]
struct ShareColors {
    bg: String,
    panel: String,
    text: String,
    accent: String,
    line: String,
}

#[derive(Serialize, Deserialize)]
struct Share {
    name: String,
    colors: ShareColors,
}

/// A theme as a share code (PD2 Filter Forge's format).
pub fn export_code(t: &Theme) -> String {
    let share = Share {
        name: t.name.clone(),
        colors: ShareColors {
            bg: hex(t.colors.bg),
            panel: hex(t.colors.panel),
            text: hex(t.colors.text),
            accent: hex(t.colors.accent),
            line: hex(t.colors.line),
        },
    };
    let json = serde_json::to_string(&share).unwrap_or_default();
    format!("ffthemes:{}", b64_encode(json.as_bytes()))
}

/// Reads a share code (`ffthemes:` or `ds1themes:`) into a custom theme
/// named `id`; `None` when it is not one.
pub fn import_code(code: &str, id: String) -> Option<CustomTheme> {
    let code = code.trim();
    let body = code
        .strip_prefix("ffthemes:")
        .or_else(|| code.strip_prefix("ds1themes:"))
        .unwrap_or(code);
    let bytes = b64_decode(body)?;
    let share: Share = serde_json::from_slice(&bytes).ok()?;
    let custom = CustomTheme {
        id,
        name: share.name.chars().take(30).collect(),
        bg: share.colors.bg,
        panel: share.colors.panel,
        text: share.colors.text,
        accent: share.colors.accent,
        line: share.colors.line,
    };
    custom.to_theme().map(|_| custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_builtins_with_unique_ids_and_the_default_first() {
        let themes = builtin_themes();
        assert_eq!(themes.len(), 9);
        assert_eq!(themes[0].id, DEFAULT_THEME);
        let mut ids: Vec<_> = themes.iter().map(|t| t.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 9);
        // `dark` agrees with the background for every built-in.
        for t in &themes {
            assert_eq!(t.dark, is_dark(t.colors.bg), "{}", t.id);
        }
    }

    #[test]
    fn studio_shades_match_ds1_studio_css() {
        let s = Shades::of(&find_theme("studio", &[]));
        assert_eq!(s.bg2, Color32::from_rgb(0x10, 0x12, 0x16));
        assert_eq!(s.accent_text, Color32::from_rgb(0xf1, 0xd9, 0xa6));
        assert_eq!(s.on_accent, Color32::from_rgb(0x1a, 0x14, 0x06));
        // Derived: graphite's muted = mix(text, bg, 58%).
        let g = Shades::of(&find_theme("graphite", &[]));
        assert_eq!(g.muted, c32(mix(rgb(0xe6e7e9), rgb(0x111214), 58)));
    }

    #[test]
    fn unknown_ids_fall_back_to_the_default() {
        assert_eq!(find_theme("nope", &[]).id, DEFAULT_THEME);
    }

    #[test]
    fn share_codes_round_trip_and_read_filter_forge_codes() {
        let snow = find_theme("snow", &[]);
        let code = export_code(&snow);
        assert!(code.starts_with("ffthemes:"));
        let back = import_code(&code, "custom-1".into()).expect("import");
        let theme = back.to_theme().expect("theme");
        assert_eq!(theme.colors, snow.colors);
        assert_eq!(theme.name, "Snow");
        assert!(!theme.dark && theme.custom);
        // btoa('{"name":"X","colors":{"bg":"#000000","panel":"#111111","text":"#ffffff","accent":"#ff0000","line":"#222222"}}')
        let js = "ds1themes:eyJuYW1lIjoiWCIsImNvbG9ycyI6eyJiZyI6IiMwMDAwMDAiLCJwYW5lbCI6IiMxMTExMTEiLCJ0ZXh0IjoiI2ZmZmZmZiIsImFjY2VudCI6IiNmZjAwMDAiLCJsaW5lIjoiIzIyMjIyMiJ9fQ==";
        let t = import_code(js, "c".into())
            .and_then(|c| c.to_theme())
            .expect("js code");
        assert_eq!(t.colors.accent, [255, 0, 0]);
        assert!(t.dark);
        assert!(import_code("ffthemes:!!!", "x".into()).is_none());
        assert!(import_code("ffthemes:e30=", "x".into()).is_none(), "{{}}");
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let enc = b64_encode(s.as_bytes());
            assert_eq!(b64_decode(&enc).as_deref(), Some(s.as_bytes()));
        }
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
    }
}
