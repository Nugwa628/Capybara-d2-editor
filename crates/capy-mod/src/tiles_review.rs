//! Reviewing tile libraries (ds1-studio `dt1Review.ts`, `duplicateDt1s.ts`,
//! `customDt1.ts`, `ownTiles.ts`): which tiles a map places, duplicate DT1s,
//! custom DT1s built from single tiles, and where a map's own tiles go.
//! Pure: DT1s are passed in as parsed models with their game paths.

use std::collections::{BTreeMap, BTreeSet};

use d2_formats::IndexedImage;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::{Dt1, Tile};

use crate::tiles_dt1::{MAX_TILE_PATH, normalize_path, recolor_tiles};

/// A tile as the game looks it up: orientation, main and sub index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub orientation: i32,
    pub main: i32,
    pub sub: i32,
}

impl TileId {
    pub fn new(orientation: i32, main: i32, sub: i32) -> TileId {
        TileId {
            orientation,
            main,
            sub,
        }
    }

    pub fn of(tile: &Tile) -> TileId {
        TileId::new(tile.orientation, tile.main_index, tile.sub_index)
    }
}

const FLOOR: i32 = 0;
const LEFT_NORTH_CORNER: i32 = 3;
const RIGHT_NORTH_CORNER: i32 = 4;
const SPECIAL_1: i32 = 10;
const SPECIAL_2: i32 = 11;
const SHADOW: i32 = 13;

/// A DS1 cell holds main 0-63 and sub 0-255.
pub const MAX_MAIN: i32 = 63;
pub const MAX_SUB: i32 = 255;

// ───────────── what a map places ─────────────

fn for_each_placed(ds1: &Ds1, mut f: impl FnMut(TileId, u32, u32)) {
    let (w, h) = (ds1.width, ds1.height);
    for layer in &ds1.floors {
        for y in 0..h {
            for x in 0..w {
                if let Some(c) = layer.get(x, y).filter(|c| !c.is_empty()) {
                    f(
                        TileId::new(FLOOR, i32::from(c.main_index()), i32::from(c.sub_index())),
                        x,
                        y,
                    );
                }
            }
        }
    }
    for layer in &ds1.walls {
        for y in 0..h {
            for x in 0..w {
                let Some(c) = layer.tiles.get(x, y).filter(|c| !c.is_empty()) else {
                    continue;
                };
                let o = layer.orientations.get(x, y).unwrap_or(0) as i32;
                let (m, s) = (i32::from(c.main_index()), i32::from(c.sub_index()));
                f(TileId::new(o, m, s), x, y);
                // a north corner wall also draws its other half
                if o == LEFT_NORTH_CORNER {
                    f(TileId::new(RIGHT_NORTH_CORNER, m, s), x, y);
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            if let Some(c) = ds1.shadow.get(x, y).filter(|c| !c.is_empty()) {
                f(
                    TileId::new(SHADOW, i32::from(c.main_index()), i32::from(c.sub_index())),
                    x,
                    y,
                );
            }
        }
    }
}

/// Every tile the map places, with how many cells use it (a north corner
/// wall also counts its other half).
pub fn map_tile_usage(ds1: &Ds1) -> BTreeMap<TileId, u32> {
    let mut out = BTreeMap::new();
    for_each_placed(ds1, |id, _, _| *out.entry(id).or_insert(0) += 1);
    out
}

/// The cells (sorted, row-major) placing a tile.
pub fn cells_using(ds1: &Ds1, id: TileId) -> Vec<(u32, u32)> {
    let mut cells = BTreeSet::new();
    for_each_placed(ds1, |k, x, y| {
        if k == id {
            cells.insert((y, x));
        }
    });
    cells.into_iter().map(|(y, x)| (x, y)).collect()
}

/// The tiles of `dt1` that the map never places (indices in file order).
/// Variants and animation frames of a placed number all count as used.
pub fn unused_tile_indices(dt1: &Dt1, used: &BTreeSet<TileId>) -> Vec<usize> {
    dt1.tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| !used.contains(&TileId::of(t)))
        .map(|(i, _)| i)
        .collect()
}

// ───────────── duplicate DT1s ─────────────

/// The tile numbers a DT1 provides.
pub fn tile_ids(dt1: &Dt1) -> BTreeSet<TileId> {
    dt1.tiles.iter().map(TileId::of).collect()
}

/// Tile numbers two DT1s share when one is a copy of the other (at least 4
/// and 80% of the smaller file's numbers), else `None`.
pub fn shared_tiles(a: &BTreeSet<TileId>, b: &BTreeSet<TileId>) -> Option<BTreeSet<TileId>> {
    let shared: BTreeSet<TileId> = a.intersection(b).copied().collect();
    let min = a.len().min(b.len());
    (shared.len() >= 4 && shared.len() * 5 >= min * 4).then_some(shared)
}

/// Two DT1s that provide (mostly) the same tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateDt1 {
    /// Loaded first / later (the order given).
    pub earlier: String,
    pub later: String,
    pub shared: BTreeSet<TileId>,
}

/// Pairs of DT1s (in load order) that are copies of each other. Where a tile
/// number has random variants the game picks among every loaded DT1, so a map
/// loading both shows a random mix.
pub fn duplicate_dt1s(libs: &[(String, &Dt1)]) -> Vec<DuplicateDt1> {
    let keyed: Vec<(&String, BTreeSet<TileId>)> = libs
        .iter()
        .map(|(p, d)| (p, tile_ids(d)))
        .filter(|(_, k)| !k.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, (pa, ka)) in keyed.iter().enumerate() {
        for (pb, kb) in keyed.iter().skip(i + 1) {
            if let Some(shared) = shared_tiles(ka, kb) {
                out.push(DuplicateDt1 {
                    earlier: (*pa).clone(),
                    later: (*pb).clone(),
                    shared,
                });
            }
        }
    }
    out
}

/// A tile number both DT1s provide, for DT1s that are not copies of each
/// other (so the game mixes them at random): `(a, b)` -> how many placed
/// cells use numbers both provide.
pub fn clashing_pairs(
    libs: &[(String, &Dt1)],
    placed: &BTreeMap<TileId, u32>,
) -> BTreeMap<(String, String), u32> {
    let copies: BTreeSet<(String, String)> = duplicate_dt1s(libs)
        .into_iter()
        .map(|d| (d.earlier, d.later))
        .collect();
    let keyed: Vec<(&String, BTreeSet<TileId>)> =
        libs.iter().map(|(p, d)| (p, tile_ids(d))).collect();
    let mut out = BTreeMap::new();
    for (id, &n) in placed {
        let owners: Vec<&String> = keyed
            .iter()
            .filter(|(_, k)| k.contains(id))
            .map(|(p, _)| *p)
            .collect();
        for (i, a) in owners.iter().enumerate() {
            for b in owners.iter().skip(i + 1) {
                let key = ((*a).clone(), (*b).clone());
                if copies.contains(&key) {
                    continue;
                }
                *out.entry(key).or_insert(0) += n;
            }
        }
    }
    out
}

/// The two pictures have the same shape and mostly the same pixels, but not
/// all: one picture in two colourings.
pub fn same_tile_recoloured(a: &IndexedImage, b: &IndexedImage) -> bool {
    if a.width != b.width
        || a.height != b.height
        || a.offset_x != b.offset_x
        || a.offset_y != b.offset_y
    {
        return false;
    }
    let (mut both, mut same) = (0usize, 0usize);
    for i in 0..a.pixels.len() {
        let (oa, ob) = (
            a.alpha.get(i).copied().unwrap_or(0) != 0,
            b.alpha.get(i).copied().unwrap_or(0) != 0,
        );
        if oa != ob {
            return false;
        }
        if oa {
            both += 1;
            if a.pixels.get(i) == b.pixels.get(i) {
                same += 1;
            }
        }
    }
    both > 0 && same < both && same * 4 >= both * 3
}

/// A tile number with versions that are one picture in different colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixedVersions {
    pub path: String,
    pub id: TileId,
    /// Tile indices in file order.
    pub indices: Vec<usize>,
}

/// In the tool's own DT1s (paths containing `tiles/studio/`), tile numbers
/// whose versions are the same picture in different colours: the game picks
/// a version at random, so some cells show the wrong colours.
pub fn mixed_versions(libs: &[(String, &Dt1)]) -> Vec<MixedVersions> {
    let mut out = Vec::new();
    for (path, dt1) in libs {
        if !normalize_path(path).contains("tiles/studio/") {
            continue;
        }
        let mut groups: BTreeMap<TileId, Vec<usize>> = BTreeMap::new();
        for (i, t) in dt1.tiles.iter().enumerate() {
            groups.entry(TileId::of(t)).or_default().push(i);
        }
        for (id, ids) in groups {
            if ids.len() < 2 {
                continue;
            }
            let imgs: Vec<Option<IndexedImage>> = ids
                .iter()
                .map(|&i| dt1.tiles.get(i).and_then(|t| t.render().ok()))
                .collect();
            let mut mixed = BTreeSet::new();
            for a in 0..ids.len() {
                for b in a + 1..ids.len() {
                    if let (Some(x), Some(y)) = (&imgs[a], &imgs[b])
                        && same_tile_recoloured(x, y)
                    {
                        mixed.insert(ids[a]);
                        mixed.insert(ids[b]);
                    }
                }
            }
            if mixed.is_empty() {
                continue;
            }
            // exact duplicates of a mixed version are versions too
            let pix = |i: usize| {
                let at = ids.iter().position(|&x| x == i)?;
                imgs[at].as_ref().map(|im| im.pixels.clone())
            };
            let shown: Vec<Option<Vec<u8>>> = mixed.iter().map(|&i| pix(i)).collect();
            out.push(MixedVersions {
                path: path.clone(),
                id,
                indices: ids
                    .iter()
                    .copied()
                    .filter(|i| mixed.contains(i) || shown.contains(&pix(*i)))
                    .collect(),
            });
        }
    }
    out
}

// ───────────── custom DT1 ─────────────

/// A picked tile: index into the `sources` slice and the tile's index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TilePick {
    pub source: usize,
    pub index: usize,
}

/// One tile of the custom DT1 and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedTile {
    pub from: TilePick,
    pub id: TileId,
    pub new_main: i32,
    pub new_sub: i32,
    /// Added because a picked tile needs it (corner-wall half or animation frame).
    pub partner: bool,
}

/// The result of [`plan_custom_dt1`].
#[derive(Clone, Debug, PartialEq)]
pub struct CustomDt1Plan {
    /// The copied tiles, renumbered where needed, parallel to `tiles`.
    pub records: Vec<Tile>,
    pub tiles: Vec<PlannedTile>,
    /// Tile groups given a new number, as "from -> to" lines.
    pub renumbered: Vec<String>,
    /// Picks left out, with why.
    pub skipped: Vec<String>,
}

fn family(o: i32) -> i32 {
    if o == LEFT_NORTH_CORNER {
        RIGHT_NORTH_CORNER
    } else {
        o
    }
}

fn short(p: &str) -> String {
    let n = p.replace('\\', "/");
    match n.to_ascii_lowercase().strip_prefix("data/global/tiles/") {
        Some(rest) => rest.to_string(),
        None => n,
    }
}

/// Plans a custom DT1 built from single tiles of other DT1s. Each tile is
/// copied unchanged (pixels, flags, sound, roof height, rarity) and keeps its
/// number unless that would clash with `taken` (numbers the level already
/// loads) or another pick; clashing groups get the first free number. Tiles
/// that belong together come along: both halves of a north corner wall and
/// every frame of an animated tile.
pub fn plan_custom_dt1(
    picks: &[TilePick],
    sources: &[(String, &Dt1)],
    taken: &BTreeSet<TileId>,
) -> CustomDt1Plan {
    struct Group {
        source: usize,
        family: i32,
        main: i32,
        sub: i32,
        members: Vec<(usize, bool)>,
    }
    let mut skipped = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for p in picks {
        let Some((path, dt1)) = sources.get(p.source) else {
            skipped.push(format!("source #{}: not found", p.source));
            continue;
        };
        let Some(t) = dt1.tiles.get(p.index) else {
            skipped.push(format!("{} #{}: not found", short(path), p.index));
            continue;
        };
        if t.orientation == SPECIAL_1 || t.orientation == SPECIAL_2 {
            skipped.push(format!(
                "{} #{}: a special tile (a marker the game uses, not a picture)",
                short(path),
                p.index
            ));
            continue;
        }
        let fam = family(t.orientation);
        let gi = match groups.iter().position(|g| {
            g.source == p.source
                && g.family == fam
                && g.main == t.main_index
                && g.sub == t.sub_index
        }) {
            Some(i) => i,
            None => {
                groups.push(Group {
                    source: p.source,
                    family: fam,
                    main: t.main_index,
                    sub: t.sub_index,
                    members: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let mut add = |index: usize, partner: bool, groups: &mut Vec<Group>| {
            if seen.insert((p.source, index)) {
                groups[gi].members.push((index, partner));
            }
        };
        add(p.index, false, &mut groups);
        for (i, o) in dt1.tiles.iter().enumerate() {
            if i == p.index || o.main_index != t.main_index || o.sub_index != t.sub_index {
                continue;
            }
            let corner_half = family(o.orientation) == fam && o.orientation != t.orientation;
            let frame = t.animated != 0 && o.orientation == t.orientation && o.animated != 0;
            if corner_half || frame {
                add(i, true, &mut groups);
            }
        }
    }

    let mut used = taken.clone();
    let mut tiles = Vec::new();
    let mut records = Vec::new();
    let mut renumbered = Vec::new();
    for g in &groups {
        let dt1 = sources[g.source].1;
        let orientations: BTreeSet<i32> = g
            .members
            .iter()
            .filter_map(|(i, _)| dt1.tiles.get(*i).map(|t| t.orientation))
            .collect();
        let fits = |used: &BTreeSet<TileId>, m: i32, s: i32| {
            orientations
                .iter()
                .all(|&o| !used.contains(&TileId::new(o, m, s)))
        };
        let (mut main, mut sub) = (g.main, g.sub);
        if main > MAX_MAIN || sub > MAX_SUB || !fits(&used, main, sub) {
            // the same sub with another main first, then any free number
            let found = if sub <= MAX_SUB {
                (0..=MAX_MAIN)
                    .find(|&m| fits(&used, m, sub))
                    .map(|m| (m, sub))
            } else {
                None
            }
            .or_else(|| {
                (0..=MAX_MAIN)
                    .flat_map(|m| (0..=MAX_SUB).map(move |s| (m, s)))
                    .find(|&(m, s)| fits(&used, m, s))
            });
            let Some((m, s)) = found else {
                skipped.push(format!(
                    "{} {}/{}: no free tile number left",
                    short(&sources[g.source].0),
                    g.main,
                    g.sub
                ));
                continue;
            };
            (main, sub) = (m, s);
            renumbered.push(format!(
                "{} {}/{} -> {main}/{sub}",
                short(&sources[g.source].0),
                g.main,
                g.sub
            ));
        }
        for &o in &orientations {
            used.insert(TileId::new(o, main, sub));
        }
        for &(index, partner) in &g.members {
            let Some(t) = dt1.tiles.get(index) else {
                continue;
            };
            let mut copy = t.clone();
            copy.main_index = main;
            copy.sub_index = sub;
            records.push(copy);
            tiles.push(PlannedTile {
                from: TilePick {
                    source: g.source,
                    index,
                },
                id: TileId::of(t),
                new_main: main,
                new_sub: sub,
                partner,
            });
        }
    }
    CustomDt1Plan {
        records,
        tiles,
        renumbered,
        skipped,
    }
}

/// The custom DT1 model. With `remap_for`, each tile's colours are converted
/// too: `remap_for(source)` gives the table for tiles copied from that
/// source (e.g. an Act 0 remap judged by the act it was drawn for); `None`
/// leaves them alone.
pub fn build_custom_dt1(
    plan: &CustomDt1Plan,
    remap_for: Option<&dyn Fn(usize) -> Option<[u8; 256]>>,
) -> Dt1 {
    let mut dt1 = Dt1 {
        version: (7, 6),
        tiles: plan.records.clone(),
    };
    if let Some(remap_for) = remap_for {
        let mut by_source: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, t) in plan.tiles.iter().enumerate() {
            by_source.entry(t.from.source).or_default().push(i);
        }
        for (source, idx) in by_source {
            if let Some(table) = remap_for(source) {
                // indices are in range by construction
                let _ = recolor_tiles(&mut dt1, Some(&idx), &table);
            }
        }
    }
    dt1
}

// ───────────── own tiles ─────────────

/// `<home>/<name>_custom.dt1`, kept within the game's tile path limit: the
/// name loses its underscores, then letters from the end.
pub fn custom_name(home: &str, name: &str) -> String {
    let room = MAX_TILE_PATH.saturating_sub(format!("{home}/_custom.dt1").len());
    let short: String = if name.len() <= room {
        name.to_string()
    } else {
        name.replace('_', "").chars().take(room.max(1)).collect()
    };
    format!("{home}/{short}_custom.dt1")
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let t = out.trim_matches('_');
    if t.is_empty() {
        "custom".into()
    } else {
        t.into()
    }
}

/// The level type's home folder (relative to `data/global/tiles`): where most
/// of the given libraries are (`files` should be the type's own loose mod
/// files when it has any).
pub fn type_home(files: &[String]) -> Option<String> {
    let mut count: BTreeMap<String, u32> = BTreeMap::new();
    for f in files.iter().filter(|f| !f.is_empty()) {
        let f = f.replace('\\', "/");
        if let Some(i) = f.rfind('/') {
            *count.entry(f[..i].to_string()).or_insert(0) += 1;
        }
    }
    count
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.len().cmp(&a.0.len())))
        .map(|(d, _)| d)
}

/// The file the map's generated tiles go into, relative to `data/global/tiles`:
/// one per level type in its home folder (`listed` holds the files the type
/// already lists, so a listed file keeps its spelling); a map without a type
/// uses `<map>_custom.dt1` next to it.
pub fn own_tiles_path(
    map_path: &str,
    level_type: Option<(&str, &[String])>,
    own_files: &[String],
) -> String {
    if let Some((type_name, listed)) = level_type
        && let Some(home) = type_home(own_files)
    {
        let name = custom_name(&home, &slug(type_name));
        let want = normalize_path(&name);
        return listed
            .iter()
            .find(|f| !f.is_empty() && normalize_path(f) == want)
            .cloned()
            .unwrap_or(name);
    }
    let m = map_path.replace('\\', "/");
    match m.to_ascii_lowercase().strip_suffix(".ds1") {
        Some(stem) => format!("{}_custom.dt1", &m[..stem.len()]),
        None => format!("{m}_custom.dt1"),
    }
}

/// The own-tiles file with `tiles` added after what it already holds, and the
/// index of the first new tile. Tiles are only ever added, so undo never
/// leaves a cell on a tile that is gone.
pub fn append_tiles(existing: Option<&Dt1>, tiles: Vec<Tile>) -> (Dt1, usize) {
    let mut dt1 = existing.cloned().unwrap_or(Dt1 {
        version: (7, 6),
        tiles: Vec::new(),
    });
    let first = dt1.tiles.len();
    dt1.tiles.extend(tiles);
    (dt1, first)
}

/// Tile numbers already in the own-tiles file.
pub fn keys_of(dt1: Option<&Dt1>) -> BTreeSet<TileId> {
    dt1.map(tile_ids).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn tile(o: i32, m: i32, s: i32) -> Tile {
        Tile {
            orientation: o,
            main_index: m,
            sub_index: s,
            ..Tile::default()
        }
    }

    fn dt1(tiles: Vec<Tile>) -> Dt1 {
        Dt1 {
            version: (7, 6),
            tiles,
        }
    }

    #[test]
    fn duplicates_need_most_tiles_shared() {
        let a = dt1((0..10).map(|i| tile(0, i, 0)).collect());
        let b = dt1((0..10).map(|i| tile(0, i, 0)).collect());
        let c = dt1((8..20).map(|i| tile(0, i, 0)).collect());
        let libs = vec![
            ("a".to_string(), &a),
            ("b".to_string(), &b),
            ("c".to_string(), &c),
        ];
        let d = duplicate_dt1s(&libs);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].earlier.as_str(), d[0].later.as_str()), ("a", "b"));
    }

    #[test]
    fn custom_plan_renumbers_clashes_and_brings_partners() {
        let mut anim = tile(0, 5, 1);
        anim.animated = 1;
        let mut anim2 = anim.clone();
        anim2.rarity = 1;
        let src = dt1(vec![
            tile(3, 2, 0),
            tile(4, 2, 0),
            anim,
            anim2,
            tile(10, 1, 1),
        ]);
        let sources = vec![("data/global/tiles/x/a.dt1".to_string(), &src)];
        let taken: BTreeSet<TileId> = [TileId::new(3, 2, 0)].into();
        let picks = [
            TilePick {
                source: 0,
                index: 0,
            },
            TilePick {
                source: 0,
                index: 2,
            },
            TilePick {
                source: 0,
                index: 4,
            },
        ];
        let plan = plan_custom_dt1(&picks, &sources, &taken);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.records.len(), 4); // corner pair + 2 frames
        assert_eq!(plan.renumbered.len(), 1);
        let corner: Vec<_> = plan.tiles.iter().filter(|t| t.id.main == 2).collect();
        assert_eq!(corner.len(), 2);
        assert!(corner.iter().all(|t| t.new_main != 2 || t.new_sub != 0));
        assert_eq!(corner[0].new_main, corner[1].new_main);
        let built = build_custom_dt1(&plan, None);
        assert_eq!(built.tiles.len(), 4);
    }

    #[test]
    fn own_paths() {
        assert_eq!(
            custom_name("act1/town", "town"),
            "act1/town/town_custom.dt1"
        );
        let long = custom_name("PD2assets/dtprivate", "dark_temple_of_the_ancients_long");
        assert!(long.len() <= MAX_TILE_PATH);
        let files = vec![
            "act1/town/a.dt1".to_string(),
            "act1/town/b.dt1".to_string(),
            "x/c.dt1".to_string(),
        ];
        assert_eq!(type_home(&files).as_deref(), Some("act1/town"));
        assert_eq!(
            own_tiles_path("data/global/tiles/a/m.DS1", None, &[]),
            "data/global/tiles/a/m_custom.dt1"
        );
        let listed = vec!["ACT1/Town/Town_Custom.dt1".to_string()];
        assert_eq!(
            own_tiles_path("m.ds1", Some(("Town", &listed)), &files),
            "ACT1/Town/Town_Custom.dt1"
        );
    }

    #[test]
    fn recoloured_pictures() {
        let mut a = IndexedImage::new(2, 2, 0, 0);
        a.pixels = vec![1, 2, 3, 4];
        a.alpha = vec![255; 4];
        let mut b = a.clone();
        b.pixels[0] = 9;
        assert!(same_tile_recoloured(&a, &b));
        assert!(!same_tile_recoloured(&a, &a));
        b.alpha[1] = 0;
        assert!(!same_tile_recoloured(&a, &b));
    }
}
