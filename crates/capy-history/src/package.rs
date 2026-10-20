//! Map packages (P13): a `.zip` holding a map, the DT1s it needs and any
//! table rows, and a merge-import into a profile's saves folder.
//!
//! Layout of the zip: `capybara-package.json` (the [`Manifest`]) first, then
//! every file at its game path with `/` separators (`data/global/tiles/...`).
//! Entries are sorted and carry a fixed timestamp, so the same input always
//! gives the same bytes.
//!
//! Imports only ever write through a [`SavesDir`] (`capybara\saves\<profile>`),
//! so each written file also lands in `CHANGED.txt`; the game folder is never
//! touched. Every path in a package is validated before the first write.

use std::collections::BTreeMap;
use std::path::Path;

use d2_assets::GamePath;
use serde::{Deserialize, Serialize};

use crate::error::{HistoryError, Result};
use crate::saves::SavesDir;
use crate::zip::{ZipWriter, read_zip};

pub const PACKAGE_FORMAT: &str = "capybara-map-package";
pub const PACKAGE_VERSION: u32 = 1;
pub const MANIFEST_NAME: &str = "capybara-package.json";

fn bad(detail: impl Into<String>) -> HistoryError {
    HistoryError::Package(detail.into())
}

/// A table row (or rows) the map needs, with the column header so the caller
/// can merge it by column name into its own copy of the table. Importing does
/// not touch tables; it hands these back in [`ImportReport::rows`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRows {
    /// Table name, e.g. `LvlPrest.txt`.
    pub table: String,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFile {
    /// Game path with `/` separators, as stored in the zip.
    pub path: String,
    pub size: u64,
    /// Hex blake3 of the bytes.
    pub blake3: String,
    /// The file comes from the base game (every player has it); importing it
    /// is optional.
    pub base_game: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    /// Game path of the map (`/`-separated); also listed in `files`.
    pub map: String,
    pub files: Vec<PackageFile>,
    #[serde(default)]
    pub rows: Vec<TableRows>,
    #[serde(default)]
    pub note: Option<String>,
}

/// One file to pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackEntry {
    pub path: GamePath,
    pub bytes: Vec<u8>,
    pub base_game: bool,
}

/// What to export.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageSpec {
    pub map_path: Option<GamePath>,
    pub map_bytes: Vec<u8>,
    /// The DT1s (and any other files) the map needs.
    pub files: Vec<PackEntry>,
    pub rows: Vec<TableRows>,
    pub note: Option<String>,
}

fn zip_name(p: &GamePath) -> String {
    p.as_str().replace('\\', "/")
}

/// Builds the package zip.
pub fn export_package(spec: &PackageSpec) -> Result<Vec<u8>> {
    let map_path = spec
        .map_path
        .as_ref()
        .ok_or_else(|| bad("the package has no map path"))?;
    let mut all: BTreeMap<String, (&[u8], bool)> = BTreeMap::new();
    all.insert(zip_name(map_path), (&spec.map_bytes, false));
    for f in &spec.files {
        all.entry(zip_name(&f.path))
            .or_insert((&f.bytes, f.base_game));
    }
    let files = all
        .iter()
        .map(|(path, (bytes, base_game))| PackageFile {
            path: path.clone(),
            size: bytes.len() as u64,
            blake3: blake3::hash(bytes).to_hex().to_string(),
            base_game: *base_game,
        })
        .collect();
    let manifest = Manifest {
        format: PACKAGE_FORMAT.into(),
        version: PACKAGE_VERSION,
        map: zip_name(map_path),
        files,
        rows: spec.rows.clone(),
        note: spec.note.clone(),
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| bad(e.to_string()))?;
    let mut zip = ZipWriter::new();
    zip.add(MANIFEST_NAME, &json)?;
    for (path, (bytes, _)) in &all {
        zip.add(path, bytes)?;
    }
    zip.finish()
}

/// [`export_package`] written to `out` (atomically).
pub fn export_package_to(out: &Path, spec: &PackageSpec) -> Result<()> {
    crate::atomic::atomic_write(out, &export_package(spec)?)
}

/// A package read back and verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub manifest: Manifest,
    /// Every file at its game path, in manifest order.
    pub files: Vec<(GamePath, Vec<u8>)>,
}

/// Reads and verifies a package: manifest format, every listed file present
/// with matching size and hash, no unlisted files, no path escaping the game
/// folder.
pub fn read_package(bytes: &[u8]) -> Result<Package> {
    let mut entries: BTreeMap<String, Vec<u8>> = read_zip(bytes)?.into_iter().collect();
    let json = entries
        .remove(MANIFEST_NAME)
        .ok_or_else(|| bad("not a Capybara map package (no manifest)"))?;
    let manifest: Manifest =
        serde_json::from_slice(&json).map_err(|e| bad(format!("manifest: {e}")))?;
    if manifest.format != PACKAGE_FORMAT {
        return Err(bad(format!("unknown package format {:?}", manifest.format)));
    }
    if manifest.version > PACKAGE_VERSION {
        return Err(bad(format!(
            "package version {} is newer than this editor understands ({PACKAGE_VERSION})",
            manifest.version
        )));
    }
    let mut files = Vec::new();
    for f in &manifest.files {
        let data = entries
            .remove(&f.path)
            .ok_or_else(|| bad(format!("{}: missing from the zip", f.path)))?;
        if data.len() as u64 != f.size || blake3::hash(&data).to_hex().as_str() != f.blake3 {
            return Err(bad(format!("{}: does not match the manifest", f.path)));
        }
        files.push((GamePath::new(&f.path), data));
    }
    if let Some(extra) = entries.keys().next() {
        return Err(bad(format!("{extra}: not listed in the manifest")));
    }
    if !manifest.files.iter().any(|f| f.path == manifest.map) {
        return Err(bad("the map is not among the package files"));
    }
    Ok(Package { manifest, files })
}

/// What to do with a file the profile's saves already hold with other content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    /// Keep the existing file.
    Skip,
    /// Replace it (the caller can snapshot the old one first).
    Overwrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOptions {
    pub on_conflict: OnConflict,
    /// Also import files flagged `base_game` (default off: players already have them).
    pub include_base_game: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            on_conflict: OnConflict::Skip,
            include_base_game: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// The map's game path.
    pub map: Option<GamePath>,
    /// Files written (new or overwritten), each also added to `CHANGED.txt`.
    pub written: Vec<GamePath>,
    /// Files already present with identical content.
    pub unchanged: Vec<GamePath>,
    /// Files present with other content and kept (`OnConflict::Skip`).
    pub conflicts: Vec<GamePath>,
    /// Base-game files left out.
    pub skipped_base_game: Vec<GamePath>,
    /// Table rows to merge (the importer does not edit tables).
    pub rows: Vec<TableRows>,
}

/// Merge-imports `files` into `saves`.
fn merge_into(
    saves: &SavesDir,
    files: &[(GamePath, Vec<u8>, bool)],
    opts: &ImportOptions,
    report: &mut ImportReport,
) -> Result<()> {
    // Validate everything first: a bad path must not leave a half import.
    let mut targets = Vec::with_capacity(files.len());
    for (p, _, _) in files {
        targets.push(saves.path_for(p)?);
    }
    for ((p, bytes, base), target) in files.iter().zip(targets) {
        if *base && !opts.include_base_game {
            report.skipped_base_game.push(p.clone());
            continue;
        }
        match std::fs::read(&target) {
            Ok(existing) if existing == *bytes => {
                report.unchanged.push(p.clone());
                continue;
            }
            Ok(_) if opts.on_conflict == OnConflict::Skip => {
                report.conflicts.push(p.clone());
                continue;
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(HistoryError::io(&target, e)),
        }
        saves.save(p, bytes)?;
        report.written.push(p.clone());
    }
    Ok(())
}

/// Imports a package into `saves` (`capybara\saves\<profile>`).
pub fn import_package(
    saves: &SavesDir,
    bytes: &[u8],
    opts: &ImportOptions,
) -> Result<ImportReport> {
    let pkg = read_package(bytes)?;
    let files: Vec<_> = pkg
        .files
        .into_iter()
        .zip(&pkg.manifest.files)
        .map(|((p, b), m)| (p, b, m.base_game))
        .collect();
    let mut report = ImportReport {
        map: Some(GamePath::new(&pkg.manifest.map)),
        rows: pkg.manifest.rows.clone(),
        ..ImportReport::default()
    };
    merge_into(saves, &files, opts, &mut report)?;
    Ok(report)
}

/// Where a loose DT1 was found.
fn find_loose(folder: &Path, want: &GamePath) -> Option<std::path::PathBuf> {
    // 1. the game path below the folder (`<folder>\data\global\tiles\...`),
    // 2. the path without `data\`, 3. the file name anywhere below the folder.
    let rel = want.as_str();
    let direct = [Some(rel), rel.strip_prefix("data\\")]
        .into_iter()
        .flatten()
        .map(|r| folder.join(r.replace('\\', std::path::MAIN_SEPARATOR_STR)))
        .find(|p| p.is_file());
    if direct.is_some() {
        return direct;
    }
    let name = want.file_name();
    let mut stack = vec![folder.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > 200_000 {
                return None;
            }
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
            {
                return Some(p);
            }
        }
    }
    None
}

/// Result of [`import_loose_map`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LooseImport {
    pub report: ImportReport,
    /// Needed DT1s found nowhere in the folder (the player's game may still have them).
    pub missing: Vec<GamePath>,
}

/// Imports a DS1 from a loose folder together with the DT1s it needs.
/// `ds1_file` is the DS1 on disk and `ds1_path` the game path to give it;
/// `needed` are the game paths of the DT1s (from the map's tile set); each is
/// looked up in `folder` (by game path, then by file name) and imported at its
/// game path.
pub fn import_loose_map(
    saves: &SavesDir,
    ds1_file: &Path,
    ds1_path: &GamePath,
    folder: &Path,
    needed: &[GamePath],
    opts: &ImportOptions,
) -> Result<LooseImport> {
    let ds1 = std::fs::read(ds1_file).map_err(|e| HistoryError::io(ds1_file, e))?;
    let mut files = vec![(ds1_path.clone(), ds1, false)];
    let mut out = LooseImport::default();
    for want in needed {
        if files.iter().any(|(p, _, _)| p == want) {
            continue;
        }
        match find_loose(folder, want) {
            Some(found) => {
                let bytes = std::fs::read(&found).map_err(|e| HistoryError::io(&found, e))?;
                files.push((want.clone(), bytes, false));
            }
            None => out.missing.push(want.clone()),
        }
    }
    out.report.map = Some(ds1_path.clone());
    merge_into(saves, &files, opts, &mut out.report)?;
    Ok(out)
}
