//! The CPU half of rendering a map, run on a worker thread: load the DT1s,
//! take the draw list, bake the atlas and lay out the quads. Everything here
//! is plain data in and out, so it is tested without Bevy.

use std::sync::Arc;
use std::time::{Duration, Instant};

use capy_render::{DrawItem, MissingTile, TileSet, Visibility, draw_list};
use d2_assets::{GamePath, Overlay};
use d2_formats::ds1::Ds1;
use d2_formats::palette::{Palette, act_palette_path};

use super::atlas::{self, AtlasKey, AtlasLayout, AtlasPixels, PaletteKey};
use super::geometry::{self, TexGeometry};

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
}

impl BuildKey {
    pub fn palette_key(&self) -> PaletteKey {
        PaletteKey {
            act: self.palette_act,
            gamma_bits: self.gamma_bits,
        }
    }
}

/// Everything visible, including special tiles: the set an atlas must cover so
/// that toggling layers never needs a re-bake.
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
}

pub struct BuildOutput {
    pub items: Arc<Vec<DrawItem>>,
    pub geometry: TexGeometry,
    pub missing: Vec<MissingTile>,
    pub layout: Arc<AtlasLayout>,
    /// `Some` when the atlas was (re)built and must be uploaded.
    pub pixels: Option<AtlasPixels>,
    /// 1:1 map-space bounds of everything drawn.
    pub bounds: Option<(i32, i32, i32, i32)>,
    pub elapsed: Duration,
}

/// The draw list, an atlas (reused when `prior` already holds every tile) and
/// the quads in draw order.
pub fn build_map(input: BuildInput) -> Result<BuildOutput, String> {
    let started = Instant::now();
    let (items, missing) =
        draw_list(&input.ds1, &input.tiles, &input.vis).map_err(|e| e.to_string())?;
    let needed: Vec<AtlasKey> = items.iter().map(geometry::atlas_key).collect();

    let reusable = input
        .prior
        .as_ref()
        .filter(|p| p.palette == input.palette_key && p.covers(&needed))
        .cloned();
    let (layout, pixels) = match reusable {
        Some(layout) => (layout, None),
        None => {
            // Cover every layer toggle: bake what the all-visible list needs.
            let (full, _) = draw_list(&input.ds1, &input.tiles, &visibility_all())
                .map_err(|e| e.to_string())?;
            let mut entries = Vec::with_capacity(full.len());
            let mut seen = std::collections::HashSet::new();
            for item in &full {
                let key = geometry::atlas_key(item);
                if seen.insert(key.clone()) {
                    let image = input.tiles.image(&item.tile).map_err(|e| e.to_string())?;
                    entries.push((key, image));
                }
            }
            let gamma = f32::from_bits(input.palette_key.gamma_bits);
            let palette = input.palette.with_gamma(gamma);
            let (layout, pixels) =
                atlas::build(&entries, &palette, input.palette_key).map_err(|e| e.to_string())?;
            (Arc::new(layout), Some(pixels))
        }
    };
    let tiles = input.tiles.clone();
    let geometry = geometry::build_quads(&items, &layout, input.div, &|r| tiles.geometry(r));
    let bounds = capy_render::map_screen_bounds(&items, &input.tiles);
    Ok(BuildOutput {
        items: Arc::new(items),
        geometry,
        missing,
        layout,
        pixels,
        bounds,
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
        }
    }

    #[test]
    fn quads_follow_the_draw_list_one_for_one() {
        let out = build_map(input(Visibility::default(), None)).expect("build");
        // Three floors + one shadow; the (9, 9) cell is missing.
        assert_eq!(out.items.len(), 4);
        assert_eq!(out.geometry.quad_count(), out.items.len());
        assert_eq!(out.missing.len(), 1);
        assert_eq!((out.missing[0].x, out.missing[0].y), (2, 2));
        for (k, item) in out.items.iter().enumerate() {
            assert_eq!(
                out.geometry.positions[4 * k][0],
                item.screen_x as f32,
                "quad {k} is item {k}"
            );
        }
        // The shadow is drawn after the floor of its cell and is a separate
        // atlas entry from the floor tiles.
        let passes: Vec<DrawPass> = out.items.iter().map(|i| i.pass).collect();
        assert!(passes.contains(&DrawPass::Shadows));
        assert!(out.pixels.is_some());
        assert!(out.bounds.is_some());
    }

    #[test]
    fn toggling_a_layer_reuses_the_atlas() {
        let first = build_map(input(Visibility::default(), None)).expect("build");
        let vis = Visibility {
            shadow: false,
            ..Visibility::default()
        };
        let second = build_map(input(vis, Some(first.layout.clone()))).expect("rebuild");
        assert!(
            second.pixels.is_none(),
            "no re-bake for a visibility change"
        );
        assert_eq!(second.items.len(), 3);
        assert!(Arc::ptr_eq(&first.layout, &second.layout));
        // A different palette key forces a new bake.
        let mut changed = input(Visibility::default(), Some(first.layout.clone()));
        changed.palette_key.gamma_bits = 1.3f32.to_bits();
        assert!(build_map(changed).expect("gamma").pixels.is_some());
    }

    #[test]
    fn hidden_layers_are_baked_so_toggling_never_misses() {
        let vis = Visibility {
            shadow: false,
            ..Visibility::default()
        };
        let hidden_first = build_map(input(vis, None)).expect("build");
        // The atlas already holds the shadow entry although it is not drawn.
        let shadow_in_atlas = hidden_first.layout.rects.keys().any(|k| k.shadow);
        assert!(shadow_in_atlas);
        let shown = build_map(input(
            Visibility::default(),
            Some(hidden_first.layout.clone()),
        ))
        .expect("show");
        assert!(shown.pixels.is_none());
        assert_eq!(shown.items.len(), 4);
    }

    #[test]
    fn baked_shadow_texels_are_premultiplied_in_the_pixels() {
        let out = build_map(input(Visibility::default(), None)).expect("build");
        let pixels = out.pixels.expect("pixels");
        let rect = out
            .layout
            .rects
            .iter()
            .find(|(k, _)| k.shadow)
            .map(|(_, r)| *r)
            .expect("shadow rect");
        let i = (((rect.y + rect.h / 2) * pixels.size + rect.x + 80) * 4) as usize;
        let texel = &pixels.data[i..i + 4];
        // Palette index 3 is (3, 1, 252); alpha 128, colour premultiplied.
        assert_eq!(texel[3], atlas::SHADOW_ALPHA);
        assert_eq!(texel[2], atlas::shadow_texel(252));
    }
}
