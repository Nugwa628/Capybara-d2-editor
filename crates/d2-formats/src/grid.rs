//! A dense two-dimensional grid, row-major with x varying fastest.

/// Row-major grid; the cell at `(x, y)` lives at index `y * width + x`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Grid<T> {
    width: u32,
    height: u32,
    cells: Vec<T>,
}

impl<T: Copy + Default> Grid<T> {
    /// A `width` x `height` grid filled with `T::default()`.
    ///
    /// The caller is responsible for sane dimensions; parsers validate sizes
    /// against the input length before calling this.
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            cells: vec![T::default(); len],
        }
    }

    /// Wraps existing cells; `None` when `cells.len() != width * height`.
    pub fn from_vec(width: u32, height: u32, cells: Vec<T>) -> Option<Self> {
        let expected = u64::from(width).checked_mul(u64::from(height))?;
        if expected != cells.len() as u64 {
            return None;
        }
        Some(Self {
            width,
            height,
            cells,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y as usize * self.width as usize + x as usize)
        } else {
            None
        }
    }

    pub fn get(&self, x: u32, y: u32) -> Option<T> {
        self.index(x, y).and_then(|i| self.cells.get(i).copied())
    }

    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut T> {
        let i = self.index(x, y)?;
        self.cells.get_mut(i)
    }

    /// Stores `value`; returns `false` (and changes nothing) when out of bounds.
    pub fn set(&mut self, x: u32, y: u32, value: T) -> bool {
        match self.get_mut(x, y) {
            Some(cell) => {
                *cell = value;
                true
            }
            None => false,
        }
    }

    pub fn as_slice(&self) -> &[T] {
        &self.cells
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.cells
    }

    /// New size, keeping the top-left overlap; new cells are `fill`.
    pub fn resized(&self, width: u32, height: u32, fill: T) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        let mut cells = vec![fill; len];
        let copy_w = self.width.min(width) as usize;
        let copy_h = self.height.min(height) as usize;
        for y in 0..copy_h {
            let src = y * self.width as usize;
            let dst = y * width as usize;
            if let (Some(from), Some(to)) = (
                self.cells.get(src..src + copy_w),
                cells.get_mut(dst..dst + copy_w),
            ) {
                to.copy_from_slice(from);
            }
        }
        Self {
            width,
            height,
            cells,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_get_set() {
        let mut g: Grid<u32> = Grid::new(3, 2);
        assert_eq!(g.as_slice().len(), 6);
        assert_eq!(g.get(2, 1), Some(0));
        assert_eq!(g.get(3, 0), None);
        assert_eq!(g.get(0, 2), None);
        assert!(g.set(2, 1, 7));
        assert!(!g.set(3, 1, 9));
        assert_eq!(g.get(2, 1), Some(7));
        assert_eq!(g.as_slice()[5], 7);
        if let Some(c) = g.get_mut(0, 0) {
            *c = 4;
        }
        assert_eq!(g.as_slice()[0], 4);
    }

    #[test]
    fn from_vec_checks_len() {
        assert!(Grid::from_vec(2, 2, vec![1u8, 2, 3]).is_none());
        let g = Grid::from_vec(2, 2, vec![1u8, 2, 3, 4]).expect("len matches");
        assert_eq!(g.get(1, 1), Some(4));
        assert_eq!(g.get(0, 1), Some(3));
        assert!(Grid::<u8>::from_vec(u32::MAX, u32::MAX, vec![]).is_none());
    }

    #[test]
    fn resized_keeps_top_left() {
        let g = Grid::from_vec(3, 2, vec![1u8, 2, 3, 4, 5, 6]).expect("len matches");
        let bigger = g.resized(4, 3, 9);
        assert_eq!(bigger.as_slice(), &[1, 2, 3, 9, 4, 5, 6, 9, 9, 9, 9, 9]);
        let smaller = g.resized(2, 1, 9);
        assert_eq!(smaller.as_slice(), &[1, 2]);
        let empty = g.resized(0, 5, 9);
        assert!(empty.as_slice().is_empty());
    }
}
