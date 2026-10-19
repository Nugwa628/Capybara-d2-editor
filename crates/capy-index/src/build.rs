//! `Index::build`: list the profile's files, decode what the index needs,
//! write everything in one transaction (cancelling leaves the old rows).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use d2_assets::tables::load_excel;
use d2_assets::{GamePath, GameVersion, KnownNames, LayerId, Overlay, Profile};
use d2_formats::ds1::Ds1;
use d2_formats::excel::ExcelTableKind;
use d2_formats::tables::{self, Level, LvlPrest, LvlType};
use d2_formats::txt::TxtTable;
use rusqlite::{Transaction, params};

use crate::error::{IndexError, Result};
use crate::index::{BuildPhase, BuildProgress, BuildReport, Index, fingerprint_text};
use crate::naming::{generated_file_name, place_from_path};
use crate::par::par_map;
use crate::query::AssetKind;
use crate::tiles::{self, Ds1Usage, Dt1Input, TileBatch, TileCtx, TileRecord, TileRun, UsedInRow};

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

/// LvlTypes, LvlPrest and Levels, with the lookups the index needs.
#[derive(Default)]
struct LevelTables {
    lvl_types: Vec<LvlType>,
    prests: Vec<LvlPrest>,
    levels: Vec<Level>,
    lvl_type_by_id: HashMap<u32, usize>,
    level_by_id: HashMap<u32, usize>,
    /// DT1 path to `(lvltype id, slot)`, lowest lvltype first.
    dt1_links: HashMap<GamePath, Vec<(u32, u32)>>,
    /// DS1 path to the first LvlPrest listing it.
    prest_of_ds1: HashMap<GamePath, usize>,
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

impl LevelTables {
    fn load(overlay: &Overlay, version: GameVersion, warnings: &mut Vec<String>) -> LevelTables {
        let lvl_types = load_table(
            overlay,
            ExcelTableKind::LvlTypes,
            version,
            tables::lvl_types,
            warnings,
        );
        let prests = load_table(
            overlay,
            ExcelTableKind::LvlPrest,
            version,
            tables::lvl_prests,
            warnings,
        );
        let levels = load_table(
            overlay,
            ExcelTableKind::Levels,
            version,
            tables::levels,
            warnings,
        );
        LevelTables::new(lvl_types, prests, levels)
    }

    fn new(mut lvl_types: Vec<LvlType>, prests: Vec<LvlPrest>, levels: Vec<Level>) -> LevelTables {
        lvl_types.sort_by_key(|l| (l.id, l.row));
        let mut lvl_type_by_id = HashMap::new();
        for (i, lt) in lvl_types.iter().enumerate() {
            lvl_type_by_id.entry(lt.id).or_insert(i);
        }
        let mut level_by_id = HashMap::new();
        for (i, l) in levels.iter().enumerate() {
            level_by_id.entry(l.id).or_insert(i);
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
        let mut prest_of_ds1 = HashMap::new();
        for (i, p) in prests.iter().enumerate() {
            for file in p.files.iter().flatten() {
                if let Some(path) = GamePath::from_tile_ref(file) {
                    prest_of_ds1.entry(path).or_insert(i);
                }
            }
        }
        LevelTables {
            lvl_types,
            prests,
            levels,
            lvl_type_by_id,
            level_by_id,
            dt1_links,
            prest_of_ds1,
        }
    }

    fn lvl_type(&self, id: u32) -> Option<&LvlType> {
        self.lvl_type_by_id.get(&id).map(|&i| &self.lvl_types[i])
    }

    /// Place name and act of a DT1 from the lowest-id LvlType that loads it.
    /// The name is the first non-empty one among all LvlTypes loading it.
    fn dt1_context(&self, path: &GamePath) -> (Option<String>, Option<u8>) {
        let Some(links) = self.dt1_links.get(path) else {
            return (None, None);
        };
        let act = links
            .first()
            .and_then(|(id, _)| self.lvl_type(*id))
            .map(|lt| lt.act);
        // "the lowest-id LvlType whose File slots load this DT1": its Name.
        let name = links
            .first()
            .and_then(|(id, _)| self.lvl_type(*id))
            .map(|lt| lt.name.trim().to_owned())
            .filter(|n| !n.is_empty());
        (name, act)
    }

    /// DT1 load order and act of a DS1 through its LvlPrest, if listed.
    ///
    /// TODO(presets): `d2_assets::presets::MapCatalog` replaces this.
    fn ds1_context(&self, path: &GamePath) -> (Vec<GamePath>, Option<u8>) {
        let Some(&i) = self.prest_of_ds1.get(path) else {
            return (Vec::new(), None);
        };
        let prest = &self.prests[i];
        let level = u32::try_from(prest.level_id)
            .ok()
            .and_then(|id| self.level_by_id.get(&id))
            .map(|&i| &self.levels[i]);
        let dt1s = level
            .and_then(|l| self.lvl_type(l.level_type))
            .map(|lt| {
                prest
                    .dt1_files(lt)
                    .filter_map(GamePath::from_tile_ref)
                    .collect()
            })
            .unwrap_or_default();
        (dt1s, level.map(|l| l.act))
    }
}

// ------------------------------------------------------------------ files

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
    act: Option<u8>,
    usage: Ds1Usage,
}

struct Processed {
    file: Option<FileInfo>,
    ds1: Option<Ds1Info>,
    warning: Option<String>,
}

fn ds1_keys(ds1: &Ds1) -> Vec<((u8, u8, u8), u32)> {
    let mut counts: HashMap<(u8, u8, u8), u32> = HashMap::new();
    let mut add = |key: (u8, u8, u8)| *counts.entry(key).or_insert(0) += 1;
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

fn process_entry(overlay: &Overlay, entry: &Entry, level_tables: &LevelTables) -> Processed {
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
            let (mut dt1_order, level_act) = level_tables.ds1_context(&entry.path);
            if dt1_order.is_empty() {
                // No LvlPrest lists this map: use the tilesets it names itself.
                dt1_order = ds1
                    .tileset_refs()
                    .filter_map(|raw| GamePath::from_tile_ref(&String::from_utf8_lossy(raw)))
                    .filter(|p| p.extension() == Some("dt1"))
                    .collect();
            }
            let act = act_from_path(name)
                .or(level_act)
                .or_else(|| (ds1.version >= 8).then(|| ds1.effective_act()));
            Processed {
                file: Some(file),
                ds1: Some(Ds1Info {
                    width: ds1.width,
                    height: ds1.height,
                    act,
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
    gen_name: String,
    src: String,
    src_tag: &'static str,
}

fn insert_asset(tx: &Transaction<'_>, profile: &str, a: &NewAsset) -> rusqlite::Result<i64> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO assets(profile, key, kind, path, act, width, height, gen_name, src, src_tag)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    stmt.execute(params![
        profile,
        a.key,
        a.kind.as_str(),
        a.path,
        a.act,
        a.width,
        a.height,
        a.gen_name,
        a.src,
        a.src_tag
    ])?;
    Ok(tx.last_insert_rowid())
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
        let layers = overlay.layers();
        let mut entries: Vec<Entry> = Vec::new();
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
            let source = layers[layer.0].name().to_owned();
            entries.push(Entry {
                path: path.clone(),
                ext,
                layer,
                src_tag: src_tag(&source),
                source,
            });
        }

        // ---- Tables
        rep.begin(BuildPhase::Tables, 3, "LvlTypes, LvlPrest, Levels")?;
        let mut table_warnings = Vec::new();
        let level_tables = LevelTables::load(overlay, profile.game_version, &mut table_warnings);
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
            |e| process_entry(overlay, e, &level_tables),
            |d| rep.tick(d as u64, total, "hashing files"),
        )?;

        // ---- Maps
        let maps: Vec<&Entry> = entries.iter().filter(|e| e.ext == "ds1").collect();
        let total = maps.len() as u64;
        rep.begin(BuildPhase::Maps, total, "reading maps")?;
        let map_results = par_map(
            &maps,
            cancel,
            |e| process_entry(overlay, e, &level_tables),
            |d| rep.tick(d as u64, total, "reading maps"),
        )?;

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
            let kind = match entry.ext {
                "ds1" => Some(AssetKind::Ds1),
                "dt1" => Some(AssetKind::Dt1),
                "dc6" => Some(AssetKind::Dc6),
                "dcc" => Some(AssetKind::Dcc),
                "cof" => Some(AssetKind::Cof),
                _ => None,
            };
            if let Some(info) = result.file {
                file_rows.push((entry, info));
            } else {
                continue;
            }
            let Some(kind) = kind else { continue };
            let mut asset = NewAsset {
                key: path.clone(),
                kind,
                path: path.clone(),
                act: None,
                width: None,
                height: None,
                gen_name: generated_file_name(&path, kind),
                src: entry.source.clone(),
                src_tag: entry.src_tag,
            };
            match kind {
                AssetKind::Ds1 => {
                    if let Some(info) = result.ds1 {
                        asset.act = info.act;
                        asset.width = Some(info.width);
                        asset.height = Some(info.height);
                        ds1_usages.push(info.usage);
                    } else {
                        asset.act = act_from_path(&path);
                    }
                }
                AssetKind::Dt1 => {
                    let (name, lvl_act) = level_tables.dt1_context(&entry.path);
                    let act = lvl_act.or_else(|| act_from_path(&path));
                    asset.act = act;
                    if let Some(links) = level_tables.dt1_links.get(&entry.path) {
                        dt1_links.push((path.clone(), links.clone()));
                    }
                    dt1_inputs.push(Dt1Input {
                        path: entry.path.clone(),
                        layer: entry.layer,
                        place: name.unwrap_or_else(|| place_from_path(&path)),
                        act,
                    });
                }
                _ => {}
            }
            assets.push(asset);
        }
        assets.sort_by(|a, b| a.key.cmp(&b.key));

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
            tx.prepare_cached(
                "INSERT OR REPLACE INTO files(profile, path, kind, layer, source, size, hash)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .and_then(|mut s| {
                s.execute(params![
                    profile_id,
                    entry.path.as_str(),
                    entry.ext,
                    i64::try_from(entry.layer.0).unwrap_or(0),
                    entry.source,
                    i64::try_from(info.size).unwrap_or(i64::MAX),
                    info.hash.as_slice()
                ])
            })
            .map_err(sq)?;
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

        if overflow > 0 {
            warnings.push(format!("... and {overflow} more warnings"));
        }
        report.warnings = warnings;
        report.millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }
}

/// A SQL string literal (profile ids are `[a-z0-9_-]+`, but be safe).
fn sql_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
