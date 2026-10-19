//! Known file names, for archives that carry no listfile (e.g. `Patch_D2.mpq`).
//!
//! [`KnownNames`] is a set of game paths. [`KnownNames::harvest`] fills it
//! from every layer's listfile, the embedded seed, the excel tables that name
//! files and every DS1 it finds, keeping only names that resolve in the
//! overlay. COF-derived names are not harvested yet.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use d2_formats::ds1::Ds1;
use d2_formats::excel::ExcelTableKind;
use d2_formats::tables;

use crate::error::{AssetError, Result};
use crate::overlay::Overlay;
use crate::path::GamePath;
use crate::profile::GameVersion;
use crate::tables::load_excel;

/// Embedded well-known paths, one per line; `#` comments and blank lines ignored.
const SEED: &str = include_str!("../../../assets/known-names/seed.txt");

const EXCEL_DIR: &str = "data\\global\\excel\\";

#[derive(Clone, Debug, Default)]
pub struct KnownNames {
    names: BTreeSet<GamePath>,
}

/// What [`KnownNames::harvest`] found. Each `from_*` counts the names that
/// source was the first to contribute (and that resolve in the overlay).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HarvestStats {
    pub from_listfiles: usize,
    pub from_seed: usize,
    pub from_tables: usize,
    pub from_ds1: usize,
    /// Stays 0 until a COF parser exists.
    pub from_cof: usize,
    /// Passes run, including the last one that added no new name.
    pub rounds: usize,
    /// Distinct candidate names that resolved in no layer. They are dropped,
    /// but counted here.
    pub unresolved: usize,
    /// Tables or DS1 files that could not be read or parsed (skipped, one
    /// line each) and layers whose names could not be listed.
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
enum Origin {
    Listfile,
    Seed,
    Table,
    Ds1,
}

type Scan = std::result::Result<Vec<String>, String>;

struct Harvester<'a> {
    overlay: &'a Overlay,
    known: KnownNames,
    rejected: HashSet<GamePath>,
    ds1_done: HashSet<GamePath>,
    stats: HarvestStats,
    changed: bool,
}

impl Harvester<'_> {
    fn accept(&mut self, path: GamePath, origin: Origin) {
        if self.known.insert(path) {
            self.changed = true;
            let counter = match origin {
                Origin::Listfile => &mut self.stats.from_listfiles,
                Origin::Seed => &mut self.stats.from_seed,
                Origin::Table => &mut self.stats.from_tables,
                Origin::Ds1 => &mut self.stats.from_ds1,
            };
            *counter += 1;
        }
    }

    /// Keep `path` if it resolves in the overlay; otherwise remember it as unresolved.
    fn offer(&mut self, path: GamePath, origin: Origin) {
        if path.as_str().is_empty() || self.known.contains(&path) || self.rejected.contains(&path) {
            return;
        }
        if self.overlay.resolve(&path).is_some() {
            self.accept(path, origin);
        } else {
            self.rejected.insert(path);
        }
    }

    fn offer_tile_ref(&mut self, raw: &str, origin: Origin) {
        if let Some(path) = GamePath::from_tile_ref(raw) {
            self.offer(path, origin);
        }
    }

    fn warn(&mut self, message: String) {
        self.stats.warnings.push(message);
    }

    fn listfiles(&mut self) {
        for layer in self.overlay.layers() {
            match layer.list() {
                Ok(paths) => {
                    // The layer itself lists these, so they resolve by construction.
                    for path in paths {
                        self.accept(path, Origin::Listfile);
                    }
                }
                Err(e) => self.warn(format!("{}: cannot list names: {e}", layer.name())),
            }
        }
    }

    fn seed(&mut self) {
        for path in KnownNames::seed().names {
            self.offer(path, Origin::Seed);
        }
    }

    /// Both the `.txt` and `.bin` form of every excel table name seen.
    fn excel_siblings(&mut self) {
        let siblings: Vec<GamePath> = self
            .known
            .iter()
            .chain(self.rejected.iter())
            .filter(|p| p.as_str().starts_with(EXCEL_DIR))
            .filter_map(|p| {
                let stem = p
                    .as_str()
                    .strip_suffix(".txt")
                    .or_else(|| p.as_str().strip_suffix(".bin"))?;
                Some([
                    GamePath::new(&format!("{stem}.txt")),
                    GamePath::new(&format!("{stem}.bin")),
                ])
            })
            .flatten()
            .collect();
        for path in siblings {
            self.offer(path, Origin::Table);
        }
    }

    /// LvlTypes `File1..32` and LvlPrest `File1..6`.
    fn table_files(&mut self, version: GameVersion) {
        let mut refs: Vec<String> = Vec::new();
        let types = load_excel(self.overlay, ExcelTableKind::LvlTypes, version)
            .map_err(|e| e.to_string())
            .and_then(|t| tables::lvl_types(&t).map_err(|e| e.to_string()));
        match types {
            Ok(rows) => refs.extend(rows.into_iter().flat_map(|r| r.files).flatten()),
            Err(e) => self.warn(format!("lvltypes: {e}")),
        }
        let prests = load_excel(self.overlay, ExcelTableKind::LvlPrest, version)
            .map_err(|e| e.to_string())
            .and_then(|t| tables::lvl_prests(&t).map_err(|e| e.to_string()));
        match prests {
            Ok(rows) => refs.extend(rows.into_iter().flat_map(|r| r.files).flatten()),
            Err(e) => self.warn(format!("lvlprest: {e}")),
        }
        for raw in refs {
            self.offer_tile_ref(&raw, Origin::Table);
        }
    }

    /// Parse every not yet scanned `.ds1` and offer its tileset references.
    fn ds1_files(&mut self) {
        let pending: Vec<GamePath> = self
            .known
            .iter()
            .filter(|p| p.extension() == Some("ds1") && !self.ds1_done.contains(*p))
            .cloned()
            .collect();
        self.ds1_done.extend(pending.iter().cloned());
        for (path, result) in pending.iter().zip(scan_ds1(self.overlay, &pending)) {
            match result {
                Ok(refs) => {
                    for raw in refs {
                        self.offer_tile_ref(&raw, Origin::Ds1);
                    }
                }
                Err(e) => self.warn(format!("{path}: {e}")),
            }
        }
    }
}

/// Tileset references of one DS1, or the reason it was skipped.
fn scan_one(overlay: &Overlay, path: &GamePath) -> Scan {
    let bytes = overlay
        .read_bytes(path)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "not found".to_owned())?;
    let ds1 = Ds1::parse(&bytes).map_err(|e| e.to_string())?;
    // Latin-1: one char per byte.
    Ok(ds1
        .tileset_refs()
        .map(|raw| raw.iter().map(|&b| char::from(b)).collect())
        .collect())
}

/// [`scan_one`] for each path, in input order, on a few threads.
fn scan_ds1(overlay: &Overlay, paths: &[GamePath]) -> Vec<Scan> {
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(8)
        .min(paths.len());
    if workers <= 1 {
        return paths.iter().map(|p| scan_one(overlay, p)).collect();
    }
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, Scan)>> = Mutex::new(Vec::with_capacity(paths.len()));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    let result = scan_one(overlay, path);
                    if let Ok(mut done) = done.lock() {
                        done.push((i, result));
                    }
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap_or_default();
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

impl KnownNames {
    pub fn new() -> KnownNames {
        KnownNames::default()
    }

    /// The embedded well-known paths.
    pub fn seed() -> KnownNames {
        let mut names = KnownNames::new();
        names.extend_from_listfile(SEED);
        names
    }

    /// Returns `true` when the name was not known yet.
    pub fn insert(&mut self, path: GamePath) -> bool {
        self.names.insert(path)
    }

    /// Add every name in `text` (split on `;`, CR and LF; empty entries,
    /// `#` comments and MPQ-internal `(…)` entries are skipped). Returns how
    /// many names were new.
    pub fn extend_from_listfile(&mut self, text: &str) -> usize {
        let mut added = 0;
        for entry in text.split([';', '\r', '\n']) {
            let entry = entry.trim();
            if entry.is_empty() || entry.starts_with('#') || entry.starts_with('(') {
                continue;
            }
            let path = GamePath::new(entry);
            if !path.as_str().is_empty() && self.names.insert(path) {
                added += 1;
            }
        }
        added
    }

    pub fn contains(&self, path: &GamePath) -> bool {
        self.names.contains(path)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Names in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &GamePath> {
        self.names.iter()
    }

    /// Gathers names from every layer's `list()`, the embedded seed, the
    /// excel tables that name files (LvlTypes `File1..32` and LvlPrest
    /// `File1..6`, plus the `.txt` and `.bin` form of every excel table name
    /// seen) and every DS1 found (its tileset references; `.tg1` maps to
    /// `.dt1`), repeating until a round adds no new name.
    ///
    /// Only names that resolve in `overlay` are kept; the rest are counted in
    /// [`HarvestStats::unresolved`]. A table or DS1 that fails to read or
    /// parse is reported in [`HarvestStats::warnings`] and skipped.
    pub fn harvest(overlay: &Overlay, version: GameVersion) -> Result<(KnownNames, HarvestStats)> {
        let mut h = Harvester {
            overlay,
            known: KnownNames::new(),
            rejected: HashSet::new(),
            ds1_done: HashSet::new(),
            stats: HarvestStats::default(),
            changed: false,
        };
        h.listfiles();
        h.seed();
        loop {
            h.changed = false;
            h.stats.rounds += 1;
            h.excel_siblings();
            // Tables do not change between rounds, so one read is enough.
            if h.stats.rounds == 1 {
                h.table_files(version);
            }
            h.ds1_files();
            if !h.changed {
                break;
            }
        }
        h.stats.unresolved = h.rejected.len();
        Ok((h.known, h.stats))
    }

    /// Read a names file (one name per line).
    pub fn load(path: &Path) -> Result<KnownNames> {
        let bytes = std::fs::read(path).map_err(|e| AssetError::io(path, e))?;
        let mut names = KnownNames::new();
        names.extend_from_listfile(&String::from_utf8_lossy(&bytes));
        Ok(names)
    }

    /// Write the names, one per line, sorted. Written to a temp file and
    /// renamed, so a crash never leaves a half-written list.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut text = String::new();
        for name in &self.names {
            text.push_str(name.as_str());
            text.push('\n');
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| AssetError::io(parent, e))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, text).map_err(|e| AssetError::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| AssetError::io(path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_embedded() {
        let seed = KnownNames::seed();
        assert!(seed.len() > 100, "seed has {} names", seed.len());
        assert!(seed.contains(&GamePath::new(r"data\global\excel\lvltypes.bin")));
        assert!(seed.contains(&GamePath::new(r"data\global\palette\act1\pal.dat")));
        assert!(seed.iter().all(|p| !p.as_str().starts_with('#')));
    }

    #[test]
    fn container_basics() {
        let mut names = KnownNames::new();
        assert!(names.is_empty());
        assert!(names.insert(GamePath::new("Data\\A.txt")));
        assert!(!names.insert(GamePath::new("data/a.TXT")));
        let added = names.extend_from_listfile(
            "# comment\r\ndata\\b.txt\r\n\r\n(listfile);DATA\\A.TXT;data\\c.ds1\n",
        );
        assert_eq!(added, 2);
        assert_eq!(names.len(), 3);
        assert!(names.contains(&GamePath::new("data\\c.ds1")));
        let order: Vec<_> = names.iter().map(GamePath::as_str).collect();
        assert_eq!(order, ["data\\a.txt", "data\\b.txt", "data\\c.ds1"]);
        assert!(KnownNames::seed().len() == KnownNames::seed().iter().count());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sub").join("names.txt");
        let mut names = KnownNames::new();
        names.extend_from_listfile("data\\z.ds1\ndata\\a.dt1");
        names.save(&file).expect("save");
        let back = KnownNames::load(&file).expect("load");
        assert_eq!(
            back.iter().collect::<Vec<_>>(),
            names.iter().collect::<Vec<_>>()
        );
        assert!(KnownNames::load(&dir.path().join("missing.txt")).is_err());
    }
}
