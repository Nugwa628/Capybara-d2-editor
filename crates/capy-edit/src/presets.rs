//! Presets (ds1-studio `presets.ts`, `PresetBuilder.tsx`): a block of cells
//! across all tile layers plus the objects standing on it, saved from a
//! selection, placed as one undoable patch, built in a 20x20 scratch map, and
//! suggested from structures that recur in maps. Pure data; the caller reads
//! and writes the JSON files and supplies what is drawable.

use std::collections::{BTreeMap, BTreeSet};

use d2_formats::ds1::{Cell, Ds1, Object};
use serde::{Deserialize, Serialize};

use crate::clipboard::{ClipSource, Clipboard, PasteMode, PastePreview};
use crate::document::Document;
use crate::error::{EditError, Result};
use crate::layer::{CellValue, LayerRef};
use crate::selection::{CellRect, SelectOp, Selection};
use crate::tools::{NewMapSpec, new_map};

/// Where presets live: inside the mod, so they travel with it.
pub const PRESET_DIR: &str = "data/ds1studio/presets/";
/// The format tag stored in the JSON.
pub const PRESET_FORMAT: &str = "ds1studio-preset";
/// The preset builder grid is this many cells on a side.
pub const BUILDER_SIZE: u32 = 20;
/// Suggested structures are at most this many cells on a side.
pub const SUGGEST_MAX_SIDE: u32 = 16;

/// A reusable block of cells plus the objects on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    pub category: String,
    pub width: u32,
    pub height: u32,
    /// Tile libraries the tiles come from (normalized game paths).
    #[serde(default)]
    pub dt1s: Vec<String>,
    /// `"orientation|main|sub"` -> the DT1 it comes from, when known.
    #[serde(default)]
    pub tile_sources: BTreeMap<String, String>,
    /// Cells relative to the top-left, floors, walls and shadow (never tags).
    pub cells: Vec<(LayerRef, u32, u32, CellValue)>,
    /// Objects with sub-tile coordinates relative to the top-left cell.
    #[serde(default, with = "crate::clipboard::object_list")]
    pub objects: Vec<Object>,
    /// Suggestions only: how often the structure occurs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrences: Option<u32>,
    /// Suggestions only: a map it was found in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_in: Option<String>,
}

fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
}

fn is_tile_layer(l: LayerRef) -> bool {
    !matches!(l, LayerRef::Tag)
}

impl Preset {
    /// A preset from a clipboard (a copied block). Tag cells are dropped.
    /// `id` is derived from the content, so the same block gets the same id.
    pub fn from_clipboard(clip: &Clipboard, name: &str, category: &str) -> Preset {
        let mut cells: Vec<_> = clip
            .cells
            .iter()
            .copied()
            .filter(|c| is_tile_layer(c.0))
            .collect();
        cells.sort_by_key(|c| (c.0, c.2, c.1));
        let mut p = Preset {
            format: PRESET_FORMAT.into(),
            version: 1,
            id: String::new(),
            name: name.to_string(),
            category: category.to_string(),
            width: clip.width,
            height: clip.height,
            dt1s: Vec::new(),
            tile_sources: BTreeMap::new(),
            cells,
            objects: clip.objects.clone(),
            occurrences: None,
            found_in: None,
        };
        p.id = p.content_id();
        p
    }

    /// Saves a selection (all of its layers, and the objects standing on its
    /// cells) as a preset. `None` when the selection is empty.
    pub fn from_selection(
        doc: &Document,
        selection: &Selection,
        name: &str,
        category: &str,
    ) -> Option<Preset> {
        let bounds = selection.bounds()?;
        let mut clip = doc.copy(selection);
        // keep only the objects on selected cells (copy takes the whole rectangle)
        let cell_of = |o: &Object| {
            (
                bounds.x0.wrapping_add_signed(o.x.div_euclid(5)),
                bounds.y0.wrapping_add_signed(o.y.div_euclid(5)),
            )
        };
        let selected: BTreeSet<(u32, u32)> = selection.iter().map(|(_, x, y)| (x, y)).collect();
        clip.objects.retain(|o| selected.contains(&cell_of(o)));
        Some(Preset::from_clipboard(&clip, name, category))
    }

    /// A stable id from the content (name, size, cells, objects).
    pub fn content_id(&self) -> String {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        fnv(&mut h, self.name.as_bytes());
        fnv(&mut h, &self.width.to_le_bytes());
        fnv(&mut h, &self.height.to_le_bytes());
        for (l, x, y, v) in &self.cells {
            fnv(&mut h, format!("{l:?}").as_bytes());
            fnv(&mut h, &x.to_le_bytes());
            fnv(&mut h, &y.to_le_bytes());
            fnv(&mut h, &v.cell.to_le_bytes());
            fnv(&mut h, &v.orientation.to_le_bytes());
        }
        for o in &self.objects {
            fnv(&mut h, &[o.kind as u8, o.id as u8]);
            fnv(&mut h, &o.x.to_le_bytes());
            fnv(&mut h, &o.y.to_le_bytes());
        }
        format!("{:08x}", (h ^ (h >> 32)) as u32)
    }

    /// The file this preset is saved as.
    pub fn path(&self) -> String {
        format!("{PRESET_DIR}{}-{}.json", slug(&self.name), self.id)
    }

    /// The preset as a clipboard for `Document::paste`, stamped with the
    /// target's act and version so no cross-map warning fires.
    pub fn to_clipboard(&self, target: &Ds1) -> Clipboard {
        Clipboard {
            width: self.width,
            height: self.height,
            cells: self.cells.clone(),
            objects: self.objects.clone(),
            source: ClipSource {
                game_path: None,
                act: target.act,
                lvl_type: None,
                version: target.version,
            },
        }
    }

    pub fn to_json(&self) -> String {
        let mut stored = self.clone();
        stored.occurrences = None;
        stored.found_in = None;
        serde_json::to_string_pretty(&stored).unwrap_or_default()
    }

    /// `None` for anything that is not a preset of this format.
    pub fn from_json(text: &str) -> Option<Preset> {
        let p: Preset = serde_json::from_str(text).ok()?;
        (p.format == PRESET_FORMAT).then_some(p)
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t: String = out.trim_matches('-').chars().take(40).collect();
    let t = t.trim_matches('-');
    if t.is_empty() {
        "preset".into()
    } else {
        t.into()
    }
}

/// Library order: category, then name.
pub fn sort_presets(presets: &mut [Preset]) {
    presets.sort_by(|a, b| {
        a.category
            .to_lowercase()
            .cmp(&b.category.to_lowercase())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// The categories of a library, sorted, each once.
pub fn categories(presets: &[Preset]) -> Vec<String> {
    let set: BTreeSet<&str> = presets.iter().map(|p| p.category.as_str()).collect();
    set.into_iter().map(str::to_string).collect()
}

/// What placing a preset would do.
pub fn preview_place(
    doc: &Document,
    preset: &Preset,
    at: (i32, i32),
    mode: PasteMode,
) -> PastePreview {
    doc.preview_paste(&preset.to_clipboard(doc.ds1()), at, mode)
}

/// Places the preset with its top-left cell at `at` as one undoable step
/// labelled "Place preset <name>" (cells, new layers and objects together).
/// Errors as `Document::paste`; nothing changes on failure.
pub fn place_preset(
    doc: &mut Document,
    preset: &Preset,
    at: (i32, i32),
    mode: PasteMode,
) -> Result<PastePreview> {
    let clip = preset.to_clipboard(doc.ds1());
    doc.begin_group(&format!("Place preset {}", preset.name));
    let r = doc.paste(&clip, at, mode);
    doc.end_group();
    r
}

// ───────────── preset builder ─────────────

/// The builder: a scratch 20x20 map with 2 floor and 4 wall layers that the
/// normal tools paint on, saved as a preset trimmed to what was drawn.
pub struct PresetBuilder {
    doc: Document,
}

impl PresetBuilder {
    /// A blank builder map for the given act.
    pub fn new(act: i32) -> Result<PresetBuilder> {
        let ds1 = new_map(&NewMapSpec {
            width: BUILDER_SIZE,
            height: BUILDER_SIZE,
            act,
            walls: 4,
            floors: 2,
            tag: false,
            version: None,
        })?;
        Ok(PresetBuilder {
            doc: Document::new(ds1),
        })
    }

    /// The scratch map (paint with the normal `Document` tools).
    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Whether anything was drawn.
    pub fn is_empty(&self) -> bool {
        self.occupied().is_empty()
    }

    fn occupied(&self) -> Selection {
        let ds1 = self.doc.ds1();
        let mut sel = Selection::new();
        let layers: Vec<LayerRef> = LayerRef::all_in(ds1)
            .into_iter()
            .filter(|l| is_tile_layer(*l))
            .collect();
        sel.box_select(
            ds1,
            CellRect::new((0, 0), (BUILDER_SIZE - 1, BUILDER_SIZE - 1)),
            &layers,
            false,
            SelectOp::Replace,
        );
        sel
    }

    /// Saves what is drawn as a preset. With `trim` the preset is cropped to
    /// the drawn cells' bounding box, else it is the whole 20x20. `None` when
    /// nothing is drawn. Objects on the drawn cells come along.
    pub fn build(&self, name: &str, category: &str, trim: bool) -> Option<Preset> {
        let sel = self.occupied();
        let drawn = sel.bounds()?;
        let mut clip = self.doc.copy(&sel);
        if !trim {
            // pin the origin at the map's top-left
            for c in &mut clip.cells {
                c.1 += drawn.x0;
                c.2 += drawn.y0;
            }
            for o in &mut clip.objects {
                o.x += (drawn.x0 * 5) as i32;
                o.y += (drawn.y0 * 5) as i32;
            }
            clip.width = BUILDER_SIZE;
            clip.height = BUILDER_SIZE;
        }
        Some(Preset::from_clipboard(&clip, name, category))
    }
}

// ───────────── suggestions ─────────────

const SPECIAL_1: u32 = 10;
const SPECIAL_2: u32 = 11;
const PILLARS: u32 = 12;
const SHADOW: u32 = 13;
const TREE: u32 = 14;
const ROOF: u32 = 15;

/// Options for [`suggest_presets`].
pub struct SuggestOptions<'a> {
    /// At most this many suggestions (ds1-studio: 60).
    pub limit: usize,
    /// Whether the current map can draw a tile `(orientation, main, sub)`.
    /// Floors with main >= 30 and the special markers are always accepted.
    pub drawable: &'a dyn Fn(u32, u8, u8) -> bool,
    /// Category for a suggestion (e.g. from its DT1s); default "Suggested".
    pub category: Option<&'a dyn Fn(&Preset) -> String>,
}

impl Default for SuggestOptions<'_> {
    fn default() -> Self {
        SuggestOptions {
            limit: 60,
            drawable: &|_, _, _| true,
            category: None,
        }
    }
}

fn wall_at(ds1: &Ds1, x: u32, y: u32) -> impl Iterator<Item = (Cell, u32)> + '_ {
    ds1.walls.iter().filter_map(move |l| {
        let c = l.tiles.get(x, y).filter(|c| !c.is_empty())?;
        Some((c, l.orientations.get(x, y).unwrap_or(0)))
    })
}

/// Wall structures plus their shadow cells, as cell indices (`y * width + x`):
/// 8-connected groups of wall-layer cells (walls, objects, trees, roofs),
/// each with the shadow cells within two cells that match one of its walls.
/// Shadows never join two separate structures.
pub fn suggestion_groups(ds1: &Ds1) -> Vec<Vec<u32>> {
    let (w, h) = (ds1.width, ds1.height);
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let n = (w * h) as usize;
    let mut solid = vec![false; n];
    for y in 0..h {
        for x in 0..w {
            solid[idx(x, y)] = wall_at(ds1, x, y).any(|(_, o)| o != SPECIAL_1 && o != SPECIAL_2);
        }
    }
    let mut owners = vec![usize::MAX; n];
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for start in 0..n {
        if !solid[start] || owners[start] != usize::MAX {
            continue;
        }
        let id = groups.len();
        let mut group = Vec::new();
        let mut todo = vec![start];
        owners[start] = id;
        while let Some(i) = todo.pop() {
            group.push(i as u32);
            let (x, y) = ((i as u32 % w) as i64, (i as u32 / w) as i64);
            for dy in -1..=1i64 {
                for dx in -1..=1i64 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(w) || ny >= i64::from(h) {
                        continue;
                    }
                    let k = idx(nx as u32, ny as u32);
                    if solid[k] && owners[k] == usize::MAX {
                        owners[k] = id;
                        todo.push(k);
                    }
                }
            }
        }
        groups.push(group);
    }
    for y in 0..h {
        for x in 0..w {
            let i = idx(x, y);
            if owners[i] != usize::MAX {
                continue;
            }
            let Some(s) = ds1.shadow.get(x, y).filter(|c| !c.is_empty()) else {
                continue;
            };
            let mut best: BTreeMap<usize, i64> = BTreeMap::new();
            for dy in -2..=2i64 {
                for dx in -2..=2i64 {
                    let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(w) || ny >= i64::from(h) {
                        continue;
                    }
                    let k = idx(nx as u32, ny as u32);
                    if !solid[k] {
                        continue;
                    }
                    let hit = wall_at(ds1, nx as u32, ny as u32).any(|(c, _)| {
                        c.main_index() == s.main_index() && c.sub_index() == s.sub_index()
                    });
                    if hit {
                        let d = dx * dx + dy * dy;
                        let e = best.entry(owners[k]).or_insert(d);
                        *e = (*e).min(d);
                    }
                }
            }
            let mut sorted: Vec<(usize, i64)> = best.into_iter().collect();
            sorted.sort_by_key(|&(_, d)| d);
            if let Some(&(g, d)) = sorted.first()
                && (sorted.len() == 1 || d < sorted[1].1)
            {
                groups[g].push(i as u32);
            }
        }
    }
    groups
}

fn describe(width: u32, height: u32, orientations: &BTreeSet<u32>) -> &'static str {
    let has = |os: &[u32]| os.iter().any(|o| orientations.contains(o));
    let others = (1..=9).any(|o| orientations.contains(&o));
    if has(&[TREE]) && !others {
        "Trees"
    } else if has(&[ROOF]) {
        "Building"
    } else if has(&[8, 9]) {
        "Wall with door"
    } else if has(&[1, 2]) && (width == 1 || height == 1) {
        "Wall run"
    } else if has(&[3, 4, 5, 6, 7]) {
        "Walls"
    } else if has(&[PILLARS]) {
        "Props"
    } else if has(&[16, 17, 18, 19]) {
        "Cliff / lower wall"
    } else {
        "Structure"
    }
}

/// Finds structures worth reusing in `maps` (path, map): connected groups of
/// wall-layer tiles at most 16x16 cells with at least two parts (wall or
/// shadow cells), keeping those whose every tile `opts.drawable` accepts.
/// Identical structures (same size and tiles, ignoring cell flags and
/// objects) merge and are ranked by how often they occur, then by area.
pub fn suggest_presets(maps: &[(String, &Ds1)], opts: &SuggestOptions<'_>) -> Vec<Preset> {
    type Key = (u32, u32, Vec<(LayerRef, Vec<(u32, u32, u32)>)>);
    let mut found: Vec<(Key, Preset)> = Vec::new();
    for (path, ds1) in maps {
        let w = ds1.width;
        for cells in suggestion_groups(ds1) {
            if cells.len() > (SUGGEST_MAX_SIDE * SUGGEST_MAX_SIDE) as usize {
                continue;
            }
            let xs = cells.iter().map(|i| i % w);
            let ys = cells.iter().map(|i| i / w);
            let (x0, x1) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
            let (y0, y1) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
            let (pw, ph) = (x1 - x0 + 1, y1 - y0 + 1);
            if pw > SUGGEST_MAX_SIDE || ph > SUGGEST_MAX_SIDE {
                continue;
            }
            let inside: BTreeSet<(u32, u32)> = cells.iter().map(|i| (i % w, i / w)).collect();
            let parts: usize = inside
                .iter()
                .map(|&(x, y)| {
                    wall_at(ds1, x, y).count()
                        + usize::from(ds1.shadow.get(x, y).is_some_and(|c| !c.is_empty()))
                })
                .sum();
            if parts < 2 {
                continue;
            }
            let mut out: Vec<(LayerRef, u32, u32, CellValue)> = Vec::new();
            for l in LayerRef::all_in(ds1) {
                if !is_tile_layer(l) {
                    continue;
                }
                for &(x, y) in &inside {
                    if let Some(v) = l.get(ds1, x, y).filter(|v| !l.is_empty_value(*v)) {
                        out.push((l, x - x0, y - y0, v));
                    }
                }
            }
            let orientations: BTreeSet<u32> = out
                .iter()
                .filter(|c| matches!(c.0, LayerRef::Wall(_)))
                .map(|c| c.3.orientation)
                .filter(|&o| o > 0)
                .collect();
            if orientations.iter().all(|&o| o == TREE || o == PILLARS) {
                out.retain(|c| !matches!(c.0, LayerRef::Floor(_)));
            }
            let drawable = out.iter().all(|&(l, _, _, v)| {
                let c = Cell(v.cell);
                let o = match l {
                    LayerRef::Wall(_) => v.orientation,
                    LayerRef::Shadow => SHADOW,
                    _ => 0,
                };
                o == SPECIAL_1
                    || o == SPECIAL_2
                    || (o == 0 && c.main_index() >= 30)
                    || (opts.drawable)(o, c.main_index(), c.sub_index())
            });
            if !drawable {
                continue;
            }
            let mut key_layers: BTreeMap<LayerRef, Vec<(u32, u32, u32)>> = BTreeMap::new();
            for &(l, x, y, v) in &out {
                if !matches!(l, LayerRef::Floor(_)) {
                    // identity ignores the low prop1 byte (cell flags) and objects
                    key_layers.entry(l).or_default().push((
                        y * pw + x,
                        (v.cell >> 8) & 0x00FF_FFFF,
                        v.orientation,
                    ));
                }
            }
            let key: Key = (pw, ph, key_layers.into_iter().collect());
            if let Some((_, p)) = found.iter_mut().find(|(k, _)| *k == key) {
                p.occurrences = Some(p.occurrences.unwrap_or(1) + 1);
                continue;
            }
            let objects: Vec<Object> = ds1
                .objects
                .iter()
                .filter(|o| {
                    inside.contains(&(o.x.div_euclid(5) as u32, o.y.div_euclid(5) as u32))
                        && o.x >= 0
                        && o.y >= 0
                })
                .map(|o| {
                    let mut o = *o;
                    o.x -= (x0 * 5) as i32;
                    o.y -= (y0 * 5) as i32;
                    o
                })
                .collect();
            let clip = Clipboard {
                width: pw,
                height: ph,
                cells: out,
                objects,
                source: ClipSource {
                    game_path: None,
                    act: ds1.act,
                    lvl_type: None,
                    version: ds1.version,
                },
            };
            let kind = describe(pw, ph, &orientations);
            let mut p =
                Preset::from_clipboard(&clip, &format!("{kind} {pw}\u{d7}{ph}"), "Suggested");
            if let Some(cat) = opts.category {
                p.category = cat(&p);
            }
            p.occurrences = Some(1);
            p.found_in = Some(path.clone());
            found.push((key, p));
        }
    }
    let mut out: Vec<Preset> = found.into_iter().map(|(_, p)| p).collect();
    out.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then((b.width * b.height).cmp(&(a.width * a.height)))
    });
    out.truncate(opts.limit);
    out
}

/// Convenience: [`EditError`] when a preset would not fit the map at all.
pub fn check_fits(doc: &Document, preset: &Preset, at: (i32, i32)) -> Result<()> {
    let (w, h) = (i64::from(doc.ds1().width), i64::from(doc.ds1().height));
    let fits = i64::from(at.0) >= 0
        && i64::from(at.1) >= 0
        && i64::from(at.0) + i64::from(preset.width) <= w
        && i64::from(at.1) + i64::from(preset.height) <= h;
    if fits {
        Ok(())
    } else {
        Err(EditError::OutOfBounds {
            x: i64::from(at.0),
            y: i64::from(at.1),
            width: doc.ds1().width,
            height: doc.ds1().height,
        })
    }
}
