//! The tile-creation tools: `tile_template`, `create_tile`,
//! `import_tile_image`, `reshape_tile`, `new_dt1`, `list_own_tiles` and
//! `inspect_tile`. Shapes, splitting into sub-blocks, image import,
//! numbering and "Add to this map" are `capy_mod::tiles_create`'s; this
//! module parses arguments, reads the profile and writes the results the way
//! the editor does: `capybara\saves\<profile>\<game path>` + `CHANGED.txt` + a
//! history snapshot per file, never the game folder or an MPQ. LvlTypes and
//! LvlPrest are saved as the editor's "Add to this map" saves them (on 1.13c
//! the compiled `.bin` fields are patched and the `.txt` forms mirrored).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use capy_edit::Patch;
use capy_history::{SnapshotKind, SnapshotStore};
use capy_mod::compat::Edit;
use capy_mod::tiles_create::{
    self as tc, BuildOptions, BuiltTile, ImportOptions, MapTarget, Placement, TileClass, TileShape,
    TileTemplate,
};
use capy_mod::tiles_dt1::{custom_name_problem, is_safe_name};
use capy_mod::tiles_register::{self as reg, Registration};
use capy_mod::tiles_review::{TileId, cells_using, tile_ids};
use capy_sprite::image_io::{ImageKind, import_image};
use capy_sprite::quantize::{Dither, QuantizeOptions};
use d2_assets::tables::load_excel;
use d2_assets::{GamePath, GameVersion};
use d2_formats::IndexedImage;
use d2_formats::dt1::{Dt1, Tile};
use d2_formats::excel::{ExcelTableKind, patch_bin_field};
use d2_formats::txt::{TxtDoc, TxtTable};
use serde_json::{Map, Value, json};

use super::extra::orientation_name;
use super::{Content, Session, ToolResult, base64, base64_decode, err_text, int, opt_int, text};

/// Largest `width`/`height` of a pixel grid (a rendered tile stops at 2048).
const MAX_GRID: i64 = 2048;
/// Largest image file `import_tile_image` reads.
const MAX_IMAGE_BYTES: u64 = 16 << 20;
/// Largest decoded picture side `import_tile_image` accepts.
const MAX_IMAGE_SIDE: u32 = 4096;
const TILES_PREFIX: &str = "data/global/tiles/";

// ------------------------------------------------------------------ paths

/// `data\global\tiles\a\b.dt1` -> `a/b.dt1` (what the tables hold).
fn tiles_rel(path: &str) -> String {
    let p = path.replace('\\', "/");
    match p.get(..TILES_PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(TILES_PREFIX) => p[TILES_PREFIX.len()..].to_owned(),
        _ => p,
    }
}

/// `a/b.dt1` -> `data\global\tiles\a\b.dt1`.
fn tiles_game(rel: &str) -> GamePath {
    GamePath::new(&format!("data\\global\\tiles\\{}", rel.replace('/', "\\")))
}

fn excel(name: &str) -> GamePath {
    GamePath::new(&format!("data\\global\\excel\\{name}"))
}

/// A DT1 named by the client: a game path under `data\global\tiles` or a path
/// relative to it. Refuses anything that could leave that folder.
fn dt1_path(v: Option<&Value>) -> Result<GamePath, String> {
    let s = v
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("dt1 must be a DT1 path, e.g. act1/outdoors/mine.dt1")?;
    let rel = tiles_rel(s);
    let unsafe_part = |p: &str| {
        p.is_empty()
            || p == "."
            || p == ".."
            || p.ends_with('.')
            || p.ends_with(' ')
            || p.chars()
                .any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
    };
    if rel.split('/').any(unsafe_part) {
        return Err(format!(
            "{s:?} is not a safe DT1 path: give a path under data/global/tiles, e.g. act1/outdoors/mine.dt1"
        ));
    }
    if !rel.to_ascii_lowercase().ends_with(".dt1") {
        return Err(format!("{s}: a tile library path ends in .dt1"));
    }
    Ok(tiles_game(&rel))
}

// ------------------------------------------------------------------ arguments

/// A parsed shape; `pair`: both halves of a north corner from one picture.
struct ShapeArg {
    shape: TileShape,
    pair: bool,
}

/// `<prefix>orientation`, `<prefix>height`, `<prefix>roof_height` and (no
/// prefix only) `corner_pair`.
fn parse_shape(args: &Value, prefix: &str) -> Result<ShapeArg, String> {
    let key = |k: &str| format!("{prefix}{k}");
    let o = int(args.get(key("orientation")), &key("orientation"), 0, 19)? as u8;
    let class = TileClass::of_orientation(i32::from(o))
        .ok_or_else(|| format!("orientation {o} is not a tile shape (0-19)"))?;
    let height = opt_int(
        args.get(key("height")),
        &key("height"),
        1,
        i64::from(tc::MAX_WALL_HEIGHT),
    )?;
    let roof = opt_int(
        args.get(key("roof_height")),
        &key("roof_height"),
        i64::from(i16::MIN),
        i64::from(i16::MAX),
    )?;
    let pair = prefix.is_empty()
        && args
            .get("corner_pair")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if pair && !matches!(o, 3 | 4) {
        return Err(
            "corner_pair needs orientation 3 or 4 (the two halves of a north corner wall).".into(),
        );
    }
    let shape = match class {
        TileClass::Floor => TileShape::floor(),
        TileClass::Roof => TileShape::roof(roof.map_or(tc::DEFAULT_ROOF_HEIGHT, |r| r as i16)),
        _ => {
            let h = height.ok_or_else(|| {
                format!(
                    "{} is required for orientation {o} ({}): the picture height in pixels, 1-{} (rounded up to a multiple of 32).",
                    key("height"),
                    orientation_name(u32::from(o)),
                    tc::MAX_WALL_HEIGHT
                )
            })?;
            TileShape::wall(o, h as u32)
        }
    };
    Ok(ShapeArg {
        shape: shape.normalized().map_err(err_text)?,
        pair,
    })
}

/// The template a picture is drawn in: the shape's, or for a corner pair the
/// full 160-wide wall of the same height (x 0-79 becomes orientation 4, x
/// 80-159 orientation 3).
fn picture_template(s: &ShapeArg) -> Result<TileTemplate, String> {
    let shape = if s.pair {
        TileShape::wall(1, s.shape.height)
    } else {
        s.shape
    };
    tc::template(shape).map_err(err_text)
}

fn parse_placement(v: Option<&Value>) -> Result<Placement, String> {
    match v.and_then(Value::as_str).unwrap_or("as_is") {
        "as_is" => Ok(Placement::AsIs),
        "center" => Ok(Placement::Center),
        "fit" => Ok(Placement::Fit),
        other => Err(format!(
            "placement {other:?}: use \"as_is\", \"center\" or \"fit\"."
        )),
    }
}

fn parse_build_options(args: &Value) -> Result<BuildOptions, String> {
    let flags = match args.get("flags").filter(|v| !v.is_null()) {
        Some(v) => {
            let a = v
                .as_array()
                .filter(|a| a.len() == 25)
                .ok_or("flags must be 25 numbers 0-255 (sub-tile y * 5 + x).")?;
            let mut f = [0u8; 25];
            for (slot, x) in f.iter_mut().zip(a) {
                *slot = int(Some(x), "each flag", 0, 255)? as u8;
            }
            Some(f)
        }
        None => None,
    };
    Ok(BuildOptions {
        flags,
        rarity: opt_int(args.get("rarity"), "rarity", 0, 1_000_000)?.unwrap_or(0) as i32,
        sound: opt_int(args.get("sound"), "sound", 0, 255)?.unwrap_or(0) as u8,
        animated: 0,
    })
}

/// The number a new tile asks for.
enum Number {
    Exact(i32, i32),
    /// The first free number, in this main when given.
    Free(Option<i32>),
}

fn parse_number(args: &Value) -> Result<Number, String> {
    let main = opt_int(args.get("main"), "main", 0, 63)?;
    let sub = opt_int(args.get("sub"), "sub", 0, 255)?;
    match (main, sub) {
        (Some(m), Some(s)) => Ok(Number::Exact(m as i32, s as i32)),
        (m, None) => Ok(Number::Free(m.map(|m| m as i32))),
        (None, Some(_)) => Err("sub needs main as well.".into()),
    }
}

fn choose(
    n: &Number,
    taken: &BTreeSet<TileId>,
    orientations: &[i32],
) -> Result<(i32, i32), String> {
    match *n {
        Number::Exact(m, s) => Ok((m, s)),
        Number::Free(prefer) => tc::free_tile_number(taken, orientations, prefer)
            .ok_or_else(|| "Every tile number (main 0-63, sub 0-255) is taken.".to_owned()),
    }
}

/// A palette-indexed picture from `pixels`/`width`/`height` (plain top-left
/// space of `t`, placed with `placement`) over an optional `fill` of the
/// drawable area; blank when neither is given. Anchor space.
fn picture(args: &Value, t: &TileTemplate) -> Result<IndexedImage, String> {
    let placement = parse_placement(args.get("placement"))?;
    let fill = opt_int(args.get("fill"), "fill", 1, 255)?;
    let key =
        opt_int(args.get("transparent_index"), "transparent_index", 0, 255)?.unwrap_or(0) as u8;
    let grid = match args.get("pixels").filter(|v| !v.is_null()) {
        Some(p) => {
            let s = p.get("data").and_then(Value::as_str).ok_or(
                "pixels must be {data, width, height}: data is base64, one palette index per byte.",
            )?;
            let w = int(p.get("width"), "pixels.width", 1, MAX_GRID)? as u32;
            let h = int(p.get("height"), "pixels.height", 1, MAX_GRID)? as u32;
            let bytes =
                base64_decode(s).map_err(|e| format!("pixels.data is not valid base64: {e}."))?;
            let want = w as usize * h as usize;
            if bytes.len() != want {
                return Err(format!(
                    "pixels.data holds {} bytes but width x height is {want}.",
                    bytes.len()
                ));
            }
            let alpha = bytes
                .iter()
                .map(|&b| if b == key { 0 } else { 255 })
                .collect();
            let plain = IndexedImage {
                width: w,
                height: h,
                offset_x: 0,
                offset_y: 0,
                pixels: bytes,
                alpha,
            };
            Some(tc::place_indexed(&plain, t, placement))
        }
        None => None,
    };
    let base = fill.map(|f| {
        let mut m = t.mask();
        for (p, a) in m.pixels.iter_mut().zip(&m.alpha) {
            if *a != 0 {
                *p = f as u8;
            }
        }
        m
    });
    Ok(match (base, grid) {
        (None, None) => IndexedImage::new(0, 0, t.rect.x, t.rect.y),
        (Some(b), None) => b,
        (None, Some(g)) => g,
        (Some(b), Some(g)) => overlay(&b, &g),
    })
}

/// `top` drawn over `base` (anchor space), on the union of both.
fn overlay(base: &IndexedImage, top: &IndexedImage) -> IndexedImage {
    let x0 = base.offset_x.min(top.offset_x);
    let y0 = base.offset_y.min(top.offset_y);
    let x1 = (base.offset_x + base.width as i32).max(top.offset_x + top.width as i32);
    let y1 = (base.offset_y + base.height as i32).max(top.offset_y + top.height as i32);
    let w = (x1 - x0) as usize;
    let mut out = IndexedImage::new(w as u32, (y1 - y0) as u32, x0, y0);
    for img in [base, top] {
        let iw = img.width as usize;
        for (i, (&p, &a)) in img.pixels.iter().zip(&img.alpha).enumerate() {
            if a == 0 || iw == 0 {
                continue;
            }
            let x = (img.offset_x - x0) as usize + i % iw;
            let y = (img.offset_y - y0) as usize + i / iw;
            let j = y * w + x;
            if let (Some(op), Some(oa)) = (out.pixels.get_mut(j), out.alpha.get_mut(j)) {
                *op = p;
                *oa = 255;
            }
        }
    }
    out
}

fn build(img: &IndexedImage, s: &ShapeArg, opts: &BuildOptions) -> Result<Vec<BuiltTile>, String> {
    if s.pair {
        let (a, b) = tc::build_corner_pair(img, s.shape.height, 0, 0, opts).map_err(err_text)?;
        Ok(vec![a, b])
    } else {
        Ok(vec![
            tc::build_tile(img, s.shape, 0, 0, opts).map_err(err_text)?,
        ])
    }
}

fn key_text(id: TileId) -> String {
    format!("o{} m{} s{}", id.orientation, id.main, id.sub)
}

fn shape_text(s: &TileShape) -> String {
    match s.class() {
        Some(TileClass::Floor) => "floor diamond 160x79".into(),
        Some(TileClass::Roof) => format!("roof diamond 160x79, roof height {}", s.roof_height),
        Some(c) => format!("{c:?} {} px tall", s.height),
        None => "not a tile shape".into(),
    }
}

/// The layer(s) a tile of `o` is painted on.
fn layer_hint(o: i32) -> &'static str {
    match o {
        0 => "floor1 or floor2",
        13 => "shadow",
        _ => "wall1-wall4 (with this orientation)",
    }
}

fn describe_tile(i: usize, t: &Tile) -> String {
    let shape = TileShape::of_tile(t).map_or("?".to_owned(), |s| shape_text(&s));
    format!(
        "#{i} {} ({}) - {shape} - rarity {} - sound {} - {} blocked sub-tile(s) - {} block(s)",
        key_text(TileId::of(t)),
        orientation_name(u32::try_from(t.orientation).unwrap_or(99)),
        t.rarity,
        t.sound,
        t.subtile_flags.iter().filter(|f| **f != 0).count(),
        t.blocks.len()
    )
}

fn flags_text(f: &[u8; 25]) -> String {
    f.chunks(5)
        .map(|r| {
            r.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The tile of `dt1` that `index`, or `orientation`/`main`/`sub`(/`variant`),
/// names.
fn pick_tile(dt1: &Dt1, args: &Value, path: &GamePath) -> Result<usize, String> {
    if let Some(i) = opt_int(args.get("index"), "index", 0, 1_000_000)? {
        let i = i as usize;
        return if i < dt1.tiles.len() {
            Ok(i)
        } else {
            Err(format!("{path} has {} tiles; no #{i}.", dt1.tiles.len()))
        };
    }
    let o = int(args.get("orientation"), "orientation", 0, 255)? as i32;
    let m = int(args.get("main"), "main", 0, 255)? as i32;
    let s = int(args.get("sub"), "sub", 0, 255)? as i32;
    let v = opt_int(args.get("variant"), "variant", 0, 1_000_000)?.unwrap_or(0) as usize;
    dt1.tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.orientation == o && t.main_index == m && t.sub_index == s)
        .nth(v)
        .map(|(i, _)| i)
        .ok_or_else(|| {
            format!(
                "{path} has no tile o{o} m{m} s{s}{} (list_own_tiles lists them).",
                if v > 0 {
                    format!(" variant {v}")
                } else {
                    String::new()
                }
            )
        })
}

/// A decoded compiled table as text, for reading only (never written back).
fn compiled_doc(t: &TxtTable) -> TxtDoc {
    let mut s = t.headers().join("\t");
    s.push_str("\r\n");
    for r in t.rows() {
        s.push_str(&r.cells().join("\t"));
        s.push_str("\r\n");
    }
    TxtDoc::parse(s.as_bytes())
}

/// LvlTypes and LvlPrest as the game reads them; `compiled`: decoded `.bin`.
struct Tables {
    types: TxtDoc,
    prest: TxtDoc,
    compiled: bool,
}

/// The open map's place in the tables.
struct MapCtx {
    path: GamePath,
    /// Relative to data/global/tiles.
    rel: String,
    type_id: u32,
    def: Option<u32>,
}

// ------------------------------------------------------------------ session side

impl Session {
    /// Writes `bytes` to the profile's saves (+ CHANGED.txt + a history
    /// snapshot), as `save_map` and the editor do; adds a line to `log`.
    fn write_saved(
        &self,
        path: &GamePath,
        bytes: &[u8],
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        let written = self
            .saves
            .save(path, bytes)
            .map_err(|e| format!("cannot write {path}: {e}"))?;
        let note = match SnapshotStore::open(&self.root.join("history"), &self.profile.id)
            .and_then(|mut s| s.record(path, bytes, SnapshotKind::Save))
        {
            Ok(_) => String::new(),
            Err(e) => format!(" (the history snapshot failed: {e})"),
        };
        log.push(format!(
            "{path} ({} bytes) -> {}{note}",
            bytes.len(),
            written.display()
        ));
        Ok(())
    }

    /// Whether the profile's saves hold `path` (only such DT1s are written).
    fn is_own(&self, path: &GamePath) -> bool {
        self.saves.path_for(path).is_ok_and(|p| p.is_file())
    }

    fn require_own(&self, path: &GamePath) -> Result<(), String> {
        if self.is_own(path) {
            Ok(())
        } else {
            Err(format!(
                "{path} is not one of this profile's own DT1s: only DT1s in capybara\\saves\\{} are changed (make one with new_dt1, or leave dt1 out to use the open map's own DT1).",
                self.profile.id
            ))
        }
    }

    fn read_dt1(&self, path: &GamePath) -> Result<Option<Dt1>, String> {
        match self.overlay.read_bytes(path).map_err(err_text)? {
            Some(b) => Dt1::parse(&b)
                .map(Some)
                .map_err(|e| format!("{path}: not a readable DT1: {e}")),
            None => Ok(None),
        }
    }

    fn read_txt(&self, name: &str) -> Result<Option<TxtDoc>, String> {
        Ok(self
            .overlay
            .read_bytes(&excel(name))
            .map_err(err_text)?
            .map(|b| TxtDoc::parse(&b)))
    }

    /// The editor's `tables_for_tiles`: decoded `.bin` on 1.13c when both
    /// exist, else the `.txt` (refused when stale against a `.bin`).
    fn tile_tables(&self) -> Result<Tables, String> {
        let v113 = self.profile.game_version == GameVersion::V113c;
        let has = |n: &str| self.overlay.resolve(&excel(n)).is_some();
        let compiled = |kind| load_excel(&self.overlay, kind, GameVersion::V113c).map_err(err_text);
        if v113 && has("lvltypes.bin") && has("lvlprest.bin") {
            return Ok(Tables {
                types: compiled_doc(&compiled(ExcelTableKind::LvlTypes)?),
                prest: compiled_doc(&compiled(ExcelTableKind::LvlPrest)?),
                compiled: true,
            });
        }
        let missing =
            |n: &str| format!("{n} is not in this profile, so tiles cannot be registered.");
        let types = self
            .read_txt("lvltypes.txt")?
            .ok_or_else(|| missing("lvltypes.txt"))?;
        let prest = self
            .read_txt("lvlprest.txt")?
            .ok_or_else(|| missing("lvlprest.txt"))?;
        if v113 {
            for (name, bin, doc, kind) in [
                (
                    "lvltypes.txt",
                    "lvltypes.bin",
                    &types,
                    ExcelTableKind::LvlTypes,
                ),
                (
                    "lvlprest.txt",
                    "lvlprest.bin",
                    &prest,
                    ExcelTableKind::LvlPrest,
                ),
            ] {
                if has(bin)
                    && let Some(msg) = reg::stale_against(name, doc, compiled(kind)?.len())
                {
                    return Err(msg);
                }
            }
        }
        Ok(Tables {
            types,
            prest,
            compiled: false,
        })
    }

    /// The editor's `save_registration`: `.txt` profiles write the edited
    /// tables; compiled ones patch only the File slot and Dt1Mask fields of
    /// the `.bin` and mirror them on the `.txt` forms that hold the rows.
    fn save_registration(
        &self,
        r: &Registration,
        ctx: &MapCtx,
        edited: (&TxtDoc, &TxtDoc),
        compiled: bool,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        if !compiled {
            if self
                .read_txt("lvltypes.txt")?
                .is_some_and(|d| d != *edited.0)
            {
                self.write_saved(&excel("lvltypes.txt"), &edited.0.to_bytes(), log)?;
            }
            if self
                .read_txt("lvlprest.txt")?
                .is_some_and(|d| d != *edited.1)
            {
                self.write_saved(&excel("lvlprest.txt"), &edited.1.to_bytes(), log)?;
            }
            return Ok(());
        }
        let patch = |file: &str,
                     kind: ExcelTableKind,
                     edits: Vec<(usize, String, String)>,
                     log: &mut Vec<String>|
         -> Result<(), String> {
            if edits.is_empty() {
                return Ok(());
            }
            let path = excel(file);
            let mut bytes = self
                .overlay
                .read_bytes(&path)
                .map_err(err_text)?
                .ok_or_else(|| format!("{file} is missing"))?;
            for (rec, col, value) in edits {
                bytes = patch_bin_field(kind, &bytes, rec, &col, &value)
                    .map_err(|e| format!("{file}: {e}"))?;
            }
            self.write_saved(&path, &bytes, log)
        };
        let mut type_edits = Vec::new();
        if r.slot_added {
            type_edits.push((
                ctx.type_id as usize,
                format!("File {}", r.slot),
                r.slot_value.clone(),
            ));
        }
        patch("lvltypes.bin", ExcelTableKind::LvlTypes, type_edits, log)?;
        let prest_edits = r
            .masks
            .iter()
            .map(|(row, mask)| (*row, "Dt1Mask".to_owned(), mask.to_string()))
            .collect();
        patch("lvlprest.bin", ExcelTableKind::LvlPrest, prest_edits, log)?;
        if let (Some(mut t), Some(mut p)) = (
            self.read_txt("lvltypes.txt")?,
            self.read_txt("lvlprest.txt")?,
        ) {
            let (t0, p0) = (t.clone(), p.clone());
            reg::mirror_registration(&mut t, &mut p, ctx.type_id, &ctx.rel, ctx.def, r);
            if t != t0 {
                self.write_saved(&excel("lvltypes.txt"), &t.to_bytes(), log)?;
            }
            if p != p0 {
                self.write_saved(&excel("lvlprest.txt"), &p.to_bytes(), log)?;
            }
        }
        Ok(())
    }

    fn map_ctx(&self) -> Result<MapCtx, String> {
        let o = self.open.as_ref().ok_or(
            "No map is open: open one with open_map or new_map, or give dt1 to work on a tile library.",
        )?;
        let info = o.info.as_ref().ok_or(
            "The open map has no known level type, so it has no own DT1 (give level_type_id to open_map, or give dt1).",
        )?;
        let type_id = info
            .lvl_type
            .ok_or("The open map has no level type, so there is nowhere to register tiles.")?;
        Ok(MapCtx {
            path: o.path.clone(),
            rel: tiles_rel(o.path.as_str()),
            type_id,
            def: info.lvlprest_def,
        })
    }

    /// The open map's own-tiles DT1 (one per level type, `<type>_custom.dt1`).
    fn own_dt1_of_map(&self) -> Result<GamePath, String> {
        let ctx = self.map_ctx()?;
        let tables = self.tile_tables()?;
        Ok(tiles_game(&reg::own_path_for(
            &tables.types,
            ctx.type_id,
            ctx.path.as_str(),
        )))
    }

    /// `dt1` when given, else the open map's own DT1.
    fn dt1_arg(&self, v: Option<&Value>) -> Result<GamePath, String> {
        match v.filter(|v| !v.is_null()) {
            Some(v) => dt1_path(Some(v)),
            None => self.own_dt1_of_map(),
        }
    }

    /// Whether the open map loads `path`.
    fn map_loads(&self, path: &GamePath) -> bool {
        self.open
            .as_ref()
            .and_then(|o| o.info.as_ref())
            .is_some_and(|i| i.dt1_files.contains(path))
    }

    /// Re-reads the open map's level type (its DT1 list may have grown) and
    /// drops the loaded tile set, so list_tiles, paint checks, check_map and
    /// render_map see new tiles.
    fn refresh_tiles(&mut self) {
        let Some(o) = self.open.as_ref() else {
            return;
        };
        let path = o.path.clone();
        let (lt, def) = o
            .info
            .as_ref()
            .map_or((None, None), |i| (i.lvl_type, i.lvlprest_def));
        let info = self
            .catalog(std::slice::from_ref(&path))
            .ok()
            .and_then(|c| match lt {
                Some(l) => c.resolve_explicit(&path, l, def).ok(),
                None => c.best(&path),
            });
        if let Some(o) = self.open.as_mut() {
            if info.is_some() {
                o.info = info;
            }
            o.tiles = None;
        }
    }

    /// The tile numbers of the DT1s the open map loads other than `path`
    /// (none when the map does not load `path`).
    fn other_loaded_ids(&self, path: &GamePath) -> Result<BTreeSet<TileId>, String> {
        let mut out = BTreeSet::new();
        if !self.map_loads(path) {
            return Ok(out);
        }
        let files = self
            .open
            .as_ref()
            .and_then(|o| o.info.as_ref())
            .map(|i| i.dt1_files.clone())
            .unwrap_or_default();
        for f in files.iter().filter(|f| *f != path) {
            if let Some(d) = self.read_dt1(f).ok().flatten() {
                out.extend(tile_ids(&d));
            }
        }
        Ok(out)
    }

    /// Every DT1 the level type lists, parsed.
    fn type_libs(&self, types: &TxtDoc, type_id: u32) -> Result<Vec<(String, Dt1)>, String> {
        let mut out = Vec::new();
        for rel in reg::type_files(types, type_id) {
            if let Some(b) = self
                .overlay
                .read_bytes(&tiles_game(&rel))
                .map_err(err_text)?
                && let Ok(d) = Dt1::parse(&b)
            {
                out.push((rel, d));
            }
        }
        Ok(out)
    }

    /// Lists `rel` in the open map's DS1 file list (one undo step).
    fn list_in_map(&mut self, files: Vec<Vec<u8>>, label: &str) -> Result<bool, String> {
        let o = self.open_mut()?;
        if o.doc.ds1().files == files {
            return Ok(false);
        }
        let before = o.doc.ds1().clone();
        let mut after = before.clone();
        after.files = files;
        o.doc
            .apply_patch(Patch::replace(label, before, after))
            .map_err(err_text)?;
        Ok(true)
    }

    fn palette_act(&mut self, args: &Value) -> Result<u8, String> {
        if let Some(a) = opt_int(args.get("act"), "act", 1, 5)? {
            return Ok((a - 1) as u8);
        }
        Ok(self.open.as_ref().map_or(0, |o| {
            o.info
                .as_ref()
                .map_or(o.doc.ds1().effective_act(), |i| i.palette_act)
        }))
    }

    /// Puts built tiles into `dt1` (an own DT1) or the open map's own DT1 and
    /// reports what happened.
    fn store(&mut self, args: &Value, built: Vec<BuiltTile>, what: &str) -> ToolResult {
        let number = parse_number(args)?;
        let mut warnings: Vec<String> = Vec::new();
        for b in &built {
            if let Some(w) = b.warning() {
                let at = b.dropped_bounds.map_or(String::new(), |r| {
                    format!(
                        " (outside the area: x {}..{}, y {}..{})",
                        r.x,
                        r.x + r.width as i32 - 1,
                        r.y,
                        r.y + r.height as i32 - 1
                    )
                });
                warnings.push(format!("orientation {}: {w}{at}", b.tile.orientation));
            }
            if b.tile.blocks.is_empty() {
                warnings.push(format!(
                    "orientation {}: the tile is blank (no pixels)",
                    b.tile.orientation
                ));
            }
        }
        let mut tiles: Vec<Tile> = built.into_iter().map(|b| b.tile).collect();
        let Some(first) = tiles.first() else {
            return Err("nothing was built".into());
        };
        let orientations = tc::number_orientations(first.orientation);
        let mut lines = Vec::new();
        let mut written = Vec::new();
        let ids: Vec<TileId>;
        match args.get("dt1").filter(|v| !v.is_null()) {
            Some(v) => {
                let path = dt1_path(Some(v))?;
                self.require_own(&path)?;
                let mut dt1 = self
                    .read_dt1(&path)?
                    .ok_or_else(|| format!("{path} is not in the profile"))?;
                let taken = tile_ids(&dt1);
                // A library the open map loads also avoids the numbers of the
                // map's other DT1s (else the game mixes them by rarity).
                let others = self.other_loaded_ids(&path)?;
                let all: BTreeSet<TileId> = taken.union(&others).copied().collect();
                let (m, s) = choose(&number, &all, &orientations)?;
                for t in &mut tiles {
                    t.main_index = m;
                    t.sub_index = s;
                    let id = TileId::of(t);
                    if taken.contains(&id) {
                        warnings.push(format!(
                            "{path} already has a tile {}: this one is another version of that number (the game picks one by rarity)",
                            key_text(id)
                        ));
                    } else if others.contains(&id) {
                        warnings.push(format!(
                            "another DT1 the open map loads already has a tile {}: the game mixes them by rarity",
                            key_text(id)
                        ));
                    }
                }
                let at = dt1.tiles.len();
                dt1.tiles.extend(tiles.iter().cloned());
                let bytes = dt1
                    .to_bytes()
                    .map_err(|e| format!("cannot write the DT1: {e}"))?;
                self.write_saved(&path, &bytes, &mut written)?;
                ids = tiles.iter().map(TileId::of).collect();
                lines.push(format!(
                    "{what} {} tile(s) into {path} as #{at}{} ({} tiles now).",
                    tiles.len(),
                    if tiles.len() > 1 {
                        format!("-#{}", at + tiles.len() - 1)
                    } else {
                        String::new()
                    },
                    dt1.tiles.len()
                ));
                if self.map_loads(&path) {
                    self.refresh_tiles();
                } else if self.open.is_some() {
                    warnings.push(format!(
                        "The open map does not load {path} (its level type does not list it), so its tiles cannot be placed there yet: new_dt1 with register makes a library the map loads, or leave dt1 out to use the map's own DT1"
                    ));
                }
            }
            None => {
                let ctx = self.map_ctx()?;
                let tables = self.tile_tables()?;
                let libs = self.type_libs(&tables.types, ctx.type_id)?;
                let own_rel = reg::own_path_for(&tables.types, ctx.type_id, ctx.path.as_str());
                let own_game = tiles_game(&own_rel);
                let existing = self.read_dt1(&own_game)?;
                let mut taken = BTreeSet::new();
                for (_, d) in &libs {
                    taken.extend(tile_ids(d));
                }
                if let Some(e) = &existing {
                    taken.extend(tile_ids(e));
                }
                let (m, s) = choose(&number, &taken, &orientations)?;
                for t in &mut tiles {
                    t.main_index = m;
                    t.sub_index = s;
                }
                let type_dt1s: Vec<(String, &Dt1)> =
                    libs.iter().map(|(p, d)| (p.clone(), d)).collect();
                let ds1 = self.open_mut()?.doc.ds1().clone();
                let out = tc::add_new_tiles_to_map(
                    &MapTarget {
                        map_path: ctx.path.as_str(),
                        ds1: &ds1,
                        types: &tables.types,
                        prest: &tables.prest,
                        type_id: ctx.type_id,
                        prest_def: ctx.def,
                        type_dt1s: &type_dt1s,
                        existing_own: existing.as_ref(),
                        existing_own_path: Some(&own_rel),
                    },
                    &tiles,
                )
                .map_err(err_text)?;
                let bytes = out
                    .dt1
                    .to_bytes()
                    .map_err(|e| format!("cannot write the DT1: {e}"))?;
                self.write_saved(&tiles_game(&out.own_path), &bytes, &mut written)?;
                self.save_registration(
                    &out.registration,
                    &ctx,
                    (&out.types, &out.prest),
                    tables.compiled,
                    &mut written,
                )?;
                lines.push(format!(
                    "{what} {} tile(s) into the map's own DT1:",
                    tiles.len()
                ));
                lines.extend(out.changes.iter().map(|c| format!("- {c}")));
                if self.list_in_map(out.ds1.files.clone(), "Add new tiles to this map")? {
                    lines.push(
                        "The map's DS1 now lists the DT1 (an undo step; save_map keeps it).".into(),
                    );
                }
                warnings.extend(out.warnings.iter().cloned());
                let placed: BTreeMap<TileId, TileId> = out.placed.clone();
                ids = tiles
                    .iter()
                    .map(|t| {
                        let id = TileId::of(t);
                        placed.get(&id).copied().unwrap_or(id)
                    })
                    .collect();
                self.refresh_tiles();
            }
        }
        let mut seen = BTreeSet::new();
        for id in &ids {
            if seen.insert(*id) {
                lines.push(format!(
                    "Tile {}: orientation {}, main {}, sub {} - paint it on {}.",
                    key_text(*id),
                    id.orientation,
                    id.main,
                    id.sub,
                    layer_hint(id.orientation)
                ));
            }
        }
        lines.push("Files written:".into());
        lines.extend(written.iter().map(|w| format!("- {w}")));
        if !warnings.is_empty() {
            lines.push("Warnings:".into());
            lines.extend(warnings.iter().map(|w| format!("- {w}")));
        }
        text(lines.join("\n"))
    }

    // -------------------------------------------------------------- tools

    pub(super) fn tile_template(&mut self, args: &Value) -> ToolResult {
        let s = parse_shape(args, "")?;
        let t = picture_template(&s)?;
        let r = t.rect;
        let o = s.shape.orientation;
        let blocks = if s.pair {
            let a = tc::template(TileShape::wall(3, s.shape.height)).map_err(err_text)?;
            let b = tc::template(TileShape::wall(4, s.shape.height)).map_err(err_text)?;
            a.cells.len() + b.cells.len()
        } else {
            t.cells.len()
        };
        let mut lines = vec![
            format!(
                "Shape: orientation {o} ({}){}, {} (stored height {}).",
                orientation_name(u32::from(o)),
                if s.pair { " with its partner half" } else { "" },
                shape_text(&s.shape),
                s.shape.stored_height()
            ),
            format!(
                "Picture: {}x{} pixels. Pixel (0,0) of create_tile's grid is anchor ({}, {}); pixels go row by row, one palette index per byte, index 0 transparent.",
                r.width, r.height, r.x, r.y
            ),
        ];
        let mask = t.mask();
        let w = mask.width as usize;
        let span = |row: usize| {
            let cols: Vec<usize> = (0..w)
                .filter(|c| mask.alpha.get(row * w + c).is_some_and(|a| *a != 0))
                .collect();
            match (cols.first(), cols.last()) {
                (Some(a), Some(b)) => format!("row {row}: x {a}-{b}"),
                _ => format!("row {row}: empty"),
            }
        };
        lines.push(match s.shape.class() {
            Some(c) if c.is_diamond() => format!(
                "Drawable: the isometric diamond only ({}; {}; {}), split into 25 sub-diamonds of 32x15.",
                span(0),
                span(39),
                span(78)
            ),
            _ if s.pair => "Drawable: the whole box; x 0-79 becomes orientation 4, x 80-159 orientation 3 (32x32 sub-blocks).".into(),
            _ if o == 3 => "Drawable: x 80-159 of the box only (the left half is orientation 4); 32x32 sub-blocks.".into(),
            _ if o == 4 => "Drawable: x 0-79 of the box only (the right half is orientation 3); 32x32 sub-blocks.".into(),
            Some(TileClass::LowerWall) => "Drawable: the whole box; the anchor is 96 pixels below its top (32x32 sub-blocks).".into(),
            _ => "Drawable: the whole box; its bottom edge is the wall's base (32x32 sub-blocks).".into(),
        });
        lines.push(format!("Sub-blocks when full: {blocks}."));
        lines.push(format!(
            "Default sub-tile flags (rows y0-y4, x0-x4): {}.",
            flags_text(&tc::default_flags(i32::from(o)))
        ));
        let mut out = Vec::new();
        match args.get("mask").and_then(Value::as_str).unwrap_or("none") {
            "none" => {}
            "base64" => lines.push(format!(
                "Mask ({}x{} bytes, 1 = drawable): {}",
                mask.width,
                mask.height,
                base64(
                    &mask
                        .alpha
                        .iter()
                        .map(|a| u8::from(*a != 0))
                        .collect::<Vec<_>>()
                )
            )),
            "png" => {
                let px: Vec<u8> = mask
                    .alpha
                    .iter()
                    .flat_map(|a| {
                        if *a != 0 {
                            [255, 255, 255, 255]
                        } else {
                            [0, 0, 0, 0]
                        }
                    })
                    .collect();
                let png = capy_render::RgbaImage {
                    width: mask.width,
                    height: mask.height,
                    pixels: px,
                }
                .to_png()
                .map_err(err_text)?;
                out.push(Content::Image(png));
            }
            other => {
                return Err(format!(
                    "mask {other:?}: use \"none\", \"base64\" or \"png\"."
                ));
            }
        }
        out.insert(0, Content::Text(lines.join("\n")));
        Ok(out)
    }

    pub(super) fn create_tile(&mut self, args: &Value) -> ToolResult {
        let s = parse_shape(args, "")?;
        let opts = parse_build_options(args)?;
        let t = picture_template(&s)?;
        let img = picture(args, &t)?;
        let built = build(&img, &s, &opts)?;
        self.store(args, built, "Created")
    }

    /// The image of `import_tile_image`: `path` (a file) or `data` (base64).
    fn image_bytes(args: &Value) -> Result<(Vec<u8>, ImageKind), String> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());
        let data = args.get("data").and_then(Value::as_str);
        let (bytes, ext) = match (path, data) {
            (Some(p), None) => {
                let p = PathBuf::from(p.trim());
                let meta = std::fs::metadata(&p)
                    .map_err(|e| format!("{}: cannot read: {e}", p.display()))?;
                if !meta.is_file() {
                    return Err(format!("{} is not a file.", p.display()));
                }
                if meta.len() > MAX_IMAGE_BYTES {
                    return Err(format!(
                        "{} is {} bytes; at most {MAX_IMAGE_BYTES}.",
                        p.display(),
                        meta.len()
                    ));
                }
                let ext = p.extension().and_then(|e| e.to_str()).map(str::to_owned);
                if ext.as_deref().and_then(ImageKind::from_extension).is_none() {
                    return Err(format!(
                        "{}: only .png, .bmp, .gif and .pcx pictures are read.",
                        p.display()
                    ));
                }
                (
                    std::fs::read(&p).map_err(|e| format!("{}: cannot read: {e}", p.display()))?,
                    ext,
                )
            }
            (None, Some(d)) => {
                if d.len() as u64 > MAX_IMAGE_BYTES / 3 * 4 + 4 {
                    return Err(format!("data is over {MAX_IMAGE_BYTES} bytes."));
                }
                (
                    base64_decode(d).map_err(|e| format!("data is not valid base64: {e}."))?,
                    None,
                )
            }
            _ => {
                return Err(
                    "Give path (an image file) or data (base64 image bytes), not both.".into(),
                );
            }
        };
        let kind = match args.get("format").and_then(Value::as_str) {
            Some(f) => ImageKind::from_extension(f)
                .ok_or_else(|| format!("format {f:?}: use png, bmp, gif or pcx."))?,
            None => match ext.as_deref().and_then(ImageKind::from_extension) {
                Some(k) => k,
                None if bytes.starts_with(b"\x89PNG") => ImageKind::Png,
                None if bytes.starts_with(b"BM") => ImageKind::Bmp,
                None if bytes.starts_with(b"GIF8") => ImageKind::Gif,
                None if bytes.first() == Some(&0x0A) => ImageKind::Pcx,
                None => return Err("The picture is not PNG, BMP, GIF or PCX (give format).".into()),
            },
        };
        Ok((bytes, kind))
    }

    pub(super) fn import_tile_image(&mut self, args: &Value) -> ToolResult {
        let s = parse_shape(args, "")?;
        let opts = parse_build_options(args)?;
        let placement = parse_placement(args.get("placement"))?;
        let (bytes, kind) = Self::image_bytes(args)?;
        let frame =
            import_image(&bytes, kind).map_err(|e| format!("the picture cannot be read: {e}"))?;
        if frame.width > MAX_IMAGE_SIDE || frame.height > MAX_IMAGE_SIDE {
            return Err(format!(
                "The picture is {}x{}; at most {MAX_IMAGE_SIDE} pixels on a side (a tile is 160 wide).",
                frame.width, frame.height
            ));
        }
        let dither = match args.get("dither").and_then(Value::as_str).unwrap_or("none") {
            "none" => Dither::None,
            "floyd_steinberg" => Dither::FloydSteinberg,
            other => {
                return Err(format!(
                    "dither {other:?}: use \"none\" or \"floyd_steinberg\"."
                ));
            }
        };
        let transparent_rgb = match args.get("transparent_rgb") {
            None => Some([0, 0, 0]),
            Some(Value::Null) => None,
            Some(v) => {
                let a = v
                    .as_array()
                    .filter(|a| a.len() == 3)
                    .ok_or("transparent_rgb must be [r, g, b] or null (alpha only).")?;
                let c = |i: usize| int(a.get(i), "transparent_rgb", 0, 255).map(|n| n as u8);
                Some([c(0)?, c(1)?, c(2)?])
            }
        };
        let quantize = QuantizeOptions {
            dither,
            transparent_rgb,
            alpha_threshold: opt_int(args.get("alpha_threshold"), "alpha_threshold", 0, 255)?
                .unwrap_or(128) as u8,
            ..QuantizeOptions::default()
        };
        let act = self.palette_act(args)?;
        let palette = self.palette(act)?;
        let place_shape = picture_template(&s)?.shape;
        let img = tc::import_tile_image(
            &bytes,
            kind,
            &palette,
            place_shape,
            &ImportOptions {
                quantize,
                placement,
            },
        )
        .map_err(err_text)?;
        let built = build(&img, &s, &opts)?;
        self.store(
            args,
            built,
            &format!(
                "Imported a {}x{} picture (act {} palette) as",
                frame.width,
                frame.height,
                act + 1
            ),
        )
    }

    pub(super) fn reshape_tile(&mut self, args: &Value) -> ToolResult {
        let path = self.dt1_arg(args.get("dt1"))?;
        self.require_own(&path)?;
        let mut dt1 = self
            .read_dt1(&path)?
            .ok_or_else(|| format!("{path} is not in the profile"))?;
        let i = pick_tile(&dt1, args, &path)?;
        let old = dt1.tiles[i].clone();
        let cur = TileShape::of_tile(&old).ok_or_else(|| {
            format!(
                "tile #{i} has orientation {}, not a tile shape",
                old.orientation
            )
        })?;
        let mut a = Map::new();
        let pick =
            |k: &str, cur: Value| args.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(cur);
        a.insert(
            "new_orientation".into(),
            pick("new_orientation", json!(cur.orientation)),
        );
        a.insert(
            "new_height".into(),
            pick(
                "new_height",
                if cur.height > 0 {
                    json!(cur.height)
                } else {
                    Value::Null
                },
            ),
        );
        a.insert(
            "new_roof_height".into(),
            pick(
                "new_roof_height",
                if cur.roof_height != 0 {
                    json!(cur.roof_height)
                } else {
                    Value::Null
                },
            ),
        );
        let s = parse_shape(&Value::Object(a), "new_")?;
        let built = tc::reshape_tile(&old, s.shape).map_err(err_text)?;
        let mut warnings: Vec<String> = built.warning().into_iter().collect();
        let (old_id, new_id) = (TileId::of(&old), TileId::of(&built.tile));
        if old_id != new_id {
            if dt1
                .tiles
                .iter()
                .enumerate()
                .any(|(j, t)| j != i && TileId::of(t) == new_id)
            {
                warnings.push(format!(
                    "{path} already has a tile {}: this one becomes another version of it",
                    key_text(new_id)
                ));
            }
            if let Some(o) = &self.open {
                let n = cells_using(o.doc.ds1(), old_id).len();
                if n > 0 && self.map_loads(&path) {
                    warnings.push(format!(
                        "{n} cell(s) of the open map use {}, which no longer exists in this DT1 under that orientation",
                        key_text(old_id)
                    ));
                }
            }
        }
        let summary = format!(
            "Reshaped #{i} {} from {} to {} {}: {} block(s).",
            key_text(old_id),
            shape_text(&cur),
            key_text(new_id),
            shape_text(&s.shape),
            built.tile.blocks.len()
        );
        dt1.tiles[i] = built.tile;
        let bytes = dt1
            .to_bytes()
            .map_err(|e| format!("cannot write the DT1: {e}"))?;
        let mut written = Vec::new();
        self.write_saved(&path, &bytes, &mut written)?;
        if self.map_loads(&path) {
            self.refresh_tiles();
        }
        let mut lines = vec![summary, "Files written:".into()];
        lines.extend(written.iter().map(|w| format!("- {w}")));
        if !warnings.is_empty() {
            lines.push("Warnings:".into());
            lines.extend(warnings.iter().map(|w| format!("- {w}")));
        }
        text(lines.join("\n"))
    }

    pub(super) fn new_dt1(&mut self, args: &Value) -> ToolResult {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let register = args
            .get("register")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let folder = match args.get("folder").and_then(Value::as_str).map(str::trim) {
            Some(f) if !f.is_empty() => {
                let f = tiles_rel(f);
                f.trim_end_matches('/').to_owned()
            }
            _ => {
                let own = tiles_rel(
                    self.own_dt1_of_map()
                        .map_err(|e| format!("Give folder (e.g. act1/outdoors): {e}"))?
                        .as_str(),
                );
                own.rsplit_once('/')
                    .map_or(String::new(), |(d, _)| d.to_owned())
            }
        };
        if folder.is_empty() || !folder.split('/').all(is_safe_name) {
            return Err(format!(
                "folder {folder:?}: give a folder under data/global/tiles made of letters, digits and _ (e.g. act1/outdoors)."
            ));
        }
        if let Some(problem) = custom_name_problem(&name, &folder) {
            return Err(format!("name {name:?}: {problem}"));
        }
        let rel = format!("{folder}/{name}.dt1");
        let game = tiles_game(&rel);
        if self.overlay.resolve(&game).is_some() {
            return Err(format!(
                "{game} already exists in this profile; choose another name."
            ));
        }
        let ctx = if register {
            Some(self.map_ctx()?)
        } else {
            None
        };
        let bytes = tc::new_dt1()
            .to_bytes()
            .map_err(|e| format!("cannot write the DT1: {e}"))?;
        let mut written = Vec::new();
        self.write_saved(&game, &bytes, &mut written)?;
        let mut lines = vec![format!("Created the empty DT1 {game}.")];
        let mut warnings = Vec::new();
        if let Some(ctx) = ctx {
            let tables = self.tile_tables()?;
            let (mut types, mut prest) = (tables.types.clone(), tables.prest.clone());
            let rows = reg::map_prest_rows(&prest, &ctx.rel, ctx.def);
            let r = reg::register_dt1(&mut types, &mut prest, ctx.type_id, &rows, &rel)
                .map_err(err_text)?;
            self.save_registration(&r, &ctx, (&types, &prest), tables.compiled, &mut written)?;
            lines.extend(r.changes.iter().map(|c| format!("- {c}")));
            warnings.extend(r.warnings.iter().cloned());
            let mut ds1 = self.open_mut()?.doc.ds1().clone();
            Edit::AddDt1s {
                paths: vec![rel.clone()],
            }
            .apply_ds1(&mut ds1);
            if self.list_in_map(ds1.files, "Add a DT1 to this map")? {
                lines.push(
                    "The map's DS1 now lists the DT1 (an undo step; save_map keeps it).".into(),
                );
            }
            self.refresh_tiles();
        }
        lines.push(format!(
            "Add tiles to it with create_tile or import_tile_image and dt1 = {rel:?}."
        ));
        lines.push("Files written:".into());
        lines.extend(written.iter().map(|w| format!("- {w}")));
        if !warnings.is_empty() {
            lines.push("Warnings:".into());
            lines.extend(warnings.iter().map(|w| format!("- {w}")));
        }
        text(lines.join("\n"))
    }

    pub(super) fn list_own_tiles(&mut self, args: &Value) -> ToolResult {
        let path = self.dt1_arg(args.get("dt1"))?;
        let limit = opt_int(args.get("limit"), "limit", 1, 100_000)?.unwrap_or(500) as usize;
        let Some(dt1) = self.read_dt1(&path)? else {
            return text(format!(
                "{path} does not exist yet (create_tile without dt1 makes the open map's own DT1)."
            ));
        };
        let mut lines = vec![format!(
            "{path}: {} tile(s); {}; {}.",
            dt1.tiles.len(),
            if self.is_own(&path) {
                "in the profile's saves (writable)"
            } else {
                "not in the profile's saves (read only here)"
            },
            if self.map_loads(&path) {
                "the open map loads it"
            } else {
                "the open map does not load it"
            }
        )];
        lines.extend(
            dt1.tiles
                .iter()
                .enumerate()
                .take(limit)
                .map(|(i, t)| describe_tile(i, t)),
        );
        if dt1.tiles.len() > limit {
            lines.push(format!("... {} more", dt1.tiles.len() - limit));
        }
        text(lines.join("\n"))
    }

    pub(super) fn inspect_tile(&mut self, args: &Value) -> ToolResult {
        let path = self.dt1_arg(args.get("dt1"))?;
        let dt1 = self
            .read_dt1(&path)?
            .ok_or_else(|| format!("{path} is not in the profile"))?;
        let i = pick_tile(&dt1, args, &path)?;
        let t = &dt1.tiles[i];
        let mut formats: BTreeMap<i16, usize> = BTreeMap::new();
        for b in &t.blocks {
            *formats.entry(b.raw_format).or_insert(0) += 1;
        }
        let mut lines = vec![
            format!("{path} {}", describe_tile(i, t)),
            format!(
                "Header: direction {}, stored height {}, width {}, roof height {}, animated {}, sound {}, rarity {}.",
                t.direction, t.height, t.width, t.roof_height, t.animated, t.sound, t.rarity
            ),
            format!(
                "Sub-tile flags (rows y0-y4): {}.",
                flags_text(&t.subtile_flags)
            ),
            format!(
                "Blocks by format: {}.",
                if formats.is_empty() {
                    "none".to_owned()
                } else {
                    formats
                        .iter()
                        .map(|(f, n)| format!("{f:#06x} x{n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
        ];
        let mode = args
            .get("render")
            .and_then(Value::as_str)
            .unwrap_or("image");
        if !matches!(mode, "none" | "image" | "file") {
            return Err(format!(
                "render {mode:?}: use \"image\", \"file\" or \"none\"."
            ));
        }
        let mut out = Vec::new();
        if mode != "none" {
            let img = t.render().map_err(err_text)?;
            lines.push(format!(
                "Picture: {}x{} at anchor ({}, {}).",
                img.width, img.height, img.offset_x, img.offset_y
            ));
            if img.width == 0 || img.height == 0 {
                lines.push("The tile has no pixels to show.".into());
            } else {
                let act = self.palette_act(args)?;
                let palette = self.palette(act)?;
                let png = capy_render::RgbaImage {
                    width: img.width,
                    height: img.height,
                    pixels: img.to_rgba(&palette),
                }
                .to_png()
                .map_err(err_text)?;
                if mode == "image" {
                    out.push(Content::Image(png));
                } else {
                    let name = match args.get("name").and_then(Value::as_str) {
                        Some(n) => n.trim().to_owned(),
                        None => format!(
                            "tile_o{}_m{}_s{}_{i}",
                            t.orientation, t.main_index, t.sub_index
                        ),
                    };
                    if !is_safe_name(&name) {
                        return Err(format!(
                            "name {name:?}: use only letters, digits and _ (the file goes to capybara\\renders\\<name>.png)."
                        ));
                    }
                    let dir = self.root.join("renders");
                    std::fs::create_dir_all(&dir)
                        .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
                    let file = dir.join(format!("{name}.png"));
                    capy_history::atomic_write(&file, &png)
                        .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
                    lines.push(format!("Wrote {} ({} bytes).", file.display(), png.len()));
                }
            }
        }
        out.insert(0, Content::Text(lines.join("\n")));
        Ok(out)
    }
}

// ------------------------------------------------------------------ schemas

pub(super) fn tools_json() -> Value {
    let t = |name: &str, description: &str, props: Value, required: &[&str]| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":props,"required":required}});
    let shape = |mut p: Map<String, Value>| {
        p.insert("orientation".into(), json!({"type":"integer","description":"0 floor, 1-9/12/14 walls (1 left, 2 right, 3/4 north corner halves, 7 south corner, 8/9 doors, 12 pillar, 14 tree), 10/11 special, 13 shadow, 15 roof, 16-19 lower walls."}));
        p.insert("height".into(), json!({"type":"integer","description":"Walls, shadows, specials, lower walls: picture height in pixels (rounded up to 32, at most 2016)."}));
        p.insert("roof_height".into(), json!({"type":"integer","description":"Roofs (15): how far up the game lifts the roof (default 80)."}));
        p.insert("corner_pair".into(), json!({"type":"boolean","description":"Orientation 3 or 4: build both north-corner halves from one 160-wide picture (x 0-79 -> 4, x 80-159 -> 3), numbered together."}));
        p
    };
    let target = |mut p: Map<String, Value>| {
        p.insert("dt1".into(), json!({"type":"string","description":"An own DT1 (in the profile's saves, e.g. from new_dt1) to add to, as data/global/tiles/... or relative to it. Leave out to add to the open map's own DT1 (<type>_custom.dt1): renumbered past every DT1 of its level type, registered in LvlTypes/LvlPrest and listed in the map."}));
        p.insert("main".into(), json!({"type":"integer","description":"0-63: wanted main index (with sub: the exact number; may be renumbered when the map's DT1s have it)."}));
        p.insert(
            "sub".into(),
            json!({"type":"integer","description":"0-255, with main."}),
        );
        p.insert("flags".into(), json!({"type":"array","items":{"type":"integer"},"minItems":25,"maxItems":25,"description":"Sub-tile flags, y * 5 + x (default: 0x07 where a wall stands)."}));
        p.insert("rarity".into(), json!({"type":"integer"}));
        p.insert("sound".into(), json!({"type":"integer"}));
        p
    };
    let placement = json!({"type":"string","enum":["as_is","center","fit"],"description":"as_is (default): top-left on the drawable box's top-left; center; fit: scaled to fit (aspect kept), centred."});
    let selector = |mut p: Map<String, Value>| {
        p.insert(
            "dt1".into(),
            json!({"type":"string","description":"DT1 path (default: the open map's own DT1)."}),
        );
        p.insert("index".into(), json!({"type":"integer","description":"Tile position in the file (instead of orientation/main/sub)."}));
        p.insert("orientation".into(), json!({"type":"integer"}));
        p.insert("main".into(), json!({"type":"integer"}));
        p.insert("sub".into(), json!({"type":"integer"}));
        p.insert(
            "variant".into(),
            json!({"type":"integer","description":"Which tile of that number (default 0)."}),
        );
        p
    };
    let obj = |v: Value| match v {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    json!([
        t(
            "tile_template",
            "The drawable area of a tile shape, so art can be drawn to fit: picture size, where its (0,0) is in tile space, what is drawable, sub-block count, default flags. mask: \"base64\" (1 byte per pixel, 1 = drawable) or \"png\".",
            Value::Object(shape(obj(
                json!({"mask":{"type":"string","enum":["none","base64","png"]}})
            ))),
            &["orientation"]
        ),
        t(
            "create_tile",
            "Create a DT1 tile from a palette-indexed pixel grid (or blank, or a fill colour) and add it to the open map's own DT1 or an own DT1. pixels is in the plain top-left space of tile_template: {data: base64 bytes, width, height}, row by row; index 0 (or transparent_index) is empty. Returns the numbers the tile got, the files written and warnings (pixels outside the drawable area, opaque index-0 pixels). Paint it afterwards with paint.",
            Value::Object(target(shape(obj(json!({
                "pixels":{"type":"object","description":"The picture: data = base64 of width*height palette indices, row by row from the top-left of tile_template's picture (at most 2048 on a side).","properties":{"data":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"}},"required":["data","width","height"]},
                "placement":placement,
                "transparent_index":{"type":"integer","description":"Grid index meaning empty (default 0)."},
                "fill":{"type":"integer","description":"1-255: fill the drawable area with this index first."}
            }))))),
            &["orientation"]
        ),
        t(
            "import_tile_image",
            "Create a tile from a PNG/BMP/GIF/PCX picture (a file path or base64 data): placed, quantized to the map's act palette (or act) and added like create_tile.",
            Value::Object(target(shape(obj(json!({
                "path":{"type":"string","description":"Image file (.png .bmp .gif .pcx, at most 16 MB)."},
                "data":{"type":"string","description":"base64 image bytes (instead of path)."},
                "format":{"type":"string","enum":["png","bmp","gif","pcx"]},
                "placement":placement,
                "dither":{"type":"string","enum":["none","floyd_steinberg"]},
                "transparent_rgb":{"description":"[r,g,b] treated as transparent (default black), or null for alpha only."},
                "alpha_threshold":{"type":"integer","description":"Alpha below this is transparent (default 128)."},
                "act":{"type":"integer","description":"1-5: palette (default: the map's)."}
            }))))),
            &["orientation"]
        ),
        t(
            "reshape_tile",
            "Change the shape of a tile in an own DT1: another wall height, orientation, floor <-> roof. Pixels stay in place in tile space; what the new area cannot hold is reported.",
            Value::Object(selector(obj(json!({
                "new_orientation":{"type":"integer"},"new_height":{"type":"integer"},"new_roof_height":{"type":"integer"}
            })))),
            &[]
        ),
        t(
            "new_dt1",
            "Create an empty own DT1 in the profile's saves. name: letters, digits, _ (checked against the game's path limit). folder: under data/global/tiles (default: the open map's own-DT1 folder). register: also register it for the open map's level type (LvlTypes File slot + LvlPrest Dt1Mask) and list it in the map.",
            json!({"name":{"type":"string"},"folder":{"type":"string"},"register":{"type":"boolean"}}),
            &["name"]
        ),
        t(
            "list_own_tiles",
            "The tiles of the open map's own DT1 (or dt1): number, kind, shape, rarity, sound, blocked sub-tiles, blocks.",
            json!({"dt1":{"type":"string"},"limit":{"type":"integer"}}),
            &[]
        ),
        t(
            "inspect_tile",
            "One tile of a DT1 in detail: header, flags, blocks, and a PNG (render \"image\" (default), \"file\" to write capybara\\renders\\<name>.png, or \"none\"), in the map's act palette or act.",
            Value::Object(selector(obj(json!({
                "render":{"type":"string","enum":["image","file","none"]},"name":{"type":"string"},"act":{"type":"integer"}
            })))),
            &[]
        ),
    ])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_and_refuses_junk() {
        for n in 0..40u8 {
            let bytes: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64(&bytes)).unwrap(), bytes);
        }
        assert_eq!(
            base64_decode("AQI").unwrap(),
            vec![1, 2],
            "padding optional"
        );
        assert!(base64_decode("not base64!").is_err());
        assert!(base64_decode("A").is_err());
        assert!(base64_decode("AA=A").is_err());
    }

    #[test]
    fn dt1_paths_stay_under_the_tiles_folder() {
        let p = |s: &str| dt1_path(Some(&json!(s)));
        assert_eq!(
            p("act1/outdoors/mine.dt1").unwrap().as_str(),
            "data\\global\\tiles\\act1\\outdoors\\mine.dt1"
        );
        assert_eq!(
            p("Data\\Global\\Tiles\\Act1\\x.DT1").unwrap().as_str(),
            "data\\global\\tiles\\act1\\x.dt1"
        );
        for bad in [
            "../x.dt1",
            "act1/../../x.dt1",
            "/etc/x.dt1",
            "c:/windows/x.dt1",
            "act1/x.ds1",
            "act1/a*.dt1",
            "act1/x .dt1/",
            "",
        ] {
            assert!(p(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn shapes_parse_and_refuse() {
        let s = parse_shape(&json!({"orientation":1,"height":70}), "").unwrap();
        assert_eq!((s.shape.orientation, s.shape.height), (1, 96));
        let s = parse_shape(&json!({"orientation":15}), "").unwrap();
        assert_eq!(s.shape.roof_height, tc::DEFAULT_ROOF_HEIGHT);
        let s = parse_shape(&json!({"orientation":4,"height":32,"corner_pair":true}), "").unwrap();
        assert!(s.pair);
        assert_eq!(picture_template(&s).unwrap().rect.width, 160);
        for bad in [
            json!({"orientation":20}),
            json!({"orientation":2}),
            json!({"orientation":2,"height":0}),
            json!({"orientation":0,"corner_pair":true}),
        ] {
            assert!(parse_shape(&bad, "").is_err(), "{bad}");
        }
    }
}
