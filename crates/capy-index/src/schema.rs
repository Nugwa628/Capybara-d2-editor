//! `index.db` schema (version 2). The tables are the contract in
//! `docs/api/capy-index.md`; the indexes below are additions for speed.

/// Bump when the tables change. Any mismatch drops the file and rebuilds.
pub(crate) const SCHEMA_VERSION: i64 = 2;

/// Bump when naming, thumbnails or hashing change what a build produces.
/// It is part of every profile's fingerprint, so profiles go stale.
pub(crate) const GENERATOR_VERSION: u32 = 2;

pub(crate) const SCHEMA: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE profiles(id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, built_unix_ms INTEGER NOT NULL);
CREATE TABLE files(profile TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL,
  layer INTEGER NOT NULL, source TEXT NOT NULL, size INTEGER NOT NULL, hash BLOB NOT NULL,
  PRIMARY KEY(profile, path));
CREATE TABLE assets(id INTEGER PRIMARY KEY, profile TEXT NOT NULL, key TEXT NOT NULL,
  kind TEXT NOT NULL,
  path TEXT NOT NULL, act INTEGER, orientation INTEGER, main INTEGER, sub INTEGER,
  variant INTEGER, rarity INTEGER, width INTEGER, height INTEGER, class TEXT,
  gen_name TEXT NOT NULL, color TEXT, color_rgb INTEGER, texture TEXT, transparency REAL,
  phash INTEGER, phash_mirror INTEGER,
  directions INTEGER, frames INTEGER,
  src TEXT NOT NULL,
  src_tag TEXT NOT NULL,
  thumb BLOB,
  UNIQUE(profile, key));
CREATE TABLE asset_lvltypes(asset_id INTEGER NOT NULL, lvltype INTEGER NOT NULL, slot INTEGER NOT NULL);
CREATE TABLE used_in(asset_id INTEGER NOT NULL, ds1_path TEXT NOT NULL, cells INTEGER NOT NULL);
CREATE VIRTUAL TABLE assets_fts USING fts5(gen_name, path, class, color, texture,
  content='assets', content_rowid='id', tokenize=\"unicode61 tokenchars '_\\'\");
CREATE INDEX assets_by_kind ON assets(profile, kind);
CREATE INDEX assets_by_tile_key ON assets(profile, orientation, main, sub);
CREATE INDEX assets_by_phash ON assets(profile, phash);
CREATE INDEX assets_by_path ON assets(profile, path);
CREATE INDEX asset_lvltypes_by_asset ON asset_lvltypes(asset_id);
CREATE INDEX used_in_by_asset ON used_in(asset_id);
";
