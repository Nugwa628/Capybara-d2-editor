//! Unused-asset report and archive/restore plan (ds1-studio `assetUsage.ts`).
//!
//! The caller scans the game folder and passes the reference graph in: which
//! maps load which DT1s (from DS1 file lists, LvlTypes/LvlPrest rows, ...),
//! and which tile numbers each map places. The report is conservative: a
//! scan with errors, or a protected asset, is never called unused. Moves are
//! plans only; the app performs them inside `capybara\`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::tiles_dt1::normalize_path;
use crate::tiles_review::TileId;

/// One map as the scan saw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapRefs {
    /// Game path of the DS1.
    pub path: String,
    /// DT1 game paths the map loads (DS1 file list, level type, LvlPrest mask).
    pub dt1s: Vec<String>,
    /// Tile numbers it places (see `tiles_review::map_tile_usage`).
    pub used: BTreeSet<TileId>,
}

/// A DT1 and everything the graph says about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dt1Asset {
    pub path: String,
    /// Tile numbers of its tiles, in file order.
    pub tiles: Vec<TileId>,
    /// Maps that load it (even if they place none of its tiles).
    pub maps: Vec<String>,
    /// Numbers placed by those maps.
    pub used: BTreeSet<TileId>,
    /// Never reported (e.g. a table-only map's libraries whose map is missing).
    pub protected: bool,
}

/// A DS1 and what refers to it (LvlPrest rows, other tables, other maps).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Asset {
    pub path: String,
    pub referenced_by: Vec<String>,
    pub protected: bool,
}

/// The reference graph the caller passes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceGraph {
    pub dt1s: Vec<Dt1Asset>,
    pub ds1s: Vec<Ds1Asset>,
    /// Anything unreadable during the scan: cleanup is then unavailable.
    pub errors: Vec<String>,
}

/// Builds the DT1 side of the graph from the scanned maps. `dt1_tiles` maps
/// every known DT1 (normalized path) to the numbers of its tiles. DT1s named
/// by `protected` are marked protected.
pub fn build_dt1_graph(
    maps: &[MapRefs],
    dt1_tiles: &BTreeMap<String, Vec<TileId>>,
    protected: &BTreeSet<String>,
) -> Vec<Dt1Asset> {
    let mut by: BTreeMap<String, Dt1Asset> = dt1_tiles
        .iter()
        .map(|(p, tiles)| {
            let p = normalize_path(p);
            (
                p.clone(),
                Dt1Asset {
                    path: p.clone(),
                    tiles: tiles.clone(),
                    protected: protected.contains(&p),
                    ..Dt1Asset::default()
                },
            )
        })
        .collect();
    for m in maps {
        for d in &m.dt1s {
            let key = normalize_path(d);
            let a = by.entry(key.clone()).or_insert_with(|| Dt1Asset {
                path: key,
                ..Dt1Asset::default()
            });
            if !a.maps.contains(&m.path) {
                a.maps.push(m.path.clone());
            }
            a.used.extend(m.used.iter().copied());
        }
    }
    by.into_values().collect()
}

/// A DT1 some maps load but not all of whose tiles are placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartlyUsed {
    pub path: String,
    /// Tile indices (file order) whose number no referencing map places.
    pub unused_tiles: Vec<usize>,
}

/// The unused-asset report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnusedReport {
    /// DT1s no map loads.
    pub unused_dt1s: Vec<String>,
    /// DS1s nothing refers to.
    pub unused_ds1s: Vec<String>,
    /// Loaded DT1s with tiles nothing places.
    pub partly_used: Vec<PartlyUsed>,
    /// Protected assets that were left out.
    pub protected: Vec<String>,
    /// Why cleanup is unavailable (the graph had errors); the lists are then empty.
    pub blocked: Vec<String>,
}

/// Which assets nothing references. Conservative: errors in the scan block
/// every list; protected assets are listed under `protected` only.
pub fn unused_report(graph: &ReferenceGraph) -> UnusedReport {
    let mut r = UnusedReport::default();
    if !graph.errors.is_empty() {
        r.blocked = graph.errors.clone();
        return r;
    }
    for d in &graph.dt1s {
        if d.protected {
            r.protected.push(d.path.clone());
        } else if d.maps.is_empty() {
            r.unused_dt1s.push(d.path.clone());
        } else {
            let unused: Vec<usize> = d
                .tiles
                .iter()
                .enumerate()
                .filter(|(_, id)| !d.used.contains(id))
                .map(|(i, _)| i)
                .collect();
            if !unused.is_empty() {
                r.partly_used.push(PartlyUsed {
                    path: d.path.clone(),
                    unused_tiles: unused,
                });
            }
        }
    }
    for s in &graph.ds1s {
        if s.protected {
            r.protected.push(s.path.clone());
        } else if s.referenced_by.is_empty() {
            r.unused_ds1s.push(s.path.clone());
        }
    }
    r
}

// ───────────── archive / restore ─────────────

/// Where archived files go, inside the tool's folder.
pub const ARCHIVE_DIR: &str = "capybara/archive";

/// One planned move (paths relative to the mod folder, `/` separators).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedMove {
    pub from: String,
    pub to: String,
}

/// What an archive run did, kept so it can be restored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveManifest {
    /// Free-form stamp (a timestamp the app supplies).
    pub stamp: String,
    pub entries: Vec<PlannedMove>,
}

impl ArchiveManifest {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<ArchiveManifest> {
        serde_json::from_str(text).ok()
    }
}

/// An archive plan: the moves and the manifest to save with them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchivePlan {
    pub moves: Vec<PlannedMove>,
    pub manifest: ArchiveManifest,
    /// Paths refused, with why.
    pub refused: Vec<String>,
}

fn clean(path: &str) -> Option<String> {
    let p = path.replace('\\', "/");
    let p = p.trim_start_matches("./").trim_start_matches('/');
    let ok = !p.is_empty()
        && !p
            .split('/')
            .any(|s| s.is_empty() || s == ".." || s == "." || s.contains(':'));
    ok.then(|| p.to_string())
}

/// Plans moving `paths` to `capybara/archive/<stamp>/<path>`. Paths already
/// inside `capybara/`, escaping paths and duplicates are refused.
pub fn plan_archive(paths: &[String], stamp: &str) -> ArchivePlan {
    let mut plan = ArchivePlan {
        manifest: ArchiveManifest {
            stamp: stamp.to_string(),
            entries: Vec::new(),
        },
        ..ArchivePlan::default()
    };
    let stamp_dir = clean(stamp).unwrap_or_else(|| "archive".into());
    let mut seen = BTreeSet::new();
    for p in paths {
        let Some(c) = clean(p) else {
            plan.refused
                .push(format!("{p}: not a path inside the mod folder"));
            continue;
        };
        let key = c.to_ascii_lowercase();
        if key.starts_with("capybara/") {
            plan.refused.push(format!("{p}: already inside capybara\\"));
            continue;
        }
        if !seen.insert(key) {
            continue;
        }
        let mv = PlannedMove {
            from: c.clone(),
            to: format!("{ARCHIVE_DIR}/{stamp_dir}/{c}"),
        };
        plan.manifest.entries.push(mv.clone());
        plan.moves.push(mv);
    }
    plan
}

/// A restore plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestorePlan {
    pub moves: Vec<PlannedMove>,
    /// Originals that exist again (restoring would overwrite them): skipped.
    pub conflicts: Vec<String>,
    /// Entries whose archived file is gone.
    pub missing: Vec<String>,
}

/// Plans moving the archived files back. `exists` answers whether a path is
/// currently on disk; an original that exists is a conflict and is left alone.
pub fn plan_restore(manifest: &ArchiveManifest, exists: &dyn Fn(&str) -> bool) -> RestorePlan {
    let mut plan = RestorePlan::default();
    for e in &manifest.entries {
        if clean(&e.from).is_none() || clean(&e.to).is_none() || !e.to.starts_with(ARCHIVE_DIR) {
            plan.missing
                .push(format!("{}: invalid manifest entry", e.from));
        } else if !exists(&e.to) {
            plan.missing.push(e.to.clone());
        } else if exists(&e.from) {
            plan.conflicts.push(e.from.clone());
        } else {
            plan.moves.push(PlannedMove {
                from: e.to.clone(),
                to: e.from.clone(),
            });
        }
    }
    plan
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn id(m: i32) -> TileId {
        TileId::new(0, m, 0)
    }

    #[test]
    fn report() {
        let maps = vec![MapRefs {
            path: "a.ds1".into(),
            dt1s: vec!["Data\\Global\\Tiles\\x.dt1".into()],
            used: [id(1)].into(),
        }];
        let tiles: BTreeMap<String, Vec<TileId>> = [
            ("data/global/tiles/x.dt1".to_string(), vec![id(1), id(2)]),
            ("data/global/tiles/y.dt1".to_string(), vec![id(1)]),
            ("data/global/tiles/z.dt1".to_string(), vec![id(1)]),
        ]
        .into();
        let prot: BTreeSet<String> = ["data/global/tiles/z.dt1".to_string()].into();
        let graph = ReferenceGraph {
            dt1s: build_dt1_graph(&maps, &tiles, &prot),
            ds1s: vec![
                Ds1Asset {
                    path: "a.ds1".into(),
                    referenced_by: vec!["lvlprest".into()],
                    protected: false,
                },
                Ds1Asset {
                    path: "b.ds1".into(),
                    ..Ds1Asset::default()
                },
            ],
            errors: vec![],
        };
        let r = unused_report(&graph);
        assert_eq!(r.unused_dt1s, vec!["data/global/tiles/y.dt1"]);
        assert_eq!(r.unused_ds1s, vec!["b.ds1"]);
        assert_eq!(
            r.partly_used,
            vec![PartlyUsed {
                path: "data/global/tiles/x.dt1".into(),
                unused_tiles: vec![1]
            }]
        );
        assert_eq!(r.protected, vec!["data/global/tiles/z.dt1"]);
        let mut bad = graph;
        bad.errors.push("c.ds1: truncated".into());
        let r = unused_report(&bad);
        assert!(r.unused_dt1s.is_empty() && !r.blocked.is_empty());
    }

    #[test]
    fn archive_and_restore() {
        let paths = vec![
            "data/global/tiles/y.dt1".to_string(),
            "../evil".into(),
            "capybara/x".into(),
        ];
        let plan = plan_archive(&paths, "2026-10-20");
        assert_eq!(plan.moves.len(), 1);
        assert_eq!(plan.refused.len(), 2);
        assert_eq!(
            plan.moves[0].to,
            "capybara/archive/2026-10-20/data/global/tiles/y.dt1"
        );
        let m = ArchiveManifest::from_json(&plan.manifest.to_json()).unwrap();
        assert_eq!(m, plan.manifest);
        let r = plan_restore(&m, &|p| p.starts_with("capybara/"));
        assert_eq!(r.moves.len(), 1);
        assert_eq!(r.moves[0].to, "data/global/tiles/y.dt1");
        let r = plan_restore(&m, &|_| true);
        assert_eq!(r.conflicts.len(), 1);
        let r = plan_restore(&m, &|_| false);
        assert_eq!(r.missing.len(), 1);
    }
}
