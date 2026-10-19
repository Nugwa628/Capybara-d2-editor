//! The asset index: open, staleness, lookups. Building lives in `build.rs`
//! and searching in `search.rs`.

use std::path::{Path, PathBuf};

use d2_assets::Overlay;
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{IndexError, Result};
use crate::query::AssetKind;
use crate::schema::{GENERATOR_VERSION, SCHEMA, SCHEMA_VERSION};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexStatus {
    /// Nothing indexed for this profile yet.
    Missing,
    UpToDate,
    /// Some layer's fingerprint differs from the one the index was built from.
    Stale {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildPhase {
    Listing,
    Tables,
    Maps,
    Tiles,
    Sprites,
    Thumbnails,
    Writing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuildProgress {
    pub phase: BuildPhase,
    pub done: u64,
    pub total: u64,
    pub current: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildReport {
    pub files: u64,
    pub assets: u64,
    pub tiles: u64,
    pub warnings: Vec<String>,
    pub millis: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetRow {
    pub id: i64,
    pub key: String,
    pub kind: AssetKind,
    pub path: String,
    pub act: Option<u8>,
    pub orientation: Option<u8>,
    pub main: Option<u8>,
    pub sub: Option<u8>,
    pub variant: Option<u32>,
    pub rarity: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub class: Option<String>,
    pub gen_name: String,
    pub color: Option<String>,
    pub texture: Option<String>,
    pub transparency: Option<f32>,
    pub src: String,
    pub src_tag: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub asset: AssetRow,
    pub score: f32,
    /// The user's alias for this asset, when it has one.
    pub alias: Option<String>,
}

/// `Send` but not `Sync`: the app opens one `Index` per thread.
pub struct Index {
    pub(crate) path: PathBuf,
    pub(crate) conn: Connection,
    /// The map catalog of the last build or refresh (per profile id), so
    /// `refresh_saves` does not have to read every tileset again.
    pub(crate) catalog: Option<(String, d2_assets::MapCatalog)>,
}

/// Columns of [`AssetRow`], aliased `a`.
pub(crate) const ASSET_COLS: &str = "a.id, a.key, a.kind, a.path, a.act, a.orientation, a.main, a.sub, \
     a.variant, a.rarity, a.width, a.height, a.class, a.gen_name, a.color, a.texture, \
     a.transparency, a.src, a.src_tag";

fn opt_int<T: TryFrom<i64>>(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<T>> {
    Ok(row
        .get::<_, Option<i64>>(idx)?
        .and_then(|v| T::try_from(v).ok()))
}

pub(crate) fn asset_from_row(row: &Row<'_>) -> rusqlite::Result<AssetRow> {
    let kind_text: String = row.get(2)?;
    let kind = AssetKind::parse(&kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            format!("unknown asset kind {kind_text:?}").into(),
        )
    })?;
    Ok(AssetRow {
        id: row.get(0)?,
        key: row.get(1)?,
        kind,
        path: row.get(3)?,
        act: opt_int(row, 4)?,
        orientation: opt_int(row, 5)?,
        main: opt_int(row, 6)?,
        sub: opt_int(row, 7)?,
        variant: opt_int(row, 8)?,
        rarity: opt_int(row, 9)?,
        width: opt_int(row, 10)?,
        height: opt_int(row, 11)?,
        class: row.get(12)?,
        gen_name: row.get(13)?,
        color: row.get(14)?,
        texture: row.get(15)?,
        #[allow(clippy::cast_possible_truncation)]
        transparency: row.get::<_, Option<f64>>(16)?.map(|t| t as f32),
        src: row.get(17)?,
        src_tag: row.get(18)?,
    })
}

fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    // Hamming distance between two 64-bit hashes stored as signed integers.
    conn.create_scalar_function(
        "hamming",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let a: Option<i64> = ctx.get(0)?;
            let b: Option<i64> = ctx.get(1)?;
            Ok(match (a, b) {
                (Some(a), Some(b)) => Some(i64::from((a ^ b).count_ones())),
                _ => None,
            })
        },
    )
}

/// Open `path`; `Ok(None)` when it exists but is not the current schema.
fn open_connection(path: &Path) -> rusqlite::Result<Option<Connection>> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    register_functions(&conn)?;
    // WAL lets the UI thread read while a build writes. It is a no-op error on
    // read-only media, which the app never uses for the cache.
    let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY;")?;
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
        [],
        |r| r.get(0),
    )?;
    if !has_meta {
        let empty: bool =
            conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM sqlite_master)", [], |r| {
                r.get(0)
            })?;
        if empty {
            create_schema(&conn)?;
            return Ok(Some(conn));
        }
        return Ok(None);
    }
    let version: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if version.as_deref() == Some(SCHEMA_VERSION.to_string().as_str()) {
        Ok(Some(conn))
    } else {
        Ok(None)
    }
}

fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('schema_version', ?1)",
        [SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

fn remove_db_files(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

impl Index {
    /// Open `cache_dir\index.db`, creating it (and the folder) when absent.
    /// A file with another schema version, or one that is not a database, is
    /// deleted and recreated empty: the cache is always safe to lose.
    pub fn open(cache_dir: &Path) -> Result<Index> {
        std::fs::create_dir_all(cache_dir).map_err(|e| {
            IndexError::Other(format!("cannot create {}: {e}", cache_dir.display()))
        })?;
        let path = cache_dir.join("index.db");
        let first = open_connection(&path);
        let conn = match first {
            Ok(Some(conn)) => conn,
            Ok(None) | Err(_) => {
                remove_db_files(&path);
                match open_connection(&path) {
                    Ok(Some(conn)) => conn,
                    Ok(None) => {
                        return Err(IndexError::Other(format!(
                            "{}: could not create a fresh index",
                            path.display()
                        )));
                    }
                    Err(e) => return Err(IndexError::sql(&path, e)),
                }
            }
        };
        Ok(Index {
            path,
            conn,
            catalog: None,
        })
    }

    pub(crate) fn sql(&self, e: rusqlite::Error) -> IndexError {
        IndexError::sql(&self.path, e)
    }

    /// Where the database file lives.
    pub fn db_path(&self) -> &Path {
        &self.path
    }

    /// Whether the stored index for `profile_id` matches the overlay's
    /// current archive fingerprints (size, modified time, entry count).
    pub fn status(&self, profile_id: &str, overlay: &Overlay) -> Result<IndexStatus> {
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT fingerprint FROM profiles WHERE id = ?1",
                [profile_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.sql(e))?;
        let Some(stored) = stored else {
            return Ok(IndexStatus::Missing);
        };
        let current = fingerprint_text(overlay)?;
        Ok(if stored == current {
            IndexStatus::UpToDate
        } else {
            IndexStatus::Stale {
                reason: describe_difference(&stored, &current),
            }
        })
    }

    pub fn asset(&self, profile_id: &str, key: &str) -> Result<Option<AssetRow>> {
        let sql = format!("SELECT {ASSET_COLS} FROM assets a WHERE a.profile = ?1 AND a.key = ?2");
        self.conn
            .query_row(&sql, params![profile_id, key.to_ascii_lowercase()], |r| {
                asset_from_row(r)
            })
            .optional()
            .map_err(|e| self.sql(e))
    }

    /// PNG bytes of the thumbnail (longest side at most 96 px).
    pub fn thumbnail(&self, asset_id: i64) -> Result<Option<Vec<u8>>> {
        let blob: Option<Option<Vec<u8>>> = self
            .conn
            .query_row("SELECT thumb FROM assets WHERE id = ?1", [asset_id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| self.sql(e))?;
        Ok(blob.flatten())
    }

    /// Every tile with this key, in DT1 path and file order.
    pub fn tiles_with_key(
        &self,
        profile_id: &str,
        orientation: u8,
        main: u8,
        sub: u8,
    ) -> Result<Vec<AssetRow>> {
        let sql = format!(
            "SELECT {ASSET_COLS} FROM assets a
             WHERE a.profile = ?1 AND a.kind = 'tile' AND a.orientation = ?2 AND a.main = ?3 AND a.sub = ?4
             ORDER BY a.path, a.variant, a.id"
        );
        let mut stmt = self.conn.prepare_cached(&sql).map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map(params![profile_id, orientation, main, sub], asset_from_row)
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| self.sql(e))
    }

    pub fn count(&self, profile_id: &str, kind: Option<AssetKind>) -> Result<u64> {
        let n: i64 = match kind {
            Some(kind) => self.conn.query_row(
                "SELECT COUNT(*) FROM assets WHERE profile = ?1 AND kind = ?2",
                params![profile_id, kind.as_str()],
                |r| r.get(0),
            ),
            None => self.conn.query_row(
                "SELECT COUNT(*) FROM assets WHERE profile = ?1",
                [profile_id],
                |r| r.get(0),
            ),
        }
        .map_err(|e| self.sql(e))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// LvlTypes rows that load a DT1 asset: `(lvltype id, slot)`, slot being
    /// the 0-based File index (bit `slot` of a LvlPrest `Dt1Mask`).
    pub fn lvltypes_of(&self, asset_id: i64) -> Result<Vec<(u32, u32)>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT lvltype, slot FROM asset_lvltypes WHERE asset_id = ?1 ORDER BY lvltype, slot",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map([asset_id], |r| {
                Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?))
            })
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| self.sql(e))
    }

    /// DS1 paths that use a tile, with how many cells use it, most first.
    pub fn used_in(&self, asset_id: i64) -> Result<Vec<(String, u32)>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT ds1_path, cells FROM used_in WHERE asset_id = ?1 ORDER BY cells DESC, ds1_path",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map([asset_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
            })
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| self.sql(e))
    }
}

/// One line per layer: `path|size|modified|entries`, after a generator line.
/// The profile's saves layer is left out: saving a map must not make the
/// index stale (`refresh_saves` picks saved maps up).
pub(crate) fn fingerprint_text(overlay: &Overlay) -> Result<String> {
    let mut text = format!("generator {GENERATOR_VERSION}\n");
    for (layer, fp) in overlay.layers().iter().zip(overlay.fingerprints()?) {
        if crate::build::src_tag(layer.name()) == "saves" {
            continue;
        }
        text.push_str(&format!(
            "{}|{}|{}|{}\n",
            fp.path.display(),
            fp.size,
            fp.modified_unix_ms,
            fp.entries
        ));
    }
    Ok(text)
}

fn layer_name(line: &str) -> String {
    let path = line.rsplitn(4, '|').last().unwrap_or(line);
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Human-readable first difference between two fingerprint texts.
fn describe_difference(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    if old_lines.first() != new_lines.first() {
        return "the index format changed".to_owned();
    }
    if old_lines.len() != new_lines.len() {
        return format!(
            "the layer list changed ({} layers before, {} now)",
            old_lines.len().saturating_sub(1),
            new_lines.len().saturating_sub(1)
        );
    }
    for (a, b) in old_lines.iter().zip(&new_lines).skip(1) {
        if a == b {
            continue;
        }
        let name = layer_name(b);
        let fields = |line: &str| -> Option<(u64, i64, u64)> {
            let mut it = line.rsplitn(4, '|');
            let entries = it.next()?.parse().ok()?;
            let modified = it.next()?.parse().ok()?;
            let size = it.next()?.parse().ok()?;
            Some((size, modified, entries))
        };
        return match (fields(a), fields(b)) {
            (Some((s1, m1, e1)), Some((s2, m2, e2))) => {
                let mut parts = Vec::new();
                if s1 != s2 {
                    parts.push(format!("size {s1} -> {s2}"));
                }
                if m1 != m2 {
                    parts.push("modified time changed".to_owned());
                }
                if e1 != e2 {
                    parts.push(format!("entries {e1} -> {e2}"));
                }
                if parts.is_empty() {
                    parts.push("path changed".to_owned());
                }
                format!("{name}: {}", parts.join(", "))
            }
            _ => format!("{name} changed"),
        };
    }
    "fingerprint changed".to_owned()
}
