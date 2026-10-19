//! Deterministic generated names. See `docs/api/capy-index.md`.
//!
//! Everything here is a pure function of its inputs (a path, an orientation,
//! a mean colour, an edge-density number), so the rules are unit-testable
//! without game files.

use crate::query::AssetKind;

const TILES_PREFIX: &str = "data\\global\\tiles\\";
const GLOBAL_PREFIX: &str = "data\\global\\";
const DATA_PREFIX: &str = "data\\";

/// The fixed 16-colour palette used to name a tile's dominant colour.
const COLORS: [(&str, [u8; 3]); 16] = [
    ("black", [20, 20, 20]),
    ("grey", [128, 128, 128]),
    ("white", [240, 240, 240]),
    ("red", [192, 32, 32]),
    ("orange", [230, 130, 30]),
    ("brown", [130, 70, 20]),
    ("yellow", [230, 220, 50]),
    ("olive", [128, 128, 0]),
    ("green", [50, 150, 50]),
    ("teal", [30, 130, 130]),
    ("cyan", [60, 220, 220]),
    ("blue", [40, 70, 200]),
    ("purple", [120, 50, 160]),
    ("magenta", [220, 40, 200]),
    ("pink", [240, 150, 180]),
    ("tan", [200, 170, 120]),
];

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

/// Nearest palette colour (squared RGB distance; ties go to the earlier
/// palette entry) to the mean opaque colour of a tile.
pub fn color_name(rgb: [u8; 3]) -> &'static str {
    let mut best = COLORS[0].0;
    let mut best_d = i64::MAX;
    for (name, c) in COLORS {
        let d: i64 = (0..3)
            .map(|i| {
                let diff = i64::from(rgb[i]) - i64::from(c[i]);
                diff * diff
            })
            .sum();
        if d < best_d {
            best_d = d;
            best = name;
        }
    }
    best
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

/// The descriptor part of a tile name. A tile whose mean opaque `luma` is
/// below [`DARK_BELOW`] is `dark`, whatever its colour name. Otherwise it is
/// the colour, plus the texture when the tile is not flat; a few pairs read
/// better as a word: green+noisy is `grass`, brown+noisy `dirt`, grey+edgy
/// `stone`, blue+flat `water`.
pub fn descriptor(color: &str, texture: &str, luma: f32) -> String {
    if luma < DARK_BELOW {
        return "dark".to_owned();
    }
    match (color, texture) {
        ("green", "noisy") => "grass".to_owned(),
        ("brown", "noisy") => "dirt".to_owned(),
        ("grey", "edgy") => "stone".to_owned(),
        ("blue", "flat") => "water".to_owned(),
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

    #[test]
    fn colour_nearest_name() {
        for (name, rgb) in COLORS {
            assert_eq!(color_name(rgb), name, "palette entry {name}");
        }
        assert_eq!(color_name([10, 10, 10]), "black");
        assert_eq!(color_name([250, 250, 250]), "white");
        assert_eq!(color_name([60, 140, 60]), "green");
        assert_eq!(color_name([100, 70, 40]), "brown");
        assert_eq!(color_name([130, 125, 135]), "grey");
        assert_eq!(color_name([30, 30, 190]), "blue");
        assert_eq!(color_name([40, 40, 40]), "black");
        assert_eq!(color_name([90, 90, 90]), "grey");
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
        assert_eq!(descriptor("red", "flat", mid), "red");
        assert_eq!(descriptor("red", "noisy", mid), "red noisy");
        assert_eq!(descriptor("green", "flat", mid), "green");
        assert_eq!(descriptor("green", "edgy", mid), "green edgy");
        assert_eq!(descriptor("grey", "noisy", mid), "grey noisy");
        // Black is a colour name like any other once the tile is not dark.
        assert_eq!(descriptor("black", "flat", 30.0), "black");
        // Dark wins over every colour and texture.
        assert_eq!(descriptor("black", "flat", 24.9), "dark");
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
