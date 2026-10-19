//! Isometric math, with no Bevy or egui types so it tests fast.
//!
//! "Iso space" is screen-like: x right, y down, in pixels at 1:1. A cell's
//! floor diamond is 160 x 80 and cell (x, y) has its top corner at
//! `((x - y) * 80, (x + y) * 40)`.

pub const TILE_W: f64 = 160.0;
pub const TILE_H: f64 = 80.0;
const HALF_W: f64 = TILE_W / 2.0;
const HALF_H: f64 = TILE_H / 2.0;

/// Zoom divisors: index 0 is 1:1, the last is 1:16.
pub const ZOOM_STEPS: [u32; 8] = [1, 2, 3, 4, 6, 8, 12, 16];

pub type Vec2d = [f64; 2];

/// Next zoom index; `zoom_in` moves toward 1:1. Saturates at both ends.
pub fn step_zoom(index: usize, zoom_in: bool) -> usize {
    let last = ZOOM_STEPS.len() - 1;
    let index = index.min(last);
    if zoom_in {
        index.saturating_sub(1)
    } else {
        (index + 1).min(last)
    }
}

/// Top corner of cell `(x, y)`'s diamond.
pub fn cell_top(x: i32, y: i32) -> Vec2d {
    [f64::from(x - y) * HALF_W, f64::from(x + y) * HALF_H]
}

/// Centre of cell `(x, y)`'s diamond.
pub fn cell_center(x: i32, y: i32) -> Vec2d {
    let [sx, sy] = cell_top(x, y);
    [sx, sy + HALF_H]
}

/// Iso position of a continuous tile-space point, where cell `(x, y)` spans
/// `[x, x + 1] x [y, y + 1]` (so `(x, y)` is the cell's top corner and
/// `(x + 0.5, y + 0.5)` its centre).
pub fn tile_to_iso(u: f64, v: f64) -> Vec2d {
    [(u - v) * HALF_W, (u + v) * HALF_H]
}

/// Inverse of [`tile_to_iso`].
pub fn iso_to_tile(p: Vec2d) -> (f64, f64) {
    let a = p[0] / HALF_W; // u - v
    let b = p[1] / HALF_H; // u + v
    ((a + b) / 2.0, (b - a) / 2.0)
}

/// The cell whose diamond contains the iso point. Not bounds-checked.
pub fn screen_to_cell(p: Vec2d) -> (i32, i32) {
    // Continuous cell coordinates with integers at diamond centres.
    let a = p[0] / HALF_W; // x - y
    let b = p[1] / HALF_H - 1.0; // x + y
    let fx = (a + b) / 2.0;
    let fy = (b - a) / 2.0;
    ((fx + 0.5).floor() as i32, (fy + 0.5).floor() as i32)
}

/// Iso-space bounding box `[min_x, min_y, max_x, max_y]` of a `w` x `h` map.
pub fn map_bounds(w: u32, h: u32) -> [f64; 4] {
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    [
        -(h - 1.0) * HALF_W - HALF_W,
        0.0,
        (w - 1.0) * HALF_W + HALF_W,
        (w - 1.0 + h - 1.0) * HALF_H + TILE_H,
    ]
}

/// Camera state: which iso point is at the viewport centre, and the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: Vec2d,
    pub zoom: usize,
}

impl Default for View {
    fn default() -> Self {
        View {
            center: [0.0, 0.0],
            zoom: 0,
        }
    }
}

impl View {
    /// Iso pixels per screen pixel (the N of 1:N).
    pub fn divisor(&self) -> f64 {
        f64::from(ZOOM_STEPS[self.zoom.min(ZOOM_STEPS.len() - 1)])
    }

    /// Iso point under a pixel offset from the viewport centre (y down).
    pub fn offset_to_iso(&self, offset_px: Vec2d) -> Vec2d {
        let d = self.divisor();
        [
            self.center[0] + offset_px[0] * d,
            self.center[1] + offset_px[1] * d,
        ]
    }

    /// Inverse of [`View::offset_to_iso`].
    pub fn iso_to_offset(&self, iso: Vec2d) -> Vec2d {
        let d = self.divisor();
        [(iso[0] - self.center[0]) / d, (iso[1] - self.center[1]) / d]
    }

    /// One zoom step, keeping the iso point under the cursor fixed.
    pub fn zoom_at(&mut self, cursor_offset_px: Vec2d, zoom_in: bool) {
        let new_zoom = step_zoom(self.zoom, zoom_in);
        if new_zoom == self.zoom {
            return;
        }
        let anchor = self.offset_to_iso(cursor_offset_px);
        self.zoom = new_zoom;
        let d = self.divisor();
        self.center = [
            anchor[0] - cursor_offset_px[0] * d,
            anchor[1] - cursor_offset_px[1] * d,
        ];
    }

    /// Drag the content by a pixel delta (content follows the cursor).
    pub fn pan_pixels(&mut self, delta_px: Vec2d) {
        let d = self.divisor();
        self.center[0] -= delta_px[0] * d;
        self.center[1] -= delta_px[1] * d;
    }

    /// Centre the map and pick the closest zoom step that shows all of it.
    pub fn fit(map_w: u32, map_h: u32, viewport_px: Vec2d) -> View {
        let [x0, y0, x1, y1] = map_bounds(map_w, map_h);
        let (vw, vh) = (viewport_px[0].max(1.0), viewport_px[1].max(1.0));
        let zoom = ZOOM_STEPS
            .iter()
            .position(|&d| {
                let d = f64::from(d);
                (x1 - x0) / d <= vw && (y1 - y0) / d <= vh
            })
            .unwrap_or(ZOOM_STEPS.len() - 1);
        View {
            center: [(x0 + x1) / 2.0, (y0 + y1) / 2.0],
            zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_projection() {
        assert_eq!(cell_top(0, 0), [0.0, 0.0]);
        assert_eq!(cell_top(1, 0), [80.0, 40.0]);
        assert_eq!(cell_top(0, 1), [-80.0, 40.0]);
        assert_eq!(cell_top(3, 2), [80.0, 200.0]);
        assert_eq!(cell_center(0, 0), [0.0, 40.0]);
    }

    #[test]
    fn inverse_round_trip_at_centres_and_inside() {
        for x in -20..40 {
            for y in -20..40 {
                let c = cell_center(x, y);
                assert_eq!(screen_to_cell(c), (x, y), "centre of ({x},{y})");
                // Points well inside the diamond map back to the same cell.
                for (dx, dy) in [(30.0, 0.0), (-30.0, 0.0), (0.0, 15.0), (0.0, -15.0)] {
                    assert_eq!(
                        screen_to_cell([c[0] + dx, c[1] + dy]),
                        (x, y),
                        "({x},{y}) + ({dx},{dy})"
                    );
                }
            }
        }
    }

    #[test]
    fn tile_space_is_the_inverse_of_the_projection() {
        for (u, v) in [(0.0, 0.0), (3.5, 7.25), (-2.0, 9.0), (100.5, 0.5)] {
            let (a, b) = iso_to_tile(tile_to_iso(u, v));
            assert!((a - u).abs() < 1e-9 && (b - v).abs() < 1e-9);
        }
        // Corners and centres agree with the cell helpers.
        assert_eq!(tile_to_iso(3.0, 2.0), cell_top(3, 2));
        assert_eq!(tile_to_iso(3.5, 2.5), cell_center(3, 2));
        let (u, v) = iso_to_tile(cell_center(5, 9));
        assert_eq!((u.floor() as i32, v.floor() as i32), (5, 9));
    }

    #[test]
    fn inverse_outside_goes_to_the_neighbour() {
        // Just past the right corner of (0,0) lies the cell (1,-1)'s left area.
        let c = cell_center(0, 0);
        assert_eq!(screen_to_cell([c[0] + 90.0, c[1]]), (1, -1));
        assert_eq!(screen_to_cell([c[0], c[1] + 50.0]), (1, 1));
        assert_eq!(screen_to_cell([c[0], c[1] - 50.0]), (-1, -1));
    }

    #[test]
    fn zoom_steps_walk_the_documented_ladder() {
        assert_eq!(ZOOM_STEPS, [1, 2, 3, 4, 6, 8, 12, 16]);
        let mut i = 0;
        let mut seen = vec![ZOOM_STEPS[i]];
        for _ in 0..10 {
            i = step_zoom(i, false);
            seen.push(ZOOM_STEPS[i]);
        }
        assert_eq!(&seen[..8], &[1, 2, 3, 4, 6, 8, 12, 16]);
        assert_eq!(seen[10], 16, "saturates at 1:16");
        for _ in 0..10 {
            i = step_zoom(i, true);
        }
        assert_eq!(i, 0, "saturates at 1:1");
        assert_eq!(
            step_zoom(99, true),
            ZOOM_STEPS.len() - 2,
            "bad index clamps"
        );
    }

    #[test]
    fn zoom_keeps_the_cursor_point_fixed() {
        let mut v = View {
            center: [123.0, -77.0],
            zoom: 2,
        };
        let cursor = [210.0, -95.0];
        let before = v.offset_to_iso(cursor);
        v.zoom_at(cursor, false);
        assert_eq!(v.zoom, 3);
        let after = v.offset_to_iso(cursor);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
        v.zoom_at(cursor, true);
        v.zoom_at(cursor, true);
        let after = v.offset_to_iso(cursor);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
        // At the limit nothing moves.
        let mut v = View::default();
        v.zoom_at([5.0, 5.0], true);
        assert_eq!(v, View::default());
    }

    #[test]
    fn pan_follows_the_cursor() {
        let mut v = View {
            center: [0.0, 0.0],
            zoom: 3, // 1:4
        };
        v.pan_pixels([10.0, -5.0]);
        assert_eq!(v.center, [-40.0, 20.0]);
        let o = v.iso_to_offset(v.offset_to_iso([7.0, 9.0]));
        assert!((o[0] - 7.0).abs() < 1e-9 && (o[1] - 9.0).abs() < 1e-9);
    }

    #[test]
    fn fit_shows_the_whole_map() {
        let v = View::fit(40, 40, [1200.0, 700.0]);
        let [x0, y0, x1, y1] = map_bounds(40, 40);
        let d = v.divisor();
        assert!((x1 - x0) / d <= 1200.0 && (y1 - y0) / d <= 700.0);
        if v.zoom > 0 {
            let smaller = f64::from(ZOOM_STEPS[v.zoom - 1]);
            assert!((x1 - x0) / smaller > 1200.0 || (y1 - y0) / smaller > 700.0);
        }
        assert_eq!(v.center, [(x0 + x1) / 2.0, (y0 + y1) / 2.0]);
        // 160 x 80 only fits 10 x 10 pixels at 1:16.
        assert_eq!(View::fit(1, 1, [10.0, 10.0]).zoom, ZOOM_STEPS.len() - 1);
    }

    #[test]
    fn bounds_cover_every_corner() {
        let [x0, y0, x1, y1] = map_bounds(5, 3);
        for (x, y) in [(0, 0), (4, 0), (0, 2), (4, 2)] {
            let [sx, sy] = cell_top(x, y);
            assert!(sx - 80.0 >= x0 && sx + 80.0 <= x1);
            assert!(sy >= y0 && sy + 80.0 <= y1);
        }
    }
}
