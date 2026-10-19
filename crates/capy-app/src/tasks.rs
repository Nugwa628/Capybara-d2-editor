//! Background work: opening a profile's overlay and harvesting known names
//! happen on a plain thread; the UI polls a channel each frame.

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use d2_assets::{
    GamePath, GameVersion, KnownNames, MapCatalog, Overlay, Profile, SourceFingerprint, SourceKind,
};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// Name prefix of the profile's own saves layer, whose contents change on
/// every save and so are excluded from the cache check.
const SAVES_PREFIX: &str = "saves\\";

pub struct SessionData {
    pub profile: Profile,
    pub overlay: Arc<Overlay>,
    pub names: KnownNames,
    pub from_cache: bool,
    pub summary: String,
    pub elapsed: Duration,
}

/// The map catalog and what it took to build.
pub struct CatalogData {
    pub catalog: Box<MapCatalog>,
    pub warnings: Vec<String>,
    pub elapsed: Duration,
}

pub enum TaskEvent {
    Stage {
        generation: u64,
        text: String,
    },
    /// The map catalog of the session opened by request `generation`.
    Catalog {
        generation: u64,
        result: Result<CatalogData, String>,
    },
    /// Progress of the asset index task.
    Index {
        generation: u64,
        event: crate::index_task::IndexEvent,
    },
    /// `Index::refresh_saves` finished after a save.
    SavesRefreshed {
        generation: u64,
        result: Result<String, String>,
    },
    Finished {
        generation: u64,
        result: Result<SessionData, String>,
    },
}

#[derive(Serialize, Deserialize)]
struct FingerprintFile {
    version: u32,
    fingerprints: Vec<SourceFingerprint>,
}

const FINGERPRINT_VERSION: u32 = 1;

/// Fingerprints of every layer except the profile's saves folder.
fn cache_fingerprints(overlay: &Overlay) -> Result<Vec<SourceFingerprint>> {
    let mut out = Vec::new();
    for layer in overlay.layers() {
        let is_saves = layer.kind() == SourceKind::Folder && layer.name().starts_with(SAVES_PREFIX);
        if is_saves {
            continue;
        }
        out.push(layer.fingerprint()?);
    }
    Ok(out)
}

/// The cache is valid when the stored fingerprints equal the current ones.
pub fn fingerprints_match(stored: &[SourceFingerprint], current: &[SourceFingerprint]) -> bool {
    stored == current
}

fn load_cached(
    paths: &Paths,
    profile_id: &str,
    current: &[SourceFingerprint],
) -> Option<KnownNames> {
    let text = std::fs::read_to_string(paths.names_fingerprints(profile_id)).ok()?;
    let stored: FingerprintFile = serde_json::from_str(&text).ok()?;
    if stored.version != FINGERPRINT_VERSION || !fingerprints_match(&stored.fingerprints, current) {
        return None;
    }
    KnownNames::load(&paths.names_cache(profile_id)).ok()
}

fn store_cache(
    paths: &Paths,
    profile_id: &str,
    names: &KnownNames,
    fingerprints: Vec<SourceFingerprint>,
) -> Result<()> {
    let json_path = paths.names_fingerprints(profile_id);
    // Invalidate first: a crash between the two writes must not leave a
    // fingerprint file that vouches for a half-written name list.
    match std::fs::remove_file(&json_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("removing the old name-cache fingerprints"),
    }
    names.save(&paths.names_cache(profile_id))?;
    let file = FingerprintFile {
        version: FINGERPRINT_VERSION,
        fingerprints,
    };
    let json = serde_json::to_string_pretty(&file).context("serialising fingerprints")?;
    crate::config::write_atomic(&json_path, json.as_bytes())
}

/// Names of loose files in folder layers (the saves folder in particular),
/// which the cache deliberately does not cover.
fn add_folder_names(overlay: &Overlay, names: &mut KnownNames) {
    for layer in overlay.layers() {
        if layer.kind() != SourceKind::Folder {
            continue;
        }
        if let Ok(list) = layer.list() {
            for path in list {
                names.insert(path);
            }
        }
    }
}

fn open_session(
    generation: u64,
    profile: Profile,
    paths: &Paths,
    tx: &Sender<TaskEvent>,
) -> Result<SessionData> {
    let started = Instant::now();
    let stage = |text: &str| {
        let _ = tx.send(TaskEvent::Stage {
            generation,
            text: text.to_owned(),
        });
    };
    stage("Opening archives");
    let overlay = profile
        .open(&paths.saves)
        .map_err(|e| anyhow!("opening profile {:?}: {e}", profile.id))?;
    stage("Checking name cache");
    let fingerprints = cache_fingerprints(&overlay)?;
    let (mut names, from_cache, summary) =
        if let Some(names) = load_cached(paths, &profile.id, &fingerprints) {
            let n = names.len();
            (names, true, format!("{n} names (cached)"))
        } else {
            stage("Harvesting file names");
            let (names, stats) = KnownNames::harvest(&overlay, profile.game_version)
                .map_err(|e| anyhow!("harvesting names: {e}"))?;
            let mut summary = format!(
                "{} names harvested in {} rounds, {} unresolved",
                names.len(),
                stats.rounds,
                stats.unresolved
            );
            if !stats.warnings.is_empty() {
                summary.push_str(&format!(", {} warnings", stats.warnings.len()));
            }
            if let Err(e) = store_cache(paths, &profile.id, &names, fingerprints) {
                summary.push_str(&format!(" (cache not written: {e:#})"));
            }
            (names, false, summary)
        };
    add_folder_names(&overlay, &mut names);
    Ok(SessionData {
        profile,
        overlay: Arc::new(overlay),
        names,
        from_cache,
        summary,
        elapsed: started.elapsed(),
    })
}

/// Start opening `profile` on a worker thread. A panic is reported as an error.
pub fn spawn_open(generation: u64, profile: Profile, paths: Paths, tx: Sender<TaskEvent>) {
    let thread_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("open-profile-{}", profile.id))
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                open_session(generation, profile, &paths, &thread_tx)
            }));
            let result = match outcome {
                Ok(Ok(data)) => Ok(data),
                Ok(Err(e)) => Err(format!("{e:#}")),
                Err(_) => Err("background task panicked".to_owned()),
            };
            let _ = thread_tx.send(TaskEvent::Finished { generation, result });
        });
    if let Err(e) = spawned {
        let _ = tx.send(TaskEvent::Finished {
            generation,
            result: Err(format!("cannot start background thread: {e}")),
        });
    }
}

/// Build the [`MapCatalog`] of an opened profile on a worker thread, then apply
/// the user's saved overrides. The result arrives as [`TaskEvent::Catalog`].
pub fn spawn_catalog(
    generation: u64,
    overlay: Arc<Overlay>,
    version: GameVersion,
    ds1s: Vec<GamePath>,
    overrides: Vec<(GamePath, u32, Option<u32>)>,
    tx: Sender<TaskEvent>,
) {
    let thread_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("map-catalog".to_owned())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let started = Instant::now();
                MapCatalog::build(&overlay, version, &ds1s)
                    .map(|(mut catalog, warnings)| {
                        for (ds1, lvl_type, def) in overrides {
                            catalog.set_override(ds1, lvl_type, def);
                        }
                        CatalogData {
                            catalog: Box::new(catalog),
                            warnings,
                            elapsed: started.elapsed(),
                        }
                    })
                    .map_err(|e| e.to_string())
            }));
            let result = outcome.unwrap_or_else(|_| Err("catalog task panicked".to_owned()));
            let _ = thread_tx.send(TaskEvent::Catalog { generation, result });
        });
    if let Err(e) = spawned {
        let _ = tx.send(TaskEvent::Catalog {
            generation,
            result: Err(format!("cannot start background thread: {e}")),
        });
    }
}

/// Every `.ds1` among the known names, sorted.
pub fn ds1_list(names: &KnownNames) -> Vec<GamePath> {
    names
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ds1")))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fp(size: u64, mtime: i64) -> SourceFingerprint {
        SourceFingerprint {
            path: PathBuf::from("a.mpq"),
            size,
            modified_unix_ms: mtime,
            entries: 3,
        }
    }

    #[test]
    fn cache_reuse_requires_every_fingerprint_to_match() {
        let a = vec![fp(10, 1), fp(20, 2)];
        assert!(fingerprints_match(&a, &a.clone()));
        assert!(!fingerprints_match(&a, &[fp(10, 1), fp(21, 2)]), "size");
        assert!(!fingerprints_match(&a, &[fp(10, 1), fp(20, 3)]), "mtime");
        assert!(!fingerprints_match(&a, &[fp(10, 1)]), "fewer layers");
    }

    #[test]
    fn cache_round_trip_and_invalidation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        let mut names = KnownNames::new();
        names.insert(GamePath::new("data\\global\\tiles\\a.ds1"));
        let fps = vec![fp(1, 1)];
        store_cache(&paths, "p", &names, fps.clone()).expect("store");
        let back = load_cached(&paths, "p", &fps).expect("hit");
        assert_eq!(back.len(), 1);
        assert!(
            load_cached(&paths, "p", &[fp(2, 1)]).is_none(),
            "changed archive"
        );
        assert!(
            load_cached(&paths, "other", &fps).is_none(),
            "other profile"
        );
        // A corrupt fingerprint file is a miss, not an error.
        std::fs::write(paths.names_fingerprints("p"), "{").expect("write");
        assert!(load_cached(&paths, "p", &fps).is_none());
    }

    #[test]
    fn ds1_list_filters_and_sorts() {
        let mut names = KnownNames::new();
        for n in ["data\\b.ds1", "data\\a.DS1", "data\\x.dt1", "data\\c.txt"] {
            names.insert(GamePath::new(n));
        }
        let list = ds1_list(&names);
        let strs: Vec<&str> = list.iter().map(GamePath::as_str).collect();
        assert_eq!(strs, vec!["data\\a.ds1", "data\\b.ds1"]);
    }
}
