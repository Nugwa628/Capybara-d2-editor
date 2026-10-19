//! "How does the game load this DS1?": the LvlPrest row(s) that name it, the
//! Level and LvlType behind them, the act, the palette and the ordered,
//! `Dt1Mask`-filtered DT1 list. Also imports DS1Edit's legacy `assets/*.ini`
//! files (`<LvlTypesId> <LvlPrestDef> <path>` per line) as overrides.
//!
//! # Dt1Mask rule
//!
//! Bit `N-1` of `Dt1Mask` enables `File N` of the LvlType, so bit 0 gates
//! `File 1` too (the loader in the reference editor, `txtread.c:844,992`;
//! the properties panel at `props_panel.c:168` always shows File 1 as on and
//! is the odd one out). This is exactly [`d2_formats::tables::LvlPrest::dt1_files`].
//!
//! # What is resolved when
//!
//! [`MapCatalog::build`] does all the I/O: it loads the three tables, parses
//! every DT1 they name (tile keys only), and reads every DS1 it is asked to
//! describe (header act, the tiles its cells use, its own file list).
//! Lookups afterwards are pure.
//!
//! # Guessing a LvlType
//!
//! Where no table gives a usable LvlType (LvlPrest rows whose Level is 0 or
//! has no tilesets, DS1s no LvlPrest row names) the candidates rank by
//!
//! 1. tile coverage: how many of the DS1's visible non-empty cells have a
//!    key in the candidate's (masked) DT1s, with the reference editor's wall
//!    fallbacks (orientation 18/19 swap, then sub index 0);
//! 2. overlap with the DS1's own tileset list;
//! 3. DT1s in the DS1's directory (and, for a LvlPrest row, fewest outside it);
//! 4. lowest id.
//!
//! Candidates are the LvlTypes with a DT1 in the DS1's directory; if there
//! are none, every LvlType competes and the winner is accepted only at a
//! coverage of [`COVERAGE_ACCEPT`].

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::{Dt1, TileKey};
use d2_formats::excel::ExcelTableKind;
use d2_formats::tables::{self, Level, LvlPrest, LvlType};
use d2_formats::txt::TxtTable;

use crate::error::{AssetError, Result};
use crate::overlay::Overlay;
use crate::path::GamePath;
use crate::profile::GameVersion;
use crate::tables::load_excel;

/// LvlTypes id whose tilesets are Act 4 art although the row says Act 5; the
/// reference editor draws it with the Act 4 palette.
const LVL_TYPE_ACT4_PALETTE: u32 = 35;
/// Act index (0-based) used for [`LVL_TYPE_ACT4_PALETTE`].
const ACT4: u8 = 3;
/// Highest 0-based act the palettes cover.
const MAX_ACT: u8 = 4;
/// Lowest tile coverage at which a LvlType found only by coverage is accepted.
pub const COVERAGE_ACCEPT: f32 = 0.90;
/// DT1 orientation of shadow tiles.
const SHADOW_ORIENTATION: u8 = 13;

/// Where a [`MapInfo`] came from; the order is the preference order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MapInfoSource {
    /// User override (e.g. imported legacy ini, or picked in the UI).
    Override,
    /// A LvlPrest row's File1..6 names this DS1; LvlType via Levels(LevelId).LevelType.
    LvlPrest,
    /// No LvlPrest row; LvlType chosen because its File slots share the DS1's
    /// directory (most shared DT1s wins; ties -> lowest id). All File slots used.
    PathGuess,
    /// No LvlPrest row and no directory candidate: the LvlType whose DT1 set
    /// resolves the most of the DS1's cells, accepted at coverage >= 0.90.
    TileCoverage,
    /// Nothing matched: act from the DS1 header, no DT1s.
    Ds1Header,
}

/// One interpretation of a DS1. (`Eq` is not derived: `coverage` is a float.)
#[derive(Clone, Debug, PartialEq)]
pub struct MapInfo {
    pub ds1: GamePath,
    pub source: MapInfoSource,
    pub lvlprest_def: Option<u32>,
    /// Which File1..6 of the LvlPrest row names the DS1 (0-based).
    pub lvlprest_slot: Option<u8>,
    pub level_id: Option<u32>,
    /// Levels `LevelName` (absent when the table stores no name).
    pub level_name: Option<String>,
    pub lvl_type: Option<u32>,
    /// 0-based act: LvlType act, else Level act, else the DS1 header's effective_act.
    pub act: u8,
    /// 0-based act whose `pal.dat` to use: `act`, except LvlType id 35 uses Act 4 (3),
    /// as the reference editor does.
    pub palette_act: u8,
    /// DT1s in load order (LvlType File slot order, masked by Dt1Mask when known).
    pub dt1_files: Vec<GamePath>,
    /// DT1s named above that do not resolve in the overlay.
    pub missing_dt1: Vec<GamePath>,
    /// Fraction of visible non-empty cells whose tile key resolves in
    /// `dt1_files`, when it was computed (every interpretation of a DS1 that
    /// `build` could read; never for overrides).
    pub coverage: Option<f32>,
}

/// One line of a legacy `assets/*.ini`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyEntry {
    pub lvl_type: u32,
    pub lvlprest_def: u32,
    pub ds1: GamePath,
}

/// Parses one legacy ini. `assets/tiles/ACT1/TOWN/townE1.ds1` maps to
/// `data\global\tiles\act1\town\towne1.ds1`. Bad lines are returned as warnings.
///
/// A line is `<LvlTypesId> <LvlPrestDef> <path>`; the path is the rest of the
/// line, so it may contain spaces. Blank lines and lines starting with `#` or
/// `;` are skipped. Both `/` and `\` separators are accepted.
pub fn parse_legacy_ini(text: &str) -> (Vec<LegacyEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        match parse_legacy_line(line) {
            Ok(entry) => entries.push(entry),
            Err(why) => warnings.push(format!("line {number}: {why}: {line:?}")),
        }
    }
    (entries, warnings)
}

fn parse_legacy_line(line: &str) -> std::result::Result<LegacyEntry, String> {
    const SHAPE: &str = "expected '<LvlTypesId> <LvlPrestDef> <path>'";
    let (type_text, rest) = split_token(line).ok_or(SHAPE)?;
    let (def_text, path_text) = split_token(rest).ok_or(SHAPE)?;
    let lvl_type: u32 = type_text
        .parse()
        .map_err(|_| format!("LvlTypes id {type_text:?} is not a number"))?;
    let lvlprest_def: u32 = def_text
        .parse()
        .map_err(|_| format!("LvlPrest def {def_text:?} is not a number"))?;
    let lower = path_text.trim().replace('\\', "/").to_ascii_lowercase();
    let relative = lower.strip_prefix("./").unwrap_or(&lower);
    let relative = relative.strip_prefix("assets/tiles/").unwrap_or(relative);
    let ds1 = GamePath::from_tile_ref(relative).ok_or("empty path")?;
    if ds1.extension() != Some("ds1") {
        return Err("path is not a .ds1 file".to_owned());
    }
    Ok(LegacyEntry {
        lvl_type,
        lvlprest_def,
        ds1,
    })
}

/// The first whitespace-delimited token and the (non-empty) rest of the line.
fn split_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace)?;
    let (token, rest) = text.split_at(end);
    let rest = rest.trim_start();
    (!token.is_empty() && !rest.is_empty()).then_some((token, rest))
}

// ------------------------------------------------------------------ catalog

/// Everything the editor knows about how DS1 files map to tilesets.
pub struct MapCatalog {
    lvl_types: BTreeMap<u32, LvlType>,
    /// Sorted by (def, row).
    prests: Vec<LvlPrest>,
    prest_by_def: BTreeMap<u32, usize>,
    levels: BTreeMap<u32, Level>,
    /// DS1 path -> (index into `prests`, slot), in `prests` order, one per row.
    by_path: HashMap<GamePath, Vec<(usize, u8)>>,
    /// Directory -> LvlType ids with a File slot in it.
    dir_index: HashMap<String, Vec<u32>>,
    /// Every LvlType DT1 that resolves in the overlay.
    dt1_present: HashSet<GamePath>,
    known: BTreeSet<GamePath>,
    /// Acts from the DS1 headers, read at build time.
    header_act: HashMap<GamePath, u8>,
    /// Non-override interpretations of every known DS1, computed at build.
    infos: HashMap<GamePath, Vec<MapInfo>>,
    overrides: BTreeMap<GamePath, (u32, Option<u32>)>,
}

/// What the cells of one DS1 ask of a tileset.
struct DsEvidence {
    /// (layer kind, key, number of cells).
    cells: Vec<(CellKind, TileKey, u32)>,
    total: u32,
    /// The DT1s named in the DS1's own file list.
    refs: HashSet<GamePath>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CellKind {
    Floor,
    Shadow,
    Wall,
}

impl DsEvidence {
    fn from_ds1(ds1: &Ds1) -> DsEvidence {
        let mut counts: HashMap<(CellKind, TileKey), u32> = HashMap::new();
        let mut add = |kind: CellKind, orientation: u8, cell: Cell| {
            if cell.is_empty() || cell.hidden() {
                return;
            }
            let key = TileKey {
                orientation,
                main: cell.main_index(),
                sub: cell.sub_index(),
            };
            *counts.entry((kind, key)).or_default() += 1;
        };
        for floor in &ds1.floors {
            for &cell in floor.as_slice() {
                add(CellKind::Floor, 0, cell);
            }
        }
        for &cell in ds1.shadow.as_slice() {
            add(CellKind::Shadow, SHADOW_ORIENTATION, cell);
        }
        for (index, layer) in ds1.walls.iter().enumerate() {
            for y in 0..layer.tiles.height() {
                for x in 0..layer.tiles.width() {
                    let (Some(cell), Some(orientation)) =
                        (layer.tiles.get(x, y), ds1.orientation(index, x, y))
                    else {
                        continue;
                    };
                    // Orientation 0 is not a wall orientation.
                    if orientation != 0 {
                        add(CellKind::Wall, orientation, cell);
                    }
                }
            }
        }
        let cells: Vec<(CellKind, TileKey, u32)> = counts
            .into_iter()
            .map(|((k, key), n)| (k, key, n))
            .collect();
        let total = cells.iter().map(|c| c.2).sum();
        let refs = ds1
            .tileset_refs()
            .filter_map(|raw| GamePath::from_tile_ref(&String::from_utf8_lossy(raw)))
            .collect();
        DsEvidence { cells, total, refs }
    }

    /// How many cells resolve in the union of `paths`' key sets.
    fn hits(&self, keys: &HashMap<GamePath, HashSet<TileKey>>, paths: &[GamePath]) -> u32 {
        let sets: Vec<&HashSet<TileKey>> = paths.iter().filter_map(|p| keys.get(p)).collect();
        self.cells
            .iter()
            .filter(|&&(kind, key, _)| sets.iter().any(|set| resolves(set, kind, key)))
            .map(|c| c.2)
            .sum()
    }

    fn coverage(&self, hits: u32) -> Option<f32> {
        (self.total > 0).then(|| hits as f32 / self.total as f32)
    }

    fn overlap(&self, paths: &[GamePath]) -> usize {
        paths.iter().filter(|p| self.refs.contains(*p)).count()
    }
}

/// The tile lookup rule of the reference editor (`ds1.c:122-262`): floors and
/// shadows match exactly; walls with orientation 18/19 also try the other
/// orientation, then that one with sub index 0.
fn resolves(set: &HashSet<TileKey>, kind: CellKind, key: TileKey) -> bool {
    if set.contains(&key) {
        return true;
    }
    if kind != CellKind::Wall {
        return false;
    }
    let other = match key.orientation {
        18 => 19,
        19 => 18,
        _ => return false,
    };
    let swapped = TileKey {
        orientation: other,
        ..key
    };
    set.contains(&swapped) || set.contains(&TileKey { sub: 0, ..swapped })
}

/// DT1 key sets plus one DS1's evidence.
struct Evidence<'a> {
    keys: &'a HashMap<GamePath, HashSet<TileKey>>,
    ds: &'a DsEvidence,
}

/// Table-loading failures that mean "this table is unusable" (reported as a
/// warning) as opposed to a broken archive (an error).
fn soft_failure(error: &AssetError) -> bool {
    matches!(error, AssetError::Profile { .. } | AssetError::Format(_))
}

fn load_typed<T>(
    overlay: &Overlay,
    kind: ExcelTableKind,
    version: GameVersion,
    warnings: &mut Vec<String>,
    parse: fn(&TxtTable) -> d2_formats::Result<Vec<T>>,
) -> Result<Vec<T>> {
    let stem = kind.file_stem();
    match load_excel(overlay, kind, version) {
        Ok(table) => match parse(&table) {
            Ok(rows) => Ok(rows),
            Err(e) => {
                warnings.push(format!("{stem}: {e}; table ignored"));
                Ok(Vec::new())
            }
        },
        Err(e) if soft_failure(&e) => {
            warnings.push(format!("{stem}: {e}; table ignored"));
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

fn parent_dir(path: &GamePath) -> &str {
    path.as_str().rsplit_once('\\').map_or("", |(dir, _)| dir)
}

/// Length of the LvlType name when the LvlPrest name starts with it (text
/// tables only: `Act 1 - Jail W` belongs to `Act 1 - Jail`), else 0. A last
/// tie-break before the lowest id, for LvlTypes that share all their DT1s.
fn name_match(prest_name: &str, type_name: &str) -> usize {
    if type_name.is_empty() {
        return 0;
    }
    let prefix = prest_name.get(..type_name.len());
    if prefix.is_some_and(|p| p.eq_ignore_ascii_case(type_name)) {
        type_name.len()
    } else {
        0
    }
}

fn append_unique(out: &mut Vec<GamePath>, path: GamePath) {
    if !out.contains(&path) {
        out.push(path);
    }
}

/// A ranked LvlType candidate.
struct Guess {
    lvl_type: u32,
    coverage: Option<f32>,
}

impl MapCatalog {
    /// Loads LvlTypes, LvlPrest and Levels with tables::load_excel. `ds1_names`
    /// is the set of DS1 paths to describe (usually KnownNames filtered to .ds1).
    ///
    /// Besides the tables this reads every DS1 in `ds1_names` and every DT1 a
    /// LvlType names (keys only), to measure tile coverage and read DS1
    /// header acts. Missing or malformed tables (and missing columns) become
    /// warnings and leave that table empty; unreadable DS1/DT1 files become
    /// warnings too. Only broken archives are errors.
    pub fn build(
        overlay: &Overlay,
        version: GameVersion,
        ds1_names: &[GamePath],
    ) -> Result<(MapCatalog, Vec<String>)> {
        let mut warnings = Vec::new();

        let types = load_typed(
            overlay,
            ExcelTableKind::LvlTypes,
            version,
            &mut warnings,
            tables::lvl_types,
        )?;
        let mut prests = load_typed(
            overlay,
            ExcelTableKind::LvlPrest,
            version,
            &mut warnings,
            tables::lvl_prests,
        )?;
        let level_rows = load_typed(
            overlay,
            ExcelTableKind::Levels,
            version,
            &mut warnings,
            tables::levels,
        )?;

        let mut lvl_types = BTreeMap::new();
        for lvl_type in types {
            let id = lvl_type.id;
            if lvl_types.insert(id, lvl_type).is_some() {
                warnings.push(format!("lvltypes: duplicate id {id}; the last row wins"));
            }
        }
        let mut levels = BTreeMap::new();
        for level in level_rows {
            levels.entry(level.id).or_insert(level);
        }
        prests.sort_by_key(|p| (p.def, p.row));
        let mut prest_by_def = BTreeMap::new();
        for (index, prest) in prests.iter().enumerate() {
            prest_by_def.entry(prest.def).or_insert(index);
        }

        let mut by_path: HashMap<GamePath, Vec<(usize, u8)>> = HashMap::new();
        for (index, prest) in prests.iter().enumerate() {
            let mut seen = Vec::new();
            for (slot, file) in prest.files.iter().enumerate() {
                let Some(path) = file.as_deref().and_then(GamePath::from_tile_ref) else {
                    continue;
                };
                if seen.contains(&path) {
                    continue;
                }
                seen.push(path.clone());
                let slot = u8::try_from(slot).unwrap_or(u8::MAX);
                by_path.entry(path).or_default().push((index, slot));
            }
        }

        // Every LvlType DT1: does it resolve, and which tile keys does it hold?
        let mut dir_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut dt1_present = HashSet::new();
        let mut dt1_keys: HashMap<GamePath, HashSet<TileKey>> = HashMap::new();
        let mut dt1_checked = HashSet::new();
        for (id, lvl_type) in &lvl_types {
            let paths: Vec<GamePath> = lvl_type
                .files
                .iter()
                .filter_map(|f| f.as_deref().and_then(GamePath::from_tile_ref))
                .collect();
            let mut dirs: BTreeSet<&str> = BTreeSet::new();
            for path in &paths {
                dirs.insert(parent_dir(path));
                if !dt1_checked.insert(path.clone()) {
                    continue;
                }
                let Some(bytes) = overlay.read_bytes(path)? else {
                    continue;
                };
                dt1_present.insert(path.clone());
                match Dt1::parse(&bytes) {
                    Ok(dt1) => {
                        let keys = dt1.tiles.iter().filter_map(|t| t.key()).collect();
                        dt1_keys.insert(path.clone(), keys);
                    }
                    Err(e) => warnings.push(format!("{path}: cannot read the DT1: {e}")),
                }
            }
            for dir in dirs {
                dir_index.entry(dir.to_owned()).or_default().push(*id);
            }
        }

        // Every known DS1: header act and the tiles it asks for.
        let known: BTreeSet<GamePath> = ds1_names.iter().cloned().collect();
        let mut header_act = HashMap::new();
        let mut evidence: HashMap<GamePath, DsEvidence> = HashMap::new();
        for ds1 in &known {
            match overlay.read_bytes(ds1)? {
                Some(bytes) => match Ds1::parse(&bytes) {
                    Ok(parsed) => {
                        header_act.insert(ds1.clone(), parsed.effective_act());
                        evidence.insert(ds1.clone(), DsEvidence::from_ds1(&parsed));
                    }
                    Err(e) => warnings.push(format!("{ds1}: cannot read the DS1: {e}")),
                },
                None => warnings.push(format!("{ds1}: named but not found in the overlay")),
            }
        }

        let mut catalog = MapCatalog {
            lvl_types,
            prests,
            prest_by_def,
            levels,
            by_path,
            dir_index,
            dt1_present,
            known,
            header_act,
            infos: HashMap::new(),
            overrides: BTreeMap::new(),
        };
        let infos: HashMap<GamePath, Vec<MapInfo>> = catalog
            .known
            .iter()
            .map(|ds1| {
                let ev = evidence.get(ds1).map(|ds| Evidence {
                    keys: &dt1_keys,
                    ds,
                });
                (ds1.clone(), catalog.compute_infos(ds1, ev.as_ref()))
            })
            .collect();
        catalog.infos = infos;
        Ok((catalog, warnings))
    }

    /// Every interpretation of this DS1, best first (Override > LvlPrest > PathGuess >
    /// TileCoverage > Ds1Header; among several LvlPrest rows, lowest Def first).
    /// Never empty for a known DS1.
    ///
    /// PathGuess is offered only when no LvlPrest row names the DS1,
    /// TileCoverage only when neither applies, and Ds1Header only when none
    /// of those does (as the enum documents them).
    pub fn lookup(&self, ds1: &GamePath) -> Vec<MapInfo> {
        let mut out = Vec::new();
        if let Some(&(lvl_type, def)) = self.overrides.get(ds1) {
            // A def that no longer exists degrades to "all File slots".
            let resolved = self
                .resolve_explicit(ds1, lvl_type, def)
                .or_else(|e| match def {
                    Some(_) => self.resolve_explicit(ds1, lvl_type, None),
                    None => Err(e),
                });
            out.extend(resolved.ok());
        }
        match self.infos.get(ds1) {
            Some(infos) => out.extend(infos.iter().cloned()),
            // Not a known DS1: no bytes were read, so no coverage evidence.
            None => out.extend(self.compute_infos(ds1, None)),
        }
        out
    }

    pub fn best(&self, ds1: &GamePath) -> Option<MapInfo> {
        self.lookup(ds1).into_iter().next()
    }

    /// The DS1Edit command-line form: an explicit LvlType id and LvlPrest Def.
    ///
    /// Fails when the LvlType (or the Def, when given) is not in the tables.
    /// The result has source [`MapInfoSource::Override`] and no `coverage`.
    pub fn resolve_explicit(
        &self,
        ds1: &GamePath,
        lvl_type: u32,
        lvlprest_def: Option<u32>,
    ) -> Result<MapInfo> {
        if !self.lvl_types.contains_key(&lvl_type) {
            return Err(self.unknown(format!("LvlTypes has no id {lvl_type}")));
        }
        let prest = match lvlprest_def {
            Some(def) => {
                let index = self
                    .prest_by_def
                    .get(&def)
                    .ok_or_else(|| self.unknown(format!("LvlPrest has no def {def}")))?;
                self.prests.get(*index)
            }
            None => None,
        };
        let slot = prest.and_then(|p| Self::slot_of(p, ds1));
        Ok(self.make_info(ds1, MapInfoSource::Override, prest, slot, Some(lvl_type)))
    }

    pub fn set_override(&mut self, ds1: GamePath, lvl_type: u32, lvlprest_def: Option<u32>) {
        self.overrides.insert(ds1, (lvl_type, lvlprest_def));
    }

    pub fn clear_override(&mut self, ds1: &GamePath) {
        self.overrides.remove(ds1);
    }

    /// Applies legacy entries as overrides; returns how many matched a known
    /// DS1. Entries for unknown DS1s are not applied. A later entry for the
    /// same DS1 replaces an earlier one (and counts again).
    pub fn import_legacy(&mut self, entries: &[LegacyEntry]) -> usize {
        let mut matched = 0;
        for entry in entries {
            if self.known.contains(&entry.ds1) {
                self.overrides.insert(
                    entry.ds1.clone(),
                    (entry.lvl_type, Some(entry.lvlprest_def)),
                );
                matched += 1;
            }
        }
        matched
    }

    /// `(ds1, LvlType id, LvlPrest Def)` of every override, sorted by path.
    pub fn overrides(&self) -> Vec<(GamePath, u32, Option<u32>)> {
        self.overrides
            .iter()
            .map(|(ds1, &(lvl_type, def))| (ds1.clone(), lvl_type, def))
            .collect()
    }

    /// Every LvlTypes id, ascending.
    pub fn lvl_type_ids(&self) -> Vec<u32> {
        self.lvl_types.keys().copied().collect()
    }

    // ------------------------------------------------------------ internals

    fn unknown(&self, detail: String) -> AssetError {
        AssetError::Profile {
            profile: "map catalog".to_owned(),
            detail,
        }
    }

    fn slot_of(prest: &LvlPrest, ds1: &GamePath) -> Option<u8> {
        prest
            .files
            .iter()
            .position(|f| f.as_deref().and_then(GamePath::from_tile_ref).as_ref() == Some(ds1))
            .and_then(|slot| u8::try_from(slot).ok())
    }

    /// The DT1s of a LvlType in load order: masked by the row's `Dt1Mask`
    /// when there is one, else every File slot. Duplicates are dropped.
    fn type_paths(lvl_type: &LvlType, prest: Option<&LvlPrest>) -> Vec<GamePath> {
        let names: Vec<&str> = match prest {
            Some(p) => p.dt1_files(lvl_type).collect(),
            None => lvl_type
                .files
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
        };
        let mut out = Vec::new();
        for raw in names {
            if let Some(path) = GamePath::from_tile_ref(raw) {
                append_unique(&mut out, path);
            }
        }
        out
    }

    /// The non-override interpretations of a DS1 (with coverage when the DS1
    /// was read).
    fn compute_infos(&self, ds1: &GamePath, ev: Option<&Evidence<'_>>) -> Vec<MapInfo> {
        let mut out = Vec::new();
        for &(index, slot) in self.by_path.get(ds1).map_or(&[][..], Vec::as_slice) {
            let Some(prest) = self.prests.get(index) else {
                continue;
            };
            let lvl_type = self.prest_type(ds1, prest, ev);
            out.push(self.make_info(
                ds1,
                MapInfoSource::LvlPrest,
                Some(prest),
                Some(slot),
                lvl_type,
            ));
        }
        if out.is_empty() {
            let dir_guess = self.rank(ds1, None, &self.dir_candidates(ds1), ev, true);
            if let Some(guess) = dir_guess {
                out.push(self.make_info(
                    ds1,
                    MapInfoSource::PathGuess,
                    None,
                    None,
                    Some(guess.lvl_type),
                ));
            } else if let Some(guess) = self.coverage_winner(ds1, None, ev) {
                out.push(self.make_info(
                    ds1,
                    MapInfoSource::TileCoverage,
                    None,
                    None,
                    Some(guess.lvl_type),
                ));
            }
        }
        if out.is_empty() && self.known.contains(ds1) {
            out.push(self.make_info(ds1, MapInfoSource::Ds1Header, None, None, None));
        }
        if let Some(ev) = ev {
            for info in &mut out {
                if !info.dt1_files.is_empty() {
                    let hits = ev.ds.hits(ev.keys, &info.dt1_files);
                    info.coverage = ev.ds.coverage(hits);
                }
            }
        }
        out
    }

    /// The LvlType for a LvlPrest row: `Levels(LevelId).LevelType`. Maze
    /// pieces (e.g. every Barracks, Catacombs, Jail row) carry LevelId 0,
    /// whose LvlType 0 has no tilesets. For those, and for a LevelType that
    /// is not in LvlTypes, the LvlType is guessed, with only the File slots
    /// the row's `Dt1Mask` enables counting: first among the LvlTypes with a
    /// slot in the DS1's directory, else the best tile coverage over all
    /// LvlTypes if it reaches [`COVERAGE_ACCEPT`]. `lvlprest_def` and the
    /// mask still come from the row.
    fn prest_type(
        &self,
        ds1: &GamePath,
        prest: &LvlPrest,
        ev: Option<&Evidence<'_>>,
    ) -> Option<u32> {
        let from_level = self.level_of(prest).map(|l| l.level_type);
        let usable = from_level.filter(|id| {
            self.lvl_types
                .get(id)
                .is_some_and(|t| t.files.iter().any(Option::is_some))
        });
        usable
            .or_else(|| {
                self.rank(ds1, Some(prest), &self.dir_candidates(ds1), ev, true)
                    .or_else(|| self.coverage_winner(ds1, Some(prest), ev))
                    .map(|g| g.lvl_type)
            })
            .or(from_level)
    }

    fn level_of(&self, prest: &LvlPrest) -> Option<&Level> {
        u32::try_from(prest.level_id)
            .ok()
            .and_then(|id| self.levels.get(&id))
    }

    fn dir_candidates(&self, ds1: &GamePath) -> Vec<u32> {
        self.dir_index
            .get(parent_dir(ds1))
            .cloned()
            .unwrap_or_default()
    }

    /// The best-covering LvlType over all of them, if it reaches
    /// [`COVERAGE_ACCEPT`] (needs the DS1's evidence).
    fn coverage_winner(
        &self,
        ds1: &GamePath,
        prest: Option<&LvlPrest>,
        ev: Option<&Evidence<'_>>,
    ) -> Option<Guess> {
        let ids: Vec<u32> = self.lvl_types.keys().copied().collect();
        self.rank(ds1, prest, &ids, ev, false)
            .filter(|g| g.coverage.is_some_and(|c| c >= COVERAGE_ACCEPT))
    }

    /// Ranks LvlTypes `ids` for a DS1 by (1) cells resolved by their DT1 set,
    /// (2) overlap with the DS1's own file list, (3) DT1s in the DS1's
    /// directory (and, for a LvlPrest row, fewest outside it), (4) lowest id.
    /// Types without DT1s never win; with `require_dir`, neither do types
    /// with no DT1 in the DS1's directory.
    fn rank(
        &self,
        ds1: &GamePath,
        prest: Option<&LvlPrest>,
        ids: &[u32],
        ev: Option<&Evidence<'_>>,
        require_dir: bool,
    ) -> Option<Guess> {
        let dir = parent_dir(ds1);
        type Score = (u32, usize, usize, Reverse<usize>, usize, Reverse<u32>);
        let mut best: Option<(Score, Guess)> = None;
        for &id in ids {
            let Some(lvl_type) = self.lvl_types.get(&id) else {
                continue;
            };
            let paths = Self::type_paths(lvl_type, prest);
            let inside = paths.iter().filter(|p| parent_dir(p) == dir).count();
            if paths.is_empty() || (require_dir && inside == 0) {
                continue;
            }
            let outside = if prest.is_some() {
                paths.len() - inside
            } else {
                0
            };
            let (hits, overlap, coverage) = match ev {
                Some(ev) => {
                    let hits = ev.ds.hits(ev.keys, &paths);
                    (hits, ev.ds.overlap(&paths), ev.ds.coverage(hits))
                }
                None => (0, 0, None),
            };
            let named = prest.map_or(0, |p| name_match(&p.name, &lvl_type.name));
            let score = (hits, overlap, inside, Reverse(outside), named, Reverse(id));
            if best.as_ref().is_none_or(|(top, _)| score > *top) {
                best = Some((
                    score,
                    Guess {
                        lvl_type: id,
                        coverage,
                    },
                ));
            }
        }
        best.map(|(_, guess)| guess)
    }

    /// Builds one [`MapInfo`] (without coverage).
    fn make_info(
        &self,
        ds1: &GamePath,
        source: MapInfoSource,
        prest: Option<&LvlPrest>,
        slot: Option<u8>,
        lvl_type: Option<u32>,
    ) -> MapInfo {
        let level = prest.and_then(|p| self.level_of(p));
        let type_row = lvl_type.and_then(|id| self.lvl_types.get(&id));
        let act = type_row
            .map(|t| t.act.min(MAX_ACT))
            .or_else(|| level.map(|l| l.act.min(MAX_ACT)))
            .or_else(|| self.header_act.get(ds1).copied())
            .unwrap_or(0);
        let palette_act = match type_row {
            Some(t) if t.id == LVL_TYPE_ACT4_PALETTE => ACT4,
            _ => act,
        };
        let dt1_files = type_row.map_or_else(Vec::new, |t| Self::type_paths(t, prest));
        let missing_dt1 = dt1_files
            .iter()
            .filter(|p| !self.dt1_present.contains(*p))
            .cloned()
            .collect();
        MapInfo {
            ds1: ds1.clone(),
            source,
            lvlprest_def: prest.map(|p| p.def),
            lvlprest_slot: slot,
            level_id: level.map(|l| l.id),
            level_name: level.filter(|l| !l.name.is_empty()).map(|l| l.name.clone()),
            lvl_type: type_row.map(|t| t.id),
            act,
            palette_act,
            dt1_files,
            missing_dt1,
            coverage: None,
        }
    }
}
