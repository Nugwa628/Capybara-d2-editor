//! The app's per-profile LvlType/Def overrides, read from
//! `capybara\config.toml` (`[[overrides]]` with `profile`, `ds1`,
//! `lvl_type` and an optional `def`), and applied to a [`MapCatalog`] the
//! same way the app does when a profile opens.
//!
//! Reading is lenient: a missing file or a missing `overrides` array means
//! no overrides; an unreadable or malformed file, or a malformed entry, is
//! skipped with a warning. The CLI never writes this file.

use std::path::Path;

use d2_assets::{GamePath, MapCatalog};

use crate::Ctx;

/// File name of the app's settings in the `capybara\` root.
pub const CONFIG_FILE: &str = "config.toml";

/// One saved override of the profile in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapOverride {
    pub ds1: GamePath,
    pub lvl_type: u32,
    pub def: Option<u32>,
}

/// A non-negative integer that fits `u32`.
fn as_u32(value: &toml::Value) -> Option<u32> {
    value.as_integer().and_then(|v| u32::try_from(v).ok())
}

/// The overrides of `profile_id` in the text of a `config.toml`, plus one
/// warning per problem found. Entries of other profiles are ignored without
/// being checked beyond their `profile` key.
pub fn parse(text: &str, profile_id: &str) -> (Vec<MapOverride>, Vec<String>) {
    let mut warnings = Vec::new();
    let table: toml::Table = match toml::from_str(text) {
        Ok(table) => table,
        Err(e) => {
            warnings.push(format!(
                "{CONFIG_FILE} is not valid TOML, its map overrides are ignored: {}",
                e.message()
            ));
            return (Vec::new(), warnings);
        }
    };
    let entries = match table.get("overrides") {
        None => return (Vec::new(), warnings),
        Some(toml::Value::Array(entries)) => entries,
        Some(_) => {
            warnings.push(format!(
                "{CONFIG_FILE}: `overrides` is not an array of tables; ignored"
            ));
            return (Vec::new(), warnings);
        }
    };
    let mut out: Vec<MapOverride> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_table() else {
            warnings.push(format!(
                "{CONFIG_FILE}: overrides entry {} is not a table; ignored",
                i + 1
            ));
            continue;
        };
        if entry.get("profile").and_then(toml::Value::as_str) != Some(profile_id) {
            continue;
        }
        let ds1 = entry
            .get("ds1")
            .and_then(toml::Value::as_str)
            .filter(|s| !s.trim().is_empty());
        let lvl_type = entry.get("lvl_type").and_then(as_u32);
        let def = match entry.get("def") {
            None => Ok(None),
            Some(v) => as_u32(v).map(Some).ok_or(()),
        };
        match (ds1, lvl_type, def) {
            (Some(ds1), Some(lvl_type), Ok(def)) => {
                let ds1 = GamePath::new(ds1);
                // A later entry for the same map replaces an earlier one.
                out.retain(|o| o.ds1 != ds1);
                out.push(MapOverride { ds1, lvl_type, def });
            }
            _ => warnings.push(format!(
                "{CONFIG_FILE}: overrides entry {} for profile {profile_id:?} needs `ds1` \
                 (text), `lvl_type` and optionally `def` (non-negative integers); ignored",
                i + 1
            )),
        }
    }
    (out, warnings)
}

/// The overrides of `profile_id` in `config_file`; a missing file has none.
pub fn load_from(config_file: &Path, profile_id: &str) -> (Vec<MapOverride>, Vec<String>) {
    match std::fs::read_to_string(config_file) {
        Ok(text) => parse(&text, profile_id),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), Vec::new()),
        Err(e) => (
            Vec::new(),
            vec![format!(
                "cannot read {}, its map overrides are ignored: {e}",
                config_file.display()
            )],
        ),
    }
}

/// Reads `capybara\config.toml` and feeds the overrides of `profile_id` to
/// `catalog` (`MapCatalog::set_override`), warning about any problem.
pub fn apply(ctx: &mut Ctx<'_>, catalog: &mut MapCatalog, profile_id: &str) {
    let (overrides, warnings) = load_from(&ctx.root.join(CONFIG_FILE), profile_id);
    for w in &warnings {
        ctx.warn(w);
    }
    for o in overrides {
        catalog.set_override(o.ds1, o.lvl_type, o.def);
    }
}
