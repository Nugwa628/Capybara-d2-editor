//! The editing session behind `capy mcp`: one open map at a time, edited with
//! the same `capy-edit` calls as the editor (so undo history works the same),
//! saved into `capybara\saves\<profile>` (+ `CHANGED.txt` + a history
//! snapshot) only when asked. Every tool takes plain JSON and returns text
//! (or a PNG for `render_map`).

use std::path::PathBuf;

use capy_edit::{
    Brush, CellRect, CellValue, Document, FloorChoice, LayerRef, NewMapSpec, PasteMode,
    RerollOptions, Rng, SelectOp, Selection, TileKey, check_objects, new_map,
};
use capy_history::{SavesDir, SnapshotKind, SnapshotStore};
use capy_render::{ObjectSprites, RenderOptions, TileSet, render_with_objects};
use d2_assets::objects::ObjectCatalog;
use d2_assets::{GamePath, MapCatalog, MapInfo, Overlay, Profile};
use d2_formats::ds1::{Cell, Ds1, Object};
use d2_formats::palette::{Palette, act_palette_path};
use serde_json::{Value, json};

use crate::{Ctx, names, overrides};

/// One piece of a tool result.
pub(crate) enum Content {
    Text(String),
    /// PNG bytes.
    Image(Vec<u8>),
}

pub(crate) type ToolResult = Result<Vec<Content>, String>;

fn text(t: impl Into<String>) -> ToolResult {
    Ok(vec![Content::Text(t.into())])
}

struct Open {
    path: GamePath,
    doc: Document,
    info: Option<MapInfo>,
    tiles: Option<TileSet>,
    /// Created by `new_map`, never saved yet.
    fresh: bool,
}

pub(crate) struct Session {
    root: PathBuf,
    profile: Profile,
    overlay: Overlay,
    saves: SavesDir,
    names: Option<Vec<GamePath>>,
    objects: Option<Option<ObjectCatalog>>,
    open: Option<Open>,
}

const LAYER_NAMES: [&str; 7] = [
    "floor1", "floor2", "wall1", "wall2", "wall3", "wall4", "shadow",
];

// ------------------------------------------------------------------ argument helpers

fn parse_layer(v: Option<&Value>) -> Result<LayerRef, String> {
    let name = v
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let bad = || {
        format!(
            "Unknown layer {name:?}. Use one of: {}.",
            LAYER_NAMES.join(", ")
        )
    };
    if name == "shadow" {
        return Ok(LayerRef::Shadow);
    }
    let (kind, n) = name.split_at(name.len().saturating_sub(1));
    let n: u8 = n.parse().map_err(|_| bad())?;
    match (kind, n) {
        ("floor", 1..=2) => Ok(LayerRef::Floor(n - 1)),
        ("wall", 1..=4) => Ok(LayerRef::Wall(n - 1)),
        _ => Err(bad()),
    }
}

fn int(v: Option<&Value>, name: &str, min: i64, max: i64) -> Result<i64, String> {
    let n = v
        .and_then(Value::as_i64)
        .filter(|n| (min..=max).contains(n));
    n.ok_or_else(|| {
        format!(
            "{name} must be a whole number from {min} to {max} (got {}).",
            v.map_or("nothing".into(), Value::to_string)
        )
    })
}

fn opt_int(v: Option<&Value>, name: &str, min: i64, max: i64) -> Result<Option<i64>, String> {
    v.filter(|v| !v.is_null())
        .map(|v| int(Some(v), name, min, max))
        .transpose()
}

fn parse_rect(v: Option<&Value>, ds1: &Ds1) -> Result<CellRect, String> {
    let o = v.ok_or("rect is required: {x0, y0, x1, y1}")?;
    let g = |k| int(o.get(k), k, 0, 9999).map(|n| n as u32);
    let (x0, y0, x1, y1) = (g("x0")?, g("y0")?, g("x1")?, g("y1")?);
    let (lx, hx, ly, hy) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    if lx >= ds1.width || ly >= ds1.height {
        return Err(format!(
            "The rectangle is outside the {}x{} map.",
            ds1.width, ds1.height
        ));
    }
    Ok(CellRect {
        x0: lx,
        y0: ly,
        x1: hx.min(ds1.width - 1),
        y1: hy.min(ds1.height - 1),
    })
}

fn parse_cells(v: Option<&Value>) -> Result<Vec<(u32, u32)>, String> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    arr.iter()
        .map(|c| {
            let a = c.as_array().filter(|a| a.len() == 2);
            let g = |i: usize| {
                a.and_then(|a| a[i].as_u64())
                    .and_then(|n| u32::try_from(n).ok())
            };
            g(0).zip(g(1))
                .ok_or_else(|| "cells must be [[x, y], ...]".to_owned())
        })
        .collect()
}

/// The orientation a tile on `layer` gets: floors 0, shadows 13, walls as given (default 1).
fn orientation_for(layer: LayerRef, o: &Value) -> Result<u32, String> {
    Ok(match layer {
        LayerRef::Floor(_) => 0,
        LayerRef::Shadow => 13,
        _ => opt_int(o.get("orientation"), "orientation", 1, 19)?.unwrap_or(1) as u32,
    })
}

fn parse_brush(layer: LayerRef, t: &Value) -> Result<Brush, String> {
    Ok(Brush {
        main: int(t.get("main"), "main", 0, 63)? as u8,
        sub: int(t.get("sub"), "sub", 0, 255)? as u8,
        orientation: orientation_for(layer, t)?,
    })
}

fn parse_key(layer: LayerRef, t: &Value) -> Result<TileKey, String> {
    let b = parse_brush(layer, t)?;
    Ok(TileKey {
        orientation: b.orientation,
        main: b.main,
        sub: b.sub,
    })
}

fn layer_name(l: LayerRef) -> String {
    match l {
        LayerRef::Floor(i) => format!("floor{}", i + 1),
        LayerRef::Wall(i) => format!("wall{}", i + 1),
        LayerRef::Shadow => "shadow".into(),
        LayerRef::Tag => "tag".into(),
    }
}

fn err_text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ------------------------------------------------------------------ session

impl Session {
    pub(crate) fn new(root: PathBuf, profile: Profile) -> Result<Session, String> {
        let saves_root = root.join("saves");
        let overlay = profile
            .open(&saves_root)
            .map_err(|e| format!("opening profile {:?}: {e}", profile.id))?;
        let saves = SavesDir::new(&saves_root, &profile.id);
        Ok(Session {
            root,
            profile,
            overlay,
            saves,
            names: None,
            objects: None,
            open: None,
        })
    }

    fn with_ctx<T>(&self, f: impl FnOnce(&mut Ctx<'_>) -> T) -> T {
        let (mut out, mut err) = (std::io::sink(), std::io::sink());
        let mut ctx = Ctx {
            root: self.root.clone(),
            out: &mut out,
            err: &mut err,
        };
        f(&mut ctx)
    }

    fn catalog(&self, names: &[GamePath]) -> Result<MapCatalog, String> {
        let (mut catalog, _) = MapCatalog::build(&self.overlay, self.profile.game_version, names)
            .map_err(|e| format!("building the map catalog: {e}"))?;
        self.with_ctx(|ctx| overrides::apply(ctx, &mut catalog, &self.profile.id));
        Ok(catalog)
    }

    fn open_mut(&mut self) -> Result<&mut Open, String> {
        self.open
            .as_mut()
            .ok_or_else(|| "No map is open: use open_map or new_map first.".to_owned())
    }

    fn ensure_tiles(&mut self) -> Result<(), String> {
        let overlay = &self.overlay;
        let open = self.open.as_mut().ok_or("No map is open.")?;
        if open.tiles.is_none() {
            let info = open
                .info
                .as_ref()
                .ok_or("This map has no known level type, so its tile sets are unknown (give level_type_id to open_map).")?;
            let (tiles, _) = TileSet::load(overlay, &info.dt1_files).map_err(err_text)?;
            open.tiles = Some(tiles);
        }
        Ok(())
    }

    fn palette(&self, act: u8) -> Result<Palette, String> {
        let path = GamePath::new(act_palette_path(act));
        let bytes = self
            .overlay
            .read_bytes(&path)
            .map_err(err_text)?
            .ok_or_else(|| format!("{path}: not found in the profile"))?;
        Palette::parse_dat(&bytes).map_err(err_text)
    }

    fn game_path(arg: Option<&Value>) -> Result<GamePath, String> {
        let s = arg
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or("path is required")?;
        let p = GamePath::new(s.trim());
        if !p.as_str().ends_with(".ds1") {
            return Err(format!("{p}: a map path ends in .ds1"));
        }
        Ok(p)
    }

    /// Runs a tool. `Err` becomes a tool result with `isError`.
    pub(crate) fn call(&mut self, name: &str, args: &Value) -> ToolResult {
        match name {
            "list_maps" => self.list_maps(args),
            "open_map" => self.open_map(args),
            "new_map" => self.new_map(args),
            "map_info" => self.map_info(),
            "get_cells" => self.get_cells(args),
            "paint" => self.paint(args),
            "set_cell" => self.set_cell(args),
            "copy_area" => self.copy_area(args),
            "find_replace" => self.find_replace(args),
            "reroll" => self.reroll(args),
            "resize_map" => self.resize_map(args),
            "set_cell_flags" => self.set_cell_flags(args),
            "list_objects" => self.list_objects(),
            "add_object" => self.add_object(args),
            "edit_objects" => self.edit_objects(args),
            "undo" => self.undo_redo(args, true),
            "redo" => self.undo_redo(args, false),
            "check_map" => self.check_map(),
            "render_map" => self.render_map(args),
            "save_map" => self.save_map(args),
            other => Err(format!("Unknown tool {other:?}.")),
        }
    }

    // -------------------------------------------------------------- files

    fn list_maps(&mut self, args: &Value) -> ToolResult {
        if self.names.is_none() {
            let known = self.with_ctx(|ctx| names::known_names(ctx, &self.profile, &self.overlay));
            let known = known.map_err(|e| format!("listing maps: {}", fail_text(e)))?;
            let mut maps: Vec<GamePath> = known
                .iter()
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ds1")))
                .cloned()
                .collect();
            maps.sort();
            self.names = Some(maps);
        }
        let needle = args
            .get("filter")
            .and_then(Value::as_str)
            .map(|s| s.to_lowercase().replace('/', "\\"));
        let limit = opt_int(args.get("limit"), "limit", 1, 100_000)?.unwrap_or(200) as usize;
        let all = self.names.as_deref().unwrap_or(&[]);
        let hits: Vec<&GamePath> = all
            .iter()
            .filter(|p| {
                needle
                    .as_ref()
                    .is_none_or(|n| p.as_str().contains(n.as_str()))
            })
            .collect();
        let mut out = format!("{} maps", hits.len());
        if hits.len() > limit {
            out.push_str(&format!(" (showing {limit})"));
        }
        for p in hits.iter().take(limit) {
            out.push('\n');
            out.push_str(p.as_str());
        }
        text(out)
    }

    fn summary(&self) -> String {
        let Some(o) = &self.open else {
            return "No map is open.".into();
        };
        let d = o.doc.ds1();
        let level = o
            .info
            .as_ref()
            .map_or("unknown level type".to_owned(), |i| {
                format!(
                    "level type {} ({} DT1s{})",
                    i.lvl_type.map_or("?".into(), |t| t.to_string()),
                    i.dt1_files.len(),
                    if i.missing_dt1.is_empty() {
                        String::new()
                    } else {
                        format!(", {} missing", i.missing_dt1.len())
                    }
                )
            });
        format!(
            "{}: {}x{} cells, act {}, DS1 v{}, {} floor / {} wall layers, {} objects, {level}{}",
            o.path,
            d.width,
            d.height,
            d.effective_act() + 1,
            d.version,
            d.floors.len(),
            d.walls.len(),
            d.objects.len(),
            if o.doc.is_dirty() {
                ", unsaved changes"
            } else {
                ""
            }
        )
    }

    fn open_map(&mut self, args: &Value) -> ToolResult {
        let path = Self::game_path(args.get("path"))?;
        let bytes = self
            .overlay
            .read_bytes(&path)
            .map_err(err_text)?
            .ok_or_else(|| format!("{path}: not found in the profile (list_maps finds maps)"))?;
        let ds1 = Ds1::parse(&bytes).map_err(|e| format!("{path}: not a readable DS1: {e}"))?;
        let catalog = self.catalog(std::slice::from_ref(&path))?;
        let lvl = opt_int(args.get("level_type_id"), "level_type_id", 0, 100_000)?;
        let info = match lvl {
            Some(l) => Some(
                catalog
                    .resolve_explicit(&path, l as u32, None)
                    .map_err(err_text)?,
            ),
            None => catalog.best(&path),
        };
        let mut doc = Document::new(ds1);
        doc.set_clip_context(
            Some(path.to_string()),
            info.as_ref().and_then(|i| i.lvl_type),
        );
        self.open = Some(Open {
            path,
            doc,
            info,
            tiles: None,
            fresh: false,
        });
        text(self.summary())
    }

    fn new_map(&mut self, args: &Value) -> ToolResult {
        let path = Self::game_path(args.get("path"))?;
        let w = int(args.get("width"), "width", 1, 1024)? as u32;
        let h = int(args.get("height"), "height", 1, 1024)? as u32;
        let lt = int(args.get("level_type_id"), "level_type_id", 0, 100_000)? as u32;
        let floors = opt_int(args.get("floor_layers"), "floor_layers", 1, 2)?.unwrap_or(1) as usize;
        let walls = opt_int(args.get("wall_layers"), "wall_layers", 1, 4)?.unwrap_or(2) as usize;
        let catalog = self.catalog(&[])?;
        let info = catalog
            .resolve_explicit(&path, lt, None)
            .map_err(err_text)?;
        let mut spec = NewMapSpec::new(w, h, i32::from(info.act));
        spec.walls = walls;
        spec.floors = floors;
        let mut ds1 = new_map(&spec).map_err(err_text)?;
        ds1.files = info
            .dt1_files
            .iter()
            .map(|p| format!("c:\\d2\\{}", p.as_str().replace(".dt1", ".tg1")).into_bytes())
            .collect();
        let mut doc = Document::new(ds1);
        doc.set_clip_context(Some(path.to_string()), Some(lt));
        self.open = Some(Open {
            path,
            doc,
            info: Some(info),
            tiles: None,
            fresh: true,
        });
        text(format!("Created {}", self.summary()))
    }

    fn map_info(&mut self) -> ToolResult {
        text(self.summary())
    }

    // -------------------------------------------------------------- reading

    fn get_cells(&mut self, args: &Value) -> ToolResult {
        let o = self.open_mut()?;
        let ds1 = o.doc.ds1();
        let rect = parse_rect(args.get("rect"), ds1)?;
        let cells = u64::from(rect.x1 - rect.x0 + 1) * u64::from(rect.y1 - rect.y0 + 1);
        if cells > 1600 {
            return Err(format!("The rectangle has {cells} cells; at most 1600."));
        }
        let layers: Vec<LayerRef> = match args.get("layers").and_then(Value::as_array) {
            Some(a) => a
                .iter()
                .map(|l| parse_layer(Some(l)))
                .collect::<Result<_, _>>()?,
            None => LayerRef::all_in(ds1)
                .into_iter()
                .filter(|l| *l != LayerRef::Tag)
                .collect(),
        };
        let mut out = String::new();
        for layer in layers {
            let mut grid = String::new();
            let mut any = false;
            for y in rect.y0..=rect.y1 {
                let row: Vec<String> = (rect.x0..=rect.x1)
                    .map(|x| match layer.get(ds1, x, y) {
                        Some(v) if !layer.is_empty_value(v) => {
                            any = true;
                            let c = Cell(v.cell);
                            let wall = matches!(layer, LayerRef::Wall(_));
                            format!(
                                "{}/{}{}{}",
                                c.main_index(),
                                c.sub_index(),
                                if wall {
                                    format!(":{}", v.orientation & 0xFF)
                                } else {
                                    String::new()
                                },
                                if c.hidden() { "h" } else { "" }
                            )
                        }
                        _ => ".".to_owned(),
                    })
                    .collect();
                grid.push_str(&row.join(" "));
                grid.push('\n');
            }
            if any {
                out.push_str(&format!("{}:\n{grid}", layer_name(layer)));
            }
        }
        if out.is_empty() {
            out = "All cells are empty.".into();
        }
        text(out)
    }

    fn list_objects(&mut self) -> ToolResult {
        let o = self.open_mut()?;
        let mut out = format!("{} objects", o.doc.ds1().objects.len());
        for (i, ob) in o.doc.ds1().objects.iter().enumerate() {
            out.push_str(&format!(
                "\n{i}: type {} id {} at sub-tile ({}, {}) = cell ({}, {})",
                ob.kind,
                ob.id,
                ob.x,
                ob.y,
                ob.x.div_euclid(5),
                ob.y.div_euclid(5)
            ));
        }
        text(out)
    }

    // -------------------------------------------------------------- edits

    /// Runs an edit and reports whether the map changed.
    fn edit(
        &mut self,
        what: &str,
        f: impl FnOnce(&mut Document) -> Result<String, String>,
    ) -> ToolResult {
        let o = self.open_mut()?;
        let before = o.doc.revision();
        let detail = f(&mut o.doc)?;
        let changed = o.doc.revision() != before;
        text(format!(
            "{what}: {detail}{}",
            if changed { "" } else { " (no change)" }
        ))
    }

    fn paint(&mut self, args: &Value) -> ToolResult {
        let layer = parse_layer(args.get("layer"))?;
        let mix: Vec<Brush> = match args.get("tiles").and_then(Value::as_array) {
            Some(a) => a
                .iter()
                .map(|t| parse_brush(layer, t))
                .collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        let seed = opt_int(args.get("seed"), "seed", 0, i64::MAX)?.unwrap_or(0) as u64;
        let cells = parse_cells(args.get("cells"))?;
        let fill_from = parse_cells(args.get("fill_from"))?;
        let args = args.clone();
        self.edit("paint", move |doc| {
            let mut rng = Rng::new(seed);
            let n = if let Some(&(x, y)) = fill_from.first() {
                let within = match args.get("within") {
                    Some(w) if !w.is_null() => {
                        let rect = parse_rect(Some(w), doc.ds1())?;
                        let mut s = Selection::new();
                        s.box_select(doc.ds1(), rect, &[layer], true, SelectOp::Replace);
                        Some(s)
                    }
                    _ => None,
                };
                let n = doc.flood_region(layer, x, y, within.as_ref()).len();
                doc.flood_fill(layer, x, y, &mix, within.as_ref(), &mut rng)
                    .map_err(err_text)?;
                n
            } else if let Some(r) = args.get("rect").filter(|r| !r.is_null()) {
                let rect = parse_rect(Some(r), doc.ds1())?;
                doc.paint_rect(layer, rect, &mix, &mut rng)
                    .map_err(err_text)?;
                ((rect.x1 - rect.x0 + 1) * (rect.y1 - rect.y0 + 1)) as usize
            } else if !cells.is_empty() {
                doc.paint_mix(layer, &cells, &mix, &mut rng)
                    .map_err(err_text)?;
                cells.len()
            } else {
                return Err("Give cells, rect or fill_from.".into());
            };
            Ok(format!(
                "{} {n} cells on {}",
                if mix.is_empty() { "erased" } else { "painted" },
                layer_name(layer)
            ))
        })
    }

    fn set_cell(&mut self, args: &Value) -> ToolResult {
        let layer = parse_layer(args.get("layer"))?;
        let x = int(args.get("x"), "x", 0, 9999)? as u32;
        let y = int(args.get("y"), "y", 0, 9999)? as u32;
        let main = opt_int(args.get("main"), "main", 0, 63)?;
        let sub = opt_int(args.get("sub"), "sub", 0, 255)?;
        let prop1 = opt_int(args.get("prop1"), "prop1", 0, 255)?;
        let hidden = args.get("hidden").and_then(Value::as_bool);
        let orientation = opt_int(args.get("orientation"), "orientation", 0, 255)?;
        self.edit("set_cell", move |doc| {
            let cur = layer.get(doc.ds1(), x, y).ok_or_else(|| {
                format!(
                    "({x}, {y}) is outside the map or {} does not exist",
                    layer_name(layer)
                )
            })?;
            let mut cell = Cell(cur.cell);
            let mut orient = cur.orientation;
            if main.is_some() || sub.is_some() {
                cell = cell.with_indices(
                    main.map_or(cell.main_index(), |m| m as u8),
                    sub.map_or(cell.sub_index(), |s| s as u8),
                );
                if cell.prop1() == 0 && prop1.is_none() {
                    let default = match layer {
                        LayerRef::Floor(_) => 0xC2,
                        LayerRef::Shadow => 0x80,
                        _ => 0x81,
                    };
                    cell = Cell(cell.0 | default);
                }
            }
            if let Some(p) = prop1 {
                cell = Cell((cell.0 & !0xFF) | p as u32);
            }
            if let Some(h) = hidden {
                cell = cell.with_hidden(h);
            }
            if let Some(o) = orientation
                && matches!(layer, LayerRef::Wall(_))
            {
                orient = o as u32;
            }
            doc.paint(
                layer,
                &[(
                    x,
                    y,
                    CellValue {
                        cell: cell.0,
                        orientation: orient,
                    },
                )],
            )
            .map_err(err_text)?;
            Ok(format!("{} ({x}, {y})", layer_name(layer)))
        })
    }

    fn copy_area(&mut self, args: &Value) -> ToolResult {
        let to_x = int(args.get("to_x"), "to_x", -9999, 9999)? as i32;
        let to_y = int(args.get("to_y"), "to_y", -9999, 9999)? as i32;
        let mv = args.get("move").and_then(Value::as_bool).unwrap_or(false);
        let rect_arg = args.get("rect").cloned();
        self.edit("copy_area", move |doc| {
            let rect = parse_rect(rect_arg.as_ref(), doc.ds1())?;
            let mut sel = Selection::new();
            let layers = LayerRef::all_in(doc.ds1());
            sel.box_select(doc.ds1(), rect, &layers, false, SelectOp::Replace);
            let Some(b) = sel.bounds() else {
                return Ok("the rectangle is empty".to_owned());
            };
            // The clipboard is relative to the selection's own bounds; place
            // the rectangle's top-left at (to_x, to_y).
            let at = (
                to_x + b.x0 as i32 - rect.x0 as i32,
                to_y + b.y0 as i32 - rect.y0 as i32,
            );
            doc.begin_group("Copy area");
            let run = (|| {
                let clip = if mv { doc.cut(&sel)? } else { doc.copy(&sel) };
                doc.paste(&clip, at, PasteMode::Overwrite)
            })();
            doc.end_group();
            let preview = run.map_err(err_text)?;
            Ok(format!(
                "{} {} cells to ({to_x}, {to_y}){}",
                if mv { "moved" } else { "copied" },
                preview.writes,
                if preview.out_of_bounds > 0 {
                    format!(", {} fell off the map", preview.out_of_bounds)
                } else {
                    String::new()
                }
            ))
        })
    }

    fn find_replace(&mut self, args: &Value) -> ToolResult {
        let name = args
            .get("layer")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let ds_layers = |ds1: &Ds1| -> Result<Vec<LayerRef>, String> {
            Ok(match name.as_str() {
                "floor" => (0..ds1.floors.len() as u8).map(LayerRef::Floor).collect(),
                "wall" => (0..ds1.walls.len() as u8).map(LayerRef::Wall).collect(),
                _ => vec![parse_layer(args.get("layer"))?],
            })
        };
        let kind_layer = if name == "floor" {
            LayerRef::Floor(0)
        } else if name == "wall" {
            LayerRef::Wall(0)
        } else {
            parse_layer(args.get("layer"))?
        };
        let from = parse_key(kind_layer, args.get("from").ok_or("from is required")?)?;
        let to = parse_key(kind_layer, args.get("to").ok_or("to is required")?)?;
        let rect = args.get("rect").filter(|r| !r.is_null()).cloned();
        self.edit("find_replace", move |doc| {
            let layers = ds_layers(doc.ds1())?;
            let within = match &rect {
                Some(r) => {
                    let rect = parse_rect(Some(r), doc.ds1())?;
                    let mut s = Selection::new();
                    s.box_select(doc.ds1(), rect, &layers, true, SelectOp::Replace);
                    Some(s)
                }
                None => None,
            };
            let n = doc
                .replace_tiles(&layers, from, to, within.as_ref())
                .map_err(err_text)?;
            Ok(format!("replaced {n} cells"))
        })
    }

    fn reroll(&mut self, args: &Value) -> ToolResult {
        let layer = parse_layer(args.get("layer"))?;
        let LayerRef::Floor(fi) = layer else {
            return Err("reroll works on floor layers (floor1, floor2).".into());
        };
        self.ensure_tiles()?;
        let seed = opt_int(args.get("seed"), "seed", 0, i64::MAX)?.unwrap_or(0) as u64;
        let preserve = args
            .get("preserve_walkability")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let rect_arg = args.get("rect").cloned();
        let o = self.open_mut()?;
        let tiles = o.tiles.as_ref().ok_or("no tile set")?;
        let doc = &mut o.doc;
        let rect = parse_rect(rect_arg.as_ref(), doc.ds1())?;
        // Mains used in the area.
        let mut mains = std::collections::BTreeSet::new();
        for y in rect.y0..=rect.y1 {
            for x in rect.x0..=rect.x1 {
                if let Some(v) = layer
                    .get(doc.ds1(), x, y)
                    .filter(|v| !layer.is_empty_value(*v))
                {
                    mains.insert(Cell(v.cell).main_index());
                }
            }
        }
        let flags_of = |main: u8, sub: u8| {
            let r = tiles.resolve(d2_formats::dt1::TileKey {
                orientation: 0,
                main,
                sub,
            })?;
            let t = tiles.tile(&r)?;
            Some(
                t.subtile_flags
                    .iter()
                    .fold(0u32, |h, f| h.wrapping_mul(31).wrapping_add(u32::from(*f))),
            )
        };
        let revision = doc.revision();
        doc.begin_group("Re-roll floors");
        let mut changed = 0;
        let mut skipped = 0;
        let mut failure = None;
        for main in &mains {
            let mut choices = Vec::new();
            for sub in 0..=255u8 {
                let key = d2_formats::dt1::TileKey {
                    orientation: 0,
                    main: *main,
                    sub,
                };
                let cands = tiles.candidates(key);
                if cands.is_empty() {
                    continue;
                }
                let rarity = cands
                    .iter()
                    .filter_map(|r| tiles.tile(r))
                    .map(|t| t.rarity.max(0) as u32)
                    .max()
                    .unwrap_or(0);
                choices.push(FloorChoice {
                    main: *main,
                    sub,
                    rarity,
                    walk: flags_of(*main, sub).unwrap_or(0),
                });
            }
            let mut area = Selection::new();
            for y in rect.y0..=rect.y1 {
                for x in rect.x0..=rect.x1 {
                    if layer.get(doc.ds1(), x, y).is_some_and(|v| {
                        !layer.is_empty_value(v) && Cell(v.cell).main_index() == *main
                    }) {
                        area.insert(layer, x, y);
                    }
                }
            }
            let opts = RerollOptions {
                preserve_walkability: preserve,
                avoid_repeats: true,
                seed,
            };
            match doc.reroll_floors(fi, Some(&area), &choices, &opts, &flags_of) {
                Ok(r) => {
                    changed += r.changed;
                    skipped += r.skipped;
                }
                Err(e) => {
                    failure = Some(err_text(e));
                    break;
                }
            }
        }
        doc.end_group();
        if let Some(f) = failure {
            return Err(f);
        }
        text(format!(
            "reroll: {changed} cells changed, {skipped} skipped{}",
            if doc.revision() == revision {
                " (no change)"
            } else {
                ""
            }
        ))
    }

    fn resize_map(&mut self, args: &Value) -> ToolResult {
        let g = |k| opt_int(args.get(k), k, -4096, 4096).map(|v| v.unwrap_or(0) as i32);
        let (l, t, r, b) = (g("left")?, g("top")?, g("right")?, g("bottom")?);
        self.edit("resize_map", move |doc| {
            doc.resize_edges(l, t, r, b).map_err(err_text)?;
            Ok(format!(
                "now {}x{} cells",
                doc.ds1().width,
                doc.ds1().height
            ))
        })
    }

    fn set_cell_flags(&mut self, args: &Value) -> ToolResult {
        let layer = if args.get("layer").is_some() {
            parse_layer(args.get("layer"))?
        } else {
            LayerRef::Floor(0)
        };
        let cells = parse_cells(args.get("cells"))?;
        let only_tiles = args
            .get("only_tiles")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hidden = args.get("hidden").and_then(Value::as_bool);
        let unwalkable = args.get("unwalkable").and_then(Value::as_bool);
        let props = [
            opt_int(args.get("prop1"), "prop1", 0, 255)?,
            opt_int(args.get("prop2"), "prop2", 0, 255)?,
            opt_int(args.get("prop3"), "prop3", 0, 255)?,
            opt_int(args.get("prop4"), "prop4", 0, 255)?,
        ];
        let rect_arg = args.get("rect").filter(|r| !r.is_null()).cloned();
        self.edit("set_cell_flags", move |doc| {
            let mut sel = Selection::new();
            if let Some(r) = &rect_arg {
                let rect = parse_rect(Some(r), doc.ds1())?;
                sel.box_select(doc.ds1(), rect, &[layer], !only_tiles, SelectOp::Replace);
            }
            for &(x, y) in &cells {
                if layer
                    .get(doc.ds1(), x, y)
                    .is_some_and(|v| !only_tiles || !layer.is_empty_value(v))
                {
                    sel.insert(layer, x, y);
                }
            }
            if sel.is_empty() {
                return Err("Give cells or rect (inside the map).".into());
            }
            doc.begin_group("Set cell flags");
            let run = (|| {
                if props.iter().any(Option::is_some) {
                    let edits: Vec<(u32, u32, CellValue)> = sel
                        .iter()
                        .filter_map(|(_, x, y)| {
                            let cur = layer.get(doc.ds1(), x, y)?;
                            let mut w = cur.cell;
                            for (i, p) in props.iter().enumerate() {
                                if let Some(p) = p {
                                    w = (w & !(0xFF << (8 * i))) | ((*p as u32) << (8 * i));
                                }
                            }
                            Some((x, y, CellValue { cell: w, ..cur }))
                        })
                        .collect();
                    doc.paint(layer, &edits)?;
                }
                if hidden.is_some() || unwalkable.is_some() {
                    doc.set_cell_flags(&sel, hidden, unwalkable)?;
                }
                Ok::<(), capy_edit::EditError>(())
            })();
            doc.end_group();
            run.map_err(err_text)?;
            Ok(format!("{} cells on {}", sel.len(), layer_name(layer)))
        })
    }

    fn add_object(&mut self, args: &Value) -> ToolResult {
        let kind = int(args.get("type"), "type", 0, 255)? as i32;
        let id = int(args.get("id"), "id", 0, 100_000)? as i32;
        let x = int(args.get("x"), "x", 0, 100_000)? as i32;
        let y = int(args.get("y"), "y", 0, 100_000)? as i32;
        self.edit("add_object", move |doc| {
            let i = doc
                .add_object(Object {
                    kind,
                    id,
                    x,
                    y,
                    flags: 0,
                })
                .map_err(err_text)?;
            Ok(format!(
                "object {i}: type {kind} id {id} at sub-tile ({x}, {y})"
            ))
        })
    }

    fn edit_objects(&mut self, args: &Value) -> ToolResult {
        let moves = args
            .get("move")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let remove: Vec<usize> = args
            .get("remove")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as usize))
                    .collect()
            })
            .unwrap_or_default();
        self.edit("edit_objects", move |doc| {
            doc.begin_group("Edit objects");
            let run = (|| -> Result<(), String> {
                for m in &moves {
                    let i = int(m.get("index"), "index", 0, 1_000_000)? as usize;
                    let x = int(m.get("x"), "x", 0, 100_000)? as i32;
                    let y = int(m.get("y"), "y", 0, 100_000)? as i32;
                    let cur = *doc
                        .ds1()
                        .objects
                        .get(i)
                        .ok_or_else(|| format!("no object {i}"))?;
                    doc.set_object(i, Object { x, y, ..cur })
                        .map_err(err_text)?;
                }
                if !remove.is_empty() {
                    doc.delete_objects(&remove).map_err(err_text)?;
                }
                Ok(())
            })();
            doc.end_group();
            run?;
            Ok(format!("moved {}, removed {}", moves.len(), remove.len()))
        })
    }

    fn undo_redo(&mut self, args: &Value, undo: bool) -> ToolResult {
        let steps = opt_int(args.get("steps"), "steps", 1, 10_000)?.unwrap_or(1);
        let o = self.open_mut()?;
        let mut done = Vec::new();
        for _ in 0..steps {
            let label = if undo {
                o.doc.undo_label()
            } else {
                o.doc.redo_label()
            }
            .map(str::to_owned);
            let ok = if undo { o.doc.undo() } else { o.doc.redo() };
            if !ok {
                break;
            }
            done.push(label.unwrap_or_default());
        }
        let verb = if undo { "Undid" } else { "Redid" };
        if done.is_empty() {
            return text(format!(
                "Nothing to {}.",
                if undo { "undo" } else { "redo" }
            ));
        }
        text(format!("{verb} {}: {}", done.len(), done.join(", ")))
    }

    // -------------------------------------------------------------- checks, picture, save

    fn check_map(&mut self) -> ToolResult {
        let mut lines = Vec::new();
        let have_tiles = self.ensure_tiles().is_ok();
        let o = self.open_mut()?;
        if let Some(info) = &o.info {
            for m in &info.missing_dt1 {
                lines.push(format!("DT1 not found in the profile: {m}"));
            }
        } else {
            lines.push(
                "No level type is known for this map, so its tiles cannot be checked.".into(),
            );
        }
        let ds1 = o.doc.ds1();
        if let (true, Some(tiles)) = (have_tiles, o.tiles.as_ref()) {
            match capy_render::draw_list(ds1, tiles, &capy_render::Visibility::default()) {
                Ok((_, missing)) => {
                    if missing.is_empty() {
                        lines.push("Every tile resolves in the map's DT1s.".into());
                    } else {
                        let mut kinds = std::collections::BTreeMap::new();
                        for m in &missing {
                            *kinds
                                .entry((m.key.orientation, m.key.main, m.key.sub))
                                .or_insert(0usize) += 1;
                        }
                        let top: Vec<String> = kinds
                            .iter()
                            .take(10)
                            .map(|((o, m, s), n)| format!("o{o}/m{m}/s{s} x{n}"))
                            .collect();
                        lines.push(format!(
                            "{} cells use tiles no DT1 has (invisible in game): {}",
                            missing.len(),
                            top.join(", ")
                        ));
                    }
                }
                Err(e) => lines.push(format!("drawing check failed: {e}")),
            }
            lines.push(format!(
                "Walkable area: {:.1} tiles squared",
                capy_render::walk::walkable_area(ds1, tiles)
            ));
        }
        for f in check_objects(
            ds1,
            &capy_edit::ObjectCheckOptions {
                is_known: None,
                teleport_pad_row: None,
            },
        ) {
            lines.push(format!("object {}: {:?}", f.index, f.problem));
        }
        let warps = capy_render::links::warp_tiles(ds1);
        for (n, cells) in &warps {
            lines.push(format!(
                "warp link {n}: {} tile(s), first at ({}, {})",
                cells.len(),
                cells[0].0,
                cells[0].1
            ));
        }
        for area in capy_render::pops::find_pops(ds1) {
            lines.push(format!(
                "hide area {} hides tiles #{}",
                area.main, area.target
            ));
        }
        if let Some(a) = capy_render::links::arrival_problem(ds1, &|_, _| false)
            && warps.is_empty()
        {
            lines.push(format!(
                    "No warp tile and no floor in the centre room (cells {}..+7, {}..+7): portal arrivals would fail unless a waypoint is placed.",
                    a.room.0, a.room.1
                ));
        }
        text(lines.join("\n"))
    }

    fn load_object_catalog(&mut self) {
        if self.objects.is_none() {
            self.objects = Some(
                ObjectCatalog::build(&self.overlay, self.profile.game_version)
                    .ok()
                    .map(|(c, _)| c),
            );
        }
    }

    fn render_map(&mut self, args: &Value) -> ToolResult {
        self.ensure_tiles()?;
        let with_objects = args.get("objects").and_then(Value::as_bool).unwrap_or(true);
        if with_objects {
            self.load_object_catalog();
        }
        let act = {
            let o = self.open_mut()?;
            o.info
                .as_ref()
                .map_or(o.doc.ds1().effective_act(), |i| i.palette_act)
        };
        let palette = self.palette(act)?;
        let catalog = self
            .objects
            .as_ref()
            .and_then(Option::as_ref)
            .filter(|_| with_objects);
        let sprites = catalog.map(|c| ObjectSprites::new(&self.overlay, c));
        let o = self.open.as_mut().ok_or("No map is open.")?;
        let tiles = o.tiles.as_ref().ok_or("no tile set")?;
        let ds1 = o.doc.ds1();
        let rect = match args.get("rect").filter(|r| !r.is_null()) {
            Some(r) => Some(parse_rect(Some(r), ds1)?),
            None => None,
        };
        let scale = args.get("scale").and_then(Value::as_f64);
        let (w, h) = (f64::from(ds1.width), f64::from(ds1.height));
        let full_px = (w + h) * 80.0;
        let region_px = rect.map_or(full_px, |r| f64::from(r.x1 - r.x0 + r.y1 - r.y0 + 2) * 80.0);
        let div = match scale {
            Some(s) if s > 0.0 => (1.0 / s).round().clamp(1.0, 16.0) as u32,
            _ => (region_px / 1568.0).ceil().clamp(1.0, 16.0) as u32,
        };
        let mut opts = RenderOptions {
            zoom_div: if rect.is_some() { 1 } else { div },
            ..RenderOptions::default()
        };
        opts.visibility.objects = with_objects;
        let out =
            render_with_objects(ds1, tiles, sprites.as_ref(), &palette, &opts).map_err(err_text)?;
        let mut img = out.image;
        if let Some(r) = rect {
            // Crop the cell rectangle (plus room for walls above it) out of the 1:1 render.
            let (x0, y0, x1, y1) = (
                i64::from(r.x0),
                i64::from(r.y0),
                i64::from(r.x1) + 1,
                i64::from(r.y1) + 1,
            );
            let left = (x0 - y1) * 80 - i64::from(out.origin.0);
            let right = (x1 - y0) * 80 - i64::from(out.origin.0) + 160;
            let top = (x0 + y0) * 40 - i64::from(out.origin.1) - 160;
            let bottom = (x1 + y1) * 40 - i64::from(out.origin.1) + 80;
            let (iw, ih) = (i64::from(img.width), i64::from(img.height));
            let (cx0, cx1) = (left.clamp(0, iw), right.clamp(0, iw));
            let (cy0, cy1) = (top.clamp(0, ih), bottom.clamp(0, ih));
            if cx1 > cx0 && cy1 > cy0 {
                let (cw, ch) = ((cx1 - cx0) as usize, (cy1 - cy0) as usize);
                let mut px = Vec::with_capacity(cw * ch * 4);
                for row in cy0..cy1 {
                    let s = (row as usize * img.width as usize + cx0 as usize) * 4;
                    px.extend_from_slice(&img.pixels[s..s + cw * 4]);
                }
                img = capy_render::RgbaImage {
                    width: cw as u32,
                    height: ch as u32,
                    pixels: px,
                };
            }
            if div > 1 {
                img = capy_render::downscale(&img, div);
            }
        }
        let png = img.to_png().map_err(err_text)?;
        Ok(vec![
            Content::Text(format!(
                "{}x{} px, 1:{div}, {} tiles missing",
                img.width,
                img.height,
                out.missing.len()
            )),
            Content::Image(png),
        ])
    }

    fn save_map(&mut self, args: &Value) -> ToolResult {
        let target = if args.get("path").is_some_and(|p| !p.is_null()) {
            Some(Self::game_path(args.get("path"))?)
        } else {
            None
        };
        let history = self.root.join("history");
        let profile_id = self.profile.id.clone();
        let saves = self.saves.clone();
        let o = self.open_mut()?;
        let path = target.unwrap_or_else(|| o.path.clone());
        let bytes = o.doc.ds1().to_bytes().map_err(|e| {
            format!(
                "cannot write this map: {e} (the lowest DS1 version that can hold it is {})",
                o.doc.ds1().min_version_for_model()
            )
        })?;
        let written = saves
            .save(&path, &bytes)
            .map_err(|e| format!("cannot write {path}: {e}"))?;
        let mut notes = String::new();
        match SnapshotStore::open(&history, &profile_id)
            .and_then(|mut s| s.record(&path, &bytes, SnapshotKind::Save))
        {
            Ok(_) => {}
            Err(e) => notes.push_str(&format!(" (the history snapshot failed: {e})")),
        }
        o.doc.mark_saved();
        o.path = path.clone();
        o.fresh = false;
        let changed = saves.changed().map_or(0, |l| l.len());
        text(format!(
            "Saved {path} ({} bytes) to {}; CHANGED.txt lists {changed} file(s){notes}",
            bytes.len(),
            written.display()
        ))
    }
}

fn fail_text(f: crate::Fail) -> String {
    match f {
        crate::Fail::Usage(m) | crate::Fail::Runtime(m) => m,
    }
}

pub(crate) fn tools_json() -> Value {
    let rect = json!({"type":"object","description":"Cell rectangle, inclusive corners (x0,y0)-(x1,y1), numbered from 0 at the map's top corner.",
        "properties":{"x0":{"type":"integer"},"y0":{"type":"integer"},"x1":{"type":"integer"},"y1":{"type":"integer"}},"required":["x0","y0","x1","y1"]});
    let tile = json!({"type":"object","description":"A DT1 tile by number: main (0-63), sub (0-255) and, on wall layers, orientation (1-19).",
        "properties":{"main":{"type":"integer"},"sub":{"type":"integer"},"orientation":{"type":"integer"}},"required":["main","sub"]});
    let layer = json!({"type":"string","enum":LAYER_NAMES,"description":"Tile layer: floor1-2, wall1-4 or shadow."});
    let cells = json!({"type":"array","items":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2},"description":"[[x, y], ...]"});
    let t = |name: &str, description: &str, props: Value, required: &[&str]| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":props,"required":required}});
    json!([
        t(
            "list_maps",
            "List the map presets (.ds1) in the game and profile, optionally filtered by a text in their path.",
            json!({"filter":{"type":"string"},"limit":{"type":"integer"}}),
            &[]
        ),
        t(
            "open_map",
            "Open a map for editing (replaces the open one; unsaved changes are lost). A map saved earlier opens from the profile's saves.",
            json!({"path":{"type":"string","description":"Game path, e.g. data/global/tiles/act1/town/townn1.ds1"},"level_type_id":{"type":"integer","description":"LvlTypes id, when the map's own level type is unknown."}}),
            &["path"]
        ),
        t(
            "new_map",
            "Create a new, empty map (not saved until save_map). Tile sets come from the level type.",
            json!({"path":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"},"level_type_id":{"type":"integer"},"floor_layers":{"type":"integer"},"wall_layers":{"type":"integer"}}),
            &["path", "width", "height", "level_type_id"]
        ),
        t("map_info", "Summary of the open map.", json!({}), &[]),
        t(
            "get_cells",
            "Read the tiles in a rectangle (at most 1600 cells): \"main/sub\" (walls \"main/sub:orientation\"), \".\" empty, \"h\" hidden.",
            json!({"rect":rect,"layers":{"type":"array","items":layer}}),
            &["rect"]
        ),
        t(
            "paint",
            "Paint (or erase) tiles on one layer: cells, a rectangle, or a flood fill from a cell. Several tiles paint a random mix; no tiles erases. One undo step.",
            json!({"layer":layer,"tiles":{"type":"array","items":tile},"cells":cells,"rect":rect,"fill_from":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2},"within":rect,"seed":{"type":"integer"}}),
            &["layer"]
        ),
        t(
            "set_cell",
            "Set one cell of one layer: tile numbers, orientation (walls), hidden flag, or the raw prop1 byte (0 = empty). One undo step.",
            json!({"layer":layer,"x":{"type":"integer"},"y":{"type":"integer"},"main":{"type":"integer"},"sub":{"type":"integer"},"orientation":{"type":"integer"},"hidden":{"type":"boolean"},"prop1":{"type":"integer"}}),
            &["layer", "x", "y"]
        ),
        t(
            "copy_area",
            "Copy (or move) a rectangle of every layer, with its objects, so its top-left cell lands at (to_x, to_y). Empty cells are transparent.",
            json!({"rect":rect,"to_x":{"type":"integer"},"to_y":{"type":"integer"},"move":{"type":"boolean"}}),
            &["rect", "to_x", "to_y"]
        ),
        t(
            "find_replace",
            "Replace every use of one tile with another on one layer, or all \"floor\"/\"wall\" layers, in the whole map or a rectangle.",
            json!({"layer":{"type":"string"},"from":tile,"to":tile,"rect":rect}),
            &["layer", "from", "to"]
        ),
        t(
            "reroll",
            "Mix up the tiles of one floor layer in a rectangle: each cell gets a tile of its own group (same main index) from the map's DT1s.",
            json!({"layer":layer,"rect":rect,"seed":{"type":"integer"},"preserve_walkability":{"type":"boolean"}}),
            &["layer", "rect"]
        ),
        t(
            "resize_map",
            "Add (positive) or remove (negative) cells on each side. Objects move with the cells.",
            json!({"left":{"type":"integer"},"top":{"type":"integer"},"right":{"type":"integer"},"bottom":{"type":"integer"}}),
            &[]
        ),
        t(
            "set_cell_flags",
            "Set or clear cell flags on one layer for many cells: hidden, unwalkable (DS1 cell bit 17), or raw prop1-prop4 bytes (applied first). One undo step.",
            json!({"layer":layer,"cells":cells,"rect":rect,"only_tiles":{"type":"boolean"},"hidden":{"type":"boolean"},"unwalkable":{"type":"boolean"},"prop1":{"type":"integer"},"prop2":{"type":"integer"},"prop3":{"type":"integer"},"prop4":{"type":"integer"}}),
            &[]
        ),
        t(
            "list_objects",
            "The objects and NPCs placed in the map (sub-tile positions, 5 per cell).",
            json!({}),
            &[]
        ),
        t(
            "add_object",
            "Place an object (type 2) or NPC (type 1) at a sub-tile position. One undo step.",
            json!({"type":{"type":"integer"},"id":{"type":"integer"},"x":{"type":"integer"},"y":{"type":"integer"}}),
            &["type", "id", "x", "y"]
        ),
        t(
            "edit_objects",
            "Move and/or remove placed objects by index (from list_objects). Removal happens after moves. One undo step.",
            json!({"move":{"type":"array","items":{"type":"object","properties":{"index":{"type":"integer"},"x":{"type":"integer"},"y":{"type":"integer"}},"required":["index","x","y"]}},"remove":{"type":"array","items":{"type":"integer"}}}),
            &[]
        ),
        t(
            "undo",
            "Undo the last edit(s).",
            json!({"steps":{"type":"integer"}}),
            &[]
        ),
        t(
            "redo",
            "Redo undone edit(s).",
            json!({"steps":{"type":"integer"}}),
            &[]
        ),
        t(
            "check_map",
            "Check the map: missing DT1s and tiles, object problems, walkable area, warps, hide areas, portal arrival.",
            json!({}),
            &[]
        ),
        t(
            "render_map",
            "A PNG of the map or a rectangle of it. Scale (1 = game pixels, 0.5 = half) defaults to fit about 1568 px.",
            json!({"rect":rect,"scale":{"type":"number"},"objects":{"type":"boolean"}}),
            &[]
        ),
        t(
            "save_map",
            "Save the open map into capybara\\saves\\<profile> (never the game folder), listing it in CHANGED.txt and recording a history snapshot. Give a path to save under another name.",
            json!({"path":{"type":"string"}}),
            &[]
        ),
    ])
}
