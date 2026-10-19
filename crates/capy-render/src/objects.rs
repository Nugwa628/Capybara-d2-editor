//! Object and monster sprites: from a DS1 object record to the layers of one
//! animation frame.
//!
//! Citations are `reference/d2-ds1-edit/src/` (DS1Edit 2.1.1) and
//! `docs/specs/cof.md`.
//!
//! # Pipeline
//!
//! 1. [`ObjectCatalog::resolve`] turns `(type, id)` into an [`ObjectInfo`].
//! 2. The COF is `{base}\{token}\cof\{token}{mode}{class}.cof` (`cof.c:253`).
//! 3. Every COF layer is a component sprite
//!    `{base}\{token}\{CC}\{token}{CC}{skin}{mode}{class}.dcc` with the layer's
//!    own weapon class (`cof.c:322-333`, [`d2_formats::cof::component_paths`]);
//!    `.dc6` is tried only when no `.dcc` exists. The skin comes from
//!    [`ObjectInfo::skins`] (default `LIT`). When a COF has several records
//!    of one component the last one wins ([`Cof::component_layer`],
//!    `cof.c:306`).
//! 4. One user direction (default 0) is mapped through the editor's
//!    direction tables (`main.c:331-334`) to the file direction, which indexes
//!    both the sprite and the COF priority table (lead decision, PLAN.md).
//! 5. The frame is chosen by [`frame_index`]; layers are listed in COF priority
//!    order (`preview.c:2095`), each with its blend (`preview.c:2131`) and, for
//!    layers with `shadow != 0`, its shadow darkness (`preview.c:2235-2259`).
//!
//! The bitmap of a layer in the reference is the whole direction box (the
//! union of the frame boxes), anchored at the box's top-left corner
//! (`cof.c:117-122`, `dc6.c:193-260`); [`FrameLayer::box_origin`] is that
//! corner, which keeps point-sampled zooms in step with the oracle.
//!
//! `capy-sprite::compose_cof_frame` is not used here: the reference draws
//! each layer as its own scaled bitmap with separately truncated offsets,
//! which a pre-composited frame cannot reproduce at zoom. The rules both
//! share live in `d2-formats`: the sprite paths and the last-record rule in
//! [`d2_formats::cof`], the `trans_a`/`trans_b` blend mapping in
//! [`Blend::for_cof_layer`]. [`ObjectFrame::to_rgba`] gives the same 1:1
//! composite `compose_cof_frame` does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use d2_assets::objects::{ObjectCatalog, ObjectInfo, SpriteBase};
use d2_assets::{GamePath, Overlay};
use d2_formats::IndexedImage;
use d2_formats::animdata::AnimData;
use d2_formats::cof::{Cof, cof_path, component_paths, path_token};
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::{Ds1, Object};
use d2_formats::palette::Palette;

use crate::compose::Blend;
use crate::tileset::{TILE_H, TILE_W};

/// File direction of user direction `n` for COFs with 8, 16 and 32
/// directions (`main.c:331-340`); 1 and 4 direction COFs use the user
/// direction itself.
const DIR8: [u8; 8] = [4, 0, 5, 1, 6, 2, 7, 3];
const DIR16: [u8; 16] = [4, 8, 0, 9, 5, 10, 1, 11, 6, 12, 2, 13, 7, 14, 3, 15];
const DIR32: [u8; 32] = [
    4, 16, 8, 17, 0, 18, 9, 19, 5, 20, 10, 21, 1, 22, 11, 23, 6, 24, 12, 25, 2, 26, 13, 27, 7, 28,
    14, 29, 3, 30, 15, 31,
];

/// Sub-tile drawing order inside one tile, `obj_sub_tile` (`main.c:342-347`),
/// indexed `[sy][sx]` as the comparator does (`objects.c:36`).
const SUB_TILE_ORDER: [[u8; 5]; 5] = [
    [0, 2, 5, 9, 14],
    [1, 4, 8, 13, 18],
    [3, 7, 12, 17, 21],
    [6, 11, 16, 20, 23],
    [10, 15, 19, 22, 24],
];

/// Why an object produced no sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissingReason {
    /// The catalog could not resolve `(type, id)` to a row.
    Unresolved,
    /// Resolved, but the row has no sprite base or token (waypoints markers,
    /// presets without art, type 1 rows the tables do not describe).
    NoSprite,
    /// The COF file is not in any layer of the overlay.
    CofNotFound(String),
    /// The COF is present but cannot be parsed or has no frames.
    CofInvalid(String),
    /// The COF loaded, but none of its components resolved to a sprite.
    NoLayers(String),
}

impl std::fmt::Display for MissingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MissingReason::Unresolved => f.write_str("unresolved"),
            MissingReason::NoSprite => f.write_str("no sprite token"),
            MissingReason::CofNotFound(p) => write!(f, "COF not found: {p}"),
            MissingReason::CofInvalid(p) => write!(f, "COF unusable: {p}"),
            MissingReason::NoLayers(p) => write!(f, "no component sprite found for {p}"),
        }
    }
}

/// A DS1 object that draws nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingObject {
    /// Index into `Ds1::objects`.
    pub index: usize,
    pub kind: i32,
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub reason: MissingReason,
}

/// One sprite layer of a frame, in COF priority order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameLayer {
    /// COF component index (0..=15).
    pub component: u8,
    /// The layer's frame; its offsets are relative to the object anchor.
    pub image: Arc<IndexedImage>,
    /// Top-left corner of the direction box, relative to the anchor: where the
    /// reference's layer bitmap starts (`lay->off_x`, `lay->off_y`).
    pub box_origin: (i32, i32),
    /// Blend of the body draw (`preview.c:2131`).
    pub blend: Blend,
    /// Shadow darkness level (28 for a transparent layer, 10 otherwise,
    /// `preview.c:2259`) when the layer casts a shadow, else `None`.
    pub shadow: Option<u8>,
}

/// The layers of one (object, direction, frame), ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectFrame {
    pub layers: Vec<FrameLayer>,
    /// `Objects.txt` `Xoffset`/`Yoffset` (`cof.c:482-496`).
    pub offset: (i32, i32),
    /// Objects.txt `OrderFlag` of the mode: 1 draws behind walls, 0 and 2 in
    /// front; anything else is never drawn (`preview.c:2410,2478`).
    pub order_flag: u8,
}

impl ObjectFrame {
    /// Whether any layer casts a shadow.
    pub fn has_shadow(&self) -> bool {
        self.layers.iter().any(|l| l.shadow.is_some())
    }

    /// The 1:1 composite of the layers (what `compose_cof_frame` produces,
    /// plus the Objects.txt offset in `offset_x`/`offset_y`), or `None` when
    /// nothing is visible. Offsets are relative to the object anchor.
    pub fn to_rgba(&self, palette: &Palette) -> Option<crate::compose::RgbaImage> {
        self.composite(palette).map(|(img, _)| img)
    }

    /// [`ObjectFrame::to_rgba`] and the position of its pixel (0,0) relative to
    /// the object anchor.
    pub fn composite(&self, palette: &Palette) -> Option<(crate::compose::RgbaImage, (i32, i32))> {
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for l in &self.layers {
            let (w, h) = (
                i32::try_from(l.image.width).ok()?,
                i32::try_from(l.image.height).ok()?,
            );
            if w == 0 || h == 0 {
                continue;
            }
            let (x0, y0) = (l.image.offset_x, l.image.offset_y);
            let (x1, y1) = (x0.checked_add(w)?, y0.checked_add(h)?);
            bounds = Some(match bounds {
                None => (x0, y0, x1, y1),
                Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
            });
        }
        let (x0, y0, x1, y1) = bounds?;
        let (w, h) = (u32::try_from(x1 - x0).ok()?, u32::try_from(y1 - y0).ok()?);
        if u64::from(w) * u64::from(h) > 1 << 26 {
            return None;
        }
        let mut canvas = crate::compose::RgbaImage::filled(w, h, [0, 0, 0, 0]);
        for l in &self.layers {
            let img = &l.image;
            let (ox, oy) = (img.offset_x - x0, img.offset_y - y0);
            for j in 0..img.height {
                for i in 0..img.width {
                    let k = (j as usize) * (img.width as usize) + i as usize;
                    let (Some(&p), Some(&a)) = (img.pixels.get(k), img.alpha.get(k)) else {
                        continue;
                    };
                    if a == 0 || p == 0 {
                        continue;
                    }
                    let (Ok(x), Ok(y)) = (
                        u32::try_from(ox + i32::try_from(i).ok()?),
                        u32::try_from(oy + i32::try_from(j).ok()?),
                    ) else {
                        continue;
                    };
                    let Some(dst) = canvas.get(x, y) else {
                        continue;
                    };
                    canvas.set(x, y, l.blend.apply(dst, palette.colors[usize::from(p)]));
                }
            }
        }
        Some((canvas, (x0 + self.offset.0, y0 + self.offset.1)))
    }
}

/// Which frame of a looping animation is shown.
///
/// `tick` is the editor's 25 Hz counter (`main.c:1842`). Each tick adds
/// `delta / 256` frames (`preview.c:1939-1943`), so the frame is
/// `tick * delta / 256 mod count`. `None` shows frame 0. The reference starts
/// every object at a random frame (`cof.c:288`, `ds1.c:988`); we do not.
pub fn frame_index(tick: Option<u32>, delta: u32, count: u32) -> u8 {
    let count = count.max(1);
    let frame = match tick {
        None => 0,
        Some(t) => (u64::from(t) * u64::from(delta) / 256) % u64::from(count),
    };
    u8::try_from(frame).unwrap_or(0)
}

/// Where the reference anchors the sprites of the object at DS1 sub-tile
/// `(x, y)`, in map space where the bitmap origin of cell (0, 0) is (0, 0), at
/// zoom `1:div` (`preview.c:2084-2086`, integer maths truncating toward zero).
/// The `4` is added in zoomed units without scaling.
pub fn object_anchor(x: i32, y: i32, div: i32) -> (i32, i32) {
    let div = div.max(1);
    let (tw, th) = (TILE_W / div, TILE_H / div);
    let dx = (y - 2).saturating_mul(-tw) / 10 + (x + 3).saturating_mul(tw) / 10;
    let dy = 4 + (y - 2).saturating_mul(th) / 10 + (x + 3).saturating_mul(th) / 10;
    (dx, dy)
}

/// An object ready to be put in the draw list.
#[derive(Clone, Debug)]
pub struct PlacedObject {
    /// Index into `Ds1::objects`.
    pub index: usize,
    pub kind: i32,
    pub id: i32,
    /// DS1 sub-tile coordinates (5 per tile).
    pub x: i32,
    pub y: i32,
    pub frame: Arc<ObjectFrame>,
}

impl PlacedObject {
    /// `(tile y, tile x, sub-tile order, index)`: the reference's drawing order
    /// (`objects.c:15-43`).
    pub fn sort_key(&self) -> (i32, i32, u8, usize) {
        let (tx, sx) = (self.x.div_euclid(5), self.x.rem_euclid(5));
        let (ty, sy) = (self.y.div_euclid(5), self.y.rem_euclid(5));
        let sub = SUB_TILE_ORDER
            .get(sy as usize)
            .and_then(|row| row.get(sx as usize))
            .copied()
            .unwrap_or(0);
        (ty, tx, sub, self.index)
    }

    /// The tile `(x, y)` the object stands on.
    pub fn tile(&self) -> (u32, u32) {
        (
            u32::try_from(self.x.div_euclid(5)).unwrap_or(0),
            u32::try_from(self.y.div_euclid(5)).unwrap_or(0),
        )
    }
}

/// Options of [`ObjectSprites::place`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectOptions {
    /// Fallback user direction for objects with no obj.txt default
    /// (`ObjectInfo::direction`) and no override; mapped to the file
    /// direction per COF.
    pub direction: u8,
    /// Animation tick, see [`frame_index`]. `None` = frame 0.
    pub animation_tick: Option<u32>,
    /// Pins the frame of individual objects, indexed like `Ds1::objects`; a
    /// `Some(frame)` (taken modulo the COF's frame count) wins over
    /// `animation_tick`. The oracle comparison uses this to reproduce the
    /// reference's seeded random frames.
    pub frame_overrides: Option<Arc<[Option<u8>]>>,
    /// Pins the user direction of individual objects, indexed like
    /// `Ds1::objects`; `None` entries use `direction`. DS1Edit's `obj.txt`
    /// gives some rows a default direction (`cof.c:553-558`).
    pub direction_overrides: Option<Arc<[Option<u8>]>>,
}

/// One loaded component sprite for the chosen direction.
struct LayerSprite {
    component: u8,
    shadow: bool,
    transparent: bool,
    draw_effect: u8,
    frames: Vec<Arc<IndexedImage>>,
    box_origin: (i32, i32),
}

/// A COF with its sprites for one direction.
struct Asset {
    cof: Cof,
    /// File direction the layers were loaded for; it also indexes the COF
    /// priority table (lead decision).
    dir: u8,
    layers: Vec<Option<LayerSprite>>,
    frames: Mutex<HashMap<FrameKey, Arc<ObjectFrame>>>,
    animdata_speed: Option<u32>,
}

/// (frame, Objects.txt offset, order flag)
type FrameKey = (u8, (i32, i32), u8);

type AssetResult = std::result::Result<Arc<Asset>, MissingReason>;

/// Loads object sprites through an overlay and caches them.
///
/// Composites are cached per (sprite key, direction, frame): the sprite key is
/// base, token, mode, class and skins, so objects sharing a sprite share one
/// decode.
pub struct ObjectSprites<'a> {
    overlay: &'a Overlay,
    catalog: &'a ObjectCatalog,
    animdata: OnceLock<Option<AnimData>>,
    assets: Mutex<HashMap<String, AssetResult>>,
}

fn file_direction(cof_dirs: u8, user: u8) -> u8 {
    let user = usize::from(user);
    let lookup = |table: &[u8]| {
        table
            .get(user)
            .or_else(|| table.first())
            .copied()
            .unwrap_or(0)
    };
    match cof_dirs {
        8 => lookup(&DIR8),
        16 => lookup(&DIR16),
        32 => lookup(&DIR32),
        4 => u8::try_from(user.min(3)).unwrap_or(0),
        _ => 0,
    }
}

fn base_folder(base: SpriteBase) -> &'static str {
    match base {
        SpriteBase::Objects => "data\\global\\objects",
        SpriteBase::Monsters => "data\\global\\monsters",
    }
}

/// The direction box origin of a set of frames: the union of the non-empty
/// frame rectangles (`dc6.c:193-260`; the DCC header box is the same union).
fn box_origin(frames: &[IndexedImage]) -> (i32, i32) {
    let mut origin: Option<(i32, i32)> = None;
    for f in frames {
        if f.width == 0 || f.height == 0 {
            continue;
        }
        origin = Some(match origin {
            None => (f.offset_x, f.offset_y),
            Some((x, y)) => (x.min(f.offset_x), y.min(f.offset_y)),
        });
    }
    origin.unwrap_or((0, 0))
}

impl<'a> ObjectSprites<'a> {
    pub fn new(overlay: &'a Overlay, catalog: &'a ObjectCatalog) -> ObjectSprites<'a> {
        ObjectSprites {
            overlay,
            catalog,
            animdata: OnceLock::new(),
            assets: Mutex::new(HashMap::new()),
        }
    }

    pub fn catalog(&self) -> &ObjectCatalog {
        self.catalog
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.overlay.read_bytes(&GamePath::new(path)).ok().flatten()
    }

    fn animdata(&self) -> Option<&AnimData> {
        self.animdata
            .get_or_init(|| {
                let bytes = self.read("data\\global\\animdata.d2")?;
                AnimData::parse(&bytes).ok()
            })
            .as_ref()
    }

    /// Resolves every object of `ds1` to a drawable frame, in the reference's
    /// drawing order. Objects outside the map are dropped like the reference
    /// does (`ds1.c:975`); the rest that draw nothing are returned as
    /// [`MissingObject`].
    pub fn place(
        &self,
        ds1: &Ds1,
        opts: &ObjectOptions,
    ) -> (Vec<PlacedObject>, Vec<MissingObject>) {
        let mut placed = Vec::new();
        let mut missing = Vec::new();
        let max_x = i64::from(ds1.width) * 5;
        let max_y = i64::from(ds1.height) * 5;
        for (index, o) in ds1.objects.iter().enumerate() {
            if o.x < 0 || o.y < 0 || i64::from(o.x) >= max_x || i64::from(o.y) >= max_y {
                continue;
            }
            let forced = opts
                .frame_overrides
                .as_ref()
                .and_then(|f| f.get(index).copied().flatten());
            // Explicit override, else the object's own default facing from
            // obj.txt (`ObjectInfo::direction`, as the reference does), else
            // the caller's default.
            let direction = opts
                .direction_overrides
                .as_ref()
                .and_then(|d| d.get(index).copied().flatten())
                .or_else(|| {
                    self.catalog
                        .resolve(ds1.effective_act(), ds1.version, o)
                        .direction
                })
                .unwrap_or(opts.direction);
            match self.frame_for(ds1, o, opts, forced, direction) {
                Ok(frame) => placed.push(PlacedObject {
                    index,
                    kind: o.kind,
                    id: o.id,
                    x: o.x,
                    y: o.y,
                    frame,
                }),
                Err(reason) => missing.push(MissingObject {
                    index,
                    kind: o.kind,
                    id: o.id,
                    x: o.x,
                    y: o.y,
                    reason,
                }),
            }
        }
        placed.sort_by_key(PlacedObject::sort_key);
        (placed, missing)
    }

    /// The frame of one object.
    pub fn frame_for(
        &self,
        ds1: &Ds1,
        object: &Object,
        opts: &ObjectOptions,
        forced_frame: Option<u8>,
        direction: u8,
    ) -> std::result::Result<Arc<ObjectFrame>, MissingReason> {
        let info = self
            .catalog
            .resolve(ds1.effective_act(), ds1.version, object);
        self.frame_of_info(&info, opts, forced_frame, direction)
    }

    /// The frame of an already resolved object.
    pub fn frame_of_info(
        &self,
        info: &ObjectInfo,
        opts: &ObjectOptions,
        forced_frame: Option<u8>,
        direction: u8,
    ) -> std::result::Result<Arc<ObjectFrame>, MissingReason> {
        let (Some(base), Some(token)) = (info.sprite_base, info.token.as_deref()) else {
            return Err(if info.source == "unresolved" {
                MissingReason::Unresolved
            } else {
                MissingReason::NoSprite
            });
        };
        let token = path_token(token);
        if token.is_empty() {
            return Err(MissingReason::NoSprite);
        }
        let mode = if path_token(&info.mode).is_empty() {
            "nu".to_owned()
        } else {
            path_token(&info.mode)
        };
        let class = if path_token(&info.weapon_class).is_empty() {
            "hth".to_owned()
        } else {
            path_token(&info.weapon_class)
        };
        let skins: Vec<(String, String)> = info
            .skins
            .iter()
            .map(|(c, s)| (c.to_ascii_uppercase(), path_token(s)))
            .collect();
        let key = format!("{base:?}|{token}|{mode}|{class}|{skins:?}|{direction}");
        let asset = {
            let mut cache = self.assets.lock().unwrap_or_else(PoisonError::into_inner);
            cache
                .entry(key)
                .or_insert_with(|| self.load_asset(base, &token, &mode, &class, &skins, direction))
                .clone()
        }?;
        // Frame choice: objects use their Objects.txt FrameDelta (0 already
        // mapped to 256); monsters the AnimData speed (cof.c:385-400,472-477).
        let count = u32::from(asset.cof.frames_per_direction.max(1));
        let delta = info
            .frame_delta
            .or(asset.animdata_speed)
            .filter(|d| *d != 0)
            .unwrap_or(256);
        let frame = match forced_frame {
            Some(f) => u8::try_from(u32::from(f) % count).unwrap_or(0),
            None => frame_index(opts.animation_tick, delta, count),
        };
        let mut frames = asset.frames.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = frames
            .entry((frame, info.offset, info.order_flag))
            .or_insert_with(|| Arc::new(build_frame(&asset, frame, info.offset, info.order_flag)))
            .clone();
        if cached.layers.is_empty() {
            return Err(MissingReason::NoLayers(format!("{token}{mode}{class}")));
        }
        Ok(cached)
    }

    fn load_asset(
        &self,
        base: SpriteBase,
        token: &str,
        mode: &str,
        class: &str,
        skins: &[(String, String)],
        user_dir: u8,
    ) -> AssetResult {
        let folder = base_folder(base);
        let cof_path = cof_path(folder, token, mode, class);
        let bytes = self
            .read(&cof_path)
            .ok_or_else(|| MissingReason::CofNotFound(cof_path.clone()))?;
        let cof = Cof::parse(&bytes).map_err(|_| MissingReason::CofInvalid(cof_path.clone()))?;
        if cof.directions == 0 || cof.frames_per_direction == 0 || cof.layers.is_empty() {
            return Err(MissingReason::CofInvalid(cof_path));
        }
        let dir = file_direction(cof.directions, user_dir).min(cof.directions - 1);
        let mut layers: Vec<Option<LayerSprite>> = (0..16).map(|_| None).collect();
        // The last record of a component wins (cof.c:306-318).
        for (idx, slot) in (0u8..).zip(layers.iter_mut()) {
            let Some(record) = cof.component_layer(idx) else {
                continue;
            };
            let Some(code) = record.component_code() else {
                continue;
            };
            // obj.txt gives every drawn component a skin; an empty cell makes
            // the reference build a name no file has, so the layer is not
            // drawn (cof.c:322-333). Without a skin list (tables other than
            // obj.txt) every component is `lit`.
            let skin = match skins.iter().find(|(c, _)| c == code) {
                Some((_, s)) => s.as_str(),
                None if skins.is_empty() => "lit",
                None => continue,
            };
            let paths = component_paths(
                folder,
                token,
                mode,
                &record.weapon_class_token(),
                code,
                skin,
            );
            let frames = self.load_component(&paths, &cof, dir);
            *slot = frames.map(|frames| LayerSprite {
                component: record.component,
                shadow: record.shadow != 0,
                transparent: record.transparent != 0,
                draw_effect: record.draw_effect,
                box_origin: box_origin(&frames),
                frames: frames.into_iter().map(Arc::new).collect(),
            });
        }
        if layers.iter().all(Option::is_none) {
            return Err(MissingReason::NoLayers(cof_path));
        }
        let animdata_speed = self
            .animdata()
            .and_then(|a| a.get(&format!("{token}{mode}{class}")))
            .map(|r| r.speed);
        Ok(Arc::new(Asset {
            cof,
            dir,
            layers,
            frames: Mutex::new(HashMap::new()),
            animdata_speed,
        }))
    }

    /// The frames of direction `dir` from the `.dcc` of `paths`, else its
    /// `.dc6` ([`component_paths`] order).
    fn load_component(
        &self,
        [dcc, dc6]: &[String; 2],
        cof: &Cof,
        dir: u8,
    ) -> Option<Vec<IndexedImage>> {
        if let Some(bytes) = self.read(dcc) {
            // No fall back to .dc6 when the .dcc exists but fails (the game
            // does not either, PLAN.md "Format decisions").
            let dcc = Dcc::parse(&bytes).ok()?;
            return dcc
                .directions
                .into_iter()
                .nth(usize::from(dir))
                .map(|d| d.frames);
        }
        let bytes = self.read(dc6)?;
        let dc6 = Dc6::parse(&bytes).ok()?;
        // dc6.c:175: the direction count must equal the COF's.
        if dc6.directions != u32::from(cof.directions) {
            return None;
        }
        (0..dc6.frames_per_direction)
            .map(|f| dc6.frame(u32::from(dir), f)?.render().ok())
            .collect()
    }
}

fn build_frame(asset: &Asset, frame: u8, offset: (i32, i32), order_flag: u8) -> ObjectFrame {
    let order = asset.cof.draw_order(asset.dir, frame).unwrap_or(&[]);
    let mut layers = Vec::new();
    for &component in order {
        let Some(Some(layer)) = asset.layers.get(usize::from(component)) else {
            continue;
        };
        // A layer with fewer frames shows its last good frame (preview.c:2107).
        let index = usize::from(frame).min(layer.frames.len().saturating_sub(1));
        let Some(image) = layer.frames.get(index) else {
            continue;
        };
        layers.push(FrameLayer {
            component: layer.component,
            image: Arc::clone(image),
            box_origin: layer.box_origin,
            blend: Blend::for_cof_layer(layer.transparent, layer.draw_effect),
            shadow: layer
                .shadow
                .then_some(if layer.transparent { 28 } else { 10 }),
        });
    }
    ObjectFrame {
        layers,
        offset,
        order_flag,
    }
}

#[cfg(test)]
mod tests;
