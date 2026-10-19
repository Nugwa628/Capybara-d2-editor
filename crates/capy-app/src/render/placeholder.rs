//! Flat-coloured diamonds, one colour per layer kind. Replaced by textured
//! tiles in the next brief; only [`PlaceholderBuilder`] has to be swapped.

use d2_formats::ds1::Ds1;

use super::{LayerBuilder, LayerGeometry};
use crate::doc::{LayerKey, LayerKind};
use crate::iso::{self, TILE_H, TILE_W};

/// sRGB colour (alpha is linear) per layer kind.
fn kind_color(kind: LayerKind) -> [f32; 4] {
    match kind {
        LayerKind::Floor => [0.33, 0.50, 0.30, 1.0],
        LayerKind::Wall => [0.62, 0.55, 0.47, 1.0],
        LayerKind::Shadow => [0.0, 0.0, 0.0, 0.45],
        LayerKind::Tag => [0.85, 0.25, 0.80, 0.55],
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_linear(c: [f32; 4]) -> [f32; 4] {
    [
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
        c[3],
    ]
}

/// Draw depth: floors under shadows under walls under tags.
fn layer_z(key: LayerKey) -> f32 {
    match key {
        LayerKey::Floor(i) => 0.1 * i as f32,
        LayerKey::Shadow => 1.0,
        LayerKey::Wall(i) => 2.0 + 0.1 * i as f32,
        LayerKey::Tag => 5.0,
    }
}

pub struct PlaceholderBuilder;

fn push_diamond(geo: &mut LayerGeometry, x: i32, y: i32, z: f32, scale: f64, color: [f32; 4]) {
    let [cx, cy] = iso::cell_center(x, y);
    let (hw, hh) = (TILE_W / 2.0 * scale, TILE_H / 2.0 * scale);
    // Iso y points down; the world is y-up, so flip.
    let p = |dx: f64, dy: f64| [(cx + dx) as f32, -((cy + dy) as f32), z];
    let base = geo.positions.len() as u32;
    geo.positions
        .extend([p(0.0, -hh), p(hw, 0.0), p(0.0, hh), p(-hw, 0.0)]);
    geo.colors.extend([color; 4]);
    geo.indices
        .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
}

fn shade(color: [f32; 4], x: i32, y: i32) -> [f32; 4] {
    // A faint checker makes individual cells readable.
    let k = if (x + y) % 2 == 0 { 1.0 } else { 0.93 };
    [color[0] * k, color[1] * k, color[2] * k, color[3]]
}

impl LayerBuilder for PlaceholderBuilder {
    fn build(&self, ds1: &Ds1, key: LayerKey) -> LayerGeometry {
        let mut geo = LayerGeometry::default();
        let kind = key.kind();
        let color = to_linear(kind_color(kind));
        let z = layer_z(key);
        let scale = if kind == LayerKind::Wall { 0.6 } else { 1.0 };
        let mut add = |x: u32, y: u32, filled: bool| {
            if filled {
                let (xi, yi) = (x as i32, y as i32);
                let c = if kind == LayerKind::Floor {
                    shade(color, xi, yi)
                } else {
                    color
                };
                push_diamond(&mut geo, xi, yi, z, scale, c);
            }
        };
        for y in 0..ds1.height {
            for x in 0..ds1.width {
                let filled = match key {
                    LayerKey::Floor(i) => ds1
                        .floors
                        .get(i)
                        .and_then(|g| g.get(x, y))
                        .is_some_and(|c| !c.is_empty()),
                    LayerKey::Wall(i) => ds1
                        .walls
                        .get(i)
                        .and_then(|w| w.tiles.get(x, y))
                        .is_some_and(|c| !c.is_empty()),
                    LayerKey::Shadow => ds1.shadow.get(x, y).is_some_and(|c| !c.is_empty()),
                    LayerKey::Tag => ds1
                        .tag
                        .as_ref()
                        .and_then(|g| g.get(x, y))
                        .is_some_and(|v| v != 0),
                };
                add(x, y, filled);
            }
        }
        geo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::ds1::Cell;

    #[test]
    fn one_diamond_per_non_empty_cell() {
        let mut ds1 = Ds1::new(4, 3, 0);
        // A cell with a main index is non-empty.
        let cell = Cell(1).with_indices(1, 1);
        assert!(!cell.is_empty());
        ds1.floors[0].set(0, 0, cell);
        ds1.floors[0].set(3, 2, cell);
        let geo = PlaceholderBuilder.build(&ds1, LayerKey::Floor(0));
        assert_eq!(geo.positions.len(), 8);
        assert_eq!(geo.colors.len(), 8);
        assert_eq!(geo.indices.len(), 12);
        assert!(
            geo.indices
                .iter()
                .all(|&i| (i as usize) < geo.positions.len())
        );
        // Empty layers produce no geometry.
        assert!(PlaceholderBuilder.build(&ds1, LayerKey::Shadow).is_empty());
        assert!(PlaceholderBuilder.build(&ds1, LayerKey::Wall(0)).is_empty());
    }

    #[test]
    fn diamond_is_160_by_80_at_the_cell() {
        let mut ds1 = Ds1::new(3, 3, 0);
        ds1.floors[0].set(2, 1, Cell(1).with_indices(1, 0));
        let geo = PlaceholderBuilder.build(&ds1, LayerKey::Floor(0));
        let xs: Vec<f32> = geo.positions.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = geo.positions.iter().map(|p| p[1]).collect();
        let top = iso::cell_top(2, 1);
        let min_x = xs.iter().cloned().fold(f32::MAX, f32::min);
        let max_x = xs.iter().cloned().fold(f32::MIN, f32::max);
        let max_y = ys.iter().cloned().fold(f32::MIN, f32::max);
        let min_y = ys.iter().cloned().fold(f32::MAX, f32::min);
        assert_eq!(max_x - min_x, 160.0);
        assert_eq!(max_y - min_y, 80.0);
        assert_eq!(max_y, -(top[1] as f32), "top corner at the cell's top");
        assert_eq!((min_x + max_x) / 2.0, top[0] as f32);
    }

    #[test]
    fn tag_layer_draws_non_zero_values() {
        let mut ds1 = Ds1::new(2, 2, 0);
        ds1.tag = Some(d2_formats::Grid::new(2, 2));
        if let Some(t) = ds1.tag.as_mut() {
            t.set(1, 1, 7);
        }
        let geo = PlaceholderBuilder.build(&ds1, LayerKey::Tag);
        assert_eq!(geo.positions.len(), 4);
    }
}
