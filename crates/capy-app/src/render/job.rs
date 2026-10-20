//! The CPU half of rendering a map, run on a worker thread: load the DT1s,
//! take the draw list (with objects when the object tables are loaded), bake
//! the atlas (tiles and every frame of every object sprite) and lay out the
//! runs. Everything here is plain data in and out, so it is tested without
//! Bevy.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capy_render::{
    DrawEntry, DrawItem, MissingObject, MissingTile, ObjectOptions, ObjectSprites, TileSet,
    Visibility, draw_list_with_objects,
};
use d2_assets::objects::ObjectCatalog;
use d2_assets::{GamePath, Overlay};
use d2_formats::ds1::Ds1;
use d2_formats::palette::{Palette, act_palette_path};

use super::atlas::{self, AtlasKey, AtlasLayout, AtlasPixels, PaletteKey};
use super::geometry;
use super::scene::{self, ObjectAnim, ObjectEntry, Piece, Run, SceneEntry};

/// Everything the textured mesh of a document depends on. When it changes
/// the mesh (and maybe the atlas) is rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildKey {
    /// `MapDoc::rev`.
    pub rev: u64,
    pub vis: Visibility,
    pub gamma_bits: u32,
    pub palette_act: u8,
    /// The DT1s in load order; also the tileset cache key.
    pub dt1_files: Vec<GamePath>,
    /// The session epoch of the object tables the objects were drawn with;
    /// `None` = terrain only (tables not loaded yet, or unavailable).
    pub objects: Option<u64>,
    /// Upper/lower wall groups and roof hiding (ds1-studio's wall
    /// categories and "as if inside").
    pub filter: TileFilter,
}

/// Wall tiles left out of the drawing on top of [`Visibility`]: the upper
/// and lower wall groups of ds1-studio's Show bar (`wallCategories.ts`) and
/// the tiles a roof-hiding area hides while "as if inside" is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileFilter {
    pub hide_upper: bool,
    pub hide_lower: bool,
    /// DT1 paths (lower case) whose wall tiles count as lower walls
    /// (`true`) or upper walls (`false`) whatever their orientation.
    pub overrides: Vec<(String, bool)>,
    /// Wall-layer cells `(layer, x, y)` not drawn, sorted.
    pub hide_cells: Arc<Vec<(usize, u32, u32)>>,
}

/// ds1-studio's automatic wall group of an orientation: `Some(true)` lower,
/// `Some(false)` upper, `None` neither (floors, shadows, roofs, specials).
pub fn wall_group(orientation: i32) -> Option<bool> {
    if orientation >= 16 {
        Some(true)
    } else if (1..=9).contains(&orientation) || orientation == 12 || orientation == 14 {
        Some(false)
    } else {
        None
    }
}

impl TileFilter {
    pub fn is_noop(&self) -> bool {
        !self.hide_upper && !self.hide_lower && self.hide_cells.is_empty()
    }

    /// Is `item` drawn?
    pub fn keeps(&self, item: &DrawItem, tiles: &TileSet) -> bool {
        if self.is_noop() || item.layer.0 != 'w' {
            return true;
        }
        if !self.hide_cells.is_empty()
            && self
                .hide_cells
                .binary_search(&(usize::from(item.layer.1), item.x, item.y))
                .is_ok()
        {
            return false;
        }
        if !self.hide_upper && !self.hide_lower {
            return true;
        }
        let Some(tile) = tiles.tile(&item.tile) else {
            return true;
        };
        let Some(auto) = wall_group(tile.orientation) else {
            return true;
        };
        let path = tiles
            .dt1_paths()
            .get(item.tile.dt1)
            .map(|p| p.as_str().to_ascii_lowercase());
        let lower = path
            .and_then(|p| {
                self.overrides
                    .iter()
                    .find(|(o, _)| *o == p)
                    .map(|(_, l)| *l)
            })
            .unwrap_or(auto);
        !(lower && self.hide_lower || !lower && self.hide_upper)
    }
}

impl BuildKey {
    pub fn palette_key(&self) -> PaletteKey {
        PaletteKey {
            act: self.palette_act,
            gamma_bits: self.gamma_bits,
        }
    }
}

/// Everything visible, including special tiles and objects: the set an atlas
/// must cover so that toggling layers never needs a re-bake.
pub fn visibility_all() -> Visibility {
    Visibility {
        floors: [true; 2],
        walls: [true; 4],
        shadow: true,
        roofs: true,
        special: true,
        objects: true,
    }
}

pub struct BuildInput {
    pub tiles: Arc<TileSet>,
    pub ds1: Arc<Ds1>,
    pub vis: Visibility,
    /// The raw act palette; gamma is applied here.
    pub palette: Palette,
    pub palette_key: PaletteKey,
    /// The layout of the atlas already on the GPU, if it is still valid for
    /// `tiles`.
    pub prior: Option<Arc<AtlasLayout>>,
    /// Zoom divisor the quads are laid out for (not part of [`BuildKey`]:
    /// a zoom change only re-lays the quads out).
    pub div: u32,
    /// Animation tick the object runs are laid out for (`None` = frame 0).
    pub tick: Option<u32>,
    pub filter: TileFilter,
}

/// Where object sprites come from: a long-lived `ObjectSprites` (its cache
/// keeps the frame `Arc`s, and so the atlas keys, stable across builds).
pub struct ObjectSource<'a> {
    pub sprites: &'a ObjectSprites<'a>,
    pub catalog: &'a ObjectCatalog,
}

pub struct BuildOutput {
    pub entries: Arc<Vec<SceneEntry>>,
    /// The animated floors among `entries`.
    pub anims: Arc<scene::TileAnims>,
    pub pieces: Vec<Piece>,
    /// The runs of each piece, laid out for `div` and `tick`.
    pub runs: Vec<Vec<Run>>,
    pub missing: Vec<MissingTile>,
    pub missing_objects: Vec<MissingObject>,
    pub layout: Arc<AtlasLayout>,
    /// `Some` when the atlas was (re)built and must be uploaded.
    pub pixels: Option<AtlasPixels>,
    /// 1:1 map-space bounds of every tile drawn.
    pub bounds: Option<(i32, i32, i32, i32)>,
    /// Tiles drawn.
    pub tile_count: usize,
    /// Object entries drawn (shadows included).
    pub object_count: usize,
    pub elapsed: Duration,
}

/// The animation of object `index` of `ds1`, starting from the frame the
/// draw list holds (frame 0): every frame of its direction, and its speed
/// as `ObjectSprites` computes it (`objects.rs` `frame_of_info`).
fn object_anim(
    src: &ObjectSource<'_>,
    ds1: &Ds1,
    index: usize,
    first: &Arc<capy_render::ObjectFrame>,
) -> ObjectAnim {
    let Some(object) = ds1.objects.get(index) else {
        return ObjectAnim {
            frames: vec![Arc::clone(first)],
            delta: 256,
        };
    };
    let info = src
        .catalog
        .resolve(ds1.effective_act(), ds1.version, object);
    let direction = info.direction.unwrap_or(0);
    // The delta rule lives in capy-render: FrameDelta, else AnimData, and
    // 0/none mean 256.
    match src.sprites.frames_of_info(&info, direction) {
        Ok((mut frames, delta)) => {
            for (k, frame) in frames.iter_mut().enumerate() {
                if k == 0 || frame.layers.is_empty() {
                    *frame = Arc::clone(first);
                }
            }
            ObjectAnim { frames, delta }
        }
        Err(_) => ObjectAnim {
            frames: vec![Arc::clone(first)],
            delta: 256,
        },
    }
}

/// The draw list as scene entries, each object with its whole animation
/// (shared through `anims`, keyed by object index).
fn scene_entries(
    entries: Vec<DrawEntry>,
    ds1: &Ds1,
    objects: Option<&ObjectSource<'_>>,
    anims: &mut HashMap<usize, Arc<ObjectAnim>>,
) -> Vec<SceneEntry> {
    entries
        .into_iter()
        .filter_map(|e| match e {
            DrawEntry::Tile(t) => Some(SceneEntry::Tile(t)),
            DrawEntry::Object(o) => {
                let src = objects?;
                let anim = anims
                    .entry(o.object)
                    .or_insert_with(|| Arc::new(object_anim(src, ds1, o.object, &o.frame)))
                    .clone();
                Some(SceneEntry::Object(ObjectEntry {
                    pass: o.pass,
                    object: o.object,
                    x: o.x,
                    y: o.y,
                    anim,
                }))
            }
        })
        .collect()
}

/// The animated floors (water, lava) of the entries: every frame as a draw
/// item placed where the game draws it (`capy_render::anim`).
pub fn tile_anims(entries: &[SceneEntry], tiles: &TileSet) -> scene::TileAnims {
    let mut by_key: HashMap<d2_formats::dt1::TileKey, Vec<capy_render::TileRef>> = HashMap::new();
    let mut out = scene::TileAnims::new();
    for (i, e) in entries.iter().enumerate() {
        let SceneEntry::Tile(item) = e else { continue };
        if item.pass != capy_render::DrawPass::Floors {
            continue;
        }
        let Some(tile) = tiles.tile(&item.tile) else {
            continue;
        };
        if tile.orientation != 0 || tile.animated == 0 {
            continue;
        }
        let Some(key) = tile.key() else { continue };
        let frames = by_key
            .entry(key)
            .or_insert_with(|| capy_render::anim::animation_frames(tiles, key));
        if frames.len() < 2 {
            continue;
        }
        let Ok(own) = tiles.image(&item.tile) else {
            continue;
        };
        let items: Vec<DrawItem> = frames
            .iter()
            .map(|r| {
                let (dx, dy) = tiles
                    .image(r)
                    .map(|img| (img.offset_x - own.offset_x, img.offset_y - own.offset_y))
                    .unwrap_or((0, 0));
                DrawItem {
                    tile: r.clone(),
                    screen_x: item.screen_x + dx,
                    screen_y: item.screen_y + dy,
                    ..item.clone()
                }
            })
            .collect();
        out.insert(i, Arc::new(items));
    }
    out
}

/// Atlas keys (with their images) the entries need: their tiles (every
/// frame of animated floors), and every frame of their objects.
fn needed_keys(
    entries: &[SceneEntry],
    tiles: &TileSet,
) -> Result<Vec<(AtlasKey, Arc<d2_formats::IndexedImage>)>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let anims = tile_anims(entries, tiles);
    for (i, e) in entries.iter().enumerate() {
        if let SceneEntry::Tile(item) = e {
            let frames = anims.get(&i).map(|f| f.as_slice()).unwrap_or(&[]);
            for item in std::iter::once(item).chain(frames.iter()) {
                let key = geometry::atlas_key(item);
                if seen.insert(key.clone()) {
                    let image = tiles.image(&item.tile).map_err(|e| e.to_string())?;
                    out.push((key, image));
                }
            }
        }
    }
    for (key, image) in scene::sprite_keys(entries) {
        if seen.insert(key.clone()) {
            out.push((key, image));
        }
    }
    Ok(out)
}

/// The draw list, an atlas (reused when `prior` already holds every image)
/// and the runs in draw order.
pub fn build_map(
    input: BuildInput,
    objects: Option<&ObjectSource<'_>>,
) -> Result<BuildOutput, String> {
    let started = Instant::now();
    let opts = ObjectOptions::default();
    let source = objects.map(|o| (o.sprites, &opts));
    let list = draw_list_with_objects(&input.ds1, &input.tiles, &input.vis, source)
        .map_err(|e| e.to_string())?;
    let mut anims = HashMap::new();
    let mut entries = scene_entries(list.entries, &input.ds1, objects, &mut anims);
    if !input.filter.is_noop() {
        let tiles = &input.tiles;
        entries.retain(|e| match e {
            SceneEntry::Tile(item) => input.filter.keeps(item, tiles),
            SceneEntry::Object(_) => true,
        });
    }
    let tile_anims = Arc::new(tile_anims(&entries, &input.tiles));
    let needed = needed_keys(&entries, &input.tiles)?;

    let reusable = input
        .prior
        .as_ref()
        .filter(|p| p.palette == input.palette_key && p.covers(needed.iter().map(|(k, _)| k)))
        .cloned();
    let (layout, pixels) = match reusable {
        Some(layout) => (layout, None),
        None => {
            // Cover every layer toggle: bake what the all-visible list needs.
            let full = draw_list_with_objects(&input.ds1, &input.tiles, &visibility_all(), source)
                .map_err(|e| e.to_string())?;
            let full = scene_entries(full.entries, &input.ds1, objects, &mut anims);
            let all = needed_keys(&full, &input.tiles)?;
            let gamma = f32::from_bits(input.palette_key.gamma_bits);
            let palette = input.palette.with_gamma(gamma);
            let (layout, pixels) =
                atlas::build(&all, &palette, input.palette_key).map_err(|e| e.to_string())?;
            (Arc::new(layout), Some(pixels))
        }
    };
    let pieces = scene::pieces(&entries);
    let tiles = input.tiles.clone();
    let runs = pieces
        .iter()
        .map(|p| {
            scene::build_piece_anim(
                &entries,
                p,
                &layout,
                input.div,
                &|r| tiles.geometry(r),
                input.tick,
                &tile_anims,
                None,
            )
        })
        .collect();
    let tile_items: Vec<DrawItem> = entries
        .iter()
        .filter_map(|e| match e {
            SceneEntry::Tile(t) => Some(t.clone()),
            SceneEntry::Object(_) => None,
        })
        .collect();
    let bounds = capy_render::map_screen_bounds(&tile_items, &input.tiles);
    let object_count = entries.len() - tile_items.len();
    Ok(BuildOutput {
        entries: Arc::new(entries),
        anims: tile_anims,
        pieces,
        runs,
        missing: list.missing,
        missing_objects: list.missing_objects,
        layout,
        pixels,
        bounds,
        tile_count: tile_items.len(),
        object_count,
        elapsed: started.elapsed(),
    })
}

/// Loads the act palette (`pal.dat`) through the overlay.
pub fn load_palette(overlay: &Overlay, act: u8) -> Result<Palette, String> {
    let path = GamePath::new(act_palette_path(act));
    let bytes = overlay
        .read_bytes(&path)
        .map_err(|e| format!("reading {path}: {e}"))?
        .ok_or_else(|| format!("{path} is not in the profile"))?;
    Palette::parse_dat(&bytes).map_err(|e| format!("parsing {path}: {e}"))
}

/// Loads the DT1s of a map. Unreadable ones come back as warnings.
pub fn load_tileset(
    overlay: &Overlay,
    dt1_files: &[GamePath],
) -> Result<(Arc<TileSet>, Vec<String>), String> {
    let (tiles, warnings) = TileSet::load(overlay, dt1_files).map_err(|e| e.to_string())?;
    Ok((Arc::new(tiles), warnings))
}

#[cfg(test)]
mod tests {
    use capy_render::DrawPass;
    use d2_formats::ds1::Cell;
    use d2_formats::dt1::{Block, BlockFormat, Dt1, Tile};

    use super::*;

    fn block(x: i16, y: i16, pixel: u8) -> Block {
        Block {
            x,
            y,
            grid_x: 0,
            grid_y: 0,
            raw_format: 1,
            format: BlockFormat::Isometric,
            data: vec![pixel; 256],
        }
    }

    fn tile(orientation: i32, main: i32, sub: i32, pixel: u8) -> Tile {
        let (height, y) = match orientation {
            0 => (-80, 0),
            13 => (-96, -15),
            _ => (-128, -15),
        };
        Tile {
            direction: 0,
            roof_height: 0,
            sound: 0,
            animated: 0,
            height,
            width: 160,
            orientation,
            main_index: main,
            sub_index: sub,
            rarity: 1,
            subtile_flags: [0; 25],
            preserved: Default::default(),
            blocks: (0..5).map(|i| block(i * 32, y, pixel)).collect(),
        }
    }

    fn tileset() -> Arc<TileSet> {
        let dt1 = Dt1 {
            version: (7, 6),
            tiles: vec![tile(0, 1, 0, 7), tile(0, 2, 0, 9), tile(13, 1, 0, 3)],
        };
        Arc::new(TileSet::from_dt1s(
            vec![GamePath::new("data\\global\\tiles\\t.dt1")],
            vec![dt1],
        ))
    }

    fn palette() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8, (i / 2) as u8, 255 - i as u8];
        }
        Palette { colors }
    }

    fn map() -> Arc<Ds1> {
        let mut ds1 = Ds1::new(3, 3, 0);
        ds1.floors[0].set(0, 0, Cell(1).with_indices(1, 0));
        ds1.floors[0].set(1, 0, Cell(1).with_indices(2, 0));
        ds1.floors[0].set(1, 1, Cell(1).with_indices(1, 0));
        ds1.floors[0].set(2, 2, Cell(1).with_indices(9, 9)); // no such tile
        ds1.shadow.set(0, 0, Cell(1).with_indices(1, 0));
        Arc::new(ds1)
    }

    fn input(vis: Visibility, prior: Option<Arc<AtlasLayout>>) -> BuildInput {
        BuildInput {
            tiles: tileset(),
            ds1: map(),
            vis,
            palette: palette(),
            palette_key: PaletteKey {
                act: 0,
                gamma_bits: 1.0f32.to_bits(),
            },
            prior,
            div: 1,
            tick: None,
            filter: TileFilter::default(),
        }
    }

    fn items(out: &BuildOutput) -> Vec<DrawItem> {
        out.entries
            .iter()
            .filter_map(|e| match e {
                SceneEntry::Tile(t) => Some(t.clone()),
                SceneEntry::Object(_) => None,
            })
            .collect()
    }

    #[test]
    fn quads_follow_the_draw_list_one_for_one() {
        let out = build_map(input(Visibility::default(), None), None).expect("build");
        // Three floors + one shadow; the (9, 9) cell is missing.
        let items = items(&out);
        assert_eq!(items.len(), 4);
        assert_eq!(out.pieces.len(), 1, "terrain only: one piece");
        assert_eq!(out.runs[0].len(), 1, "one premultiplied run");
        let geometry = &out.runs[0][0].geo;
        assert_eq!(geometry.quad_count(), items.len());
        assert_eq!(out.missing.len(), 1);
        assert_eq!((out.missing[0].x, out.missing[0].y), (2, 2));
        for (k, item) in items.iter().enumerate() {
            assert_eq!(
                geometry.positions[4 * k][0],
                item.screen_x as f32,
                "quad {k} is item {k}"
            );
        }
        // The shadow is drawn after the floor of its cell and is a separate
        // atlas entry from the floor tiles.
        let passes: Vec<DrawPass> = items.iter().map(|i| i.pass).collect();
        assert!(passes.contains(&DrawPass::Shadows));
        assert!(out.pixels.is_some());
        assert!(out.bounds.is_some());
    }

    #[test]
    fn toggling_a_layer_reuses_the_atlas() {
        let first = build_map(input(Visibility::default(), None), None).expect("build");
        let vis = Visibility {
            shadow: false,
            ..Visibility::default()
        };
        let second = build_map(input(vis, Some(first.layout.clone())), None).expect("rebuild");
        assert!(
            second.pixels.is_none(),
            "no re-bake for a visibility change"
        );
        assert_eq!(items(&second).len(), 3);
        assert!(Arc::ptr_eq(&first.layout, &second.layout));
        // A different palette key forces a new bake.
        let mut changed = input(Visibility::default(), Some(first.layout.clone()));
        changed.palette_key.gamma_bits = 1.3f32.to_bits();
        assert!(build_map(changed, None).expect("gamma").pixels.is_some());
    }

    #[test]
    fn hidden_layers_are_baked_so_toggling_never_misses() {
        let vis = Visibility {
            shadow: false,
            ..Visibility::default()
        };
        let hidden_first = build_map(input(vis, None), None).expect("build");
        // The atlas already holds the shadow entry although it is not drawn.
        let shadow_in_atlas = hidden_first
            .layout
            .rects
            .keys()
            .any(AtlasKey::is_shadow_tile);
        assert!(shadow_in_atlas);
        let shown = build_map(
            input(Visibility::default(), Some(hidden_first.layout.clone())),
            None,
        )
        .expect("show");
        assert!(shown.pixels.is_none());
        assert_eq!(items(&shown).len(), 4);
    }

    #[test]
    fn baked_shadow_texels_are_premultiplied_in_the_pixels() {
        let out = build_map(input(Visibility::default(), None), None).expect("build");
        let pixels = out.pixels.expect("pixels");
        let rect = out
            .layout
            .rects
            .iter()
            .find(|(k, _)| k.is_shadow_tile())
            .map(|(_, r)| *r)
            .expect("shadow rect");
        let i = (((rect.y + rect.h / 2) * pixels.size + rect.x + 80) * 4) as usize;
        let texel = &pixels.data[i..i + 4];
        // Palette index 3 is (3, 1, 252); alpha 128, colour premultiplied.
        assert_eq!(texel[3], atlas::SHADOW_ALPHA);
        assert_eq!(texel[2], atlas::shadow_texel(252));
    }

    #[test]
    fn animated_floors_carry_every_frame_and_the_atlas_holds_them() {
        let mut a = tile(0, 3, 0, 20);
        let mut b = tile(0, 3, 0, 40);
        a.animated = 1;
        b.animated = 1;
        a.rarity = 0;
        b.rarity = 1;
        let dt1 = Dt1 {
            version: (7, 6),
            tiles: vec![a, b, tile(0, 1, 0, 7)],
        };
        let tiles = Arc::new(TileSet::from_dt1s(
            vec![GamePath::new("data\\global\\tiles\\w.dt1")],
            vec![dt1],
        ));
        let mut ds1 = Ds1::new(2, 2, 0);
        ds1.floors[0].set(0, 0, Cell(1).with_indices(3, 0));
        ds1.floors[0].set(1, 0, Cell(1).with_indices(1, 0));
        let out = build_map(
            BuildInput {
                tiles,
                ds1: Arc::new(ds1),
                ..input(Visibility::default(), None)
            },
            None,
        )
        .expect("build");
        assert_eq!(out.anims.len(), 1, "only the water cell animates");
        let frames = out.anims.values().next().expect("frames");
        assert_eq!(frames.len(), 2);
        for f in frames.iter() {
            assert!(out.layout.rects.contains_key(&geometry::atlas_key(f)));
        }
        assert_eq!(scene::tile_frame(frames, Some(0.0)), 0);
        assert_eq!(scene::tile_frame(frames, Some(0.15)), 1);
        assert_eq!(scene::tile_frame(frames, None), 0);
    }

    #[test]
    fn the_filter_hides_wall_groups_and_cells() {
        let t = tileset();
        let item = DrawItem {
            pass: DrawPass::LowerWalls,
            order: 0,
            layer: ('w', 0),
            x: 1,
            y: 2,
            tile: capy_render::TileRef { dt1: 0, tile: 0 },
            screen_x: 0,
            screen_y: 0,
        };
        // Tile 0 is a floor: in no wall group, kept unless its cell is hidden.
        let f = TileFilter {
            hide_lower: true,
            ..TileFilter::default()
        };
        assert!(f.keeps(&item, &t));
        let f = TileFilter {
            hide_cells: Arc::new(vec![(0, 1, 2)]),
            ..TileFilter::default()
        };
        assert!(!f.keeps(&item, &t));
        assert_eq!(wall_group(17), Some(true));
        assert_eq!(wall_group(3), Some(false));
        assert_eq!(wall_group(15), None);
    }
}
