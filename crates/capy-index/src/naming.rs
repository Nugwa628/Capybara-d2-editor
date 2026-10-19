//! Deterministic generated names. See `docs/api/capy-index.md`.
//!
//! Everything here is a pure function of its inputs (a path, an orientation,
//! a mean colour, an edge-density number), so the rules are unit-testable
//! without game files.

use crate::query::AssetKind;

const TILES_PREFIX: &str = "data\\global\\tiles\\";
const GLOBAL_PREFIX: &str = "data\\global\\";
const DATA_PREFIX: &str = "data\\";

/// Edge density (mean Sobel magnitude) below which a tile is `flat`.
pub const FLAT_BELOW: f32 = 20.0;
/// Edge density below which a tile is `noisy`; at or above it is `edgy`.
pub const NOISY_BELOW: f32 = 60.0;
/// Mean opaque luma (0..=255) below which a tile is simply `dark`.
pub const DARK_BELOW: f32 = 25.0;

/// Rec. 601 luma of an RGB colour.
pub fn luma(rgb: [u8; 3]) -> f32 {
    0.299 * f32::from(rgb[0]) + 0.587 * f32::from(rgb[1]) + 0.114 * f32::from(rgb[2])
}

/// Saturation (HSV) below which a colour is `grey` or `white`.
pub const GREY_BELOW_SATURATION: f32 = 0.15;
/// Mean luma below which a colour is `grey`; at or above it is `white`.
pub const WHITE_FROM_LUMA: f32 = 170.0;
/// HSV value below which red, orange and yellow read as `brown`.
pub const BROWN_BELOW_VALUE: f32 = 0.55;
/// HSV saturation below which orange and yellow (bright enough) read as `tan`.
pub const TAN_BELOW_SATURATION: f32 = 0.35;

/// Hue sectors in degrees: the name applies from the listed start up to the
/// next start. Red wraps around 0 (its sector is [345, 360) and [0, 15)).
const HUE_SECTORS: [(f32, &str); 10] = [
    (15.0, "orange"),
    (45.0, "yellow"),
    (70.0, "green"),
    (160.0, "teal"),
    (185.0, "cyan"),
    (200.0, "blue"),
    (255.0, "purple"),
    (290.0, "magenta"),
    (320.0, "pink"),
    (345.0, "red"),
];

/// HSV of an RGB colour: hue in degrees `[0, 360)`, saturation and value in
/// `[0, 1]`. Grey colours have hue 0.
fn hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let value = max;
    let saturation = if max <= 0.0 { 0.0 } else { delta / max };
    let hue = if delta <= 0.0 {
        0.0
    } else if (max - r).abs() <= f32::EPSILON {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() <= f32::EPSILON {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (hue.rem_euclid(360.0), saturation, value)
}

/// The colour name of a tile's mean opaque colour, by the HSV rule of
/// `docs/api/capy-index.md`: `dark` below luma 25; `grey` or `white` below
/// saturation 0.15 (split at luma 170); otherwise the hue sector, with red,
/// orange and yellow below value 0.55 turned `brown` and pale orange or
/// yellow turned `tan`.
pub fn color_name(rgb: [u8; 3]) -> &'static str {
    let y = luma(rgb);
    if y < DARK_BELOW {
        return "dark";
    }
    let (hue, saturation, value) = hsv(rgb);
    if saturation < GREY_BELOW_SATURATION {
        return if y < WHITE_FROM_LUMA { "grey" } else { "white" };
    }
    let sector = HUE_SECTORS
        .iter()
        .rev()
        .find(|(start, _)| hue >= *start)
        .map_or("red", |(_, name)| *name);
    match sector {
        "red" | "orange" | "yellow" if value < BROWN_BELOW_VALUE => "brown",
        "orange" | "yellow" if saturation < TAN_BELOW_SATURATION => "tan",
        other => other,
    }
}

/// `flat` below 20, `noisy` below 60, `edgy` otherwise. NaN counts as flat.
pub fn texture_class(edge_density: f32) -> &'static str {
    if edge_density.is_nan() || edge_density < FLAT_BELOW {
        "flat"
    } else if edge_density < NOISY_BELOW {
        "noisy"
    } else {
        "edgy"
    }
}

/// The tile class for a DT1 orientation: 0 Floor; 1-9 Wall; 10-11 Special;
/// 12 Wall; 13 Shadow; 14 Tree; 15 Roof; 16-19 Lower wall. Anything else is
/// `Wall`, the least surprising bucket.
pub fn class_name(orientation: u8) -> &'static str {
    match orientation {
        0 => "Floor",
        1..=9 | 12 => "Wall",
        10 | 11 => "Special",
        13 => "Shadow",
        14 => "Tree",
        15 => "Roof",
        16..=19 => "Lower wall",
        _ => "Wall",
    }
}

/// The descriptor part of a tile name. A `dark` colour, or a tile whose mean
/// opaque `luma` is below [`DARK_BELOW`], is `dark`. Otherwise it is the
/// colour, plus the texture when the tile is not flat; a few pairs read
/// better as a word: green+noisy is `grass`, brown+noisy `dirt`, grey+edgy
/// `stone`, blue, cyan or teal + flat `water`.
pub fn descriptor(color: &str, texture: &str, luma: f32) -> String {
    if luma < DARK_BELOW || color == "dark" {
        return "dark".to_owned();
    }
    match (color, texture) {
        ("green", "noisy") => "grass".to_owned(),
        ("brown", "noisy") => "dirt".to_owned(),
        ("grey", "edgy") => "stone".to_owned(),
        ("blue" | "cyan" | "teal", "flat") => "water".to_owned(),
        (color, "flat") => color.to_owned(),
        (color, texture) => format!("{color} {texture}"),
    }
}

/// `<place> · <class> · <descriptor> · <main:02>/<sub:02>`, for example
/// `Act1 Town · Floor · grass · 05/02`.
pub fn generated_tile_name(
    place: &str,
    orientation: u8,
    descriptor: &str,
    main: u8,
    sub: u8,
) -> String {
    format!(
        "{place} · {} · {descriptor} · {main:02}/{sub:02}",
        class_name(orientation)
    )
}

/// `<place> · <kind>`, for example `Act1 Town Towne1 · DS1`.
pub fn generated_file_name(path: &str, kind: AssetKind) -> String {
    format!("{} · {}", place_from_path(path), kind.label())
}

fn title_case(segment: &str) -> String {
    let mut chars = segment.chars();
    match chars.next() {
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.extend(chars.flat_map(char::to_lowercase));
            out
        }
        None => String::new(),
    }
}

/// A readable place for a path.
///
/// `data\global\tiles\` is dropped when present (so is `data\global\`, then
/// `data\`), the extension is dropped, every segment is title-cased and the
/// segments are joined with spaces. A last segment that repeats the directory
/// before it is dropped:
/// `mxl\zone\whimsy\shire.dt1` becomes `Mxl Zone Whimsy Shire`,
/// `act1\town\floor.dt1` becomes `Act1 Town Floor`, `act1\town\town.dt1`
/// becomes `Act1 Town`.
pub fn place_from_path(path: &str) -> String {
    let normal = path.replace('/', "\\").to_ascii_lowercase();
    let mut rest = normal.trim_start_matches('\\');
    for prefix in [TILES_PREFIX, GLOBAL_PREFIX, DATA_PREFIX] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped;
            break;
        }
    }
    let mut segments: Vec<&str> = rest.split('\\').filter(|s| !s.is_empty()).collect();
    if let Some(last) = segments.last_mut()
        && let Some((stem, _ext)) = last.rsplit_once('.')
        && !stem.is_empty()
    {
        *last = stem;
    }
    if segments.len() >= 2 && segments[segments.len() - 1] == segments[segments.len() - 2] {
        segments.pop();
    }
    segments
        .iter()
        .map(|s| title_case(s))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn place_examples_from_the_contract() {
        assert_eq!(
            place_from_path("mxl\\zone\\whimsy\\shire.dt1"),
            "Mxl Zone Whimsy Shire"
        );
        assert_eq!(place_from_path("act1\\town\\floor.dt1"), "Act1 Town Floor");
        assert_eq!(
            place_from_path("data\\global\\tiles\\act1\\town\\floor.dt1"),
            "Act1 Town Floor"
        );
        assert_eq!(
            place_from_path("data\\global\\tiles\\act1\\town\\towne1.ds1"),
            "Act1 Town Towne1"
        );
    }

    #[test]
    fn place_edge_cases() {
        // The last segment repeats the directory.
        assert_eq!(place_from_path("act1\\town\\town.dt1"), "Act1 Town");
        // Slashes, case and leading separators are normalised.
        assert_eq!(place_from_path("/Act1/Town/Floor.DT1"), "Act1 Town Floor");
        // Other prefixes.
        assert_eq!(
            place_from_path("data\\global\\objects\\ba\\foo.dc6"),
            "Objects Ba Foo"
        );
        assert_eq!(place_from_path("data\\global\\excel\\x.bin"), "Excel X");
        // No directory, no extension, empty.
        assert_eq!(place_from_path("floor.dt1"), "Floor");
        assert_eq!(place_from_path("floor"), "Floor");
        assert_eq!(place_from_path(""), "");
        // A dot-file keeps its name.
        assert_eq!(place_from_path("a\\.hidden"), "A .hidden");
        // Only the final repeat is dropped.
        assert_eq!(place_from_path("a\\a\\a.dt1"), "A A");
    }

    #[test]
    fn class_from_orientation() {
        let expected = [
            (0, "Floor"),
            (1, "Wall"),
            (2, "Wall"),
            (3, "Wall"),
            (4, "Wall"),
            (5, "Wall"),
            (6, "Wall"),
            (7, "Wall"),
            (8, "Wall"),
            (9, "Wall"),
            (10, "Special"),
            (11, "Special"),
            (12, "Wall"),
            (13, "Shadow"),
            (14, "Tree"),
            (15, "Roof"),
            (16, "Lower wall"),
            (17, "Lower wall"),
            (18, "Lower wall"),
            (19, "Lower wall"),
            (20, "Wall"),
            (255, "Wall"),
        ];
        for (orientation, class) in expected {
            assert_eq!(class_name(orientation), class, "orientation {orientation}");
        }
    }

    /// An RGB colour of the given hue (degrees), saturation and value.
    fn from_hsv(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
        let c = value * saturation;
        let x = c * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = value - c;
        let (r, g, b) = match hue {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let to_u8 = |v: f32| ((v + m) * 255.0).round() as u8;
        [r, g, b].map(to_u8)
    }

    #[test]
    fn hue_sector_boundaries() {
        // Fully saturated, bright colours: the value/saturation exceptions
        // do not apply, so the name is the pure hue sector.
        let expected = [
            (0.0, "red"),
            (14.0, "red"),
            (16.0, "orange"),
            (44.0, "orange"),
            (46.0, "yellow"),
            (69.0, "yellow"),
            (71.0, "green"),
            (159.0, "green"),
            (161.0, "teal"),
            (184.0, "teal"),
            (186.0, "cyan"),
            (199.0, "cyan"),
            (201.0, "blue"),
            (254.0, "blue"),
            (256.0, "purple"),
            (289.0, "purple"),
            (291.0, "magenta"),
            (319.0, "magenta"),
            (321.0, "pink"),
            (344.0, "pink"),
            (346.0, "red"),
            (359.0, "red"),
        ];
        for (hue, name) in expected {
            let rgb = from_hsv(hue, 1.0, 1.0);
            assert_eq!(color_name(rgb), name, "hue {hue} = {rgb:?}");
        }
        assert_eq!(color_name([255, 0, 0]), "red");
        assert_eq!(color_name([255, 255, 0]), "yellow");
        assert_eq!(color_name([0, 255, 0]), "green");
        assert_eq!(color_name([0, 255, 255]), "teal", "hue 180");
        assert_eq!(color_name([0, 230, 255]), "cyan", "hue 188");
        assert_eq!(color_name([0, 0, 255]), "blue");
        assert_eq!(color_name([255, 0, 255]), "magenta");
        assert_eq!(color_name([255, 0, 128]), "pink");
    }

    #[test]
    fn dark_grey_and_white_boundaries() {
        // Luma 25 is the dark boundary (strictly below is dark).
        assert_eq!(color_name([24, 24, 24]), "dark");
        assert_eq!(color_name([25, 25, 25]), "grey");
        // Dark wins over any hue.
        assert_eq!(color_name([0, 0, 40]), "dark", "luma 4.6");
        assert_eq!(color_name([60, 0, 0]), "dark", "luma 17.9");
        assert_eq!(color_name([0, 0, 0]), "dark");
        // Saturation 0.15 is the grey boundary (strictly below is grey).
        assert_eq!(color_name([200, 169, 169]), "red", "s = 0.155");
        assert_eq!(
            color_name([200, 171, 171]),
            "white",
            "s < 0.15, luma >= 170"
        );
        assert_eq!(color_name([120, 103, 103]), "grey", "s = 0.142, luma < 170");
        // Luma 170 is the white boundary (grey below, white from 170).
        assert_eq!(color_name([169, 169, 169]), "grey");
        assert_eq!(color_name([170, 170, 170]), "white");
        assert_eq!(color_name([255, 255, 255]), "white");
        assert_eq!(color_name([128, 128, 128]), "grey");
    }

    #[test]
    fn brown_and_tan_boundaries() {
        // Value 0.55 is 140.25 of 255: 140 is below, 141 is not.
        assert_eq!(color_name([140, 70, 0]), "brown", "orange hue, v < 0.55");
        assert_eq!(color_name([141, 70, 0]), "orange", "orange hue, v >= 0.55");
        assert_eq!(color_name([140, 0, 0]), "brown", "red, v < 0.55");
        assert_eq!(color_name([141, 0, 0]), "red");
        assert_eq!(color_name([140, 140, 0]), "brown", "yellow, v < 0.55");
        assert_eq!(color_name([141, 141, 0]), "yellow");
        // Other hues are never brown.
        assert_eq!(color_name([0, 100, 0]), "green");
        assert_eq!(color_name([40, 40, 200]), "blue");
        // Tan: orange or yellow, s < 0.35, v >= 0.55. At v = 200, s = 0.35 is min = 130.
        assert_eq!(color_name([200, 160, 131]), "tan", "orange hue, s = 0.345");
        assert_eq!(
            color_name([200, 160, 129]),
            "orange",
            "orange hue, s = 0.355"
        );
        assert_eq!(color_name([200, 200, 131]), "tan", "yellow hue, s = 0.345");
        assert_eq!(
            color_name([200, 200, 129]),
            "yellow",
            "yellow hue, s = 0.355"
        );
        // Dull and dark is brown, not tan.
        assert_eq!(color_name([100, 85, 70]), "brown");
        // Red is never tan, however pale.
        assert_eq!(color_name([200, 140, 140]), "red");
        // Grey takes precedence over tan.
        assert_eq!(color_name([200, 195, 190]), "white");
    }

    #[test]
    fn texture_thresholds() {
        assert_eq!(texture_class(0.0), "flat");
        assert_eq!(texture_class(19.99), "flat");
        assert_eq!(texture_class(20.0), "noisy");
        assert_eq!(texture_class(59.99), "noisy");
        assert_eq!(texture_class(60.0), "edgy");
        assert_eq!(texture_class(500.0), "edgy");
        assert_eq!(texture_class(f32::NAN), "flat");
    }

    #[test]
    fn descriptors() {
        let mid = 80.0;
        assert_eq!(descriptor("green", "noisy", mid), "grass");
        assert_eq!(descriptor("brown", "noisy", mid), "dirt");
        assert_eq!(descriptor("grey", "edgy", mid), "stone");
        assert_eq!(descriptor("blue", "flat", mid), "water");
        assert_eq!(descriptor("cyan", "flat", mid), "water");
        assert_eq!(descriptor("teal", "flat", mid), "water");
        assert_eq!(descriptor("cyan", "noisy", mid), "cyan noisy");
        assert_eq!(descriptor("dark", "edgy", mid), "dark");
        assert_eq!(descriptor("red", "flat", mid), "red");
        assert_eq!(descriptor("red", "noisy", mid), "red noisy");
        assert_eq!(descriptor("green", "flat", mid), "green");
        assert_eq!(descriptor("green", "edgy", mid), "green edgy");
        assert_eq!(descriptor("grey", "noisy", mid), "grey noisy");
        // Dark wins over every colour and texture.
        assert_eq!(descriptor("grey", "flat", 24.9), "dark");
        assert_eq!(descriptor("green", "noisy", 10.0), "dark");
        assert_eq!(descriptor("brown", "edgy", 0.0), "dark");
        assert_eq!(descriptor("brown", "edgy", 25.0), "brown edgy");
        assert!((luma([255, 255, 255]) - 255.0).abs() < 0.01);
    }

    #[test]
    fn full_names() {
        assert_eq!(
            generated_tile_name("Act1 Town", 0, "grass", 5, 2),
            "Act1 Town · Floor · grass · 05/02"
        );
        assert_eq!(
            generated_tile_name("Mxl Zone Whimsy Shire", 13, "dark", 12, 0),
            "Mxl Zone Whimsy Shire · Shadow · dark · 12/00"
        );
        assert_eq!(
            generated_file_name(
                "data\\global\\tiles\\act1\\town\\towne1.ds1",
                AssetKind::Ds1
            ),
            "Act1 Town Towne1 · DS1"
        );
    }
}
