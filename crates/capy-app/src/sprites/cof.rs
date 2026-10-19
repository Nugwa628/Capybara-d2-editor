//! COF viewing: which token/mode/class a COF is, the component sprite of
//! each layer (through `capy_sprite::cof_component_paths`), the armour
//! variants on offer, and the composite of one frame.
//!
//! A layer whose sprite cannot be found or decoded is listed as missing with
//! the reason; it never fails the whole COF.

use d2_formats::cof::COMPONENT_CODES;
use capy_sprite::{CofLayerSprite, RgbaFrame, Sprite, SpriteFormat, UnitKind, compose_cof_frame};
use d2_assets::{GamePath, Overlay};
use d2_formats::cof::Cof;
use d2_formats::palette::Palette;

/// Token, mode and weapon class of a COF (lower case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CofName {
    /// `None` for a loose file outside a `data\global\<kind>\` tree.
    pub kind: Option<UnitKind>,
    pub token: String,
    pub mode: String,
    pub class: String,
}

impl CofName {
    /// The AnimData key (`{token}{mode}{class}`).
    pub fn anim_key(&self) -> String {
        format!("{}{}{}", self.token, self.mode, self.class)
    }

    /// The COF's own game path for `kind`.
    pub fn game_path(&self, kind: UnitKind) -> GamePath {
        let t = &self.token;
        GamePath::new(&format!(
            "data\\global\\{}\\{t}\\cof\\{t}{}{}.cof",
            folder(kind),
            self.mode,
            self.class
        ))
    }
}

pub fn folder(kind: UnitKind) -> &'static str {
    match kind {
        UnitKind::Object => "objects",
        UnitKind::Monster => "monsters",
        UnitKind::Character => "chars",
    }
}

fn kind_of_folder(name: &str) -> Option<UnitKind> {
    match name.to_ascii_lowercase().as_str() {
        "objects" => Some(UnitKind::Object),
        "monsters" => Some(UnitKind::Monster),
        "chars" => Some(UnitKind::Character),
        _ => None,
    }
}

/// Splits a COF path (game path or file path, either slash) into its name
/// parts. Inside `...\{objects|monsters|chars}\{token}\cof\` the token is
/// the folder name; elsewhere it is taken as the first two letters of the
/// file name (every shipped token is two letters). Mode is two letters,
/// the class is the rest.
pub fn parse_cof_path(path: &str) -> Option<CofName> {
    let lower = path.to_ascii_lowercase().replace('/', "\\");
    let parts: Vec<&str> = lower.split('\\').filter(|s| !s.is_empty()).collect();
    let file = parts.last()?;
    let stem = file.strip_suffix(".cof")?;
    let n = parts.len();
    let (kind, token) = if n >= 4 && parts[n - 2] == "cof" {
        (kind_of_folder(parts[n - 4]), parts[n - 3].to_owned())
    } else {
        (None, stem.get(..2)?.to_owned())
    };
    let rest = stem.strip_prefix(token.as_str())?;
    if rest.len() < 3 || !rest.is_ascii() {
        return None;
    }
    Some(CofName {
        kind,
        token,
        mode: rest[..2].to_owned(),
        class: rest[2..].to_owned(),
    })
}

/// Every (mode, class) with a COF for `kind`/`token` among `names` (sorted).
pub fn variants(names: &[GamePath], kind: UnitKind, token: &str) -> Vec<(String, String)> {
    let prefix = format!("data\\global\\{}\\{token}\\cof\\", folder(kind));
    let mut out: Vec<(String, String)> = prefixed(names, &prefix)
        .filter_map(|p| parse_cof_path(p.as_str()))
        .filter(|c| c.token == token)
        .map(|c| (c.mode, c.class))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The names starting with `prefix` (the list is sorted).
fn prefixed<'a>(names: &'a [GamePath], prefix: &'a str) -> impl Iterator<Item = &'a GamePath> {
    let start = names.partition_point(|p| p.as_str() < prefix);
    names[start..]
        .iter()
        .take_while(move |p| p.as_str().starts_with(prefix))
}

/// Armour tokens that have a sprite file for this layer:
/// `{token}{cc}{armor}{mode}{class}.dc?` in the component folder.
pub fn armors(
    names: &[GamePath],
    kind: UnitKind,
    token: &str,
    code: &str,
    mode: &str,
    class: &str,
) -> Vec<String> {
    let cc = code.to_ascii_lowercase();
    let dir = format!("data\\global\\{}\\{token}\\{cc}\\", folder(kind));
    let head = format!("{token}{cc}");
    let tail = format!("{mode}{class}");
    let mut out: Vec<String> = prefixed(names, &dir)
        .filter_map(|p| {
            let file = p.as_str().strip_prefix(dir.as_str())?;
            let stem = file
                .strip_suffix(".dcc")
                .or_else(|| file.strip_suffix(".dc6"))?;
            let mid = stem.strip_prefix(head.as_str())?.strip_suffix(tail.as_str())?;
            (!mid.is_empty() && !mid.contains('\\')).then(|| mid.to_owned())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[derive(Clone, Debug)]
pub struct CofLayerView {
    pub component: u8,
    pub code: &'static str,
    pub weapon_class: String,
    pub armor: String,
    pub armors: Vec<String>,
    /// The file that was loaded.
    pub path: Option<GamePath>,
    pub sprite: Option<Sprite>,
    /// Why there is no sprite.
    pub problem: Option<String>,
    pub visible: bool,
    pub draw_effect: u8,
    pub transparent: bool,
}

pub struct CofView {
    pub name: CofName,
    /// The kind the layers were resolved with.
    pub kind: UnitKind,
    pub path: Option<GamePath>,
    pub cof: Cof,
    pub layers: Vec<CofLayerView>,
    /// Other (mode, class) pairs of the token.
    pub variants: Vec<(String, String)>,
    compose: Vec<CofLayerSprite>,
}

fn clean_class(raw: &[u8; 4]) -> String {
    String::from_utf8_lossy(raw)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_ascii_lowercase()
}

fn read(overlay: Option<&Overlay>, path: &str) -> Option<Vec<u8>> {
    overlay?.read_bytes(&GamePath::new(path)).ok().flatten()
}

impl CofView {
    /// Resolves every layer. `prefer` maps component codes (`"TR"`) to the
    /// armour wanted (e.g. an object's obj.txt skins); otherwise `lit`, else
    /// the first variant on offer.
    pub fn load(
        cof: Cof,
        name: CofName,
        path: Option<GamePath>,
        overlay: Option<&Overlay>,
        names: &[GamePath],
        prefer: &[(String, String)],
    ) -> CofView {
        // A loose COF: use the first kind whose folders know the token.
        let kind = name.kind.unwrap_or_else(|| {
            [UnitKind::Monster, UnitKind::Object, UnitKind::Character]
                .into_iter()
                .find(|&k| !variants(names, k, &name.token).is_empty())
                .unwrap_or(UnitKind::Monster)
        });
        // The last record of a component wins; keep COF order otherwise.
        let mut records = Vec::new();
        for (i, r) in cof.layers.iter().enumerate() {
            if usize::from(r.component) < COMPONENT_CODES.len()
                && !cof.layers[i + 1..].iter().any(|l| l.component == r.component)
            {
                records.push(r.clone());
            }
        }
        let layers = records
            .iter()
            .map(|r| {
                let code = COMPONENT_CODES[usize::from(r.component)];
                let weapon_class = clean_class(&r.weapon_class);
                let armors = armors(names, kind, &name.token, code, &name.mode, &weapon_class);
                let wanted = prefer
                    .iter()
                    .find(|(c, _)| c.eq_ignore_ascii_case(code))
                    .map(|(_, a)| a.to_ascii_lowercase());
                let armor = wanted
                    .or_else(|| armors.iter().find(|a| *a == "lit").cloned())
                    .or_else(|| armors.first().cloned())
                    .unwrap_or_else(|| "lit".to_owned());
                let mut layer = CofLayerView {
                    component: r.component,
                    code,
                    weapon_class,
                    armor,
                    armors,
                    path: None,
                    sprite: None,
                    problem: None,
                    visible: true,
                    draw_effect: r.draw_effect,
                    transparent: r.transparent != 0,
                };
                resolve_layer(&mut layer, kind, &name, overlay);
                layer
            })
            .collect();
        let variants = variants(names, kind, &name.token);
        let mut view = CofView {
            name,
            kind,
            path,
            cof,
            layers,
            variants,
            compose: Vec::new(),
        };
        view.rebuild();
        view
    }

    /// Picks another armour for layer `i` and reloads its sprite.
    pub fn set_armor(&mut self, i: usize, armor: &str, overlay: Option<&Overlay>) {
        let (kind, name) = (self.kind, self.name.clone());
        if let Some(layer) = self.layers.get_mut(i) {
            layer.armor = armor.to_owned();
            resolve_layer(layer, kind, &name, overlay);
        }
        self.rebuild();
    }

    pub fn set_visible(&mut self, i: usize, visible: bool) {
        if let Some(layer) = self.layers.get_mut(i) {
            layer.visible = visible;
        }
        self.rebuild();
    }

    /// Refreshes the layer list handed to the compositor.
    fn rebuild(&mut self) {
        self.compose = self
            .layers
            .iter()
            .filter(|l| l.visible)
            .filter_map(|l| {
                Some(CofLayerSprite {
                    component: l.component,
                    sprite: l.sprite.clone()?,
                    draw_effect: l.draw_effect,
                    transparent: l.transparent,
                })
            })
            .collect();
    }

    pub fn missing(&self) -> usize {
        self.layers.iter().filter(|l| l.sprite.is_none()).count()
    }

    pub fn directions(&self) -> usize {
        usize::from(self.cof.directions)
    }

    pub fn frames(&self) -> usize {
        usize::from(self.cof.frames_per_direction)
    }

    /// Composite of one (file direction, frame) of the visible layers.
    pub fn compose(&self, dir: usize, frame: usize, palette: &Palette) -> Result<RgbaFrame, String> {
        let (Ok(d), Ok(f)) = (u8::try_from(dir), u8::try_from(frame)) else {
            return Err("direction or frame out of range".to_owned());
        };
        compose_cof_frame(&self.cof, &self.compose, d, f, palette).map_err(|e| e.to_string())
    }

    /// Union box of the visible layers' sprites (anchor space).
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.compose
            .iter()
            .filter_map(|l| l.sprite.bounds())
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
    }
}

/// Loads the sprite of one layer: `.dcc` first, then `.dc6`.
fn resolve_layer(layer: &mut CofLayerView, kind: UnitKind, name: &CofName, overlay: Option<&Overlay>) {
    layer.sprite = None;
    layer.path = None;
    let candidates = capy_sprite::cof_component_paths(
        kind,
        &name.token,
        &name.mode,
        &layer.weapon_class,
        layer.code,
        &layer.armor,
    );
    for candidate in &candidates {
        let Some(bytes) = read(overlay, candidate) else {
            continue;
        };
        let format = if candidate.ends_with(".dc6") {
            SpriteFormat::Dc6
        } else {
            SpriteFormat::Dcc
        };
        match Sprite::decode(&bytes, format) {
            Ok(sprite) => {
                layer.sprite = Some(sprite);
                layer.path = Some(GamePath::new(candidate));
                layer.problem = None;
            }
            // No fallback to .dc6 when the .dcc exists but is broken.
            Err(e) => {
                layer.problem = Some(format!("cannot decode {candidate}: {e}"));
                layer.path = Some(GamePath::new(candidate));
            }
        }
        return;
    }
    layer.problem = Some(format!(
        "missing: {}",
        candidates.first().map_or("", String::as_str)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cof_paths_split_into_token_mode_class() {
        let c = parse_cof_path("data\\global\\monsters\\zm\\cof\\zmnuhth.cof").expect("parses");
        assert_eq!(c.kind, Some(UnitKind::Monster));
        assert_eq!((c.token.as_str(), c.mode.as_str(), c.class.as_str()), ("zm", "nu", "hth"));
        assert_eq!(c.anim_key(), "zmnuhth");
        assert_eq!(
            c.game_path(UnitKind::Monster).as_str(),
            "data\\global\\monsters\\zm\\cof\\zmnuhth.cof"
        );
        let loose = parse_cof_path("C:/mods/AMA1HS.COF").expect("loose");
        assert_eq!(loose.kind, None);
        assert_eq!((loose.token.as_str(), loose.mode.as_str(), loose.class.as_str()), ("am", "a1", "hs"));
        assert!(parse_cof_path("data\\x.dcc").is_none());
        assert!(parse_cof_path("ab.cof").is_none(), "too short");
    }

    fn names(list: &[&str]) -> Vec<GamePath> {
        let mut v: Vec<GamePath> = list.iter().map(|s| GamePath::new(s)).collect();
        v.sort();
        v
    }

    #[test]
    fn variants_and_armors_come_from_known_names() {
        let n = names(&[
            "data\\global\\monsters\\zm\\cof\\zmnuhth.cof",
            "data\\global\\monsters\\zm\\cof\\zma1hth.cof",
            "data\\global\\monsters\\zm\\tr\\zmtrlitnuhth.dcc",
            "data\\global\\monsters\\zm\\tr\\zmtrmednuhth.dc6",
            "data\\global\\monsters\\zm\\tr\\zmtrlita1hth.dcc",
            "data\\global\\monsters\\zz\\cof\\zznuhth.cof",
        ]);
        assert_eq!(
            variants(&n, UnitKind::Monster, "zm"),
            vec![("a1".to_owned(), "hth".to_owned()), ("nu".to_owned(), "hth".to_owned())]
        );
        assert_eq!(
            armors(&n, UnitKind::Monster, "zm", "TR", "nu", "hth"),
            vec!["lit".to_owned(), "med".to_owned()]
        );
        assert!(armors(&n, UnitKind::Monster, "zm", "HD", "nu", "hth").is_empty());
    }

    fn cof_bytes() -> Vec<u8> {
        // 2 layers (TR, HD), 1 direction, 1 frame.
        let mut b = vec![2u8, 1, 1, 0];
        b.extend([0u8; 24]);
        b.extend([1, 0, 1, 0, 0, b'h', b't', b'h', 0]);
        b.extend([0, 0, 1, 0, 0, b'h', b't', b'h', 0]);
        b.push(0); // keyframe
        b.extend([1, 0]); // priority
        b
    }

    #[test]
    fn missing_layers_are_listed_not_fatal() {
        let cof = Cof::parse(&cof_bytes()).expect("cof");
        let name = parse_cof_path("data\\global\\monsters\\zm\\cof\\zmnuhth.cof").expect("name");
        let view = CofView::load(cof, name, None, None, &[], &[]);
        assert_eq!(view.layers.len(), 2);
        assert_eq!(view.missing(), 2);
        assert!(
            view.layers[0]
                .problem
                .as_deref()
                .is_some_and(|p| p.contains("zmtrlitnuhth.dcc")),
            "{:?}",
            view.layers[0].problem
        );
        let frame = view
            .compose(0, 0, &super::super::palette::grey_palette())
            .expect("an empty composite");
        assert_eq!((frame.width, frame.height), (0, 0));
        assert!(view.compose(3, 0, &super::super::palette::grey_palette()).is_err());
    }
}
