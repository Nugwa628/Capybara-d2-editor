//! A DT1 opened in the Sprites tab, for previewing (and converting) tiles:
//! every tile rendered once with `Tile::render`, plus its key. Read-only;
//! the only DT1 writer is palette index remapping.

use d2_formats::IndexedImage;
use d2_formats::dt1::Dt1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileInfo {
    pub orientation: i32,
    pub main: i32,
    pub sub: i32,
    pub rarity: i32,
    /// Index among the tiles with the same (orientation, main, sub).
    pub n: u32,
}

pub struct Dt1View {
    /// The parsed file, kept for the editor.
    pub dt1: Dt1,
    pub tiles: Vec<TileInfo>,
    pub images: Vec<IndexedImage>,
}

impl Dt1View {
    pub fn load(bytes: &[u8]) -> Result<Dt1View, String> {
        let dt1 = Dt1::parse(bytes).map_err(|e| match e {
            d2_formats::FormatError::UnsupportedVersion { version, .. } => format!(
                "this DT1 uses an undocumented version ({version}) that neither the game's maps \
                 nor this editor read (six vanilla act 1 files are like this)"
            ),
            e => format!("not a DT1 this editor reads: {e}"),
        })?;
        if dt1.tiles.is_empty() {
            return Err("the DT1 has no tiles".to_owned());
        }
        let mut tiles = Vec::with_capacity(dt1.tiles.len());
        let mut images = Vec::with_capacity(dt1.tiles.len());
        for t in &dt1.tiles {
            let n = tiles
                .iter()
                .filter(|i: &&TileInfo| {
                    (i.orientation, i.main, i.sub) == (t.orientation, t.main_index, t.sub_index)
                })
                .count() as u32;
            tiles.push(TileInfo {
                orientation: t.orientation,
                main: t.main_index,
                sub: t.sub_index,
                rarity: t.rarity,
                n,
            });
            // A tile whose blocks cannot be composed shows as empty.
            images.push(t.render().unwrap_or_else(|_| IndexedImage::new(0, 0, 0, 0)));
        }
        Ok(Dt1View { dt1, tiles, images })
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// "tile 12: orientation 0, main 3, sub 1 (#0)".
    pub fn label(&self, i: usize) -> String {
        match self.tiles.get(i) {
            Some(t) => format!(
                "tile {i}: orientation {}, main {}, sub {} (#{}, rarity {})",
                t.orientation, t.main, t.sub, t.n, t.rarity
            ),
            None => format!("tile {i}"),
        }
    }

    /// The first tile with this key (and `n`-th duplicate when given).
    pub fn find(&self, orientation: i32, main: i32, sub: i32, n: Option<u32>) -> Option<usize> {
        self.tiles.iter().position(|t| {
            (t.orientation, t.main, t.sub) == (orientation, main, sub) && n.is_none_or(|n| t.n == n)
        })
    }
}
