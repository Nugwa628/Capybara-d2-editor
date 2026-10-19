//! The known-names list of a profile, cached in `capybara\cache\` in the
//! same files and format the app uses (`known-names-<id>.txt` and
//! `known-names-<id>.json`, the latter holding the layers' fingerprints), so
//! the CLI and the GUI reuse each other's harvest.

use d2_assets::{KnownNames, Overlay, Profile, SourceFingerprint, SourceKind};
use serde::{Deserialize, Serialize};

use crate::{Ctx, Fail};

/// Name prefix of the profile's own saves layer, whose contents change on
/// every save and so are excluded from the cache check.
const SAVES_PREFIX: &str = "saves\\";
const FINGERPRINT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct FingerprintFile {
    version: u32,
    fingerprints: Vec<SourceFingerprint>,
}

fn fingerprints(overlay: &Overlay) -> Result<Vec<SourceFingerprint>, Fail> {
    let mut out = Vec::new();
    for layer in overlay.layers() {
        if layer.kind() == SourceKind::Folder && layer.name().starts_with(SAVES_PREFIX) {
            continue;
        }
        out.push(
            layer
                .fingerprint()
                .map_err(|e| Fail::Runtime(format!("fingerprinting {}: {e}", layer.name())))?,
        );
    }
    Ok(out)
}

fn load_cached(ctx: &Ctx<'_>, id: &str, current: &[SourceFingerprint]) -> Option<KnownNames> {
    let cache = ctx.cache_dir();
    let text = std::fs::read_to_string(cache.join(format!("known-names-{id}.json"))).ok()?;
    let stored: FingerprintFile = serde_json::from_str(&text).ok()?;
    if stored.version != FINGERPRINT_VERSION || stored.fingerprints != current {
        return None;
    }
    KnownNames::load(&cache.join(format!("known-names-{id}.txt"))).ok()
}

fn store(
    ctx: &Ctx<'_>,
    id: &str,
    names: &KnownNames,
    fingerprints: Vec<SourceFingerprint>,
) -> Result<(), String> {
    let cache = ctx.cache_dir();
    let json_path = cache.join(format!("known-names-{id}.json"));
    // Invalidate first: a crash between the two writes must not leave a
    // fingerprint file that vouches for a half-written name list.
    match std::fs::remove_file(&json_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    names
        .save(&cache.join(format!("known-names-{id}.txt")))
        .map_err(|e| e.to_string())?;
    let file = FingerprintFile {
        version: FINGERPRINT_VERSION,
        fingerprints,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&json_path, json).map_err(|e| e.to_string())
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

/// Harvested (or cached) names of `profile`, plus the loose files of its
/// folder layers.
pub fn known_names(
    ctx: &mut Ctx<'_>,
    profile: &Profile,
    overlay: &Overlay,
) -> Result<KnownNames, Fail> {
    let current = fingerprints(overlay)?;
    let mut names = match load_cached(ctx, &profile.id, &current) {
        Some(names) => names,
        None => {
            let (names, stats) = KnownNames::harvest(overlay, profile.game_version)
                .map_err(|e| Fail::Runtime(format!("harvesting file names: {e}")))?;
            for w in stats.warnings.iter().take(5) {
                ctx.warn(w);
            }
            if let Err(e) = store(ctx, &profile.id, &names, current) {
                ctx.warn(format!("name cache not written: {e}"));
            }
            names
        }
    };
    add_folder_names(overlay, &mut names);
    Ok(names)
}
