//! Shared helpers: load the private corpus tables, and resolve edited tables
//! through the `d2_assets` presets catalog.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use capy_mod::level_tables::LevelTables;
use d2_assets::{
    AssetSource, FolderSource, GamePath, GameVersion, MapCatalog, MapInfo, Overlay,
    SourceFingerprint, SourceKind,
};
use d2_formats::txt::TxtDoc;

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

/// `CAPY_PRIVATE_DIR`, else the main checkout's `testdata\private`.
pub fn private_dir() -> PathBuf {
    std::env::var_os("CAPY_PRIVATE_DIR").map_or_else(
        || PathBuf::from(r"C:\Users\zffit\Capybara-d2-editor\testdata\private"),
        PathBuf::from,
    )
}

/// `extracted/<profile>` when present.
pub fn corpus(profile: &str) -> Option<PathBuf> {
    let p = private_dir().join("extracted").join(profile);
    p.join("data").is_dir().then_some(p)
}

pub fn read_game_file(root: &std::path::Path, game_path: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(GamePath::new(game_path).to_native(root))
}

pub fn read_doc(root: &std::path::Path, name: &str) -> std::io::Result<TxtDoc> {
    Ok(TxtDoc::parse(&read_game_file(
        root,
        &format!(r"data\global\excel\{name}"),
    )?))
}

pub fn load_tables(root: &std::path::Path) -> std::io::Result<LevelTables> {
    Ok(LevelTables {
        prest: read_doc(root, "lvlprest.txt")?,
        levels: read_doc(root, "levels.txt")?,
        types: read_doc(root, "lvltypes.txt")?,
        lvl_sub: read_doc(root, "lvlsub.txt").ok(),
    })
}

/// Files held in memory, as the top layer of an overlay.
pub struct MemorySource {
    pub files: HashMap<GamePath, Vec<u8>>,
}

impl AssetSource for MemorySource {
    fn name(&self) -> &str {
        "memory"
    }
    fn kind(&self) -> SourceKind {
        SourceKind::Folder
    }
    fn contains(&self, path: &GamePath) -> bool {
        self.files.contains_key(path)
    }
    fn read(&self, path: &GamePath) -> d2_assets::Result<Option<Vec<u8>>> {
        Ok(self.files.get(path).cloned())
    }
    fn list(&self) -> d2_assets::Result<Vec<GamePath>> {
        Ok(self.files.keys().cloned().collect())
    }
    fn fingerprint(&self) -> d2_assets::Result<SourceFingerprint> {
        Ok(SourceFingerprint {
            path: PathBuf::from("memory"),
            size: 0,
            modified_unix_ms: 0,
            entries: self.files.len() as u64,
        })
    }
}

/// The catalog of a corpus with `tables` (and any `extra` files) laid over it.
/// V114d reads the `.txt` forms, which is what the memory layer holds.
pub fn catalog_with(
    root: &std::path::Path,
    tables: &LevelTables,
    extra: &[(&str, Vec<u8>)],
    ds1_names: &[&str],
) -> Result<MapCatalog, Box<dyn std::error::Error>> {
    let mut files: HashMap<GamePath, Vec<u8>> = HashMap::new();
    files.insert(
        GamePath::new(r"data\global\excel\levels.txt"),
        tables.levels.to_bytes(),
    );
    files.insert(
        GamePath::new(r"data\global\excel\lvlprest.txt"),
        tables.prest.to_bytes(),
    );
    files.insert(
        GamePath::new(r"data\global\excel\lvltypes.txt"),
        tables.types.to_bytes(),
    );
    for (p, b) in extra {
        files.insert(GamePath::new(p), b.clone());
    }
    let overlay = Overlay::new(vec![
        Box::new(MemorySource { files }),
        Box::new(FolderSource::new(root, "corpus")),
    ]);
    let names: Vec<GamePath> = ds1_names.iter().map(|n| GamePath::new(n)).collect();
    let (catalog, _warnings) = MapCatalog::build(&overlay, GameVersion::V114d, &names)?;
    Ok(catalog)
}

pub fn best(catalog: &MapCatalog, ds1: &str) -> Option<MapInfo> {
    catalog.best(&GamePath::new(ds1))
}

/// `a` and `b` agree on every row up to the last non-blank row of `a`
/// (appended rows go after it).
pub fn old_rows_unchanged(a: &TxtDoc, b: &TxtDoc) -> bool {
    let last = (0..a.row_count())
        .rev()
        .find(|&r| {
            a.row_cells(r)
                .is_some_and(|c| c.iter().any(|x| !x.is_empty()))
        })
        .map_or(0, |r| r + 1);
    a.headers() == b.headers()
        && (0..last).all(|r| a.row_cells(r) == b.row_cells(r))
        && b.row_count() >= last
}
