//! How the automap is drawn over a map and in the automap editor: each
//! category of tile (walls, walkable floors, water, roofs, objects, shadows)
//! and some parts of them (left walls, trees...) has its own colour and
//! opacity and can be shown or not; pieces can be thicker; the map under them
//! can be dimmed. This module holds the settings and the classification rules;
//! painting is the renderer's job.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Category of a tile for drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Walls, corners, doors, wall ends, lower walls.
    Walls,
    /// Walkable floors.
    Floors,
    /// Unwalkable dark or bluish floors.
    Water,
    /// Roof tiles.
    Roofs,
    /// Columns, props, trees.
    Objects,
    /// Shadow tiles.
    Shadows,
}

impl Kind {
    /// All kinds in display order.
    pub const ALL: [Kind; 6] = [
        Kind::Walls,
        Kind::Floors,
        Kind::Water,
        Kind::Roofs,
        Kind::Objects,
        Kind::Shadows,
    ];

    /// Short label.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Walls => "Walls",
            Kind::Floors => "Walkable floors",
            Kind::Water => "Water",
            Kind::Roofs => "Roofs",
            Kind::Objects => "Objects & trees",
            Kind::Shadows => "Shadows",
        }
    }
}

/// The category of a tile code; floors are water or walkable by tile.
pub fn kind_of_code(code: &str, water: bool) -> Kind {
    match code {
        "fl" if water => Kind::Water,
        "fl" => Kind::Floors,
        "co" | "tr" => Kind::Objects,
        "rf" => Kind::Roofs,
        "sh" => Kind::Shadows,
        _ => Kind::Walls,
    }
}

/// A part of a category that can have its own look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    /// Stable id used as the key in [`AutomapStyle::parts`].
    pub id: &'static str,
    /// Category it belongs to.
    pub kind: Kind,
    /// Label.
    pub label: &'static str,
    /// Tile codes in it.
    pub codes: &'static [&'static str],
}

/// Walls by shape, objects by kind.
pub const PARTS: [Part; 8] = [
    Part {
        id: "left",
        kind: Kind::Walls,
        label: "Left walls",
        codes: &["wl"],
    },
    Part {
        id: "right",
        kind: Kind::Walls,
        label: "Right walls",
        codes: &["wr"],
    },
    Part {
        id: "corners",
        kind: Kind::Walls,
        label: "Corners",
        codes: &["wtlr", "wtll", "wtr", "wbl", "wbr"],
    },
    Part {
        id: "doors",
        kind: Kind::Walls,
        label: "Doors",
        codes: &["wld", "wrd"],
    },
    Part {
        id: "ends",
        kind: Kind::Walls,
        label: "Wall ends",
        codes: &["wle", "wre"],
    },
    Part {
        id: "lower",
        kind: Kind::Walls,
        label: "Lower walls",
        codes: &["ld", "lr", "lf", "ls"],
    },
    Part {
        id: "trees",
        kind: Kind::Objects,
        label: "Trees",
        codes: &["tr"],
    },
    Part {
        id: "props",
        kind: Kind::Objects,
        label: "Columns & props",
        codes: &["co"],
    },
];

/// The part a tile code belongs to, if its category has parts.
pub fn part_of_code(code: &str) -> Option<&'static str> {
    PARTS.iter().find(|p| p.codes.contains(&code)).map(|p| p.id)
}

/// How one category or part is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Look {
    /// Whether it is drawn.
    pub show: bool,
    /// `#rrggbb`.
    pub colour: String,
    /// 0.1 to 1.
    pub opacity: f64,
}

/// Whether pieces use the game's palette colours or each category's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Colours {
    /// Colour by category.
    Kind,
    /// The game's own colours.
    Game,
}

/// The full automap drawing style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomapStyle {
    /// Palette or category colours.
    pub colours: Colours,
    /// Looks per category, in [`Kind::ALL`] order.
    pub kinds: Vec<(Kind, Look)>,
    /// Looks of parts drawn their own way, by part id.
    pub parts: Vec<(String, Look)>,
    /// Extra pixels around each piece pixel (0-3).
    pub thickness: u8,
    /// How dark the map under the automap is (0-0.95).
    pub dim: f64,
    /// Outline walls that have no AutoMap.txt entry.
    pub missing: bool,
    /// Colour of that outline.
    pub missing_colour: String,
}

fn look(show: bool, colour: &str, opacity: f64) -> Look {
    Look {
        show,
        colour: colour.to_string(),
        opacity,
    }
}

impl Default for AutomapStyle {
    fn default() -> Self {
        AutomapStyle {
            colours: Colours::Game,
            kinds: vec![
                (Kind::Walls, look(true, "#ffd24a", 1.0)),
                (Kind::Floors, look(true, "#b8c4d0", 0.8)),
                (Kind::Water, look(true, "#3d8bff", 1.0)),
                (Kind::Roofs, look(true, "#e0664f", 1.0)),
                (Kind::Objects, look(true, "#7ee081", 1.0)),
                (Kind::Shadows, look(true, "#9a8fc0", 0.7)),
            ],
            parts: Vec::new(),
            thickness: 1,
            dim: 0.72,
            missing: true,
            missing_colour: "#ff5ac8".to_string(),
        }
    }
}

impl AutomapStyle {
    /// The look of a category.
    pub fn kind_look(&self, kind: Kind) -> &Look {
        self.kinds
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, l)| l)
            .unwrap_or(&self.kinds[0].1)
    }

    /// How a piece is drawn: its part's own look if it has one, else its
    /// category's.
    pub fn look_of(&self, kind: Kind, part: Option<&str>) -> &Look {
        part.and_then(|p| self.parts.iter().find(|(id, _)| id == p))
            .map_or_else(|| self.kind_look(kind), |(_, l)| l)
    }
}

fn colour(v: Option<&Value>, default: &str) -> String {
    match v.and_then(Value::as_str) {
        Some(s)
            if s.len() == 7
                && s.starts_with('#')
                && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            s.to_string()
        }
        _ => default.to_string(),
    }
}

fn number(v: Option<&Value>, lo: f64, hi: f64, default: f64) -> f64 {
    match v.and_then(Value::as_f64) {
        Some(n) if n.is_finite() => n.clamp(lo, hi),
        _ => default,
    }
}

/// A stored style made whole: missing or bad fields take the defaults, so a
/// style saved by an older version carries over (one opacity for everything,
/// one "other" kind for roofs and shadows).
pub fn normalize(stored: &Value) -> AutomapStyle {
    let d = AutomapStyle::default();
    let kinds_in = stored.get("kinds");
    let kind_key = |k: Kind| match k {
        Kind::Walls => "walls",
        Kind::Floors => "floors",
        Kind::Water => "water",
        Kind::Roofs => "roofs",
        Kind::Objects => "objects",
        Kind::Shadows => "shadows",
    };
    let old_opacity = stored.get("opacity");
    let kinds: Vec<(Kind, Look)> = Kind::ALL
        .iter()
        .map(|&k| {
            let own = kinds_in.and_then(|m| m.get(kind_key(k)));
            let v = own.or_else(|| {
                matches!(k, Kind::Roofs | Kind::Shadows)
                    .then(|| kinds_in.and_then(|m| m.get("other")))
                    .flatten()
            });
            let base = d.kind_look(k);
            let l = Look {
                show: v
                    .and_then(|v| v.get("show"))
                    .and_then(Value::as_bool)
                    .unwrap_or(base.show),
                colour: colour(own.and_then(|v| v.get("colour")), &base.colour),
                opacity: number(
                    v.and_then(|v| v.get("opacity")).or(old_opacity),
                    0.1,
                    1.0,
                    base.opacity,
                ),
            };
            (k, l)
        })
        .collect();
    let mut parts = Vec::new();
    for p in PARTS {
        let Some(v) = stored
            .get("parts")
            .and_then(|m| m.get(p.id))
            .filter(|v| v.is_object())
        else {
            continue;
        };
        let base = kinds
            .iter()
            .find(|(k, _)| *k == p.kind)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| look(true, "#ffffff", 1.0));
        parts.push((
            p.id.to_string(),
            Look {
                show: v.get("show").and_then(Value::as_bool).unwrap_or(base.show),
                colour: colour(v.get("colour"), &base.colour),
                opacity: number(v.get("opacity"), 0.1, 1.0, base.opacity),
            },
        ));
    }
    AutomapStyle {
        colours: Colours::Game,
        kinds,
        parts,
        thickness: number(stored.get("thickness"), 0.0, 3.0, f64::from(d.thickness)).round() as u8,
        dim: number(stored.get("dim"), 0.0, 0.95, d.dim),
        missing: stored
            .get("missing")
            .and_then(Value::as_bool)
            .unwrap_or(d.missing),
        missing_colour: colour(stored.get("missingColour"), &d.missing_colour),
    }
}

/// Whether a floor's average colour looks like water: clearly bluer than it
/// is red (rivers, lakes), or blue-green (Act 3's swamps).
pub fn looks_like_water([r, g, b]: [i32; 3]) -> bool {
    let blue = b >= r + 12 && b >= g - 6;
    let teal = g >= r + 12 && b >= r + 12 && (g - b).abs() <= 25;
    blue || teal
}

/// Whether a floor tile is water: nobody can walk on any of it (every
/// sub-tile blocks walking) and its colour is not that of ground or lava.
/// Without a colour every unwalkable floor counts.
pub fn is_water_tile(subtile_flags: &[u8], rgb: Option<[i32; 3]>) -> bool {
    if subtile_flags.len() != 25 || !subtile_flags.iter().all(|v| v & 1 != 0) {
        return false;
    }
    let Some(rgb) = rgb else { return true };
    let [r, g, b] = rgb;
    looks_like_water(rgb) || r.max(g).max(b) < 28 || b >= r - 2
}

/// Size of a map's automap image and where cell (0, 0)'s north corner is.
pub fn frame(width: u32, height: u32) -> (i32, i32, u32, u32) {
    (
        height as i32 * 8 + 16,
        40,
        (width + height) * 8 + 32,
        (width + height) * 4 + 56,
    )
}

/// Automap-pixel position of a cell's cel origin relative to the origin.
pub fn cell_origin(x: i32, y: i32) -> (i32, i32) {
    ((x - y) * 8, (x + y) * 4)
}
