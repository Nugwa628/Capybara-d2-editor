//! `user.db`: aliases, tags, notes, favorites, recents and saved searches.
//!
//! Rows are keyed by profile id plus the stable asset key, so they survive a
//! rebuild of the index. The file is never dropped: schema changes are
//! forward-only migrations.
//!
//! Ordering conventions:
//! * Group, favorite and saved-search positions are dense (0..n) after every
//!   write; `move_*` removes the item and re-inserts it at `new_position`
//!   (clamped to the end).
//! * Recents are newest first. Every `record_use` is strictly newer than all
//!   earlier uses of the same profile, even within one millisecond.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{IndexError, Result};

const SCHEMA_VERSION: i64 = 1;
/// Recents beyond this many per profile are forgotten.
const MAX_RECENTS: i64 = 1000;

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS aliases(profile TEXT NOT NULL, key TEXT NOT NULL, alias TEXT NOT NULL, PRIMARY KEY(profile, key));
CREATE TABLE IF NOT EXISTS tags(profile TEXT NOT NULL, key TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY(profile, key, tag));
CREATE TABLE IF NOT EXISTS notes(profile TEXT NOT NULL, key TEXT NOT NULL, note TEXT NOT NULL, PRIMARY KEY(profile, key));
CREATE TABLE IF NOT EXISTS favorite_groups(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, position INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS favorites(group_id INTEGER NOT NULL, profile TEXT NOT NULL, key TEXT NOT NULL, position INTEGER NOT NULL,
  PRIMARY KEY(group_id, profile, key));
CREATE TABLE IF NOT EXISTS recents(profile TEXT NOT NULL, key TEXT NOT NULL, last_used_unix_ms INTEGER NOT NULL, uses INTEGER NOT NULL,
  PRIMARY KEY(profile, key));
CREATE TABLE IF NOT EXISTS saved_searches(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, query TEXT NOT NULL,
  position INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS tags_by_tag ON tags(profile, tag COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS recents_by_time ON recents(profile, last_used_unix_ms);
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FavoriteGroup {
    pub id: i64,
    pub name: String,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSearch {
    pub id: i64,
    pub name: String,
    pub query: String,
    pub position: i64,
    pub pinned: bool,
}

pub struct UserDb {
    path: PathBuf,
    conn: Connection,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

/// Re-insert `id` at `new_position` among `ids` (ids in current order).
fn reorder(ids: &mut Vec<i64>, id: i64, new_position: i64) -> bool {
    let Some(from) = ids.iter().position(|x| *x == id) else {
        return false;
    };
    ids.remove(from);
    let to = usize::try_from(new_position).unwrap_or(0).min(ids.len());
    ids.insert(to, id);
    true
}

impl UserDb {
    /// Open (creating the file and its folder if needed) and migrate.
    pub fn open(path: &Path) -> Result<UserDb> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| {
                IndexError::Other(format!("cannot create {}: {e}", parent.display()))
            })?;
        }
        let conn = Connection::open(path).map_err(|e| IndexError::sql(path, e))?;
        let db = UserDb {
            path: path.to_path_buf(),
            conn,
        };
        db.conn
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| db.sql(e))?;
        db.migrate()?;
        Ok(db)
    }

    fn sql(&self, e: rusqlite::Error) -> IndexError {
        IndexError::sql(&self.path, e)
    }

    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .map_err(|e| self.sql(e))?;
        let found: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.sql(e))?;
        let version = match found {
            Some(text) => text.trim().parse::<i64>().map_err(|_| {
                IndexError::Other(format!(
                    "{}: unreadable schema_version {text:?}",
                    self.path.display()
                ))
            })?,
            None => 0,
        };
        if version > SCHEMA_VERSION {
            return Err(IndexError::Other(format!(
                "{} was written by a newer editor (schema {version}, this build knows {SCHEMA_VERSION})",
                self.path.display()
            )));
        }
        if version < 1 {
            self.conn
                .execute_batch(SCHEMA_V1)
                .map_err(|e| self.sql(e))?;
        }
        // Forward-only migrations go here: `if version < 2 { ... }`.
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [SCHEMA_VERSION.to_string()],
            )
            .map_err(|e| self.sql(e))?;
        Ok(())
    }

    /// The stored schema version.
    pub fn schema_version(&self) -> Result<i64> {
        let text: String = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| self.sql(e))?;
        text.trim()
            .parse()
            .map_err(|_| IndexError::Other(format!("unreadable schema_version {text:?}")))
    }

    fn txn<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let run = || -> rusqlite::Result<T> {
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            match f(&self.conn) {
                Ok(v) => {
                    self.conn.execute_batch("COMMIT")?;
                    Ok(v)
                }
                Err(e) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        };
        run().map_err(|e| self.sql(e))
    }

    // ---------------------------------------------------------------- aliases

    pub fn alias(&self, profile: &str, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT alias FROM aliases WHERE profile = ?1 AND key = ?2",
                params![profile, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.sql(e))
    }

    /// Set the alias; `None` or blank removes it.
    pub fn set_alias(&self, profile: &str, key: &str, alias: Option<&str>) -> Result<()> {
        match alias.map(str::trim).filter(|a| !a.is_empty()) {
            Some(alias) => self.conn.execute(
                "INSERT INTO aliases(profile, key, alias) VALUES(?1, ?2, ?3)
                 ON CONFLICT(profile, key) DO UPDATE SET alias = excluded.alias",
                params![profile, key, alias],
            ),
            None => self.conn.execute(
                "DELETE FROM aliases WHERE profile = ?1 AND key = ?2",
                params![profile, key],
            ),
        }
        .map(drop)
        .map_err(|e| self.sql(e))
    }

    /// Every alias of a profile, key to alias (used by search).
    pub(crate) fn aliases(&self, profile: &str) -> Result<HashMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, alias FROM aliases WHERE profile = ?1")
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map([profile], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(|e| self.sql(e))
    }

    // ------------------------------------------------------------------- tags

    /// Tags of one asset, sorted case-insensitively.
    pub fn tags(&self, profile: &str, key: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT tag FROM tags WHERE profile = ?1 AND key = ?2 ORDER BY tag COLLATE NOCASE, tag",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map(params![profile, key], |r| r.get(0))
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| self.sql(e))
    }

    /// Add a tag. Tags are trimmed; a tag that already exists in any letter
    /// case is not added again. A blank tag is an error.
    pub fn add_tag(&self, profile: &str, key: &str, tag: &str) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() {
            return Err(IndexError::Other("a tag cannot be empty".to_owned()));
        }
        self.conn
            .execute(
                "INSERT INTO tags(profile, key, tag)
                 SELECT ?1, ?2, ?3
                 WHERE NOT EXISTS (SELECT 1 FROM tags
                                   WHERE profile = ?1 AND key = ?2 AND tag = ?3 COLLATE NOCASE)",
                params![profile, key, tag],
            )
            .map(drop)
            .map_err(|e| self.sql(e))
    }

    /// Remove a tag (case-insensitive); a missing tag is not an error.
    pub fn remove_tag(&self, profile: &str, key: &str, tag: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM tags WHERE profile = ?1 AND key = ?2 AND tag = ?3 COLLATE NOCASE",
                params![profile, key, tag.trim()],
            )
            .map(drop)
            .map_err(|e| self.sql(e))
    }

    /// Keys carrying `tag` (case-insensitive), sorted.
    pub fn keys_with_tag(&self, profile: &str, tag: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT key FROM tags WHERE profile = ?1 AND tag = ?2 COLLATE NOCASE ORDER BY key",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map(params![profile, tag.trim()], |r| r.get(0))
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| self.sql(e))
    }

    // ------------------------------------------------------------------ notes

    pub fn note(&self, profile: &str, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT note FROM notes WHERE profile = ?1 AND key = ?2",
                params![profile, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.sql(e))
    }

    /// Set the note; `None` or an empty string removes it.
    pub fn set_note(&self, profile: &str, key: &str, note: Option<&str>) -> Result<()> {
        match note.filter(|n| !n.trim().is_empty()) {
            Some(note) => self.conn.execute(
                "INSERT INTO notes(profile, key, note) VALUES(?1, ?2, ?3)
                 ON CONFLICT(profile, key) DO UPDATE SET note = excluded.note",
                params![profile, key, note],
            ),
            None => self.conn.execute(
                "DELETE FROM notes WHERE profile = ?1 AND key = ?2",
                params![profile, key],
            ),
        }
        .map(drop)
        .map_err(|e| self.sql(e))
    }

    // -------------------------------------------------------------- favorites

    /// Groups in display order.
    pub fn favorite_groups(&self) -> Result<Vec<FavoriteGroup>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, position FROM favorite_groups ORDER BY position, id")
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FavoriteGroup {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    position: r.get(2)?,
                })
            })
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| self.sql(e))
    }

    fn group_exists(&self, id: i64) -> Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM favorite_groups WHERE id = ?1", [id], |_| {
                Ok(())
            })
            .optional()
            .map(|o| o.is_some())
            .map_err(|e| self.sql(e))
    }

    fn require_group(&self, id: i64) -> Result<()> {
        if self.group_exists(id)? {
            Ok(())
        } else {
            Err(IndexError::Other(format!("no favorite group with id {id}")))
        }
    }

    /// Append a group. Names are trimmed and unique.
    pub fn create_group(&self, name: &str) -> Result<FavoriteGroup> {
        let name = name.trim();
        if name.is_empty() {
            return Err(IndexError::Other("a group name cannot be empty".to_owned()));
        }
        let created = self.txn(|c| {
            let position: i64 = c.query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM favorite_groups",
                [],
                |r| r.get(0),
            )?;
            c.execute(
                "INSERT INTO favorite_groups(name, position) VALUES(?1, ?2)",
                params![name, position],
            )?;
            Ok(FavoriteGroup {
                id: c.last_insert_rowid(),
                name: name.to_owned(),
                position,
            })
        });
        match created {
            Err(IndexError::Sql { source, .. }) if is_unique_violation(&source) => Err(
                IndexError::Other(format!("a favorite group named {name:?} already exists")),
            ),
            other => other,
        }
    }

    pub fn rename_group(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(IndexError::Other("a group name cannot be empty".to_owned()));
        }
        self.require_group(id)?;
        match self.conn.execute(
            "UPDATE favorite_groups SET name = ?2 WHERE id = ?1",
            params![id, name],
        ) {
            Ok(_) => Ok(()),
            Err(e) if is_unique_violation(&e) => Err(IndexError::Other(format!(
                "a favorite group named {name:?} already exists"
            ))),
            Err(e) => Err(self.sql(e)),
        }
    }

    /// Delete a group and its favorites; later groups move up.
    pub fn delete_group(&self, id: i64) -> Result<()> {
        self.txn(|c| {
            c.execute("DELETE FROM favorites WHERE group_id = ?1", [id])?;
            c.execute("DELETE FROM favorite_groups WHERE id = ?1", [id])?;
            Self::renumber(
                c,
                "SELECT id FROM favorite_groups ORDER BY position, id",
                "UPDATE favorite_groups SET position = ?2 WHERE id = ?1",
                [],
            )
        })
    }

    /// Move a group to `new_position` (clamped); the others shift.
    pub fn move_group(&self, id: i64, new_position: i64) -> Result<()> {
        self.require_group(id)?;
        self.txn(|c| {
            let mut ids = Self::ids(
                c,
                "SELECT id FROM favorite_groups ORDER BY position, id",
                [],
            )?;
            reorder(&mut ids, id, new_position);
            Self::write_positions(
                c,
                "UPDATE favorite_groups SET position = ?2 WHERE id = ?1",
                &ids,
            )
        })
    }

    fn ids<P: rusqlite::Params>(c: &Connection, sql: &str, p: P) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = c.prepare(sql)?;
        let rows = stmt.query_map(p, |r| r.get(0))?;
        rows.collect()
    }

    fn write_positions(c: &Connection, sql: &str, ids: &[i64]) -> rusqlite::Result<()> {
        let mut stmt = c.prepare(sql)?;
        for (pos, id) in (0_i64..).zip(ids) {
            stmt.execute(params![id, pos])?;
        }
        Ok(())
    }

    fn renumber<P: rusqlite::Params>(
        c: &Connection,
        select: &str,
        update: &str,
        p: P,
    ) -> rusqlite::Result<()> {
        let ids = Self::ids(c, select, p)?;
        Self::write_positions(c, update, &ids)
    }

    /// `(profile, key)` pairs of a group, in order.
    pub fn favorites(&self, group_id: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT profile, key FROM favorites WHERE group_id = ?1 ORDER BY position, profile, key",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| self.sql(e))
    }

    /// Append to the group's end; already being a favorite is not an error
    /// and keeps the old position.
    pub fn add_favorite(&self, group_id: i64, profile: &str, key: &str) -> Result<()> {
        self.require_group(group_id)?;
        self.conn
            .execute(
                "INSERT INTO favorites(group_id, profile, key, position)
                 SELECT ?1, ?2, ?3, COALESCE(MAX(position) + 1, 0) FROM favorites WHERE group_id = ?1
                 ON CONFLICT(group_id, profile, key) DO NOTHING",
                params![group_id, profile, key],
            )
            .map(drop)
            .map_err(|e| self.sql(e))
    }

    /// Remove a favorite; later ones move up. Missing is not an error.
    pub fn remove_favorite(&self, group_id: i64, profile: &str, key: &str) -> Result<()> {
        self.txn(|c| {
            c.execute(
                "DELETE FROM favorites WHERE group_id = ?1 AND profile = ?2 AND key = ?3",
                params![group_id, profile, key],
            )?;
            Self::renumber_favorites(c, group_id)
        })
    }

    /// Move a favorite to `new_position` (clamped) within its group.
    pub fn move_favorite(
        &self,
        group_id: i64,
        profile: &str,
        key: &str,
        new_position: i64,
    ) -> Result<()> {
        let moved = self.txn(|c| {
            let mut stmt = c.prepare(
                "SELECT profile, key FROM favorites WHERE group_id = ?1 ORDER BY position, profile, key",
            )?;
            let mut items: Vec<(String, String)> = stmt
                .query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let Some(from) = items.iter().position(|(p, k)| p == profile && k == key) else {
                return Ok(false);
            };
            let item = items.remove(from);
            let to = usize::try_from(new_position).unwrap_or(0).min(items.len());
            items.insert(to, item);
            let mut update = c.prepare(
                "UPDATE favorites SET position = ?4 WHERE group_id = ?1 AND profile = ?2 AND key = ?3",
            )?;
            for (pos, (p, k)) in (0_i64..).zip(&items) {
                update.execute(params![group_id, p, k, pos])?;
            }
            Ok(true)
        })?;
        if moved {
            Ok(())
        } else {
            Err(IndexError::Other(format!(
                "{key} is not a favorite in group {group_id}"
            )))
        }
    }

    fn renumber_favorites(c: &Connection, group_id: i64) -> rusqlite::Result<()> {
        let mut stmt = c.prepare(
            "SELECT profile, key FROM favorites WHERE group_id = ?1 ORDER BY position, profile, key",
        )?;
        let items: Vec<(String, String)> = stmt
            .query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut update = c.prepare(
            "UPDATE favorites SET position = ?4 WHERE group_id = ?1 AND profile = ?2 AND key = ?3",
        )?;
        for (pos, (p, k)) in (0_i64..).zip(&items) {
            update.execute(params![group_id, p, k, pos])?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- recents

    /// Mark an asset as just used. The time is strictly later than every
    /// earlier use in the same profile, so the order is deterministic.
    pub fn record_use(&self, profile: &str, key: &str) -> Result<()> {
        self.txn(|c| {
            let newest: i64 = c.query_row(
                "SELECT COALESCE(MAX(last_used_unix_ms), 0) FROM recents WHERE profile = ?1",
                [profile],
                |r| r.get(0),
            )?;
            let stamp = now_ms().max(newest.saturating_add(1));
            c.execute(
                "INSERT INTO recents(profile, key, last_used_unix_ms, uses) VALUES(?1, ?2, ?3, 1)
                 ON CONFLICT(profile, key) DO UPDATE SET last_used_unix_ms = excluded.last_used_unix_ms,
                                                         uses = uses + 1",
                params![profile, key, stamp],
            )?;
            c.execute(
                "DELETE FROM recents WHERE profile = ?1 AND key IN
                   (SELECT key FROM recents WHERE profile = ?1
                    ORDER BY last_used_unix_ms DESC LIMIT -1 OFFSET ?2)",
                params![profile, MAX_RECENTS],
            )?;
            Ok(())
        })
    }

    /// Keys, most recently used first.
    pub fn recents(&self, profile: &str, limit: usize) -> Result<Vec<String>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT key FROM recents WHERE profile = ?1
                 ORDER BY last_used_unix_ms DESC, key LIMIT ?2",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map(params![profile, limit], |r| r.get(0))
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| self.sql(e))
    }

    /// How many times an asset was used.
    pub fn use_count(&self, profile: &str, key: &str) -> Result<u32> {
        let uses: Option<i64> = self
            .conn
            .query_row(
                "SELECT uses FROM recents WHERE profile = ?1 AND key = ?2",
                params![profile, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| self.sql(e))?;
        Ok(uses.and_then(|u| u32::try_from(u).ok()).unwrap_or(0))
    }

    // --------------------------------------------------------- saved searches

    /// Pinned searches first, then by position.
    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, query, position, pinned FROM saved_searches
                 ORDER BY pinned DESC, position, id",
            )
            .map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SavedSearch {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    query: r.get(2)?,
                    position: r.get(3)?,
                    pinned: r.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(|e| self.sql(e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| self.sql(e))
    }

    /// Save a search under `name`. Saving an existing name replaces its
    /// query and keeps its position and pin.
    pub fn save_search(&self, name: &str, query: &str) -> Result<SavedSearch> {
        let name = name.trim();
        if name.is_empty() {
            return Err(IndexError::Other(
                "a search name cannot be empty".to_owned(),
            ));
        }
        self.txn(|c| {
            let existing: Option<i64> = c
                .query_row(
                    "SELECT id FROM saved_searches WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match existing {
                Some(id) => {
                    c.execute(
                        "UPDATE saved_searches SET query = ?2 WHERE id = ?1",
                        params![id, query],
                    )?;
                    id
                }
                None => {
                    c.execute(
                        "INSERT INTO saved_searches(name, query, position)
                         VALUES(?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM saved_searches))",
                        params![name, query],
                    )?;
                    c.last_insert_rowid()
                }
            };
            c.query_row(
                "SELECT id, name, query, position, pinned FROM saved_searches WHERE id = ?1",
                [id],
                |r| {
                    Ok(SavedSearch {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        query: r.get(2)?,
                        position: r.get(3)?,
                        pinned: r.get::<_, i64>(4)? != 0,
                    })
                },
            )
        })
    }

    pub fn delete_search(&self, id: i64) -> Result<()> {
        self.txn(|c| {
            c.execute("DELETE FROM saved_searches WHERE id = ?1", [id])?;
            Self::renumber(
                c,
                "SELECT id FROM saved_searches ORDER BY position, id",
                "UPDATE saved_searches SET position = ?2 WHERE id = ?1",
                [],
            )
        })
    }

    fn require_search(&self, id: i64) -> Result<()> {
        let found = self
            .conn
            .query_row("SELECT 1 FROM saved_searches WHERE id = ?1", [id], |_| {
                Ok(())
            })
            .optional()
            .map_err(|e| self.sql(e))?;
        match found {
            Some(()) => Ok(()),
            None => Err(IndexError::Other(format!("no saved search with id {id}"))),
        }
    }

    /// Rename a saved search; its query, position and pin stay. Names are
    /// trimmed and unique (renaming to its own name is a no-op).
    pub fn rename_search(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(IndexError::Other(
                "a search name cannot be empty".to_owned(),
            ));
        }
        self.require_search(id)?;
        match self.conn.execute(
            "UPDATE saved_searches SET name = ?2 WHERE id = ?1",
            params![id, name],
        ) {
            Ok(_) => Ok(()),
            Err(e) if is_unique_violation(&e) => Err(IndexError::Other(format!(
                "a saved search named {name:?} already exists"
            ))),
            Err(e) => Err(self.sql(e)),
        }
    }

    /// Move a saved search to `new_position` (clamped to the end); the
    /// others shift and positions stay dense. Positions are one order over
    /// all searches; [`UserDb::saved_searches`] still lists pinned ones first.
    pub fn move_search(&self, id: i64, new_position: i64) -> Result<()> {
        self.require_search(id)?;
        self.txn(|c| {
            let mut ids = Self::ids(c, "SELECT id FROM saved_searches ORDER BY position, id", [])?;
            reorder(&mut ids, id, new_position);
            Self::write_positions(
                c,
                "UPDATE saved_searches SET position = ?2 WHERE id = ?1",
                &ids,
            )
        })
    }

    pub fn set_search_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn
            .execute(
                "UPDATE saved_searches SET pinned = ?2 WHERE id = ?1",
                params![id, i64::from(pinned)],
            )
            .map(drop)
            .map_err(|e| self.sql(e))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, UserDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = UserDb::open(&dir.path().join("capybara").join("user.db")).unwrap();
        (dir, db)
    }

    fn reopen(dir: &tempfile::TempDir) -> UserDb {
        UserDb::open(&dir.path().join("capybara").join("user.db")).unwrap()
    }

    #[test]
    fn schema_version_row() {
        let (dir, db) = open();
        assert_eq!(db.schema_version().unwrap(), 1);
        drop(db);
        let db = reopen(&dir);
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn newer_schema_is_refused_not_dropped() {
        let (dir, db) = open();
        db.set_alias("p", "k", Some("kept")).unwrap();
        db.conn
            .execute(
                "UPDATE meta SET value = '99' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        drop(db);
        let err = UserDb::open(&dir.path().join("capybara").join("user.db"))
            .err()
            .expect("must refuse");
        assert!(err.to_string().contains("newer"), "{err}");
        // The data is still there for the newer editor.
        let conn = Connection::open(dir.path().join("capybara").join("user.db")).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM aliases", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn aliases_and_notes() {
        let (dir, db) = open();
        assert_eq!(db.alias("p", "k").unwrap(), None);
        db.set_alias("p", "k", Some("  Mossy floor ")).unwrap();
        assert_eq!(db.alias("p", "k").unwrap().as_deref(), Some("Mossy floor"));
        db.set_alias("p", "k", Some("Other")).unwrap();
        assert_eq!(db.alias("p", "k").unwrap().as_deref(), Some("Other"));
        // Another profile or key is separate.
        assert_eq!(db.alias("q", "k").unwrap(), None);
        assert_eq!(db.alias("p", "k2").unwrap(), None);
        db.set_alias("p", "k2", Some("two")).unwrap();
        let all = db.aliases("p").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all["k2"], "two");

        db.set_note("p", "k", Some("remember this\nline two"))
            .unwrap();
        assert_eq!(
            db.note("p", "k").unwrap().as_deref(),
            Some("remember this\nline two")
        );
        drop(db);
        let db = reopen(&dir);
        assert_eq!(db.alias("p", "k").unwrap().as_deref(), Some("Other"));
        assert_eq!(
            db.note("p", "k").unwrap().as_deref(),
            Some("remember this\nline two")
        );
        db.set_alias("p", "k", None).unwrap();
        db.set_alias("p", "k2", Some("   ")).unwrap();
        db.set_note("p", "k", None).unwrap();
        assert_eq!(db.alias("p", "k").unwrap(), None);
        assert_eq!(db.alias("p", "k2").unwrap(), None);
        assert_eq!(db.note("p", "k").unwrap(), None);
        // Removing what is not there is fine.
        db.set_alias("p", "nothing", None).unwrap();
        db.set_note("p", "nothing", Some("")).unwrap();
    }

    #[test]
    fn tags() {
        let (dir, db) = open();
        db.add_tag("p", "a", "Grass").unwrap();
        db.add_tag("p", "a", "arena").unwrap();
        db.add_tag("p", "a", " grass ").unwrap(); // same tag, other case
        db.add_tag("p", "b", "grass").unwrap();
        db.add_tag("q", "a", "grass").unwrap();
        assert_eq!(db.tags("p", "a").unwrap(), vec!["arena", "Grass"]);
        assert_eq!(db.tags("p", "none").unwrap(), Vec::<String>::new());
        assert_eq!(db.keys_with_tag("p", "GRASS").unwrap(), vec!["a", "b"]);
        assert_eq!(db.keys_with_tag("q", "grass").unwrap(), vec!["a"]);
        assert_eq!(
            db.keys_with_tag("p", "missing").unwrap(),
            Vec::<String>::new()
        );
        assert!(db.add_tag("p", "a", "  ").is_err());
        drop(db);
        let db = reopen(&dir);
        db.remove_tag("p", "a", "GRASS").unwrap();
        assert_eq!(db.tags("p", "a").unwrap(), vec!["arena"]);
        assert_eq!(db.keys_with_tag("p", "grass").unwrap(), vec!["b"]);
        db.remove_tag("p", "a", "never-there").unwrap();
    }

    #[test]
    fn groups_and_favorites_ordering() {
        let (dir, db) = open();
        let a = db.create_group("A").unwrap();
        let b = db.create_group(" B ").unwrap();
        let c = db.create_group("C").unwrap();
        assert_eq!((a.position, b.position, c.position), (0, 1, 2));
        assert_eq!(b.name, "B");
        assert!(db.create_group("a").is_ok(), "names are case-sensitive");
        let dup = db.create_group("B").unwrap_err();
        assert!(dup.to_string().contains("already exists"), "{dup}");
        assert!(db.create_group("  ").is_err());
        let order = |db: &UserDb| {
            db.favorite_groups()
                .unwrap()
                .into_iter()
                .map(|g| g.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&db), ["A", "B", "C", "a"]);

        db.move_group(c.id, 0).unwrap();
        assert_eq!(order(&db), ["C", "A", "B", "a"]);
        db.move_group(c.id, 99).unwrap();
        assert_eq!(order(&db), ["A", "B", "a", "C"]);
        db.move_group(b.id, 2).unwrap();
        assert_eq!(order(&db), ["A", "a", "B", "C"]);
        db.move_group(a.id, -5).unwrap();
        assert_eq!(order(&db), ["A", "a", "B", "C"]);
        let positions: Vec<i64> = db
            .favorite_groups()
            .unwrap()
            .iter()
            .map(|g| g.position)
            .collect();
        assert_eq!(positions, [0, 1, 2, 3], "positions stay dense");
        assert!(db.move_group(12345, 0).is_err());

        db.rename_group(b.id, "Bee").unwrap();
        assert!(db.rename_group(b.id, "C").is_err());
        assert!(db.rename_group(12345, "x").is_err());
        assert_eq!(order(&db), ["A", "a", "Bee", "C"]);

        // Favorites.
        for key in ["k1", "k2", "k3"] {
            db.add_favorite(a.id, "p", key).unwrap();
        }
        db.add_favorite(a.id, "q", "k1").unwrap();
        db.add_favorite(a.id, "p", "k2").unwrap(); // duplicate: no-op, keeps place
        assert!(db.add_favorite(9999, "p", "k1").is_err());
        let keys = |db: &UserDb, g: i64| {
            db.favorites(g)
                .unwrap()
                .into_iter()
                .map(|(p, k)| format!("{p}:{k}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&db, a.id), ["p:k1", "p:k2", "p:k3", "q:k1"]);
        db.move_favorite(a.id, "q", "k1", 0).unwrap();
        assert_eq!(keys(&db, a.id), ["q:k1", "p:k1", "p:k2", "p:k3"]);
        db.move_favorite(a.id, "q", "k1", 99).unwrap();
        assert_eq!(keys(&db, a.id), ["p:k1", "p:k2", "p:k3", "q:k1"]);
        db.move_favorite(a.id, "p", "k3", 1).unwrap();
        assert_eq!(keys(&db, a.id), ["p:k1", "p:k3", "p:k2", "q:k1"]);
        assert!(db.move_favorite(a.id, "p", "absent", 0).is_err());
        db.remove_favorite(a.id, "p", "k1").unwrap();
        assert_eq!(keys(&db, a.id), ["p:k3", "p:k2", "q:k1"]);
        db.remove_favorite(a.id, "p", "absent").unwrap();
        // The same key in another group is independent.
        db.add_favorite(c.id, "p", "k2").unwrap();
        assert_eq!(keys(&db, c.id), ["p:k2"]);

        drop(db);
        let db = reopen(&dir);
        assert_eq!(keys(&db, a.id), ["p:k3", "p:k2", "q:k1"]);
        assert_eq!(order(&db), ["A", "a", "Bee", "C"]);

        // Deleting a group removes its favorites and closes the gap.
        db.delete_group(a.id).unwrap();
        assert_eq!(keys(&db, a.id), Vec::<String>::new());
        assert_eq!(order(&db), ["a", "Bee", "C"]);
        let positions: Vec<i64> = db
            .favorite_groups()
            .unwrap()
            .iter()
            .map(|g| g.position)
            .collect();
        assert_eq!(positions, [0, 1, 2]);
        assert_eq!(keys(&db, c.id), ["p:k2"]);
        db.delete_group(a.id).unwrap(); // already gone
    }

    #[test]
    fn recents_newest_first_and_bumping() {
        let (dir, db) = open();
        assert_eq!(db.recents("p", 10).unwrap(), Vec::<String>::new());
        for key in ["a", "b", "c", "d"] {
            db.record_use("p", key).unwrap();
        }
        db.record_use("q", "other").unwrap();
        assert_eq!(db.recents("p", 10).unwrap(), ["d", "c", "b", "a"]);
        assert_eq!(db.recents("p", 2).unwrap(), ["d", "c"]);
        assert_eq!(db.recents("p", 0).unwrap(), Vec::<String>::new());
        assert_eq!(db.recents("q", 10).unwrap(), ["other"]);
        // Using an older one bumps it to the front, without a duplicate.
        db.record_use("p", "a").unwrap();
        assert_eq!(db.recents("p", 10).unwrap(), ["a", "d", "c", "b"]);
        db.record_use("p", "a").unwrap();
        assert_eq!(db.use_count("p", "a").unwrap(), 3);
        assert_eq!(db.use_count("p", "b").unwrap(), 1);
        assert_eq!(db.use_count("p", "nope").unwrap(), 0);
        drop(db);
        let db = reopen(&dir);
        assert_eq!(db.recents("p", 10).unwrap(), ["a", "d", "c", "b"]);
        db.record_use("p", "c").unwrap();
        assert_eq!(db.recents("p", 3).unwrap(), ["c", "a", "d"]);
    }

    #[test]
    fn recents_are_capped() {
        let (_dir, db) = open();
        for i in 0..(MAX_RECENTS + 5) {
            db.record_use("p", &format!("k{i}")).unwrap();
        }
        let all = db.recents("p", 5000).unwrap();
        assert_eq!(i64::try_from(all.len()).unwrap(), MAX_RECENTS);
        assert_eq!(all[0], format!("k{}", MAX_RECENTS + 4));
        assert!(!all.contains(&"k0".to_owned()));
    }

    #[test]
    fn saved_searches() {
        let (dir, db) = open();
        assert!(db.saved_searches().unwrap().is_empty());
        let a = db.save_search("Grass", "type:floor grass").unwrap();
        let b = db.save_search("Walls", "type:wall").unwrap();
        let c = db.save_search("Doors", "idx:8").unwrap();
        assert_eq!((a.position, b.position, c.position), (0, 1, 2));
        assert!(!a.pinned);
        let names = |db: &UserDb| {
            db.saved_searches()
                .unwrap()
                .into_iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&db), ["Grass", "Walls", "Doors"]);
        // Re-saving a name replaces the query and keeps place.
        let again = db.save_search("Walls", "type:wall act:2").unwrap();
        assert_eq!(again.id, b.id);
        assert_eq!(again.position, 1);
        assert_eq!(again.query, "type:wall act:2");
        assert_eq!(db.saved_searches().unwrap().len(), 3);
        // Pinned ones come first.
        db.set_search_pinned(c.id, true).unwrap();
        assert_eq!(names(&db), ["Doors", "Grass", "Walls"]);
        assert!(db.saved_searches().unwrap()[0].pinned);
        db.set_search_pinned(c.id, false).unwrap();
        assert_eq!(names(&db), ["Grass", "Walls", "Doors"]);
        assert!(db.save_search(" ", "x").is_err());
        drop(db);
        let db = reopen(&dir);
        assert_eq!(names(&db), ["Grass", "Walls", "Doors"]);
        db.delete_search(a.id).unwrap();
        assert_eq!(names(&db), ["Walls", "Doors"]);
        let positions: Vec<i64> = db
            .saved_searches()
            .unwrap()
            .iter()
            .map(|s| s.position)
            .collect();
        assert_eq!(positions, [0, 1]);
        db.delete_search(a.id).unwrap();
        let d = db.save_search("New", "q").unwrap();
        assert_eq!(d.position, 2);
    }

    #[test]
    fn rename_and_move_saved_searches() {
        let (dir, db) = open();
        let a = db.save_search("Grass", "type:floor grass").unwrap();
        let b = db.save_search("Walls", "type:wall").unwrap();
        let c = db.save_search("Doors", "idx:8").unwrap();
        let d = db.save_search("Roofs", "type:roof").unwrap();
        let listed = |db: &UserDb| {
            db.saved_searches()
                .unwrap()
                .into_iter()
                .map(|s| (s.name, s.position))
                .collect::<Vec<_>>()
        };
        let names = |db: &UserDb| {
            db.saved_searches()
                .unwrap()
                .into_iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };

        // Rename keeps id, query, position and pin.
        db.set_search_pinned(b.id, true).unwrap();
        db.rename_search(b.id, "  Stone walls ").unwrap();
        let renamed = db.saved_searches().unwrap();
        assert_eq!(renamed[0].id, b.id);
        assert_eq!(renamed[0].name, "Stone walls");
        assert_eq!(renamed[0].query, "type:wall");
        assert_eq!(renamed[0].position, 1);
        assert!(renamed[0].pinned);
        db.set_search_pinned(b.id, false).unwrap();
        // Same name again is fine; empty, taken or unknown ids are errors.
        db.rename_search(b.id, "Stone walls").unwrap();
        assert!(db.rename_search(b.id, "  ").is_err());
        let taken = db.rename_search(b.id, "Grass").unwrap_err();
        assert!(taken.to_string().contains("already exists"), "{taken}");
        assert!(db.rename_search(9999, "Nope").is_err());
        assert_eq!(names(&db), ["Grass", "Stone walls", "Doors", "Roofs"]);
        // Saving under the new name updates the renamed search.
        let again = db.save_search("Stone walls", "type:wall act:3").unwrap();
        assert_eq!(again.id, b.id);
        assert_eq!(db.saved_searches().unwrap().len(), 4);

        // Moves keep positions dense.
        db.move_search(d.id, 0).unwrap();
        assert_eq!(
            listed(&db),
            [
                ("Roofs".to_owned(), 0),
                ("Grass".to_owned(), 1),
                ("Stone walls".to_owned(), 2),
                ("Doors".to_owned(), 3)
            ]
        );
        db.move_search(d.id, 99).unwrap();
        assert_eq!(names(&db), ["Grass", "Stone walls", "Doors", "Roofs"]);
        db.move_search(a.id, 2).unwrap();
        assert_eq!(names(&db), ["Stone walls", "Doors", "Grass", "Roofs"]);
        db.move_search(c.id, -5).unwrap();
        assert_eq!(names(&db), ["Doors", "Stone walls", "Grass", "Roofs"]);
        db.move_search(c.id, 0).unwrap();
        assert_eq!(names(&db), ["Doors", "Stone walls", "Grass", "Roofs"]);
        assert!(db.move_search(9999, 0).is_err());
        // Pinned searches still list first; positions are unchanged by it.
        db.set_search_pinned(a.id, true).unwrap();
        assert_eq!(names(&db), ["Grass", "Doors", "Stone walls", "Roofs"]);
        assert_eq!(db.saved_searches().unwrap()[0].position, 2);
        db.set_search_pinned(a.id, false).unwrap();
        // Deleting after a move stays dense, and the order survives reopening.
        db.delete_search(c.id).unwrap();
        drop(db);
        let db = reopen(&dir);
        assert_eq!(
            listed(&db),
            [
                ("Stone walls".to_owned(), 0),
                ("Grass".to_owned(), 1),
                ("Roofs".to_owned(), 2)
            ]
        );
        assert_eq!(db.save_search("New", "q").unwrap().position, 3);
    }
}
