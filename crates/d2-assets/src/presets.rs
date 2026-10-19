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
//! [`MapCatalog::build`] does all the I/O: it loads the three tables, checks
//! which DT1s resolve, and reads the header of every DS1 whose act no table
//! supplies (the [`MapInfoSource::Ds1Header`] fallback) to learn its act.
//! Lookups afterwards are pure.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use d2_formats::ds1::Ds1;
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
    /// Nothing matched: act from the DS1 header, no DT1s.
    Ds1Header,
}

/// One interpretation of a DS1.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Directory -> (LvlType id, number of its File slots in that directory).
    dir_index: HashMap<String, Vec<(u32, usize)>>,
    /// Every LvlType DT1 that resolves in the overlay.
    dt1_present: HashSet<GamePath>,
    known: BTreeSet<GamePath>,
    /// Header acts, read at build time for DS1s whose act no table gives.
    header_act: HashMap<GamePath, u8>,
    /// Subtracted from `LvlType.act` to make it 0-based (see `act_shift_probe`).
    act_shift: u8,
    overrides: BTreeMap<GamePath, (u32, Option<u32>)>,
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

// TEMPORARY ADAPTER (delete this function, the `act_shift` field and its one
// use in `type_act` once `d2_formats::tables::LvlType.act` is 0-based
// everywhere): PLAN.md says LvlType.act is 0-based, but when this was written
// d2-formats still returned LvlTypes.txt's raw 1-based value. Instead of
// guessing, ask d2-formats what it does with an `Act` cell of 1.
fn act_shift_probe() -> u8 {
    let probe = TxtTable::from_parts(
        vec!["Id".to_owned(), "Act".to_owned()],
        vec![vec!["0".to_owned(), "1".to_owned()]],
    );
    match tables::lvl_types(&probe)
        .ok()
        .and_then(|v| v.first().map(|t| t.act))
    {
        Some(1) => 1, // still 1-based: shift down
        _ => 0,
    }
}

fn parent_dir(path: &GamePath) -> &str {
    path.as_str().rsplit_once('\\').map_or("", |(dir, _)| dir)
}

fn append_unique(out: &mut Vec<GamePath>, path: GamePath) {
    if !out.contains(&path) {
        out.push(path);
    }
}

impl MapCatalog {
    /// Loads LvlTypes, LvlPrest and Levels with tables::load_excel. `ds1_names`
    /// is the set of DS1 paths to describe (usually KnownNames filtered to .ds1).
    ///
    /// Missing or malformed tables (and missing columns) become warnings and
    /// leave that table empty; only broken archives are errors.
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

        let mut dir_index: HashMap<String, Vec<(u32, usize)>> = HashMap::new();
        let mut dt1_present = HashSet::new();
        let mut dt1_checked = HashSet::new();
        for (id, lvl_type) in &lvl_types {
            let paths: Vec<GamePath> = lvl_type
                .files
                .iter()
                .filter_map(|f| f.as_deref().and_then(GamePath::from_tile_ref))
                .collect();
            let mut per_dir: BTreeMap<&str, usize> = BTreeMap::new();
            for path in &paths {
                *per_dir.entry(parent_dir(path)).or_default() += 1;
                if dt1_checked.insert(path.clone()) && overlay.resolve(path).is_some() {
                    dt1_present.insert(path.clone());
                }
            }
            for (dir, count) in per_dir {
                dir_index
                    .entry(dir.to_owned())
                    .or_default()
                    .push((*id, count));
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
            known: ds1_names.iter().cloned().collect(),
            header_act: HashMap::new(),
            act_shift: act_shift_probe(),
            overrides: BTreeMap::new(),
        };

        // The DS1 header is only opened for maps whose act no table gives.
        let needs_header: Vec<GamePath> = catalog
            .known
            .iter()
            .filter(|ds1| catalog.table_infos(ds1).iter().any(|(_, needs)| *needs))
            .cloned()
            .collect();
        for ds1 in needs_header {
            match overlay.read_bytes(&ds1)? {
                Some(bytes) => match Ds1::parse(&bytes) {
                    Ok(parsed) => {
                        catalog.header_act.insert(ds1, parsed.effective_act());
                    }
                    Err(e) => warnings.push(format!("{ds1}: cannot read the DS1 header: {e}")),
                },
                None => warnings.push(format!("{ds1}: named but not found in the overlay")),
            }
        }
        Ok((catalog, warnings))
    }

    /// Every interpretation of this DS1, best first (Override > LvlPrest > PathGuess > Ds1Header;
    /// among several LvlPrest rows, lowest Def first). Never empty for a known DS1.
    ///
    /// PathGuess is offered only when no LvlPrest row names the DS1, and
    /// Ds1Header only when neither applies (as the enum documents them).
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
        out.extend(self.table_infos(ds1).into_iter().map(|(info, _)| info));
        out
    }

    pub fn best(&self, ds1: &GamePath) -> Option<MapInfo> {
        self.lookup(ds1).into_iter().next()
    }

    /// The DS1Edit command-line form: an explicit LvlType id and LvlPrest Def.
    ///
    /// Fails when the LvlType (or the Def, when given) is not in the tables.
    /// The result has source [`MapInfoSource::Override`].
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
        let (info, _) = self.make_info(ds1, MapInfoSource::Override, prest, slot, Some(lvl_type));
        Ok(info)
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

    /// 0-based act of a LvlType row.
    fn type_act(&self, lvl_type: &LvlType) -> u8 {
        lvl_type.act.saturating_sub(self.act_shift).min(MAX_ACT)
    }

    /// The non-override interpretations, each with "act came from the DS1 header".
    fn table_infos(&self, ds1: &GamePath) -> Vec<(MapInfo, bool)> {
        let mut out = Vec::new();
        for &(index, slot) in self.by_path.get(ds1).map_or(&[][..], Vec::as_slice) {
            let Some(prest) = self.prests.get(index) else {
                continue;
            };
            let lvl_type = self.prest_type(ds1, prest);
            out.push(self.make_info(
                ds1,
                MapInfoSource::LvlPrest,
                Some(prest),
                Some(slot),
                lvl_type,
            ));
        }
        if out.is_empty()
            && let Some(id) = self.guess_type(ds1)
        {
            out.push(self.make_info(ds1, MapInfoSource::PathGuess, None, None, Some(id)));
        }
        if out.is_empty() && self.known.contains(ds1) {
            out.push(self.make_info(ds1, MapInfoSource::Ds1Header, None, None, None));
        }
        out
    }

    /// The LvlType for a LvlPrest row: `Levels(LevelId).LevelType`, as the
    /// contract says. Maze pieces (e.g. every Barracks, Catacombs, Jail row)
    /// carry LevelId 0, whose LvlType 0 has no tilesets; for those, and for a
    /// LevelType that is not in LvlTypes, the LvlType is guessed from the DS1's
    /// directory like [`MapInfoSource::PathGuess`] does, except that only the
    /// File slots the row's `Dt1Mask` enables count (`lvlprest_def` and the
    /// mask still come from the row).
    fn prest_type(&self, ds1: &GamePath, prest: &LvlPrest) -> Option<u32> {
        let from_level = self.level_of(prest).map(|l| l.level_type);
        let usable = from_level.filter(|id| {
            self.lvl_types
                .get(id)
                .is_some_and(|t| t.files.iter().any(Option::is_some))
        });
        usable
            .or_else(|| self.guess_masked_type(ds1, prest))
            .or(from_level)
    }

    /// Among the LvlTypes with a File slot in the DS1's directory, the one
    /// whose mask-enabled files are most in that directory (then fewest
    /// outside it, then lowest id).
    fn guess_masked_type(&self, ds1: &GamePath, prest: &LvlPrest) -> Option<u32> {
        let dir = parent_dir(ds1);
        self.dir_index
            .get(dir)?
            .iter()
            .filter_map(|&(id, _)| {
                let lvl_type = self.lvl_types.get(&id)?;
                let (mut inside, mut outside) = (0usize, 0usize);
                for path in prest
                    .dt1_files(lvl_type)
                    .filter_map(GamePath::from_tile_ref)
                {
                    if parent_dir(&path) == dir {
                        inside += 1;
                    } else {
                        outside += 1;
                    }
                }
                (inside > 0).then_some((inside, std::cmp::Reverse(outside), std::cmp::Reverse(id)))
            })
            .max()
            .map(|(_, _, std::cmp::Reverse(id))| id)
            .or_else(|| self.guess_type(ds1))
    }

    fn level_of(&self, prest: &LvlPrest) -> Option<&Level> {
        u32::try_from(prest.level_id)
            .ok()
            .and_then(|id| self.levels.get(&id))
    }

    /// The LvlType sharing the most File slots with the DS1's directory.
    fn guess_type(&self, ds1: &GamePath) -> Option<u32> {
        let candidates = self.dir_index.get(parent_dir(ds1))?;
        candidates
            .iter()
            .max_by_key(|&&(id, shared)| (shared, std::cmp::Reverse(id)))
            .map(|&(id, _)| id)
    }

    /// Builds one [`MapInfo`]; the flag says the act had to come from the DS1 header.
    fn make_info(
        &self,
        ds1: &GamePath,
        source: MapInfoSource,
        prest: Option<&LvlPrest>,
        slot: Option<u8>,
        lvl_type: Option<u32>,
    ) -> (MapInfo, bool) {
        let level = prest.and_then(|p| self.level_of(p));
        let type_row = lvl_type.and_then(|id| self.lvl_types.get(&id));
        let table_act = type_row
            .map(|t| self.type_act(t))
            .or_else(|| level.map(|l| l.act.min(MAX_ACT)));
        let from_header = table_act.is_none();
        let act = table_act
            .or_else(|| self.header_act.get(ds1).copied())
            .unwrap_or(0);
        let palette_act = match type_row {
            Some(t) if t.id == LVL_TYPE_ACT4_PALETTE => ACT4,
            _ => act,
        };

        let mut dt1_files = Vec::new();
        if let Some(t) = type_row {
            let names: Vec<&str> = match prest {
                Some(p) => p.dt1_files(t).collect(),
                None => t.files.iter().flatten().map(String::as_str).collect(),
            };
            for raw in names {
                if let Some(path) = GamePath::from_tile_ref(raw) {
                    append_unique(&mut dt1_files, path);
                }
            }
        }
        let missing_dt1 = dt1_files
            .iter()
            .filter(|p| !self.dt1_present.contains(*p))
            .cloned()
            .collect();

        let info = MapInfo {
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
        };
        (info, from_header)
    }
}
