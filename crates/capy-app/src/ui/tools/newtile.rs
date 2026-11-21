//! New tile (Map > Tile libraries > New tile, the Tiles panel): make a
//! brand-new DT1 tile with `capy_mod::tiles_create`. Pick the kind (floor,
//! roof, shadow, an upper wall by orientation, both halves of a north corner,
//! a lower wall) and the wall height, start blank, from one colour or from a
//! picture (PNG, BMP, GIF, PCX; placed and matched to the map's act palette),
//! check the live preview (the template, the tile as the map draws it on its
//! cell, and what did not fit) and the sub-tile flags, then:
//!
//! - **Add to this map**: the "Add to this map" preview, then the tiles go
//!   into the level type's own DT1 (`add_new_tiles_to_map`), registered and
//!   saved like copied tiles, and the brush becomes the new tile;
//! - **Add to <library>**: appended to the DT1 open in Tile libraries (one
//!   undo step there; save it from that window);
//! - **Open in editor**: the tiles in the sprite editor, every pixel of the
//!   template editable; its "Add to this map" / "Add to <library>" button
//!   rebuilds them from what was painted.
//!
//! The shape picker is shared with "Shape..." (an existing tile re-split into
//! another shape, in Tile libraries and in the sprite editor).

use std::collections::BTreeSet;
use std::path::PathBuf;

use bevy_egui::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use capy_mod::automap::{AUTOMAP_CODES, code_name};
use capy_mod::tiles_create::{
    BuildOptions, BuiltTile, DEFAULT_ROOF_HEIGHT, ImportOptions, MAX_WALL_HEIGHT, MapTarget,
    Placement, TileClass, TileShape, WALL_BLOCK, add_new_tiles_to_map, build_corner_pair,
    build_tile, default_flags, free_tile_number, import_tile_image, number_orientations, template,
};
use capy_mod::tiles_flags::FlagOp;
use capy_mod::tiles_review::TileId;
use capy_sprite::image_io::ImageKind;
use d2_formats::IndexedImage;
use d2_formats::dt1::{Dt1, Tile};
use d2_formats::palette::Palette;

use super::{addmap, dt1};
use crate::sprites::convert::MatchSettings;
use crate::sprites::dt1edit::{Dt1Doc, NewTilesTarget};
use crate::state::AppState;

const WARN: Color32 = Color32::from_rgb(235, 150, 40);
const ERROR: Color32 = Color32::from_rgb(235, 90, 90);

// ---------------------------- shapes ----------------------------

/// What kind of tile to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Floor,
    Roof,
    Shadow,
    UpperWall,
    /// Orientations 3 and 4 from one picture, same number.
    CornerPair,
    LowerWall,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Floor,
        Kind::Roof,
        Kind::Shadow,
        Kind::UpperWall,
        Kind::CornerPair,
        Kind::LowerWall,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Floor => "Floor",
            Kind::Roof => "Roof",
            Kind::Shadow => "Shadow",
            Kind::UpperWall => "Upper wall",
            Kind::CornerPair => "North corner (both halves)",
            Kind::LowerWall => "Lower wall",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Kind::Floor => "Orientation 0: the 160x79 floor diamond of one cell.",
            Kind::Roof => {
                "Orientation 15: a diamond like a floor, drawn lifted by the roof height."
            }
            Kind::Shadow => {
                "Orientation 13: drawn on the shadow layer, standing on the cell like a wall."
            }
            Kind::UpperWall => {
                "Orientations 1-9, 12 and 14: a picture 160 wide standing on the cell (its base is the cell's floor)."
            }
            Kind::CornerPair => {
                "Orientations 4 (left half, x 0-80) and 3 (right half, x 80-160) cut from one picture; the game draws both when a cell holds orientation 3."
            }
            Kind::LowerWall => {
                "Orientations 16-19: drawn under the floor (cliff faces, pits), from 96 pixels above the cell's base downwards."
            }
        }
    }

    fn of_orientation(o: u8) -> Kind {
        match TileClass::of_orientation(i32::from(o)) {
            Some(TileClass::Floor) | None => Kind::Floor,
            Some(TileClass::Roof) => Kind::Roof,
            Some(TileClass::Shadow) => Kind::Shadow,
            Some(TileClass::LowerWall) => Kind::LowerWall,
            Some(TileClass::UpperWall | TileClass::Special) => Kind::UpperWall,
        }
    }

    /// The kind a script names: floor, roof, shadow, wall, corner, lower.
    pub fn parse(word: &str) -> Option<Kind> {
        Some(match word {
            "floor" => Kind::Floor,
            "roof" => Kind::Roof,
            "shadow" => Kind::Shadow,
            "wall" => Kind::UpperWall,
            "corner" => Kind::CornerPair,
            "lower" => Kind::LowerWall,
            _ => return None,
        })
    }
}

/// Upper wall orientations a new tile can take (10 and 11, the special
/// marker tiles, are left out: the map flow skips them).
pub const UPPER_WALLS: [u8; 11] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 14];
pub const LOWER_WALLS: [u8; 4] = [16, 17, 18, 19];

/// "Left wall (1)": the automap name of an orientation.
pub fn orientation_name(o: u8) -> String {
    let name = AUTOMAP_CODES
        .get(usize::from(o))
        .map_or("Unknown", |c| code_name(c));
    format!("{name} ({o})")
}

/// A shape as the picker shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeChoice {
    pub kind: Kind,
    /// Upper or lower wall orientation (ignored for the other kinds).
    pub orientation: u8,
    /// Wall picture height as typed (the core rounds it up to 32).
    pub height: u32,
    pub roof_height: i16,
}

impl Default for ShapeChoice {
    fn default() -> Self {
        ShapeChoice {
            kind: Kind::Floor,
            orientation: 1,
            height: 96,
            roof_height: DEFAULT_ROOF_HEIGHT,
        }
    }
}

impl ShapeChoice {
    /// The choice showing `shape`.
    pub fn of(shape: TileShape) -> ShapeChoice {
        let kind = Kind::of_orientation(shape.orientation);
        let d = ShapeChoice::default();
        ShapeChoice {
            kind,
            orientation: if matches!(kind, Kind::UpperWall | Kind::LowerWall) {
                shape.orientation
            } else {
                d.orientation
            },
            height: if shape.height == 0 {
                d.height
            } else {
                shape.height
            },
            roof_height: if kind == Kind::Roof {
                shape.roof_height
            } else {
                d.roof_height
            },
        }
    }

    /// The orientation of the (first) tile.
    pub fn first_orientation(&self) -> u8 {
        match self.kind {
            Kind::Floor => 0,
            Kind::Roof => 15,
            Kind::Shadow => 13,
            Kind::CornerPair => 3,
            Kind::UpperWall | Kind::LowerWall => self.orientation,
        }
    }

    /// The single shape; `None` for the corner pair (two tiles).
    pub fn shape(&self) -> Option<TileShape> {
        Some(match self.kind {
            Kind::Floor => TileShape::floor(),
            Kind::Roof => TileShape::roof(self.roof_height),
            Kind::CornerPair => return None,
            _ => TileShape::wall(self.first_orientation(), self.height),
        })
    }

    /// Every tile's shape (both halves for the corner pair: 3, then 4).
    pub fn shapes(&self) -> Vec<TileShape> {
        match self.shape() {
            Some(s) => vec![s],
            None => vec![
                TileShape::wall(3, self.height),
                TileShape::wall(4, self.height),
            ],
        }
    }

    /// The area a picture is placed in: the whole 160-wide wall for the
    /// corner pair.
    pub fn placement_shape(&self) -> TileShape {
        self.shape()
            .unwrap_or_else(|| TileShape::wall(1, self.height))
    }

    /// The height the core stores (whole 32-pixel blocks).
    pub fn rounded_height(&self) -> u32 {
        self.height.clamp(1, MAX_WALL_HEIGHT).div_ceil(WALL_BLOCK) * WALL_BLOCK
    }

    fn is_wall(&self) -> bool {
        !matches!(self.kind, Kind::Floor | Kind::Roof)
    }
}

/// The kind, orientation and height picker. `pair` offers the corner pair
/// (two tiles; not for re-shaping one tile). True when something changed.
pub fn shape_picker(ui: &mut Ui, salt: &str, c: &mut ShapeChoice, pair: bool) -> bool {
    let before = *c;
    egui::Grid::new(("shape_picker", salt))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Kind");
            egui::ComboBox::from_id_salt(("shape_kind", salt))
                .selected_text(c.kind.label())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        if k == Kind::CornerPair && !pair {
                            continue;
                        }
                        ui.selectable_value(&mut c.kind, k, k.label())
                            .on_hover_text(k.help());
                    }
                });
            ui.end_row();
            let list: &[u8] = match c.kind {
                Kind::UpperWall => &UPPER_WALLS,
                Kind::LowerWall => &LOWER_WALLS,
                _ => &[],
            };
            if !list.is_empty() {
                if !list.contains(&c.orientation) {
                    c.orientation = list[0];
                }
                ui.label("Orientation");
                egui::ComboBox::from_id_salt(("shape_orientation", salt))
                    .selected_text(orientation_name(c.orientation))
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for &o in list {
                            ui.selectable_value(&mut c.orientation, o, orientation_name(o));
                        }
                    })
                    .response
                    .on_hover_text(
                        "Orientations 10 and 11 (special marker tiles such as warps) cannot be created here: the map flow leaves them out.",
                    );
                ui.end_row();
            }
            if c.is_wall() {
                ui.label("Height");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut c.height)
                            .range(1..=MAX_WALL_HEIGHT)
                            .suffix(" px"),
                    );
                    let r = c.rounded_height();
                    if r != c.height {
                        ui.weak(format!("stored as {r} px (whole 32-pixel blocks)"));
                    } else {
                        ui.weak(format!("{} block row(s)", r / WALL_BLOCK));
                    }
                });
                ui.end_row();
            }
            if c.kind == Kind::Roof {
                ui.label("Roof height");
                ui.add(
                    egui::DragValue::new(&mut c.roof_height)
                        .range(0..=1024)
                        .suffix(" px"),
                )
                .on_hover_text("How far up the game lifts the roof (most shipped roofs: 80)");
                ui.end_row();
            }
        });
    ui.horizontal(|ui| {
        orientation_diagram(ui, c);
        ui.add(egui::Label::new(RichText::new(c.kind.help()).weak()).wrap());
    });
    *c != before
}

/// A small cell diamond with where the tile stands on it: the cells a wall
/// of the orientation blocks by default (orange), the whole cell for floors
/// and roofs.
fn orientation_diagram(ui: &mut Ui, c: &ShapeChoice) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(100.0, 54.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let cell = |x: usize, y: usize| {
        let cw = rect.width() / 10.0;
        let ch = (rect.height() - 4.0) / 10.0;
        let cx = rect.center().x + (x as f32 - y as f32) * cw;
        let top = rect.top() + 2.0 + (x as f32 + y as f32) * ch;
        vec![
            egui::pos2(cx, top),
            egui::pos2(cx + cw, top + ch),
            egui::pos2(cx, top + 2.0 * ch),
            egui::pos2(cx - cw, top + ch),
        ]
    };
    let stands: Vec<usize> = default_flags(i32::from(c.first_orientation()))
        .iter()
        .enumerate()
        .filter(|(_, f)| **f != 0)
        .map(|(i, _)| i)
        .collect();
    for y in 0..5 {
        for x in 0..5 {
            let fill = match c.kind {
                Kind::Floor => Color32::from_rgb(60, 110, 70),
                Kind::Roof => Color32::from_rgb(70, 90, 140),
                Kind::Shadow => Color32::from_gray(70),
                Kind::LowerWall => Color32::from_rgb(70, 55, 45),
                _ if stands.contains(&(y * 5 + x)) => Color32::from_rgb(220, 140, 50),
                _ => Color32::from_gray(45),
            };
            painter.add(egui::Shape::convex_polygon(
                cell(x, y),
                fill,
                egui::Stroke::new(1.0, Color32::from_gray(20)),
            ));
        }
    }
    painter.text(
        rect.left_top(),
        egui::Align2::LEFT_TOP,
        "N",
        egui::FontId::proportional(9.0),
        Color32::from_gray(150),
    );
}

// ------------------------- sources and building -------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Blank,
    /// The whole template in one palette index.
    Solid,
    Image,
}

/// A picture file picked as the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub kind: ImageKind,
}

impl Picked {
    pub fn read(path: PathBuf) -> Result<Picked, String> {
        let kind = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageKind::from_extension)
            .ok_or("pick a PNG, BMP, GIF or PCX file")?;
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Ok(Picked { path, bytes, kind })
    }
}

/// Everything a build depends on (rebuilt when it changes).
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub choice: ShapeChoice,
    pub source: SourceKind,
    pub color: u8,
    pub image: Option<Picked>,
    pub placement: Placement,
    pub matching: MatchSettings,
    /// `None`: the orientation's defaults.
    pub flags: Option<[u8; 25]>,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            choice: ShapeChoice::default(),
            source: SourceKind::Blank,
            color: 0,
            image: None,
            placement: Placement::Fit,
            matching: MatchSettings::default(),
            flags: None,
        }
    }
}

impl Inputs {
    /// The sub-tile flags the (first) tile gets.
    pub fn effective_flags(&self) -> [u8; 25] {
        self.flags
            .unwrap_or_else(|| default_flags(i32::from(self.choice.first_orientation())))
    }
}

/// Built tiles and the picture they came from (anchor space).
#[derive(Clone, Debug)]
pub struct Built {
    pub tiles: Vec<BuiltTile>,
    pub shapes: Vec<TileShape>,
    pub picture: IndexedImage,
}

impl Built {
    pub fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = self.tiles.iter().filter_map(BuiltTile::warning).collect();
        out.dedup();
        out
    }

    pub fn into_tiles(self) -> Vec<Tile> {
        self.tiles.into_iter().map(|b| b.tile).collect()
    }
}

/// Builds the tiles of `inp`, numbered `main`/`sub`, with `palette` for a
/// picture.
pub fn build(inp: &Inputs, palette: &Palette, main: i32, sub: i32) -> Result<Built, String> {
    let c = &inp.choice;
    let place = c.placement_shape();
    let picture = match inp.source {
        SourceKind::Blank => IndexedImage::new(0, 0, 0, 0),
        SourceKind::Solid => {
            let mut m = template(place).map_err(|e| e.to_string())?.mask();
            let v = inp.color.max(1);
            for (p, a) in m.pixels.iter_mut().zip(&m.alpha) {
                *p = if *a != 0 { v } else { 0 };
            }
            m
        }
        SourceKind::Image => {
            let pic = inp.image.as_ref().ok_or("pick a picture first")?;
            import_tile_image(
                &pic.bytes,
                pic.kind,
                palette,
                place,
                &ImportOptions {
                    quantize: inp.matching.quantize(),
                    placement: inp.placement,
                },
            )
            .map_err(|e| e.to_string())?
        }
    };
    let opts = BuildOptions {
        flags: inp.flags,
        ..BuildOptions::default()
    };
    let shapes = c.shapes();
    let tiles = match c.shape() {
        Some(shape) => {
            vec![build_tile(&picture, shape, main, sub, &opts).map_err(|e| e.to_string())?]
        }
        None => {
            let (r, l) = build_corner_pair(&picture, c.rounded_height(), main, sub, &opts)
                .map_err(|e| e.to_string())?;
            vec![r, l]
        }
    };
    Ok(Built {
        tiles,
        shapes,
        picture,
    })
}

/// Every tile number of `dt1s`.
fn numbers<'a>(dt1s: impl IntoIterator<Item = &'a Dt1>) -> BTreeSet<TileId> {
    dt1s.into_iter()
        .flat_map(|d| d.tiles.iter())
        .map(|t| TileId::new(t.orientation, t.main_index, t.sub_index))
        .collect()
}

/// A number for a new tile of orientation `o` that `taken` does not use.
fn free_number(taken: &BTreeSet<TileId>, o: u8) -> Result<(i32, i32), String> {
    free_tile_number(taken, &number_orientations(i32::from(o)), None)
        .ok_or_else(|| "every tile number of this orientation is taken".to_owned())
}

// ------------------------- the dialog's state -------------------------

struct Preview {
    inputs: Inputs,
    act: u8,
    built: Result<Built, String>,
    /// Texture, its size and its top-left in cell space.
    tex: Option<(TextureHandle, egui::Vec2, egui::Vec2)>,
}

pub struct NewTileUi {
    pub inputs: Inputs,
    pub target: NewTilesTarget,
    /// The act palette pictures are matched to (the map's when it opens).
    pub act: u8,
    pub flag_op: usize,
    pub flag_bits: u8,
    pub status: String,
    palette: Option<(u8, Palette)>,
    preview: Option<Preview>,
}

impl Default for NewTileUi {
    fn default() -> Self {
        NewTileUi {
            inputs: Inputs::default(),
            target: NewTilesTarget::Map,
            act: 0,
            flag_op: 0,
            flag_bits: 0x07,
            status: String::new(),
            palette: None,
            preview: None,
        }
    }
}

impl NewTileUi {
    fn palette(&mut self, state: &AppState) -> Result<Palette, String> {
        if self.palette.as_ref().map(|p| p.0) != Some(self.act) {
            let p = dt1::load_palette(state, self.act)
                .ok_or_else(|| format!("the act {} palette cannot be read", self.act + 1))?;
            self.palette = Some((self.act, p));
        }
        self.palette
            .as_ref()
            .map(|p| p.1.clone())
            .ok_or_else(|| "no palette".to_owned())
    }

    /// The preview build, redone when the inputs changed.
    fn preview(&mut self, state: &AppState) -> &mut Preview {
        let fresh = self
            .preview
            .as_ref()
            .is_some_and(|p| p.inputs == self.inputs && p.act == self.act);
        if !fresh {
            let built = self
                .palette(state)
                .and_then(|pal| build(&self.inputs, &pal, 0, 0));
            self.preview = Some(Preview {
                inputs: self.inputs.clone(),
                act: self.act,
                built,
                tex: None,
            });
        }
        self.preview.get_or_insert_with(|| Preview {
            inputs: Inputs::default(),
            act: 0,
            built: Err(String::new()),
            tex: None,
        })
    }

    /// The preview's warnings (index 0, pixels outside), or its error.
    pub fn preview_messages(&mut self, state: &AppState) -> Result<Vec<String>, String> {
        match &self.preview(state).built {
            Ok(b) => Ok(b.warnings()),
            Err(e) => Err(e.clone()),
        }
    }
}

/// The act palette of the open map (act 1 without one).
fn map_act(state: &AppState) -> u8 {
    state
        .active_doc()
        .and_then(|d| d.info.as_ref())
        .map_or(0, |i| i.palette_act)
}

/// Opens the dialog for `target`.
pub fn open(state: &mut AppState, target: NewTilesTarget) {
    state.tools.newtile.target = target;
    state.tools.newtile.act = map_act(state);
    state.tools.newtile.status.clear();
    state.tools.open.insert("tiles.newTile");
}

/// "New tile into this DT1": the dialog aimed at the library open in Tile
/// libraries.
pub fn open_for_library(state: &mut AppState) -> Result<(), String> {
    let path = state
        .tools
        .dt1
        .lib
        .as_ref()
        .map(|l| l.path.clone())
        .ok_or("Open a library in Tile libraries first")?;
    open(state, NewTilesTarget::Library(path));
    Ok(())
}

/// The dialog opened from the registry command.
pub fn on_open(state: &mut AppState) {
    state.tools.newtile.act = map_act(state);
    if matches!(state.tools.newtile.target, NewTilesTarget::Library(_))
        && state.tools.dt1.lib.is_none()
    {
        state.tools.newtile.target = NewTilesTarget::Map;
    }
}

// ---------------------------- actions ----------------------------

/// "Add to this map": builds the tiles at a number free in the map's level
/// type and opens the Add to this map preview.
pub fn add_to_map(state: &mut AppState) -> Result<(), String> {
    let ctx = addmap::map_context(state)?;
    let mut ui = std::mem::take(&mut state.tools.newtile);
    let built = (|| {
        let palette = ui.palette(state)?;
        let taken = numbers(ctx.type_libs.iter().map(|(_, d)| d).chain(&ctx.existing));
        let (m, s) = free_number(&taken, ui.inputs.choice.first_orientation())?;
        build(&ui.inputs, &palette, m, s)
    })();
    state.tools.newtile = ui;
    let built = built?;
    for w in built.warnings() {
        state.warn(format!("New tile: {w}"));
    }
    let what = describe(&built);
    add_tiles_to_map_with(state, ctx, built.into_tiles(), what)
}

/// "New tile(s): Left wall (1), 96 px." for the preview.
fn describe(built: &Built) -> String {
    let names: Vec<String> = built
        .shapes
        .iter()
        .map(|s| {
            let mut n = orientation_name(s.orientation);
            if s.height > 0 {
                n.push_str(&format!(", {} px", stored_px(s)));
            }
            n
        })
        .collect();
    format!("New tile(s): {}.", names.join(" + "))
}

/// The wall height `shape` stores (rounded up to whole blocks).
fn stored_px(shape: &TileShape) -> u32 {
    shape.normalized().map_or(shape.height, |s| s.height)
}

/// Opens the Add to this map preview for created `tiles`.
pub fn add_tiles_to_map(
    state: &mut AppState,
    tiles: Vec<Tile>,
    what: String,
) -> Result<(), String> {
    let ctx = addmap::map_context(state)?;
    add_tiles_to_map_with(state, ctx, tiles, what)
}

fn add_tiles_to_map_with(
    state: &mut AppState,
    ctx: addmap::MapContext,
    tiles: Vec<Tile>,
    what: String,
) -> Result<(), String> {
    if tiles.iter().any(|t| matches!(t.orientation, 10 | 11)) {
        return Err("special tiles (orientations 10 and 11) cannot be added to a map".into());
    }
    let doc = state.active_doc().ok_or("Open a map first")?;
    let type_dt1s = ctx.type_dt1s();
    let out = add_new_tiles_to_map(
        &MapTarget {
            map_path: &ctx.map_path,
            ds1: doc.edit.ds1(),
            types: &ctx.types,
            prest: &ctx.prest,
            type_id: ctx.type_id,
            prest_def: ctx.def,
            type_dt1s: &type_dt1s,
            existing_own: ctx.existing.as_ref(),
            existing_own_path: Some(&ctx.own_rel),
        },
        &tiles,
    )
    .map_err(|e| e.to_string());
    let plan = out.map(|out| addmap::Plan::new(out, ctx, Some((0, 0))));
    let failed = plan.as_ref().err().cloned();
    addmap::open_plan(state, what, plan);
    failed.map_or(Ok(()), Err)
}

/// Appends created `tiles` to the library open in Tile libraries, at
/// numbers free there (one undo step in that window). Returns a message.
pub fn add_tiles_to_library(
    state: &mut AppState,
    path: &str,
    tiles: Vec<Tile>,
) -> Result<String, String> {
    let lib = state
        .tools
        .dt1
        .lib
        .as_mut()
        .filter(|l| l.path.eq_ignore_ascii_case(path))
        .ok_or_else(|| format!("{path} is no longer open in Tile libraries"))?;
    let placed = lib.edit(|d| {
        let mut taken = numbers([&*d]);
        let mut placed = Vec::new();
        // tiles sharing a number (corner halves) keep sharing one
        let mut given: Vec<((i32, i32), (i32, i32))> = Vec::new();
        for mut t in tiles {
            let key = (t.main_index, t.sub_index);
            let to = match given.iter().find(|(k, _)| *k == key) {
                Some((_, to)) => *to,
                None => {
                    let o = u8::try_from(t.orientation).unwrap_or(0);
                    let to = free_number(&taken, o)?;
                    given.push((key, to));
                    to
                }
            };
            (t.main_index, t.sub_index) = to;
            taken.insert(TileId::new(t.orientation, to.0, to.1));
            placed.push(format!("{}:{}/{}", t.orientation, to.0, to.1));
            d.tiles.push(t);
        }
        Ok(placed)
    })?;
    let lib = state.tools.dt1.lib.as_mut().ok_or("No library is open")?;
    lib.tile = lib.dt1.tiles.len().saturating_sub(1);
    let msg = format!(
        "Added {} new tile(s) to {path} as {} (unsaved: Save to the saves folder in Tile libraries)",
        placed.len(),
        placed.join(", ")
    );
    state.tools.dt1.status = msg.clone();
    state.info(msg.clone());
    Ok(msg)
}

/// Adds the dialog's tiles to the library it aims at.
pub fn add_to_library(state: &mut AppState) -> Result<String, String> {
    let NewTilesTarget::Library(path) = state.tools.newtile.target.clone() else {
        return Err("The dialog is not aimed at a library".into());
    };
    let mut ui = std::mem::take(&mut state.tools.newtile);
    let built = ui.palette(state).and_then(|p| build(&ui.inputs, &p, 0, 0));
    state.tools.newtile = ui;
    let built = built?;
    for w in built.warnings() {
        state.warn(format!("New tile: {w}"));
    }
    add_tiles_to_library(state, &path, built.into_tiles())
}

/// "Open in editor": the tiles in the sprite editor (Sprites mode), every
/// pixel of their template editable, with the dialog's act palette.
pub fn open_in_editor(state: &mut AppState) -> Result<(), String> {
    let mut ui = std::mem::take(&mut state.tools.newtile);
    let built = ui.palette(state).and_then(|p| build(&ui.inputs, &p, 0, 0));
    let (act, target) = (ui.act, ui.target.clone());
    state.tools.newtile = ui;
    let built = built?;
    let shapes = built.shapes.clone();
    let stem = match &built.shapes[..] {
        [s, ..] => format!("new_tile_{}", s.orientation),
        [] => "new_tile".to_owned(),
    };
    let doc = Dt1Doc::new_tiles(built.into_tiles(), &shapes, target)?;
    state.sprites.palette.choice = crate::sprites::palette::PaletteChoice::Act(act);
    let palette = state.sprite_palette();
    let editor = crate::sprites::editor::EditorState::new_dt1(
        doc,
        crate::sprites::SpriteSource::Imported(stem),
        palette,
    )?;
    state.install_editor(editor);
    state.config.mode = crate::config::AppMode::Sprites;
    state.config_dirty = true;
    Ok(())
}

/// The sprite editor's "Add to this map" / "Add to <library>" for a
/// new-tiles document: the painted tiles rebuilt and added.
pub fn add_from_editor(state: &mut AppState) -> Result<String, String> {
    let e = state
        .sprites
        .editor
        .as_ref()
        .ok_or("Nothing is open in the editor")?;
    let target = e
        .dt1
        .as_ref()
        .and_then(|d| d.new_tiles.clone())
        .ok_or("The editor does not hold new tiles")?;
    let (tiles, warnings) = e.dt1_tiles()?;
    for w in warnings {
        state.warn(format!("New tile: {w}"));
    }
    let msg = match target {
        NewTilesTarget::Map => {
            let n = tiles.len();
            add_tiles_to_map(
                state,
                tiles,
                format!("{n} new tile(s) painted in the editor."),
            )?;
            "Review the changes in Add to this map, then press Add".to_owned()
        }
        NewTilesTarget::Library(path) => add_tiles_to_library(state, &path, tiles)?,
    };
    if let Some(e) = state.sprites.editor.as_mut() {
        e.session.mark_saved();
    }
    Ok(msg)
}

// ---------------------------- the window ----------------------------

pub fn window(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.newtile);
    let has_map = state.active_doc().is_some();
    let lib_open = state.tools.dt1.lib.as_ref().map(|l| l.path.clone());
    let mut action: Option<u8> = None;

    ui.horizontal(|ui| {
        ui.label("Into:");
        ui.selectable_value(&mut d.target, NewTilesTarget::Map, "This map's own DT1");
        if let Some(p) = &lib_open {
            let short = p.rsplit('\\').next().unwrap_or(p);
            ui.selectable_value(
                &mut d.target,
                NewTilesTarget::Library(p.clone()),
                format!("{short} (Tile libraries)"),
            );
        }
    });
    if let NewTilesTarget::Library(p) = &d.target
        && lib_open.as_deref() != Some(p.as_str())
    {
        d.target = NewTilesTarget::Map;
    }
    ui.separator();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(380.0);
            ui.strong("Shape");
            shape_picker(ui, "newtile", &mut d.inputs.choice, true);
            ui.separator();
            ui.strong("Picture");
            source_controls(ui, state, &mut d);
            ui.separator();
            ui.strong("Sub-tile flags");
            flags_controls(ui, &mut d);
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.strong("Preview");
            preview_panel(ui, state, &mut d);
        });
    });
    ui.separator();
    ui.horizontal(|ui| {
        match &d.target {
            NewTilesTarget::Map => {
                if ui
                    .add_enabled(has_map, egui::Button::new("Add to this map..."))
                    .on_hover_text(
                        "Into the level type's own DT1, registered in LvlTypes and LvlPrest, saved to the profile's saves; the brush becomes the new tile",
                    )
                    .on_disabled_hover_text("Open a map first")
                    .clicked()
                {
                    action = Some(0);
                }
            }
            NewTilesTarget::Library(p) => {
                let short = p.rsplit('\\').next().unwrap_or(p).to_owned();
                if ui
                    .button(format!("Add to {short}"))
                    .on_hover_text("Appended at a free number; undo and save in Tile libraries")
                    .clicked()
                {
                    action = Some(1);
                }
            }
        }
        if ui
            .button("Open in editor")
            .on_hover_text(
                "Paint the tile in the sprite editor first (every pixel of the template is editable); its Add button adds the painted tile",
            )
            .clicked()
        {
            action = Some(2);
        }
    });
    super::message(ui, &d.status);
    state.tools.newtile = d;
    let result = match action {
        Some(0) => add_to_map(state).map(|()| String::new()),
        Some(1) => add_to_library(state),
        Some(2) => open_in_editor(state).map(|()| "Opened in the sprite editor".to_owned()),
        _ => return,
    };
    state.tools.newtile.status = match result {
        Ok(m) => m,
        Err(e) => format!("Cannot make the tile: {e}"),
    };
}

fn source_controls(ui: &mut Ui, state: &mut AppState, d: &mut NewTileUi) {
    ui.horizontal(|ui| {
        ui.radio_value(&mut d.inputs.source, SourceKind::Blank, "Blank");
        ui.radio_value(&mut d.inputs.source, SourceKind::Solid, "One colour");
        ui.radio_value(&mut d.inputs.source, SourceKind::Image, "Picture file");
    });
    match d.inputs.source {
        SourceKind::Blank => {
            ui.weak("No pixels yet: paint it with Open in editor (a blank tile draws nothing).");
        }
        SourceKind::Solid => {
            ui.horizontal(|ui| {
                ui.label("Palette index");
                let mut c = d.inputs.color.max(1);
                ui.add(egui::DragValue::new(&mut c).range(1..=255));
                d.inputs.color = c;
                if let Ok(p) = d.palette(state) {
                    let [r, g, b] = p.colors[usize::from(c)];
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                }
            });
        }
        SourceKind::Image => {
            ui.horizontal(|ui| {
                if ui.button("Pick a picture...").clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .add_filter("Pictures", &["png", "bmp", "gif", "pcx"])
                        .pick_file()
                {
                    match Picked::read(p) {
                        Ok(pic) => d.inputs.image = Some(pic),
                        Err(e) => d.status = format!("Cannot use the picture: {e}"),
                    }
                }
                match &d.inputs.image {
                    Some(p) => {
                        ui.weak(p.path.file_name().map_or_else(
                            || p.path.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        ));
                    }
                    None => {
                        ui.weak("PNG, BMP, GIF (first frame) or PCX");
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Placement");
                for (p, label, tip) in [
                    (
                        Placement::AsIs,
                        "As is",
                        "Unscaled, top-left on the drawable area's top-left (a picture painted on the template)",
                    ),
                    (Placement::Center, "Centre", "Unscaled, centred"),
                    (
                        Placement::Fit,
                        "Fit",
                        "Scaled (nearest neighbour, aspect kept) to fit the drawable area, centred",
                    ),
                ] {
                    ui.radio_value(&mut d.inputs.placement, p, label)
                        .on_hover_text(tip);
                }
            });
            crate::ui::sprites::converter::matching(ui, &mut d.inputs.matching, "newtile");
        }
    }
    ui.horizontal(|ui| {
        ui.label("Palette");
        egui::ComboBox::from_id_salt("newtile_act")
            .selected_text(format!("Act {}", d.act + 1))
            .show_ui(ui, |ui| {
                for a in 0..5u8 {
                    ui.selectable_value(&mut d.act, a, format!("Act {}", a + 1));
                }
            })
            .response
            .on_hover_text(
                "Colours are matched to this act's palette (the map's when the dialog opens)",
            );
    });
}

fn flags_controls(ui: &mut Ui, d: &mut NewTileUi) {
    let flags = d.inputs.effective_flags();
    ui.horizontal(|ui| {
        if d.inputs.flags.is_none() {
            ui.weak("The orientation's defaults (the most common shipped pattern).");
        } else if ui.small_button("Back to the defaults").clicked() {
            d.inputs.flags = None;
        }
    });
    dt1::flag_brush(ui, &mut d.flag_op, &mut d.flag_bits);
    let cells = dt1::flags_diamond(ui, &flags, egui::vec2(240.0, 120.0));
    if !cells.is_empty() {
        let op = dt1::flag_op(d.flag_op, d.flag_bits);
        let mut f = flags;
        for c in cells {
            if let Some(v) = f.get_mut(c) {
                *v = op.apply(*v);
            }
        }
        if f != flags {
            d.inputs.flags = Some(f);
        }
    }
}

/// Where anchor-space (0, 0) of a tile of `shape` lies in cell space (the
/// cell's floor diamond is at (0, 0)-(160, 80)), as the map view draws it:
/// walls stand 80 pixels down (`capy_render::TileGeometry`), roofs are
/// lifted by their roof height.
fn anchor_in_cell(shape: TileShape) -> (i32, i32) {
    match shape.class() {
        Some(TileClass::Floor) | None => (0, 0),
        Some(TileClass::Roof) => (0, -i32::from(shape.roof_height)),
        Some(_) => (0, 80),
    }
}

/// The preview as one picture in cell space: the template tinted, the
/// tile in its colours, picture pixels that do not fit in red. Returns the
/// image and its top-left in cell space.
fn preview_image(built: &Built, palette: &Palette) -> (ColorImage, (i32, i32)) {
    let shapes = &built.shapes;
    let templates: Vec<_> = shapes.iter().filter_map(|s| template(*s).ok()).collect();
    let anchor = shapes.first().map_or((0, 0), |s| anchor_in_cell(*s));
    // bounds in cell space
    let (mut x0, mut y0, mut x1, mut y1) = (0i32, 0i32, 160i32, 80i32);
    let mut grow = |x: i32, y: i32, w: u32, h: u32| {
        if w == 0 || h == 0 {
            return;
        }
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x.saturating_add(w as i32));
        y1 = y1.max(y.saturating_add(h as i32));
    };
    for t in &templates {
        grow(
            t.rect.x + anchor.0,
            t.rect.y + anchor.1,
            t.rect.width,
            t.rect.height,
        );
    }
    let p = &built.picture;
    // a picture far bigger than any tile is not drawn outside the tile
    let pic_ok = p.width <= 4096 && p.height <= 4096;
    if pic_ok {
        grow(
            p.offset_x + anchor.0,
            p.offset_y + anchor.1,
            p.width,
            p.height,
        );
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let mut put = |x: i32, y: i32, c: [u8; 4]| {
        let (cx, cy) = (x - x0, y - y0);
        if cx < 0 || cy < 0 || cx as usize >= w || cy as usize >= h {
            return;
        }
        let i = (cy as usize * w + cx as usize) * 4;
        if let Some(px) = rgba.get_mut(i..i + 4) {
            px.copy_from_slice(&c);
        }
    };
    let in_template = |x: i32, y: i32| {
        templates
            .iter()
            .any(|t| t.contains(x - anchor.0, y - anchor.1))
    };
    for y in y0..y1 {
        for x in x0..x1 {
            if in_template(x, y) {
                put(x, y, [70, 110, 170, 80]);
            }
        }
    }
    if pic_ok {
        for row in 0..p.height as i32 {
            for col in 0..p.width as i32 {
                let i = (row * p.width as i32 + col) as usize;
                if p.alpha.get(i).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let (x, y) = (p.offset_x + col + anchor.0, p.offset_y + row + anchor.1);
                if !in_template(x, y) {
                    put(x, y, [230, 60, 60, 220]);
                }
            }
        }
    }
    for b in &built.tiles {
        let Ok(img) = b.tile.render() else { continue };
        for row in 0..img.height as i32 {
            for col in 0..img.width as i32 {
                let i = (row * img.width as i32 + col) as usize;
                let (Some(&a), Some(&v)) = (img.alpha.get(i), img.pixels.get(i)) else {
                    continue;
                };
                if a == 0 || v == 0 {
                    continue;
                }
                let [r, g, bl] = palette.colors[usize::from(v)];
                put(
                    img.offset_x + col + anchor.0,
                    img.offset_y + row + anchor.1,
                    [r, g, bl, 255],
                );
            }
        }
    }
    (ColorImage::from_rgba_unmultiplied([w, h], &rgba), (x0, y0))
}

fn preview_panel(ui: &mut Ui, state: &AppState, d: &mut NewTileUi) {
    let palette = d.palette(state);
    let p = d.preview(state);
    let built = match &p.built {
        Ok(b) => b.clone(),
        Err(e) => {
            ui.colored_label(ERROR, e);
            return;
        }
    };
    let Ok(palette) = palette else {
        ui.colored_label(ERROR, "The act palette cannot be read");
        return;
    };
    if p.tex.is_none() {
        let (img, (x0, y0)) = preview_image(&built, &palette);
        let size = egui::vec2(img.size[0] as f32, img.size[1] as f32);
        let tex = ui
            .ctx()
            .load_texture("newtile-preview", img, TextureOptions::NEAREST);
        p.tex = Some((tex, size, egui::vec2(x0 as f32, y0 as f32)));
    }
    let Some((tex, size, origin)) = p.tex.clone() else {
        return;
    };
    let max = egui::vec2(380.0, 380.0);
    let scale = (max.x / size.x).min(max.y / size.y).clamp(0.1, 3.0);
    let (rect, _) = ui.allocate_exact_size(size * scale, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(24));
    painter.image(
        tex.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    // the cell's floor diamond, as the map view's grid draws it
    let at = |x: f32, y: f32| rect.min + (egui::vec2(x, y) - origin) * scale;
    let diamond = vec![
        at(80.0, 0.0),
        at(160.0, 40.0),
        at(80.0, 80.0),
        at(0.0, 40.0),
        at(80.0, 0.0),
    ];
    painter.add(egui::Shape::line(
        diamond,
        egui::Stroke::new(1.0, Color32::from_rgb(240, 220, 120)),
    ));
    // the template's box
    for s in &built.shapes {
        if let Ok(t) = template(*s) {
            let a = anchor_in_cell(*s);
            let r = egui::Rect::from_min_max(
                at((t.rect.x + a.0) as f32, (t.rect.y + a.1) as f32),
                at(
                    (t.rect.x + a.0) as f32 + t.rect.width as f32,
                    (t.rect.y + a.1) as f32 + t.rect.height as f32,
                ),
            );
            painter.rect_stroke(
                r,
                0.0,
                egui::Stroke::new(1.0, Color32::from_rgb(110, 160, 230)),
                egui::StrokeKind::Inside,
            );
        }
    }
    ui.weak("Blue: where pixels can go. Yellow: the cell. Red: picture pixels left out.");
    for (b, s) in built.tiles.iter().zip(&built.shapes) {
        let opaque = b
            .tile
            .render()
            .map_or(0, |i| i.alpha.iter().filter(|&&a| a != 0).count());
        let tall = if s.height > 0 {
            format!(", {} px tall", stored_px(s))
        } else {
            String::new()
        };
        ui.label(format!(
            "{}: {} block(s), {opaque} pixel(s){tall}",
            orientation_name(s.orientation),
            b.tile.blocks.len(),
        ));
    }
    for w in built.warnings() {
        ui.add(egui::Label::new(RichText::new(w).color(WARN)).wrap());
    }
}

// ---------------------------- scripts ----------------------------

/// `key=value` of a script token.
fn kv<'a>(tok: &'a str, key: &str) -> Option<&'a str> {
    tok.strip_prefix(key)?.strip_prefix('=')
}

/// A shape from script words: `<kind> [o=<n>] [h=<px>] [roof=<px>]` (kind:
/// floor, roof, shadow, wall, corner, lower). Unknown tokens are returned.
pub fn parse_shape<'a>(args: &[&'a str]) -> Result<(ShapeChoice, Vec<&'a str>), String> {
    let (kind, rest) = args
        .split_first()
        .ok_or("expected floor|roof|shadow|wall|corner|lower")?;
    let mut c = ShapeChoice {
        kind: Kind::parse(kind).ok_or_else(|| format!("{kind:?} is not a tile kind"))?,
        ..ShapeChoice::default()
    };
    if c.kind == Kind::LowerWall {
        c.orientation = LOWER_WALLS[0];
    }
    let num = |v: &str| {
        v.parse::<i64>()
            .map_err(|_| format!("{v:?} is not a number"))
    };
    let mut left = Vec::new();
    for &t in rest {
        if let Some(v) = kv(t, "o") {
            c.orientation = u8::try_from(num(v)?).map_err(|_| "bad orientation".to_owned())?;
        } else if let Some(v) = kv(t, "h") {
            c.height = u32::try_from(num(v)?).map_err(|_| "bad height".to_owned())?;
        } else if let Some(v) = kv(t, "roof") {
            c.roof_height = i16::try_from(num(v)?).map_err(|_| "bad roof height".to_owned())?;
        } else {
            left.push(t);
        }
    }
    let ok = match c.kind {
        Kind::UpperWall => UPPER_WALLS.contains(&c.orientation),
        Kind::LowerWall => LOWER_WALLS.contains(&c.orientation),
        _ => true,
    };
    if !ok {
        return Err(format!(
            "orientation {} is not a {}",
            c.orientation,
            c.kind.label()
        ));
    }
    Ok((c, left))
}

/// `tool newtile <kind> [o=] [h=] [roof=] [solid=<index>] [image=<file>]
/// [place=as-is|center|fit] [dither=fs|none] [act=1-5] [into=map|lib]`:
/// sets the dialog's inputs and opens it.
pub fn script_set(state: &mut AppState, args: &[&str]) -> Result<(), String> {
    let (choice, rest) = parse_shape(args)?;
    let mut inp = Inputs {
        choice,
        ..Inputs::default()
    };
    let mut target = NewTilesTarget::Map;
    let mut act = map_act(state);
    for t in rest {
        if let Some(v) = kv(t, "solid") {
            inp.source = SourceKind::Solid;
            inp.color = v
                .parse()
                .map_err(|_| format!("{v:?} is not a palette index"))?;
        } else if let Some(v) = kv(t, "image") {
            inp.source = SourceKind::Image;
            inp.image = Some(Picked::read(PathBuf::from(v))?);
        } else if let Some(v) = kv(t, "place") {
            inp.placement = match v {
                "as-is" => Placement::AsIs,
                "center" | "centre" => Placement::Center,
                "fit" => Placement::Fit,
                _ => return Err(format!("{v:?}: place is as-is, center or fit")),
            };
        } else if let Some(v) = kv(t, "dither") {
            inp.matching.dither = match v {
                "fs" => capy_sprite::Dither::FloydSteinberg,
                _ => capy_sprite::Dither::None,
            };
        } else if let Some(v) = kv(t, "act") {
            act = v
                .parse::<u8>()
                .ok()
                .filter(|a| (1..=5).contains(a))
                .ok_or("act is 1 to 5")?
                - 1;
        } else if let Some(v) = kv(t, "into") {
            target = match v {
                "map" => NewTilesTarget::Map,
                "lib" => NewTilesTarget::Library(
                    state
                        .tools
                        .dt1
                        .lib
                        .as_ref()
                        .map(|l| l.path.clone())
                        .ok_or("no library is open in Tile libraries")?,
                ),
                _ => return Err(format!("{v:?}: into is map or lib")),
            };
        } else if t == "blank" {
            inp.source = SourceKind::Blank;
        } else {
            return Err(format!("unknown newtile option {t:?}"));
        }
    }
    open(state, target);
    let ui = &mut state.tools.newtile;
    ui.inputs = inp;
    ui.act = act;
    Ok(())
}

/// `tool newtile-flags <op> <hex> <cells|all>` on the dialog's flags.
pub fn script_flags(state: &mut AppState, op: FlagOp, cells: &[usize]) {
    let ui = &mut state.tools.newtile;
    let mut f = ui.inputs.effective_flags();
    for &c in cells {
        if let Some(v) = f.get_mut(c) {
            *v = op.apply(*v);
        }
    }
    ui.inputs.flags = Some(f);
}

/// The preview's warnings, for `tool newtile-assert-warning`.
pub fn script_warnings(state: &mut AppState) -> Result<Vec<String>, String> {
    let mut ui = std::mem::take(&mut state.tools.newtile);
    let r = ui.preview_messages(state);
    state.tools.newtile = ui;
    r
}

// ---------------------------- Shape... ----------------------------

/// What a reshape loss was worked out for: the shape, the tile and the
/// revision of the document.
pub type LossKey = (ShapeChoice, usize, u64);

/// The "Shape..." window's state: the picked shape and the pixel loss
/// worked out for it.
#[derive(Clone, Debug)]
pub struct ShapeDialog {
    pub choice: ShapeChoice,
    pub loss: Option<(LossKey, Result<usize, String>)>,
}

impl ShapeDialog {
    pub fn new(shape: TileShape) -> ShapeDialog {
        ShapeDialog {
            choice: ShapeChoice::of(shape),
            loss: None,
        }
    }
}

/// "Left wall (1), 96 px" / "Roof (15), roof height 80" / "Floor (0)".
pub fn describe_shape(s: TileShape) -> String {
    let mut n = orientation_name(s.orientation);
    match s.class() {
        Some(TileClass::Roof) => n.push_str(&format!(", roof height {}", s.roof_height)),
        Some(TileClass::Floor) | None => {}
        Some(_) => n.push_str(&format!(", {} px", stored_px(&s))),
    }
    n
}

/// The warning shown before re-splitting a tile that loses `dropped`
/// pixels; `None` when every pixel fits.
pub fn reshape_warning(dropped: usize) -> Option<String> {
    (dropped > 0).then(|| {
        format!(
            "{dropped} pixel(s) lie outside the new shape and will be lost (undo brings them back)"
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn grey() -> Palette {
        crate::sprites::palette::grey_palette()
    }

    #[test]
    fn heights_round_up_to_whole_blocks() {
        let c = ShapeChoice {
            kind: Kind::UpperWall,
            height: 70,
            ..ShapeChoice::default()
        };
        assert_eq!(c.rounded_height(), 96);
        let shape = c.shape().unwrap().normalized().unwrap();
        assert_eq!(shape.height, 96);
    }

    #[test]
    fn a_solid_floor_fills_the_diamond() {
        let inp = Inputs {
            source: SourceKind::Solid,
            color: 30,
            ..Inputs::default()
        };
        let b = build(&inp, &grey(), 4, 1).unwrap();
        assert_eq!(b.tiles.len(), 1);
        let t = &b.tiles[0].tile;
        assert_eq!((t.orientation, t.main_index, t.sub_index), (0, 4, 1));
        // all 25 cells full: isometric blocks
        assert_eq!(t.blocks.len(), 25);
        assert!(b.warnings().is_empty());
    }

    #[test]
    fn the_corner_pair_builds_both_halves_with_one_number() {
        let inp = Inputs {
            choice: ShapeChoice {
                kind: Kind::CornerPair,
                height: 64,
                ..ShapeChoice::default()
            },
            source: SourceKind::Solid,
            color: 9,
            ..Inputs::default()
        };
        let b = build(&inp, &grey(), 2, 7).unwrap();
        let o: Vec<(i32, i32, i32)> = b
            .tiles
            .iter()
            .map(|t| (t.tile.orientation, t.tile.main_index, t.tile.sub_index))
            .collect();
        assert_eq!(o, [(3, 2, 7), (4, 2, 7)]);
        assert_eq!(b.shapes.len(), 2);
    }

    #[test]
    fn custom_flags_replace_the_defaults() {
        let mut inp = Inputs {
            choice: ShapeChoice {
                kind: Kind::UpperWall,
                orientation: 2,
                ..ShapeChoice::default()
            },
            ..Inputs::default()
        };
        assert_eq!(inp.effective_flags(), default_flags(2));
        let mut f = [0u8; 25];
        f[12] = 0x01;
        inp.flags = Some(f);
        let b = build(&inp, &grey(), 0, 0).unwrap();
        assert_eq!(b.tiles[0].tile.subtile_flags, f);
    }

    #[test]
    fn a_picture_too_big_reports_what_is_left_out() {
        // an 8x8 opaque PNG placed as-is on a floor: the top-left corner
        // lies outside the diamond
        let img = capy_render::RgbaImage::filled(8, 8, [200, 200, 200, 255]);
        let inp = Inputs {
            source: SourceKind::Image,
            image: Some(Picked {
                path: PathBuf::from("x.png"),
                bytes: img.to_png().unwrap(),
                kind: ImageKind::Png,
            }),
            placement: Placement::AsIs,
            ..Inputs::default()
        };
        let b = build(&inp, &grey(), 0, 0).unwrap();
        assert!(b.tiles[0].dropped > 0);
        assert!(b.warnings()[0].contains("outside the tile's drawable area"));
        let (img, origin) = preview_image(&b, &grey());
        assert_eq!(origin, (0, 0));
        assert_eq!(img.size, [160, 80]);
    }

    #[test]
    fn walls_stand_on_their_cell_in_the_preview() {
        assert_eq!(anchor_in_cell(TileShape::wall(1, 96)), (0, 80));
        assert_eq!(anchor_in_cell(TileShape::roof(80)), (0, -80));
        assert_eq!(anchor_in_cell(TileShape::floor()), (0, 0));
        assert_eq!(orientation_name(1), "Left wall (1)");
        assert_eq!(orientation_name(16), "Lower wall, left (16)");
    }
}
