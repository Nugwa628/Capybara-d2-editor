//! Walkability analysis (P10): the game's collision flags per sub-tile and the
//! walkable-area count.
//!
//! Port of ds1-studio's `walkability` (`render/scene.ts`), `walkableArea`
//! (`game/walkArea.ts`) and `blankFillFlags`. A cell is 5x5 sub-tiles, indexed
//! `(cy * width + cx) * 25 + sy * 5 + sx`. `Tile::subtile_flags` is already in
//! that order (`d2-formats` reorders the file's bottom-row-first layout the
//! same way ds1-studio's `addFlags` does).
//!
//! Where flags live: a DT1 tile's 25 sub-tile flags (blocks walk 0x01, sight
//! 0x02, jump 0x04, player-walk 0x08) and the DS1 cell's own bits (bit 17
//! "unwalkable" blocks all 25 sub-tiles; bit 16 adds the jump/LOS bit).
//! ds1-studio's walk painting writes BOTH, by two different tools:
//! sub-tile painting (`walkEdit.ts`) writes DT1 flags only, into a per-map
//! `<map>_walk.dt1` plus hidden blocker floors in the cells; the whole-cell
//! tool (`cellFlags.ts`) writes the DS1 cell flag (bit 17, `prop3 & 2`).

use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::TileKey;

use crate::tileset::TileSet;

/// DT1 sub-tile flag bits.
pub const BLOCK_WALK: u8 = 0x01;
pub const BLOCK_LIGHT: u8 = 0x02;
pub const BLOCK_JUMP: u8 = 0x04;
pub const BLOCK_PLAYER_WALK: u8 = 0x08;

/// Special-tile orientations (warps, pops, "Map entry").
pub const SPECIAL_1: u8 = 10;
pub const SPECIAL_2: u8 = 11;

pub(crate) fn is_special(o: u8) -> bool {
    o == SPECIAL_1 || o == SPECIAL_2
}

/// Sub-tile flags of a whole map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubTileGrid {
    pub width: u32,
    pub height: u32,
    pub flags: Vec<u8>,
}

impl SubTileGrid {
    pub fn index(&self, cx: u32, cy: u32, sx: u32, sy: u32) -> usize {
        ((cy * self.width + cx) * 25 + sy * 5 + sx) as usize
    }
    /// Flags at map-wide sub-tile coordinates (`sx` in 0..width*5).
    pub fn at(&self, sx: u32, sy: u32) -> u8 {
        let i = ((sy / 5) * self.width + sx / 5) * 25 + (sy % 5) * 5 + (sx % 5);
        self.flags.get(i as usize).copied().unwrap_or(0)
    }
}

/// How cells without a floor are treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collision {
    /// WinDS1's overlay: a cell with no floor at all blocks walking.
    Editor,
    /// As the game builds collision: every sub-tile starts free and only tiles
    /// add flags. `blank` (LvlPrest FillBlanks=1, see [`blank_fill_flags`])
    /// is added to cells with no first-layer floor.
    Game { blank: Option<[u8; 25]> },
}

fn or_into(out: &mut [u8], width: u32, x: u32, y: u32, f: &[u8; 25]) {
    let base = ((y * width + x) * 25) as usize;
    if let Some(dst) = out.get_mut(base..base + 25) {
        for (d, s) in dst.iter_mut().zip(f) {
            *d |= *s;
        }
    }
}

/// Combined sub-tile flags per cell: every floor, wall and roof tile placed in
/// the cell OR'd together (shadows and special tiles excluded), hidden tiles
/// included with every variant of their key (the game picks any), plus the
/// DS1 cell bits. Visible tiles use the renderer's variant rule
/// ([`TileSet::resolve`]); ds1-studio picks one at random.
pub fn sub_tile_flags(ds1: &Ds1, tiles: &TileSet, mode: Collision) -> SubTileGrid {
    let (w, h) = (ds1.width, ds1.height);
    let mut out = vec![0u8; (w * h * 25) as usize];
    let visible = |out: &mut Vec<u8>, x: u32, y: u32, key: TileKey| {
        if let Some(r) = tiles.resolve(key) {
            if let Some(t) = tiles.tile(&r) {
                or_into(out, w, x, y, &t.subtile_flags);
            }
            if key.orientation == 3
                && let Some(p) = tiles.corner_pair(&r).and_then(|p| tiles.tile(&p))
            {
                or_into(out, w, x, y, &p.subtile_flags);
            }
        }
    };
    let all_variants = |out: &mut Vec<u8>, x: u32, y: u32, key: TileKey| {
        for r in tiles.candidates(key) {
            if let Some(t) = tiles.tile(r) {
                or_into(out, w, x, y, &t.subtile_flags);
            }
        }
    };
    for y in 0..h {
        for x in 0..w {
            for floor in &ds1.floors {
                let c = floor.get(x, y).unwrap_or(Cell::EMPTY);
                if c.is_empty() {
                    continue;
                }
                let key = TileKey {
                    orientation: 0,
                    main: c.main_index(),
                    sub: c.sub_index(),
                };
                if c.hidden() {
                    all_variants(&mut out, x, y, key);
                } else {
                    visible(&mut out, x, y, key);
                }
            }
            for (li, layer) in ds1.walls.iter().enumerate() {
                let c = layer.tiles.get(x, y).unwrap_or(Cell::EMPTY);
                if c.is_empty() {
                    continue;
                }
                let o = ds1.orientation(li, x, y).unwrap_or(0);
                if is_special(o) {
                    continue;
                }
                let key = TileKey {
                    orientation: o,
                    main: c.main_index(),
                    sub: c.sub_index(),
                };
                if c.hidden() {
                    all_variants(&mut out, x, y, key);
                    if o == 3 {
                        all_variants(
                            &mut out,
                            x,
                            y,
                            TileKey {
                                orientation: 4,
                                ..key
                            },
                        );
                    }
                } else {
                    visible(&mut out, x, y, key);
                }
            }
            // Whole-cell DS1 bits.
            let layers = || {
                ds1.floors
                    .iter()
                    .map(|g| g.get(x, y).unwrap_or(Cell::EMPTY))
                    .chain(
                        ds1.walls
                            .iter()
                            .map(|l| l.tiles.get(x, y).unwrap_or(Cell::EMPTY)),
                    )
            };
            let marked = layers().any(|c| !c.is_empty() && c.unwalkable());
            let fill_los = layers().any(|c| !c.is_empty() && c.0 & (1 << 16) != 0);
            let no_floor = ds1
                .floors
                .iter()
                .all(|g| g.get(x, y).is_none_or(Cell::is_empty));
            let base = ((y * w + x) * 25) as usize;
            if let Some(cell) = out.get_mut(base..base + 25) {
                if fill_los {
                    cell.iter_mut().for_each(|f| *f |= BLOCK_JUMP);
                }
                if marked || (no_floor && mode == Collision::Editor) {
                    cell.iter_mut().for_each(|f| *f |= BLOCK_WALK);
                }
            }
            if let Collision::Game { blank: Some(b) } = mode
                && ds1
                    .floors
                    .first()
                    .is_none_or(|g| g.get(x, y).is_none_or(Cell::is_empty))
            {
                or_into(&mut out, w, x, y, &b);
            }
        }
    }
    SubTileGrid {
        width: w,
        height: h,
        flags: out,
    }
}

/// The flags of the hidden blank tile the game puts under cells without a
/// first-layer floor when LvlPrest FillBlanks=1: floor 30 / sequence 0 (1 in
/// the Arcane Sanctuary, level 74), else the first orientation-10 0/0 tile;
/// a sub-tile counts blocked only if every variant blocks it. All 0 when
/// there is neither.
pub fn blank_fill_flags(tiles: &TileSet, level_id: i32) -> [u8; 25] {
    let sub = if level_id == 74 { 1 } else { 0 };
    let mut refs = tiles
        .candidates(TileKey {
            orientation: 0,
            main: 30,
            sub,
        })
        .to_vec();
    if refs.is_empty() {
        refs = tiles
            .candidates(TileKey {
                orientation: SPECIAL_1,
                main: 0,
                sub: 0,
            })
            .iter()
            .take(1)
            .cloned()
            .collect();
    }
    let mut it = refs.iter().filter_map(|r| tiles.tile(r));
    let Some(first) = it.next() else {
        return [0; 25];
    };
    let mut out = first.subtile_flags;
    for t in it {
        for (o, f) in out.iter_mut().zip(&t.subtile_flags) {
            *o &= *f;
        }
    }
    out
}

/// Sub-tiles a player can stand on (no `BLOCK_WALK`).
pub fn walkable_subtiles(grid: &SubTileGrid) -> usize {
    grid.flags.iter().filter(|f| **f & BLOCK_WALK == 0).count()
}

/// The map's walkable area in tiles squared (25 sub-tiles to a tile), as
/// ds1-studio shows it in the badge: editor collision (no floor = blocked).
pub fn walkable_area(ds1: &Ds1, tiles: &TileSet) -> f64 {
    walkable_subtiles(&sub_tile_flags(ds1, tiles, Collision::Editor)) as f64 / 25.0
}

/// Size bands of the badge colour (tiles squared).
pub const AREA_MIN: f64 = 3300.0;
pub const AREA_GREEN: f64 = 3800.0;
pub const AREA_ORANGE: f64 = 4300.0;
pub const AREA_DEEP: f64 = 5300.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaBand {
    Small,
    Green,
    Orange,
    Red,
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let m = |i: usize| (f64::from(a[i]) + (f64::from(b[i]) - f64::from(a[i])) * t).round() as u8;
    [m(0), m(1), m(2)]
}

/// Badge colour for a walkable area: grey below 3,300; green blending to
/// orange at 3,800; orange to red by 4,300; deep red by 5,300.
pub fn area_colour(area: f64) -> ([u8; 3], AreaBand) {
    let (grey, green, orange, red, deep) = (
        [128, 132, 140],
        [60, 200, 90],
        [255, 150, 40],
        [255, 80, 40],
        [200, 20, 20],
    );
    if area < AREA_MIN {
        (grey, AreaBand::Small)
    } else if area < AREA_GREEN {
        (
            mix(green, orange, (area - AREA_MIN) / (AREA_GREEN - AREA_MIN)),
            AreaBand::Green,
        )
    } else if area < AREA_ORANGE {
        (
            mix(
                orange,
                red,
                (area - AREA_GREEN) / (AREA_ORANGE - AREA_GREEN),
            ),
            AreaBand::Orange,
        )
    } else {
        (
            mix(red, deep, (area - AREA_ORANGE) / (AREA_DEEP - AREA_ORANGE)),
            AreaBand::Red,
        )
    }
}
