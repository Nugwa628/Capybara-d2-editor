//! `Index::build`: list the profile's files, decode what the index needs,
//! write everything in one transaction (cancelling leaves the old rows).

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use d2_assets::tables::load_excel;
use d2_assets::{GamePath, GameVersion, KnownNames, LayerId, MapCatalog, Overlay, Profile};
use d2_formats::ds1::Ds1;
use d2_formats::excel::ExcelTableKind;
use d2_formats::tables::{self, LvlType};
use d2_formats::txt::TxtTable;
use rusqlite::{OptionalExtension, Transaction, params};

use crate::error::{IndexError, Result};
use crate::index::{BuildPhase, BuildProgress, BuildReport, Index, fingerprint_text};
use crate::naming::{generated_file_name, place_from_path};
use crate::par::par_map;
use crate::query::AssetKind;
use crate::tiles::{
    self, Ds1Usage, Dt1Input, TileBatch, TileCtx, TileKeys, TileRecord, TileRun, UsedInRow,
    resolve_used_in,
};

/// File extensions that get a row in `files`.
const FILE_KINDS: &[&str] = &[
    "ds1", "dt1", "dc6", "dcc", "cof", "tbl", "txt", "bin", "dat", "pl2",
];

/// Archives that ship with the game; anything else above them is a mod.
const BASE_ARCHIVES: &[&str] = &[
    "d2data.mpq",
    "d2exp.mpq",
    "d2char.mpq",
    "patch_d2.mpq",
    "d2music.mpq",
    "d2sfx.mpq",
    "d2speech.mpq",
    "d2video.mpq",
    "d2xmusic.mpq",
    "d2xtalk.mpq",
    "d2xvideo.mpq",
];

const MAX_WARNINGS: usize = 500;

/// `saves` for the saves folder, `base` for the game's own archives, `mod`
/// for everything else.
pub(crate) fn src_tag(source_name: &str) -> &'static str {
    let lower = source_name.to_ascii_lowercase();
    if lower.starts_with("saves\\") || lower.starts_with("saves/") || lower == "saves" {
        "saves"
    } else if BASE_ARCHIVES.contains(&lower.as_str()) {
        "base"
    } else {
        "mod"
    }
}

/// The act (0-based) a path names: an `act1`..`act5` folder, or `expansion`
/// (Act 5).
pub(crate) fn act_from_path(path: &str) -> Option<u8> {
    for segment in path.split(['\\', '/']) {
        if let Some(Ok(n @ 1..=5)) = segment.strip_prefix("act").map(str::parse::<u8>) {
            return Some(n - 1);
        }
        if segment == "expansion" {
            return Some(4);
        }
    }
    None
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

// ------------------------------------------------------------- reporting

struct Reporter<'a> {
    callback: &'a mut dyn FnMut(&BuildProgress),
    cancel: &'a AtomicBool,
    last: Instant,
    phase: BuildPhase,
}

impl<'a> Reporter<'a> {
    fn new(callback: &'a mut dyn FnMut(&BuildProgress), cancel: &'a AtomicBool) -> Self {
        Reporter {
            callback,
            cancel,
            last: Instant::now(),
            phase: BuildPhase::Listing,
        }
    }

    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(IndexError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Start a phase: always reports.
    fn begin(&mut self, phase: BuildPhase, total: u64, current: &str) -> Result<()> {
        self.check()?;
        self.phase = phase;
        self.last = Instant::now();
        (self.callback)(&BuildProgress {
            phase,
            done: 0,
            total,
            current: current.to_owned(),
        });
        Ok(())
    }

    /// Progress inside a phase: reports at most every 200 ms.
    fn tick(&mut self, done: u64, total: u64, current: &str) -> Result<()> {
        self.check()?;
        if self.last.elapsed().as_millis() >= 200 {
            self.last = Instant::now();
            (self.callback)(&BuildProgress {
                phase: self.phase,
                done,
                total,
                current: current.to_owned(),
            });
        }
        Ok(())
    }
}

// ----------------------------------------------------------------- tables

/// LvlTypes with the DT1 lookups the index needs (place names, acts, links).
/// Which DT1s a *map* loads is `MapCatalog`'s job.
#[derive(Default)]
struct LevelTables {
    lvl_types: Vec<LvlType>,
    lvl_type_by_id: HashMap<u32, usize>,
    /// DT1 path to `(lvltype id, slot)`, lowest lvltype first.
    dt1_links: HashMap<GamePath, Vec<(u32, u32)>>,
}

fn load_table<T>(
    overlay: &Overlay,
    kind: ExcelTableKind,
    version: GameVersion,
    parse: fn(&TxtTable) -> d2_formats::Result<Vec<T>>,
    warnings: &mut Vec<String>,
) -> Vec<T> {
    let stem = kind.file_stem();
    match load_excel(overlay, kind, version) {
        Ok(table) => match parse(&table) {
            Ok(rows) => rows,
            Err(e) => {
                warnings.push(format!("{stem}: {e}"));
                Vec::new()
            }
        },
        Err(e) => {
            warnings.push(format!("{stem}: {e}"));
            Vec::new()
        }
    }
}

/// LvlTypes id whose tilesets are Act 4 art although the row says Act 5.
const LVL_TYPE_ACT4_PALETTE: u32 = 35;

impl LevelTables {
    fn load(overlay: &Overlay, version: GameVersion, warnings: &mut Vec<String>) -> LevelTables {
        let lvl_types = load_table(
            overlay,
            ExcelTableKind::LvlTypes,
            version,
            tables::lvl_types,
            warnings,
        );
        LevelTables::new(lvl_types)
    }

    fn new(mut lvl_types: Vec<LvlType>) -> LevelTables {
        lvl_types.sort_by_key(|l| (l.id, l.row));
        let mut lvl_type_by_id = HashMap::new();
        for (i, lt) in lvl_types.iter().enumerate() {
            lvl_type_by_id.entry(lt.id).or_insert(i);
        }
        let mut dt1_links: HashMap<GamePath, Vec<(u32, u32)>> = HashMap::new();
        for lt in &lvl_types {
            for (slot, file) in lt.files.iter().enumerate() {
                let Some(path) = file.as_deref().and_then(GamePath::from_tile_ref) else {
                    continue;
                };
                if let Ok(slot) = u32::try_from(slot) {
                    dt1_links.entry(path).or_default().push((lt.id, slot));
                }
            }
        }
        LevelTables {
            lvl_types,
            lvl_type_by_id,
            dt1_links,
        }
    }

    fn lowest_lvl_type(&self, path: &GamePath) -> Option<&LvlType> {
        let (id, _) = self.dt1_links.get(path)?.first()?;
        self.lvl_type_by_id
            .get(id)
            .and_then(|&i| self.lvl_types.get(i))
    }

    /// Place name and act of a DT1 from the lowest-id LvlType that loads it.
    fn dt1_context(&self, path: &GamePath) -> (Option<String>, Option<u8>) {
        match self.lowest_lvl_type(path) {
            Some(lt) => (
                Some(lt.name.trim().to_owned()).filter(|n| !n.is_empty()),
                Some(lt.act),
            ),
            None => (None, None),
        }
    }

    /// Palette act from the lowest-id LvlType that loads the DT1 (the
    /// reference rule for LvlTypes 35), for DT1s no map uses.
    fn dt1_palette_act(&self, path: &GamePath) -> Option<u8> {
        self.lowest_lvl_type(path).map(|lt| {
            if lt.id == LVL_TYPE_ACT4_PALETTE {
                3
            } else {
                lt.act.min(4)
            }
        })
    }
}

// ------------------------------------------------------------------ files

#[derive(Clone)]
struct Entry {
    path: GamePath,
    ext: &'static str,
    layer: LayerId,
    source: String,
    src_tag: &'static str,
}

struct FileInfo {
    size: u64,
    hash: [u8; 32],
}

struct Ds1Info {
    width: u32,
    height: u32,
    act: u8,
    level_name: Option<String>,
    lvl_type: Option<u32>,
    palette_act: u8,
    usage: Ds1Usage,
}

struct Processed {
    file: Option<FileInfo>,
    ds1: Option<Ds1Info>,
    warning: Option<String>,
}

fn ds1_keys(ds1: &Ds1) -> Vec<((u8, u8, u8), u32)> {
    let mut counts: HashMap<(u8, u8, u8), u32> = HashMap::new();
    let mut add = |key: (u8, u8, u8)| {
        let n = counts.entry(key).or_insert(0);
        *n = n.saturating_add(1);
    };
    for floor in &ds1.floors {
        for cell in floor.as_slice() {
            if !cell.is_empty() {
                add((0, cell.main_index(), cell.sub_index()));
            }
        }
    }
    for cell in ds1.shadow.as_slice() {
        if !cell.is_empty() {
            add((13, cell.main_index(), cell.sub_index()));
        }
    }
    for (wall, layer) in ds1.walls.iter().enumerate() {
        let width = layer.tiles.width();
        if width == 0 {
            continue;
        }
        for (i, cell) in layer.tiles.as_slice().iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            let Ok(i) = u32::try_from(i) else {
                continue;
            };
            let Some(orientation) = ds1.orientation(wall, i % width, i / width) else {
                continue;
            };
            add((orientation, cell.main_index(), cell.sub_index()));
        }
    }
    let mut keys: Vec<_> = counts.into_iter().collect();
    keys.sort();
    keys
}

fn process_entry(overlay: &Overlay, entry: &Entry, catalog: &MapCatalog) -> Processed {
    let name = entry.path.as_str();
    let bytes = match overlay.read_from(entry.layer, &entry.path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return Processed {
                file: None,
                ds1: None,
                warning: Some(format!("{name}: listed but unreadable (not in its layer)")),
            };
        }
        Err(e) => {
            return Processed {
                file: None,
                ds1: None,
                warning: Some(format!("{name}: {e}")),
            };
        }
    };
    let file = FileInfo {
        size: bytes.len() as u64,
        hash: *blake3::hash(&bytes).as_bytes(),
    };
    if entry.ext != "ds1" {
        return Processed {
            file: Some(file),
            ds1: None,
            warning: None,
        };
    }
    match Ds1::parse_with_report(&bytes) {
        Ok((ds1, _report)) => {
            let header_act = ds1.effective_act();
            let (act, palette_act, level_name, lvl_type, dt1_order) =
                match catalog.best(&entry.path) {
                    Some(info) => (
                        info.act,
                        info.palette_act,
                        info.level_name,
                        info.lvl_type,
                        info.dt1_files,
                    ),
                    None => (header_act, header_act, None, None, Vec::new()),
                };
            Processed {
                file: Some(file),
                ds1: Some(Ds1Info {
                    width: ds1.width,
                    height: ds1.height,
                    act,
                    level_name,
                    lvl_type,
                    palette_act,
                    usage: Ds1Usage {
                        path: name.to_owned(),
                        dt1_order,
                        keys: ds1_keys(&ds1),
                    },
                }),
                warning: None,
            }
        }
        Err(e) => Processed {
            file: Some(file),
            ds1: None,
            warning: Some(format!("{name}: {e}")),
        },
    }
}

// ---------------------------------------------------------------- writing

struct NewAsset {
    key: String,
    kind: AssetKind,
    path: String,
    act: Option<u8>,
    width: Option<u32>,
    height: Option<u32>,
    /// Sprites: number of directions.
    directions: Option<u32>,
    /// Sprites: frames per direction.
    frames: Option<u32>,
    /// COF: `"<n> layers"`.
    class: Option<String>,
    gen_name: String,
    src: String,
    src_tag: &'static str,
    thumb: Option<Vec<u8>>,
}

impl NewAsset {
    fn file(entry: &Entry, kind: AssetKind) -> NewAsset {
        let path = entry.path.as_str().to_owned();
        NewAsset {
            key: path.clone(),
            kind,
            gen_name: generated_file_name(&path, kind),
            path,
            act: None,
            width: None,
            height: None,
            directions: None,
            frames: None,
            class: None,
            src: entry.source.clone(),
            src_tag: entry.src_tag,
            thumb: None,
        }
    }

    /// A DS1 asset; `info` is `None` when the file did not parse.
    fn ds1(entry: &Entry, info: Option<&Ds1Info>) -> NewAsset {
        let mut asset = NewAsset::file(entry, AssetKind::Ds1);
        match info {
            Some(info) => {
                asset.act = Some(info.act);
                asset.width = Some(info.width);
                asset.height = Some(info.height);
                if let Some(level) = &info.level_name {
                    asset.gen_name = format!("{} · {level}", asset.gen_name);
                }
            }
            None => asset.act = act_from_path(entry.path.as_str()),
        }
        asset
    }
}

fn insert_asset(tx: &Transaction<'_>, profile: &str, a: &NewAsset) -> rusqlite::Result<i64> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO assets(profile, key, kind, path, act, width, height, directions, frames, class,
                            gen_name, src, src_tag, thumb)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?;
    stmt.execute(params![
        profile,
        a.key,
        a.kind.as_str(),
        a.path,
        a.act,
        a.width,
        a.height,
        a.directions,
        a.frames,
        a.class,
        a.gen_name,
        a.src,
        a.src_tag,
        a.thumb
    ])?;
    Ok(tx.last_insert_rowid())
}

fn insert_file_row(
    tx: &Transaction<'_>,
    profile: &str,
    entry: &Entry,
    info: &FileInfo,
) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT OR REPLACE INTO files(profile, path, kind, layer, source, size, hash)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    stmt.execute(params![
        profile,
        entry.path.as_str(),
        entry.ext,
        i64::try_from(entry.layer.0).unwrap_or(0),
        entry.source,
        i64::try_from(info.size).unwrap_or(i64::MAX),
        info.hash.as_slice()
    ])?;
    Ok(())
}

struct TileOwner<'a> {
    act: Option<u8>,
    src: &'a str,
    src_tag: &'static str,
}

fn insert_tile(
    tx: &Transaction<'_>,
    profile: &str,
    dt1_path: &str,
    owner: &TileOwner<'_>,
    t: &TileRecord,
) -> rusqlite::Result<i64> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO assets(profile, key, kind, path, act, orientation, main, sub, variant, rarity,
                            width, height, class, gen_name, color, color_rgb, texture, transparency,
                            phash, phash_mirror, src, src_tag, thumb)
         VALUES(?1, ?2, 'tile', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22)",
    )?;
    stmt.execute(params![
        profile,
        t.key,
        dt1_path,
        owner.act,
        t.orientation,
        t.main,
        t.sub,
        t.variant,
        t.rarity,
        t.width,
        t.height,
        t.class,
        t.gen_name,
        t.color,
        t.color_rgb,
        t.texture,
        f64::from(t.transparency),
        t.phash,
        t.phash_mirror,
        owner.src,
        owner.src_tag,
        t.thumb
    ])?;
    Ok(tx.last_insert_rowid())
}

fn push_warning(warnings: &mut Vec<String>, overflow: &mut usize, text: String) {
    if warnings.len() < MAX_WARNINGS {
        warnings.push(text);
    } else {
        *overflow += 1;
    }
}

/// Sprite files per parallel batch.
const SPRITE_CHUNK: usize = 256;

fn list_entries(overlay: &Overlay, known: &KnownNames) -> Vec<Entry> {
    let layers = overlay.layers();
    let mut entries = Vec::new();
    for path in known.iter() {
        let Some(ext) = path
            .extension()
            .and_then(|e| FILE_KINDS.iter().copied().find(|k| *k == e))
        else {
            continue;
        };
        let Some(layer) = overlay.resolve(path) else {
            continue;
        };
        let Some(source) = layers.get(layer.0).map(|l| l.name().to_owned()) else {
            continue;
        };
        entries.push(Entry {
            path: path.clone(),
            ext,
            layer,
            src_tag: src_tag(&source),
            source,
        });
    }
    entries
}

impl Index {
    /// Full rebuild of one profile (its old rows are replaced atomically).
    ///
    /// Runs on the calling thread, with a few helper threads for reading and
    /// rendering. `progress` is called at least every 250 ms; `cancel` is
    /// checked between items and ends the build with
    /// [`IndexError::Cancelled`], leaving the previous index untouched.
    pub fn build(
        &mut self,
        profile: &Profile,
        overlay: &Overlay,
        progress: &mut dyn FnMut(&BuildProgress),
        cancel: &AtomicBool,
    ) -> Result<BuildReport> {
        self.build_with(profile, overlay, progress, cancel, tiles::run)
    }

    pub(crate) fn build_with(
        &mut self,
        profile: &Profile,
        overlay: &Overlay,
        progress: &mut dyn FnMut(&BuildProgress),
        cancel: &AtomicBool,
        run_tiles: TileRun,
    ) -> Result<BuildReport> {
        let started = Instant::now();
        let mut rep = Reporter::new(progress, cancel);
        let mut warnings: Vec<String> = Vec::new();
        let mut overflow = 0usize;
        let profile_id = profile.id.as_str();
        let db_path = self.path.clone();
        let sq = |e: rusqlite::Error| IndexError::sql(&db_path, e);

        // The fingerprint is taken first, so a change during the build makes
        // the profile stale again rather than going unnoticed.
        let fingerprint = fingerprint_text(overlay)?;

        // ---- Listing
        rep.begin(BuildPhase::Listing, 0, "collecting file names")?;
        let (known, stats) = KnownNames::harvest(overlay, profile.game_version)?;
        for w in stats.warnings {
            push_warning(&mut warnings, &mut overflow, w);
        }
        rep.check()?;
        let entries = list_entries(overlay, &known);

        // ---- Tables
        rep.begin(BuildPhase::Tables, 3, "LvlTypes, LvlPrest, Levels")?;
        let mut table_warnings = Vec::new();
        let level_tables = LevelTables::load(overlay, profile.game_version, &mut table_warnings);
        let ds1_paths: Vec<GamePath> = entries
            .iter()
            .filter(|e| e.ext == "ds1")
            .map(|e| e.path.clone())
            .collect();
        let (catalog, catalog_warnings) =
            MapCatalog::build(overlay, profile.game_version, &ds1_paths)?;
        table_warnings.extend(catalog_warnings);
        for w in table_warnings {
            push_warning(&mut warnings, &mut overflow, w);
        }
        rep.check()?;

        // ---- Everything that is not a map: size and hash.
        let others: Vec<&Entry> = entries.iter().filter(|e| e.ext != "ds1").collect();
        let total = others.len() as u64;
        rep.begin(BuildPhase::Listing, total, "hashing files")?;
        let other_results = par_map(
            &others,
            cancel,
            |e| process_entry(overlay, e, &catalog),
            |d| rep.tick(d as u64, total, "hashing files"),
        )?;

        // ---- Maps
        let maps: Vec<&Entry> = entries.iter().filter(|e| e.ext == "ds1").collect();
        let total = maps.len() as u64;
        rep.begin(BuildPhase::Maps, total, "reading maps")?;
        let map_results = par_map(
            &maps,
            cancel,
            |e| process_entry(overlay, e, &catalog),
            |d| rep.tick(d as u64, total, "reading maps"),
        )?;

        // The palette of a DT1 is the one of the lowest-id LvlType among the
        // maps that load it (MapCatalog applies the Level `Pal` rule).
        let mut dt1_palette: HashMap<GamePath, (u32, u8)> = HashMap::new();
        for info in map_results.iter().filter_map(|r| r.ds1.as_ref()) {
            let Some(lvl_type) = info.lvl_type else {
                continue;
            };
            for dt1 in &info.usage.dt1_order {
                let slot = dt1_palette
                    .entry(dt1.clone())
                    .or_insert((lvl_type, info.palette_act));
                if lvl_type < slot.0 {
                    *slot = (lvl_type, info.palette_act);
                }
            }
        }

        // ---- Assemble rows in memory.
        let mut file_rows: Vec<(&Entry, FileInfo)> = Vec::new();
        let mut assets: Vec<NewAsset> = Vec::new();
        let mut dt1_inputs: Vec<Dt1Input> = Vec::new();
        let mut dt1_links: Vec<(String, Vec<(u32, u32)>)> = Vec::new();
        let mut ds1_usages: Vec<Ds1Usage> = Vec::new();

        for (entry, result) in others
            .iter()
            .copied()
            .zip(other_results)
            .chain(maps.iter().copied().zip(map_results))
        {
            if let Some(w) = result.warning {
                push_warning(&mut warnings, &mut overflow, w);
            }
            let path = entry.path.as_str().to_owned();
            if let Some(info) = result.file {
                file_rows.push((entry, info));
            } else {
                continue;
            }
            match entry.ext {
                "ds1" => {
                    assets.push(NewAsset::ds1(entry, result.ds1.as_ref()));
                    if let Some(info) = result.ds1 {
                        ds1_usages.push(info.usage);
                    }
                }
                "dt1" => {
                    let (name, lvl_act) = level_tables.dt1_context(&entry.path);
                    let mut asset = NewAsset::file(entry, AssetKind::Dt1);
                    asset.act = lvl_act.or_else(|| act_from_path(&path));
                    if let Some(links) = level_tables.dt1_links.get(&entry.path) {
                        dt1_links.push((path.clone(), links.clone()));
                    }
                    dt1_inputs.push(Dt1Input {
                        path: entry.path.clone(),
                        layer: entry.layer,
                        place: name.unwrap_or_else(|| place_from_path(&path)),
                        act: dt1_palette
                            .get(&entry.path)
                            .map(|p| p.1)
                            .or_else(|| level_tables.dt1_palette_act(&entry.path))
                            .or(asset.act),
                    });
                    assets.push(asset);
                }
                _ => {}
            }
        }
        assets.sort_by(|a, b| a.key.cmp(&b.key));
        let sprite_entries: Vec<&Entry> = file_rows
            .iter()
            .map(|(e, _)| *e)
            .filter(|e| matches!(e.ext, "dc6" | "dcc" | "cof"))
            .collect();

        // ---- Writing
        rep.begin(BuildPhase::Writing, 0, "writing index")?;
        let tx = self.conn.transaction().map_err(sq)?;
        tx.execute_batch(&format!(
            "INSERT INTO assets_fts(assets_fts, rowid, gen_name, path, class, color, texture)
               SELECT 'delete', id, gen_name, path, class, color, texture FROM assets WHERE profile = {q};
             DELETE FROM used_in WHERE asset_id IN (SELECT id FROM assets WHERE profile = {q});
             DELETE FROM asset_lvltypes WHERE asset_id IN (SELECT id FROM assets WHERE profile = {q});
             DELETE FROM assets WHERE profile = {q};
             DELETE FROM files WHERE profile = {q};
             DELETE FROM profiles WHERE id = {q};",
            q = sql_quote(profile_id)
        ))
        .map_err(sq)?;

        let mut counter = 0u64;
        for (entry, info) in &file_rows {
            insert_file_row(&tx, profile_id, entry, info).map_err(sq)?;
            counter += 1;
            if counter.is_multiple_of(512) {
                rep.tick(counter, file_rows.len() as u64, "files")?;
            }
        }

        let mut dt1_ids: HashMap<String, i64> = HashMap::new();
        let mut owners: HashMap<String, (Option<u8>, String, &'static str)> = HashMap::new();
        for a in &assets {
            let id = insert_asset(&tx, profile_id, a).map_err(sq)?;
            if a.kind == AssetKind::Dt1 {
                dt1_ids.insert(a.path.clone(), id);
                owners.insert(a.path.clone(), (a.act, a.src.clone(), a.src_tag));
            }
            counter += 1;
            if counter.is_multiple_of(512) {
                rep.tick(counter, (file_rows.len() + assets.len()) as u64, "assets")?;
            }
        }
        for (path, links) in &dt1_links {
            let Some(&id) = dt1_ids.get(path) else {
                continue;
            };
            for (lvltype, slot) in links {
                tx.prepare_cached(
                    "INSERT INTO asset_lvltypes(asset_id, lvltype, slot) VALUES(?1, ?2, ?3)",
                )
                .and_then(|mut s| s.execute(params![id, lvltype, slot]))
                .map_err(sq)?;
            }
        }
        let mut report = BuildReport {
            files: file_rows.len() as u64,
            assets: assets.len() as u64,
            ..BuildReport::default()
        };

        // ---- Tiles (streamed into the same transaction)
        rep.begin(BuildPhase::Tiles, dt1_inputs.len() as u64, "")?;
        let mut tile_ids: HashMap<String, i64> = HashMap::new();
        let mut tile_warnings: Vec<String> = Vec::new();
        let mut tile_count = 0u64;
        let used_in: Vec<UsedInRow> = {
            let ctx = TileCtx {
                overlay,
                dt1s: &dt1_inputs,
                ds1s: &ds1_usages,
                cancel,
            };
            let mut on_progress =
                |done: u64, total: u64, current: &str| rep.tick(done, total, current);
            let mut sink = |batch: TileBatch| -> Result<()> {
                tile_warnings.extend(batch.warnings);
                let (act, src, tag) = owners.get(batch.dt1.as_str()).map_or_else(
                    || (None, String::new(), "mod"),
                    |(a, s, t)| (*a, s.clone(), *t),
                );
                let owner = TileOwner {
                    act,
                    src: &src,
                    src_tag: tag,
                };
                for t in &batch.tiles {
                    let id =
                        insert_tile(&tx, profile_id, batch.dt1.as_str(), &owner, t).map_err(sq)?;
                    tile_ids.insert(t.key.clone(), id);
                    tile_count += 1;
                }
                Ok(())
            };
            run_tiles(&ctx, &mut on_progress, &mut sink)?
        };
        for w in tile_warnings {
            push_warning(&mut warnings, &mut overflow, w);
        }
        report.tiles = tile_count;
        report.assets += tile_count;

        // ---- Sprites: size, counts and a thumbnail of the first frame.
        let sprite_total = sprite_entries.len() as u64;
        rep.begin(BuildPhase::Sprites, sprite_total, "")?;
        let palette = tiles::sprite_palette(overlay);
        let mut sprites_done = 0u64;
        for chunk in sprite_entries.chunks(SPRITE_CHUNK) {
            let results = par_map(
                chunk,
                cancel,
                |e| tiles::process_sprite(overlay, &e.path, e.layer, e.ext, &palette),
                |d| rep.tick(sprites_done + d as u64, sprite_total, "sprites"),
            )?;
            sprites_done += chunk.len() as u64;
            for (entry, sprite) in chunk.iter().zip(results) {
                if let Some(w) = sprite.warning {
                    push_warning(&mut warnings, &mut overflow, w);
                }
                let kind = match entry.ext {
                    "dc6" => AssetKind::Dc6,
                    "dcc" => AssetKind::Dcc,
                    _ => AssetKind::Cof,
                };
                let mut asset = NewAsset::file(entry, kind);
                asset.width = sprite.width;
                asset.height = sprite.height;
                asset.directions = sprite.directions;
                asset.frames = sprite.frames;
                asset.class = sprite.layers.map(|n| format!("{n} layers"));
                asset.thumb = sprite.thumb;
                insert_asset(&tx, profile_id, &asset).map_err(sq)?;
                report.assets += 1;
            }
        }

        rep.begin(BuildPhase::Writing, used_in.len() as u64, "used-in links")?;
        {
            let mut stmt = tx
                .prepare_cached("INSERT INTO used_in(asset_id, ds1_path, cells) VALUES(?1, ?2, ?3)")
                .map_err(sq)?;
            for (i, row) in used_in.iter().enumerate() {
                if let Some(&id) = tile_ids.get(&row.tile_key) {
                    stmt.execute(params![id, row.ds1, row.cells]).map_err(sq)?;
                }
                if i.is_multiple_of(4096) {
                    rep.tick(i as u64, used_in.len() as u64, "used-in links")?;
                }
            }
        }
        rep.check()?;
        tx.execute(
            "INSERT INTO assets_fts(rowid, gen_name, path, class, color, texture)
             SELECT id, gen_name, path, class, color, texture FROM assets WHERE profile = ?1",
            [profile_id],
        )
        .map_err(sq)?;
        tx.execute(
            "INSERT INTO profiles(id, fingerprint, built_unix_ms) VALUES(?1, ?2, ?3)",
            params![profile_id, fingerprint, now_ms()],
        )
        .map_err(sq)?;
        rep.check()?;
        tx.commit().map_err(sq)?;
        self.catalog = Some((profile_id.to_owned(), catalog));

        if overflow > 0 {
            warnings.push(format!("... and {overflow} more warnings"));
        }
        report.warnings = warnings;
        report.millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }

    /// Cheap incremental update after maps were saved: re-reads only the
    /// saves layer's DS1 files (and any DS1 that was served from the saves
    /// layer before and no longer is) and updates their ds1 assets,
    /// `src`/`src_tag`, `files` row and `used_in` rows. The rest of the
    /// profile is untouched; the profile must have been built.
    pub fn refresh_saves(&mut self, profile: &Profile, overlay: &Overlay) -> Result<BuildReport> {
        let started = Instant::now();
        let profile_id = profile.id.as_str();
        let db_path = self.path.clone();
        let sq = |e: rusqlite::Error| IndexError::sql(&db_path, e);
        let built: Option<i64> = self
            .conn
            .query_row(
                "SELECT built_unix_ms FROM profiles WHERE id = ?1",
                [profile_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sq)?;
        if built.is_none() {
            return Err(IndexError::Other(format!(
                "profile {profile_id} has no index yet; build it first"
            )));
        }

        // DS1s to look at: those in the saves layer(s), plus those the index
        // still attributes to the saves layer (deleted or reverted since).
        let mut paths: BTreeSet<GamePath> = BTreeSet::new();
        for layer in overlay.layers() {
            if src_tag(layer.name()) == "saves" {
                for p in layer.list()? {
                    if p.extension() == Some("ds1") {
                        paths.insert(p);
                    }
                }
            }
        }
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT path FROM assets WHERE profile = ?1 AND kind = 'ds1' AND src_tag = 'saves'",
                )
                .map_err(sq)?;
            let rows = stmt
                .query_map([profile_id], |r| r.get::<_, String>(0))
                .map_err(sq)?;
            for row in rows {
                paths.insert(GamePath::new(&row.map_err(sq)?));
            }
        }
        let mut report = BuildReport::default();
        if paths.is_empty() {
            return Ok(report);
        }

        let catalog = match self.catalog.take() {
            Some((id, c)) if id == profile_id => c,
            _ => MapCatalog::build(overlay, profile.game_version, &[])?.0,
        };
        let layers = overlay.layers();
        let entries: Vec<(GamePath, Option<Entry>)> = paths
            .into_iter()
            .map(|path| {
                let entry = overlay.resolve(&path).and_then(|layer| {
                    let source = layers.get(layer.0)?.name().to_owned();
                    Some(Entry {
                        path: path.clone(),
                        ext: "ds1",
                        layer,
                        src_tag: src_tag(&source),
                        source,
                    })
                });
                (path, entry)
            })
            .collect();
        let never = AtomicBool::new(false);
        let processed = par_map(
            &entries,
            &never,
            |(_, entry)| entry.as_ref().map(|e| process_entry(overlay, e, &catalog)),
            |_| Ok(()),
        )?;

        let tx = self.conn.transaction().map_err(sq)?;
        for ((path, entry), result) in entries.iter().zip(processed) {
            let key = path.as_str();
            // Forget the old row's FTS entry and used_in links first.
            tx.execute(
                "INSERT INTO assets_fts(assets_fts, rowid, gen_name, path, class, color, texture)
                 SELECT 'delete', id, gen_name, path, class, color, texture FROM assets
                 WHERE profile = ?1 AND key = ?2",
                params![profile_id, key],
            )
            .map_err(sq)?;
            tx.execute(
                "DELETE FROM used_in WHERE ds1_path = ?1
                   AND asset_id IN (SELECT id FROM assets WHERE profile = ?2)",
                params![key, profile_id],
            )
            .map_err(sq)?;
            let (Some(entry), Some(result)) = (entry, result) else {
                // Gone from every layer.
                tx.execute(
                    "DELETE FROM assets WHERE profile = ?1 AND key = ?2",
                    params![profile_id, key],
                )
                .map_err(sq)?;
                tx.execute(
                    "DELETE FROM files WHERE profile = ?1 AND path = ?2",
                    params![profile_id, key],
                )
                .map_err(sq)?;
                report.assets += 1;
                continue;
            };
            if let Some(w) = result.warning {
                report.warnings.push(w);
            }
            let Some(file) = result.file else {
                continue;
            };
            insert_file_row(&tx, profile_id, entry, &file).map_err(sq)?;
            report.files += 1;
            let asset = NewAsset::ds1(entry, result.ds1.as_ref());
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM assets WHERE profile = ?1 AND key = ?2",
                    params![profile_id, key],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sq)?;
            let id = match existing {
                Some(id) => {
                    tx.execute(
                        "UPDATE assets SET act = ?2, width = ?3, height = ?4, gen_name = ?5,
                                           src = ?6, src_tag = ?7 WHERE id = ?1",
                        params![
                            id,
                            asset.act,
                            asset.width,
                            asset.height,
                            asset.gen_name,
                            asset.src,
                            asset.src_tag
                        ],
                    )
                    .map_err(sq)?;
                    id
                }
                None => insert_asset(&tx, profile_id, &asset).map_err(sq)?,
            };
            tx.execute(
                "INSERT INTO assets_fts(rowid, gen_name, path, class, color, texture)
                 SELECT id, gen_name, path, class, color, texture FROM assets WHERE id = ?1",
                [id],
            )
            .map_err(sq)?;
            report.assets += 1;
            if let Some(info) = result.ds1 {
                relink_used_in(&tx, profile_id, &info.usage).map_err(sq)?;
            }
        }
        tx.commit().map_err(sq)?;
        self.catalog = Some((profile_id.to_owned(), catalog));
        report.millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }
}

/// Recompute the `used_in` rows of one DS1 from the tiles already indexed.
fn relink_used_in(tx: &Transaction<'_>, profile: &str, usage: &Ds1Usage) -> rusqlite::Result<()> {
    let mut by_dt1: TileKeys = HashMap::new();
    let mut ids: HashMap<String, i64> = HashMap::new();
    let mut load = |dt1: Option<&GamePath>| -> rusqlite::Result<()> {
        let mut stmt = tx.prepare_cached(
            "SELECT id, key, path, orientation, main, sub FROM assets
             WHERE profile = ?1 AND kind = 'tile' AND (?2 IS NULL OR path = ?2)",
        )?;
        let rows = stmt.query_map(params![profile, dt1.map(GamePath::as_str)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u8>(3)?,
                r.get::<_, u8>(4)?,
                r.get::<_, u8>(5)?,
            ))
        })?;
        for row in rows {
            let (id, key, path, o, m, s) = row?;
            by_dt1
                .entry(GamePath::new(&path))
                .or_default()
                .entry((o, m, s))
                .or_default()
                .push(key.clone());
            ids.insert(key, id);
        }
        Ok(())
    };
    if usage.dt1_order.is_empty() {
        load(None)?;
    } else {
        for dt1 in &usage.dt1_order {
            load(Some(dt1))?;
        }
    }
    let rows = resolve_used_in(std::slice::from_ref(usage), &by_dt1);
    let mut stmt =
        tx.prepare_cached("INSERT INTO used_in(asset_id, ds1_path, cells) VALUES(?1, ?2, ?3)")?;
    for row in rows {
        if let Some(&id) = ids.get(&row.tile_key) {
            stmt.execute(params![id, row.ds1, row.cells])?;
        }
    }
    Ok(())
}

/// A SQL string literal (profile ids are `[a-z0-9_-]+`, but be safe).
fn sql_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
