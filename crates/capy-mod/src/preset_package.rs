//! Preset packages (ds1-studio `presetPackage.ts`, `presetLibrary.ts`): one
//! preset, a category or all of them in a `.zip` with the mod's own tile
//! libraries they use, so they can be shared between mods; and copying the
//! tiles a preset needs into a DT1 (`preparePresetLibrary`).
//!
//! ```text
//! ds1studio-presets.json   { format, version, presets: [file names], dt1s: [tile paths] }
//! presets/<file>.json      the presets, as saved in data/ds1studio/presets
//! tiles/<path>             the mod DT1s they use, at their path under data/global/tiles
//! ```
//!
//! Pure: files come in and go out as `(path, bytes)` lists; the caller does
//! the zip and the file I/O. The game's own DT1s are left out of a package:
//! everyone has them.

use std::collections::{BTreeMap, BTreeSet};

use capy_edit::presets::Preset;
use capy_edit::{CellValue, tools::tile_key};
use d2_formats::dt1::Dt1;
use serde::{Deserialize, Serialize};

use crate::tiles_dt1::{MAX_TILE_PATH, normalize_path};
use crate::tiles_review::{CustomDt1Plan, TileId, TilePick, build_custom_dt1, plan_custom_dt1};

/// The format tag in the manifest.
pub const PRESET_PACKAGE: &str = "ds1studio-presets";
/// The manifest's file name in the zip.
pub const MANIFEST: &str = "ds1studio-presets.json";
const TILES: &str = "data/global/tiles/";

/// Why a package or library step failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PresetPackageError(pub String);

type Result<T> = std::result::Result<T, PresetPackageError>;

fn fail<T>(m: impl Into<String>) -> Result<T> {
    Err(PresetPackageError(m.into()))
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    presets: Vec<String>,
    dt1s: Vec<String>,
}

/// `data/global/tiles/a/b.dt1` -> `a/b.dt1`.
fn rel(p: &str) -> String {
    let p = p.replace('\\', "/");
    let p = p.trim_start_matches('/');
    match p.to_ascii_lowercase().strip_prefix(TILES) {
        Some(rest) => p[p.len() - rest.len()..].to_owned(),
        None => p.to_owned(),
    }
}

fn unsafe_path(p: &str) -> bool {
    let p = p.replace('\\', "/");
    p.starts_with('/') || p.contains(':') || p.split('/').any(|s| s == "..")
}

/// The key a preset's `tile_sources` uses: `orientation|main|sub`.
pub fn source_key(id: TileId) -> String {
    format!("{}|{}|{}", id.orientation, id.main, id.sub)
}

/// Every tile number a preset places (specials excluded: they are markers,
/// not pictures), with how many cells use it.
pub fn preset_tiles(p: &Preset) -> BTreeMap<TileId, usize> {
    let mut out = BTreeMap::new();
    for &(layer, _, _, v) in &p.cells {
        if let Some(k) = tile_key(layer, v) {
            let id = TileId::new(k.orientation as i32, i32::from(k.main), i32::from(k.sub));
            if id.orientation == 10 || id.orientation == 11 {
                continue;
            }
            *out.entry(id).or_insert(0) += 1;
        }
    }
    out
}

/// Records where each tile of the preset comes from (`tile_sources`, `dt1s`).
/// `source_of` gives the game path of the DT1 a tile number resolves to in the
/// map the preset was made from.
pub fn fill_sources(p: &mut Preset, source_of: &dyn Fn(TileId) -> Option<String>) {
    p.tile_sources.clear();
    for id in preset_tiles(p).into_keys() {
        if let Some(s) = source_of(id) {
            p.tile_sources.insert(source_key(id), s);
        }
    }
    let set: BTreeSet<String> = p.tile_sources.values().cloned().collect();
    p.dt1s = set.into_iter().collect();
}

/// A tile library file found in the game or mod.
pub struct TileFile {
    pub bytes: Vec<u8>,
    /// From the game's own archives (left out of packages).
    pub base_game: bool,
    /// The path as it exists (case as stored).
    pub path: String,
}

/// A built package: the files to zip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetPackage {
    /// `(zip path, bytes)`: the manifest, `presets/...` and `tiles/...`.
    pub files: Vec<(String, Vec<u8>)>,
    /// The mod tile libraries included (tiles-relative).
    pub dt1s: Vec<String>,
    /// Tile libraries the presets use that could not be found.
    pub missing: Vec<String>,
}

fn file_name(p: &Preset) -> String {
    p.path()
        .rsplit('/')
        .next()
        .unwrap_or("preset.json")
        .to_owned()
}

/// The package for these presets: their files plus every tile library they use
/// that isn't one of the game's own. `locate` finds a library by game path.
pub fn build_preset_package(
    presets: &[Preset],
    locate: &dyn Fn(&str) -> Option<TileFile>,
) -> PresetPackage {
    let mut files = Vec::new();
    let mut names = Vec::new();
    for p in presets {
        let name = file_name(p);
        files.push((format!("presets/{name}"), p.to_json().into_bytes()));
        names.push(name);
    }
    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    for p in presets {
        for d in p.dt1s.iter().chain(p.tile_sources.values()) {
            wanted.entry(normalize_path(d)).or_insert_with(|| d.clone());
        }
    }
    let (mut dt1s, mut missing) = (Vec::new(), Vec::new());
    for (norm, orig) in wanted {
        match locate(&norm).or_else(|| locate(&orig)) {
            None => missing.push(rel(&norm)),
            Some(f) if f.base_game => {}
            Some(f) => {
                let path = rel(&f.path);
                files.push((format!("tiles/{path}"), f.bytes));
                dt1s.push(path);
            }
        }
    }
    let manifest = Manifest {
        format: PRESET_PACKAGE.into(),
        version: 1,
        presets: names,
        dt1s: dt1s.clone(),
    };
    files.insert(
        0,
        (
            MANIFEST.into(),
            serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
        ),
    );
    PresetPackage {
        files,
        dt1s,
        missing,
    }
}

/// What importing a package (or one preset file) would add.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresetImportPlan {
    /// The presets to save (new ids where one is taken; tile paths changed
    /// where a DT1 had to be renamed).
    pub presets: Vec<Preset>,
    /// Tile libraries to write into the mod: `(full game path, bytes)`.
    pub dt1_writes: Vec<(String, Vec<u8>)>,
    /// DT1s already in the mod exactly as packaged (not written again).
    pub same: Vec<String>,
    /// DT1s whose path holds a different file here: written under a new name.
    pub renamed: Vec<(String, String)>,
    /// Presets already here unchanged (skipped).
    pub duplicates: Vec<String>,
    /// Things that will be wrong after importing.
    pub problems: Vec<String>,
}

fn compare_form(p: &Preset) -> String {
    let mut q = p.clone();
    q.occurrences = None;
    q.found_in = None;
    q.to_json()
}

fn fnv(s: &str) -> u32 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    (h ^ (h >> 32)) as u32
}

/// A free name for a clashing DT1 in its folder: `<name>2.dt1`, `<name>3.dt1`,
/// shortened to fit the path limit.
fn free_name(path: &str, taken: &dyn Fn(&str) -> bool) -> Option<String> {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let base = file
        .strip_suffix(".dt1")
        .or_else(|| file.strip_suffix(".DT1"))
        .unwrap_or(file);
    for n in 2..100 {
        let suffix = n.to_string();
        let room = MAX_TILE_PATH
            .checked_sub(dir.len() + suffix.len() + 4)
            .filter(|r| *r >= 1)?;
        let cand = format!(
            "{dir}{}{suffix}.dt1",
            base.chars().take(room).collect::<String>()
        );
        if !taken(&format!("{TILES}{cand}")) {
            return Some(cand);
        }
    }
    None
}

/// What importing `bytes` (a `.zip` package, or a single preset `.json`)
/// would add, without writing anything. `read` finds a file in this game or
/// mod by full game path; `entries` is the zip's content (`read_zip`) when
/// `bytes` is a zip, else ignored.
pub fn plan_preset_import(
    bytes: &[u8],
    zip_entries: Option<&[(String, Vec<u8>)]>,
    existing: &[Preset],
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<PresetImportPlan> {
    let mut plan = PresetImportPlan::default();
    let mut preset_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut tiles: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(entries) = zip_entries {
        let Some((_, m)) = entries.iter().find(|(n, _)| n == MANIFEST) else {
            return fail("This .zip is not a preset package (no ds1studio-presets.json).");
        };
        let manifest: Manifest = serde_json::from_slice(m).map_err(|_| {
            PresetPackageError(
                "The package description (ds1studio-presets.json) is damaged.".into(),
            )
        })?;
        if manifest.format != PRESET_PACKAGE {
            return fail("This .zip is not a preset package.");
        }
        for (name, data) in entries {
            if unsafe_path(name) {
                continue;
            }
            if let Some(n) = name
                .strip_prefix("presets/")
                .filter(|n| n.ends_with(".json"))
            {
                preset_files.push((n.to_owned(), data.clone()));
            } else if let Some(n) = name
                .strip_prefix("tiles/")
                .filter(|n| n.to_ascii_lowercase().ends_with(".dt1"))
            {
                tiles.push((n.to_owned(), data.clone()));
            }
        }
    } else {
        preset_files.push(("preset.json".into(), bytes.to_vec()));
    }
    if preset_files.is_empty() {
        return fail("There are no presets in this file.");
    }

    // Tile libraries: kept at their path when free or identical, else written
    // under a new name the presets point to.
    let mut moved: BTreeMap<String, String> = BTreeMap::new();
    for (path, data) in tiles {
        let full = format!("{TILES}{path}");
        match read(&full) {
            Some(here) if here == data => plan.same.push(path),
            None => plan.dt1_writes.push((full, data)),
            Some(_) => {
                let pending: Vec<String> = plan
                    .dt1_writes
                    .iter()
                    .map(|(p, _)| normalize_path(p))
                    .collect();
                let taken = |p: &str| read(p).is_some() || pending.contains(&normalize_path(p));
                match free_name(&path, &taken) {
                    Some(target) => {
                        plan.dt1_writes.push((format!("{TILES}{target}"), data));
                        moved.insert(normalize_path(&full), format!("{TILES}{target}"));
                        plan.renamed.push((path, target));
                    }
                    None => plan.problems.push(format!(
                        "{path}: a different file with this name is already in your mod, and no free shorter name fits the game's {MAX_TILE_PATH}-character limit. Import skipped it."
                    )),
                }
            }
        }
    }

    let mut taken: BTreeSet<String> = existing.iter().map(|p| p.id.clone()).collect();
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for (name, data) in preset_files {
        let Some(p) = std::str::from_utf8(&data).ok().and_then(Preset::from_json) else {
            plan.problems.push(format!("{name}: not a preset."));
            continue;
        };
        let swap = |d: &String| {
            moved
                .get(&normalize_path(d))
                .cloned()
                .unwrap_or_else(|| d.clone())
        };
        let mut next = p.clone();
        next.dt1s = p.dt1s.iter().map(swap).collect();
        next.tile_sources = p
            .tile_sources
            .iter()
            .map(|(k, v)| (k.clone(), swap(v)))
            .collect();
        if existing
            .iter()
            .any(|e| e.id == p.id && compare_form(e) == compare_form(&next))
        {
            plan.duplicates.push(p.name);
            continue;
        }
        let mut n = 0;
        while taken.contains(&next.id) {
            n += 1;
            next.id = format!("{:08x}", fnv(&format!("{}#{n}", p.id)));
        }
        taken.insert(next.id.clone());
        for d in &next.dt1s {
            let norm = normalize_path(d);
            if read(d).is_none()
                && !plan
                    .dt1_writes
                    .iter()
                    .any(|(w, _)| normalize_path(w) == norm)
            {
                missing.insert(rel(d));
            }
        }
        plan.presets.push(next);
    }
    for m in missing {
        plan.problems.push(format!(
            "{m} isn't in this game or mod (and wasn't in the package): presets using it show those tiles as missing."
        ));
    }
    Ok(plan)
}

// ───────────── copying a preset's tiles into a DT1 ─────────────

/// The result of [`prepare_preset_library`].
#[derive(Clone, Debug)]
pub struct PreparedLibrary {
    /// The tiles copied and any renumbering.
    pub plan: CustomDt1Plan,
    /// The new library, or `None` when nothing needs copying.
    pub dt1: Option<Dt1>,
    /// The preset with its cells renumbered and its sources pointing at `path`.
    pub preset: Preset,
}

/// Copies only the tile numbers the preset needs and the map lacks (including
/// variants, animation frames and corner partners) into a new library at
/// `path`. `target(id)` gives the sources of the map's tiles with that number
/// (empty = the map has none); a number whose tiles all come from the
/// preset's source already is left alone, and `keep` holds numbers where the
/// map's own version is chosen. `taken` are numbers to avoid when renumbering
/// (the map's tiles, or every library of its level type). `read` loads a
/// source DT1 by game path.
pub fn prepare_preset_library(
    preset: &Preset,
    path: &str,
    target: &dyn Fn(TileId) -> Vec<String>,
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
    keep: &BTreeSet<TileId>,
    taken: &BTreeSet<TileId>,
) -> Result<PreparedLibrary> {
    let mut parsed: Vec<(String, Dt1)> = Vec::new();
    let mut picks: Vec<TilePick> = Vec::new();
    let mut needed: BTreeMap<TileId, usize> = BTreeMap::new();
    for id in preset_tiles(preset).into_keys() {
        let from = preset.tile_sources.get(&source_key(id));
        let variants = target(id);
        if !variants.is_empty()
            && from.is_none_or(|f| {
                variants
                    .iter()
                    .all(|v| normalize_path(v) == normalize_path(f))
            })
        {
            continue;
        }
        if !variants.is_empty() && keep.contains(&id) {
            continue;
        }
        let Some(from) = from else {
            return fail(format!(
                "The source of preset tile {} is unknown. Re-save this preset from its original map.",
                source_key(id)
            ));
        };
        let norm = normalize_path(from);
        let si = match parsed.iter().position(|(p, _)| *p == norm) {
            Some(i) => i,
            None => {
                let Some(bytes) = read(&norm) else {
                    return fail(format!("Preset tile library is missing: {norm}"));
                };
                let dt1 =
                    Dt1::parse(&bytes).map_err(|e| PresetPackageError(format!("{norm}: {e}")))?;
                parsed.push((norm.clone(), dt1));
                parsed.len() - 1
            }
        };
        let matches: Vec<usize> = parsed[si]
            .1
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| TileId::of(t) == id)
            .map(|(i, _)| i)
            .collect();
        if matches.is_empty() {
            return fail(format!(
                "Preset tile {} no longer exists in {norm}.",
                source_key(id)
            ));
        }
        picks.extend(
            matches
                .into_iter()
                .map(|index| TilePick { source: si, index }),
        );
        needed.insert(id, si);
    }
    let sources: Vec<(String, &Dt1)> = parsed.iter().map(|(p, d)| (p.clone(), d)).collect();
    let plan = plan_custom_dt1(&picks, &sources, taken);
    if !plan.skipped.is_empty() {
        return fail(plan.skipped.join("\n"));
    }
    let remap: BTreeMap<TileId, (i32, i32)> = plan
        .tiles
        .iter()
        .filter(|t| needed.get(&t.id) == Some(&t.from.source))
        .map(|t| (t.id, (t.new_main, t.new_sub)))
        .collect();
    let mut next = preset.clone();
    let mut tile_sources = preset.tile_sources.clone();
    for (layer, _, _, v) in &mut next.cells {
        let Some(k) = tile_key(*layer, *v) else {
            continue;
        };
        let id = TileId::new(k.orientation as i32, i32::from(k.main), i32::from(k.sub));
        let Some(&(m, s)) = remap.get(&id) else {
            continue;
        };
        let cell = d2_formats::ds1::Cell(v.cell).with_indices(m as u8, s as u8);
        *v = CellValue { cell: cell.0, ..*v };
        tile_sources.insert(
            source_key(TileId::new(id.orientation, m, s)),
            path.to_owned(),
        );
    }
    let used: BTreeMap<String, String> = preset_tiles(&next)
        .into_keys()
        .filter_map(|id| {
            let k = source_key(id);
            tile_sources.get(&k).map(|s| (k, s.clone()))
        })
        .collect();
    next.dt1s = used
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    next.tile_sources = used;
    let dt1 = (!plan.records.is_empty()).then(|| build_custom_dt1(&plan, None));
    Ok(PreparedLibrary {
        plan,
        dt1,
        preset: next,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use capy_edit::LayerRef;
    use d2_formats::dt1::Tile;

    fn preset(id: &str, name: &str, dt1s: &[&str]) -> Preset {
        let mut p = Preset {
            format: capy_edit::presets::PRESET_FORMAT.into(),
            version: 1,
            id: id.into(),
            name: name.into(),
            category: "Buildings".into(),
            width: 1,
            height: 1,
            dt1s: dt1s.iter().map(|s| (*s).to_owned()).collect(),
            tile_sources: BTreeMap::new(),
            cells: vec![(
                LayerRef::Floor(0),
                0,
                0,
                CellValue {
                    cell: 0xC2 | (3 << 20) | (1 << 8),
                    orientation: 0,
                },
            )],
            objects: Vec::new(),
            occurrences: None,
            found_in: None,
        };
        for d in dt1s {
            p.tile_sources.insert("0|3|1".into(), (*d).to_owned());
        }
        p
    }

    const MOD_DT1: &str = "data/global/tiles/PD2assets/custom/house.dt1";
    const GAME_DT1: &str = "data/global/tiles/ACT1/Town/floor.dt1";

    fn locate(p: &str) -> Option<TileFile> {
        let n = normalize_path(p);
        if n == normalize_path(MOD_DT1) {
            Some(TileFile {
                bytes: vec![2, 2, 2],
                base_game: false,
                path: MOD_DT1.into(),
            })
        } else if n == normalize_path(GAME_DT1) {
            Some(TileFile {
                bytes: vec![1, 1],
                base_game: true,
                path: GAME_DT1.into(),
            })
        } else {
            None
        }
    }

    fn zip_of(p: &PresetPackage) -> Vec<(String, Vec<u8>)> {
        p.files.clone()
    }

    #[test]
    fn packs_the_presets_and_only_the_mods_own_libraries() {
        let a = preset("aaaa1111", "House", &[MOD_DT1, GAME_DT1]);
        let b = preset("bbbb2222", "Wall", &[GAME_DT1]);
        let built = build_preset_package(&[a, b], &locate);
        let mut names: Vec<&str> = built.files.iter().map(|f| f.0.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names.len(), 4, "{names:?}");
        assert!(names.contains(&"ds1studio-presets.json"));
        assert!(names.contains(&"tiles/PD2assets/custom/house.dt1"));
        assert_eq!(built.dt1s, vec!["PD2assets/custom/house.dt1"]);
        assert!(built.missing.is_empty());
    }

    #[test]
    fn imports_into_a_mod_without_the_library() {
        let a = preset("aaaa1111", "House", &[MOD_DT1]);
        let built = build_preset_package(&[a], &locate);
        let entries = zip_of(&built);
        let plan = plan_preset_import(b"PK", Some(&entries), &[], &|_| None).unwrap();
        assert_eq!(plan.presets.len(), 1);
        assert_eq!(plan.dt1_writes.len(), 1);
        assert_eq!(
            plan.dt1_writes[0].0,
            "data/global/tiles/PD2assets/custom/house.dt1"
        );
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    }

    #[test]
    fn a_different_library_under_the_same_name_is_kept_and_the_import_renamed() {
        let a = preset("aaaa1111", "House", &[MOD_DT1]);
        let entries = zip_of(&build_preset_package(&[a], &locate));
        let read = |p: &str| (normalize_path(p) == normalize_path(MOD_DT1)).then(|| vec![9, 9]);
        let plan = plan_preset_import(b"PK", Some(&entries), &[], &read).unwrap();
        assert_eq!(
            plan.renamed,
            vec![(
                "PD2assets/custom/house.dt1".to_owned(),
                "PD2assets/custom/house2.dt1".to_owned()
            )]
        );
        assert_eq!(
            plan.dt1_writes[0].0,
            "data/global/tiles/PD2assets/custom/house2.dt1"
        );
        assert!(
            plan.presets[0]
                .dt1s
                .contains(&"data/global/tiles/PD2assets/custom/house2.dt1".to_owned())
        );
        assert!(!plan.presets[0].dt1s.contains(&MOD_DT1.to_owned()));
    }

    #[test]
    fn the_same_library_is_not_rewritten_duplicates_skipped_taken_ids_renewed() {
        let a = preset("aaaa1111", "House", &[MOD_DT1]);
        let b = preset("bbbb2222", "Wall", &[GAME_DT1]);
        let entries = zip_of(&build_preset_package(&[a.clone(), b.clone()], &locate));
        let mut changed_b = b.clone();
        changed_b.name = "Wall (theirs)".into();
        let read = |p: &str| locate(p).map(|f| f.bytes);
        let plan = plan_preset_import(b"PK", Some(&entries), &[a, changed_b], &read).unwrap();
        assert_eq!(plan.same, vec!["PD2assets/custom/house.dt1"]);
        assert!(plan.dt1_writes.is_empty());
        assert_eq!(plan.duplicates, vec!["House"]);
        assert_eq!(plan.presets.len(), 1);
        assert_eq!(plan.presets[0].name, "Wall");
        assert_ne!(plan.presets[0].id, "bbbb2222");
    }

    #[test]
    fn a_single_preset_file_imports_and_names_the_missing_libraries() {
        let a = preset("aaaa1111", "House", &[MOD_DT1]);
        let plan = plan_preset_import(a.to_json().as_bytes(), None, &[], &|_| None).unwrap();
        assert_eq!(plan.presets.len(), 1);
        assert!(
            plan.problems
                .join(" ")
                .contains("house.dt1 isn't in this game or mod")
        );
    }

    #[test]
    fn files_that_are_not_packages_are_refused() {
        let entries = vec![("readme.txt".to_owned(), vec![1])];
        assert!(plan_preset_import(b"PK", Some(&entries), &[], &|_| None).is_err());
        let plan = plan_preset_import(b"{\"hello\":1}", None, &[], &|_| None).unwrap();
        assert!(plan.presets.is_empty());
        assert_eq!(plan.problems.len(), 1);
    }

    #[test]
    fn fill_sources_records_where_each_tile_comes_from() {
        let mut p = preset("x", "X", &[]);
        fill_sources(&mut p, &|id| (id.main == 3).then(|| GAME_DT1.to_owned()));
        assert_eq!(
            p.tile_sources.get("0|3|1").map(String::as_str),
            Some(GAME_DT1)
        );
        assert_eq!(p.dt1s, vec![GAME_DT1]);
    }

    fn dt1_with(tiles: &[(i32, i32, i32)]) -> Vec<u8> {
        let mut d = Dt1 {
            version: (7, 6),
            tiles: Vec::new(),
        };
        for &(o, m, s) in tiles {
            let mut t = Tile::default();
            t.orientation = o;
            t.main_index = m;
            t.sub_index = s;
            d.tiles.push(t);
        }
        d.to_bytes().unwrap()
    }

    #[test]
    fn prepare_copies_only_missing_numbers_and_renumbers_clashes() {
        let mut p = preset("x", "X", &[]);
        p.tile_sources
            .insert("0|3|1".into(), "data/global/tiles/src.dt1".into());
        p.dt1s = vec!["data/global/tiles/src.dt1".into()];
        let src = dt1_with(&[(0, 3, 1)]);
        let read = |path: &str| (path == "data/global/tiles/src.dt1").then(|| src.clone());
        // The map has nothing with 3/1: copied unchanged to the new library.
        let none = |_: TileId| Vec::new();
        let out = prepare_preset_library(
            &p,
            "data/global/tiles/own.dt1",
            &none,
            &read,
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(out.dt1.as_ref().map(|d| d.tiles.len()), Some(1));
        assert_eq!(
            out.preset.tile_sources.get("0|3|1").map(String::as_str),
            Some("data/global/tiles/own.dt1")
        );
        // The map already has the number from the same source: nothing to copy.
        let same = |_: TileId| vec!["data/global/tiles/src.dt1".to_owned()];
        let out = prepare_preset_library(
            &p,
            "x.dt1",
            &same,
            &read,
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(out.dt1.is_none());
        // The map has another picture under that number and it is taken: renumbered.
        let other = |_: TileId| vec!["data/global/tiles/other.dt1".to_owned()];
        let taken: BTreeSet<TileId> = [TileId::new(0, 3, 1)].into_iter().collect();
        let out = prepare_preset_library(
            &p,
            "data/global/tiles/own.dt1",
            &other,
            &read,
            &BTreeSet::new(),
            &taken,
        )
        .unwrap();
        assert_eq!(out.plan.renumbered.len(), 1);
        let v = out.preset.cells[0].3;
        assert_ne!(
            d2_formats::ds1::Cell(v.cell).main_index() as i32 * 1000
                + d2_formats::ds1::Cell(v.cell).sub_index() as i32,
            3001
        );
        // Keeping the map's own version copies nothing.
        let keep: BTreeSet<TileId> = [TileId::new(0, 3, 1)].into_iter().collect();
        let out = prepare_preset_library(&p, "x.dt1", &other, &read, &keep, &taken).unwrap();
        assert!(out.dt1.is_none());
        // An unknown source is an error.
        let mut unknown = p.clone();
        unknown.tile_sources.clear();
        assert!(
            prepare_preset_library(
                &unknown,
                "x.dt1",
                &none,
                &read,
                &BTreeSet::new(),
                &BTreeSet::new()
            )
            .is_err()
        );
    }
}
