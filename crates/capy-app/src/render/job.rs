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
}

/// Where object sprites come from: a long-lived `ObjectSprites` (its cache
/// keeps the frame `Arc`s, and so the atlas keys, stable across builds).
pub struct ObjectSource<'a> {
    pub sprites: &'a ObjectSprites<'a>,
    pub catalog: &'a ObjectCatalog,
}

pub struct BuildOutput {
    pub entries: Arc<Vec<SceneEntry>>,
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

/// Atlas keys (with their images) the entries need: their tiles, and every
/// frame of their objects.
fn needed_keys(
    entries: &[SceneEntry],
    tiles: &TileSet,
) -> Result<Vec<(AtlasKey, Arc<d2_formats::IndexedImage>)>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for e in entries {
        if let SceneEntry::Tile(item) = e {
            let key = geometry::atlas_key(item);
            if seen.insert(key.clone()) {
                let image = tiles.image(&item.tile).map_err(|e| e.to_string())?;
                out.push((key, image));
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
    let entries = scene_entries(list.entries, &input.ds1, objects, &mut anims);
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
            scene::build_piece(
                &entries,
                p,
                &layout,
                input.div,
                &|r| tiles.geometry(r),
                input.tick,
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
}
