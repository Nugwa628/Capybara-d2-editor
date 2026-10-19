//! Robustness: a damaged or hand-edited `index.db` / `user.db` (rows with
//! negative, huge or wrongly typed values, NULLs, dropped tables, flipped
//! bytes in the file) must give errors, never a panic.
//!
//! Each loop runs `CAPY_FUZZ_ITERS` iterations (default 30; each opens, damages
//! and queries two databases).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::{Fx, fake_run};
use crate::index::Index;
use crate::query::{Filter, parse_query};
use crate::user::UserDb;

fn iters() -> u64 {
    let n = std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    println!("CAPY_FUZZ_ITERS = {n}");
    n
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// SQL literals that break integer, text and blob expectations.
const NASTY: &[&str] = &[
    "NULL",
    "-1",
    "-9223372036854775808",
    "9223372036854775807",
    "4294967296",
    "256",
    "300",
    "1e308",
    "-1e308",
    "0.5",
    "'text'",
    "''",
    "X'00FF'",
    "X''",
    "'tile'",
    "'zzz'",
    "'é'",
    "0",
];

const INDEX_COLUMNS: &[(&str, &[&str])] = &[
    (
        "assets",
        &[
            "id",
            "profile",
            "key",
            "kind",
            "path",
            "act",
            "orientation",
            "main",
            "sub",
            "variant",
            "rarity",
            "width",
            "height",
            "class",
            "gen_name",
            "color",
            "color_rgb",
            "texture",
            "transparency",
            "phash",
            "phash_mirror",
            "directions",
            "frames",
            "src",
            "src_tag",
            "thumb",
        ],
    ),
    ("used_in", &["asset_id", "ds1_path", "cells"]),
    ("asset_lvltypes", &["asset_id", "lvltype", "slot"]),
    ("profiles", &["id", "fingerprint", "built_unix_ms"]),
    (
        "files",
        &["profile", "path", "kind", "layer", "source", "size", "hash"],
    ),
    ("meta", &["key", "value"]),
];

const USER_COLUMNS: &[(&str, &[&str])] = &[
    ("aliases", &["profile", "key", "alias"]),
    ("tags", &["profile", "key", "tag"]),
    ("notes", &["profile", "key", "note"]),
    ("favorite_groups", &["id", "name", "position"]),
    ("favorites", &["group_id", "profile", "key", "position"]),
    ("recents", &["profile", "key", "last_used_unix_ms", "uses"]),
    (
        "saved_searches",
        &["id", "name", "query", "position", "pinned"],
    ),
    (
        "sprite_palettes",
        &["profile", "key", "palette", "updated_unix_ms"],
    ),
    ("meta", &["key", "value"]),
];

/// One random damaging statement against `columns`' tables.
fn damage(rng: &mut Rng, conn: &rusqlite::Connection, columns: &[(&str, &[&str])]) {
    let (table, cols) = columns[rng.below(columns.len())];
    let col = rng.pick(cols);
    let value = rng.pick(NASTY);
    let sql = match rng.below(10) {
        0 => format!("DELETE FROM {table} WHERE rowid % 3 = {}", rng.below(3)),
        1 => format!("DROP TABLE IF EXISTS {table}"),
        2 => format!(
            "DROP INDEX IF EXISTS {}",
            rng.pick(&["assets_by_kind", "assets_by_phash", "tags_by_tag"])
        ),
        3 => "DELETE FROM assets_fts".to_owned(),
        4 => format!(
            "UPDATE {table} SET {col} = {value} WHERE rowid % 2 = {}",
            rng.below(2)
        ),
        _ => format!("UPDATE {table} SET {col} = {value}"),
    };
    // Constraint violations and missing tables are fine: they just do nothing.
    let _ = conn.execute_batch(&sql);
}

const QUERIES: &[&str] = &[
    "",
    "grass",
    "type:floor",
    "type:ds1 act:1",
    "idx:0:5/2",
    "similar:data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/0",
    "mirrored:yes",
    "-mirrored:yes",
    "used-in:towne1",
    "src:mod",
    "src:saves",
    "tag:mine",
    "-tag:mine",
    "color:green file:floor",
    "shire floor",
    "whimsy",
];

fn exercise_index(idx: &Index, user: &UserDb, fx: &Fx) {
    for q in QUERIES {
        let query = parse_query(q).unwrap();
        let _ = idx.search("test", &query, user, 50);
        let _ = idx.search("test", &query, user, usize::MAX);
    }
    let mut any = parse_query("").unwrap();
    any.filters
        .push(Filter::AnyFile(vec!["floor".into(), "é".into()]));
    let _ = idx.search("test", &any, user, 10);
    for id in [i64::MIN, -1, 0, 1, 2, 3, 5, 8, 13, i64::MAX] {
        let _ = idx.thumbnail(id);
        let _ = idx.lvltypes_of(id);
        let _ = idx.used_in(id);
    }
    for key in [
        "data\\global\\tiles\\act1\\town\\towne1.ds1",
        "data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/0",
        "",
    ] {
        let _ = idx.asset("test", key);
    }
    let _ = idx.tiles_with_key("test", 0, 5, 2);
    let _ = idx.tiles_with_key("test", 255, 255, 255);
    let _ = idx.count("test", None);
    let _ = idx.count("test", Some(crate::query::AssetKind::Tile));
    let _ = idx.status("test", &fx.overlay());
}

fn exercise_user(user: &UserDb) {
    let _ = user.schema_version();
    let _ = user.alias("test", "k");
    let _ = user.set_alias("test", "k", Some("A"));
    let _ = user.tags("test", "k");
    let _ = user.add_tag("test", "k", "mine");
    let _ = user.keys_with_tag("test", "mine");
    let _ = user.remove_tag("test", "k", "other");
    let _ = user.note("test", "k");
    let _ = user.set_note("test", "k", Some("n"));
    let groups = user.favorite_groups().unwrap_or_default();
    let _ = user.create_group("g-new");
    for g in &groups {
        let _ = user.favorites(g.id);
        let _ = user.add_favorite(g.id, "test", "k");
        let _ = user.move_favorite(g.id, "test", "k", i64::MAX);
        let _ = user.move_favorite(g.id, "test", "k", i64::MIN);
        let _ = user.move_group(g.id, i64::MIN);
        let _ = user.move_group(g.id, i64::MAX);
        let _ = user.rename_group(g.id, "renamed");
    }
    if let Some(g) = groups.first() {
        let _ = user.remove_favorite(g.id, "test", "k");
        let _ = user.delete_group(g.id);
    }
    let _ = user.record_use("test", "k");
    let _ = user.recents("test", usize::MAX);
    let _ = user.use_count("test", "k");
    let searches = user.saved_searches().unwrap_or_default();
    let _ = user.save_search("s-new", "type:floor");
    for s in &searches {
        let _ = parse_query(&s.query);
        let _ = user.move_search(s.id, i64::MAX);
        let _ = user.move_search(s.id, -5);
        let _ = user.rename_search(s.id, "r");
        let _ = user.set_search_pinned(s.id, true);
    }
    if let Some(s) = searches.first() {
        let _ = user.delete_search(s.id);
    }
    for key in [r"data\global\ui\a.dc6", "data/global/ui/new.dc6", "", "\\"] {
        let _ = user.sprite_palette("test", key);
        let _ = user.folder_palette("test", key);
    }
    let _ = user.set_sprite_palette("test", r"data\global\ui\b.dc6", Some("act2"));
    let _ = user.set_sprite_palette("test", r"data\global\ui\a.dc6", None);
}

/// A built index plus a user.db with some rows of every kind, both copied
/// to pristine files so each iteration can start over.
struct Setup {
    fx: Fx,
    cache: tempfile::TempDir,
    pristine_index: PathBuf,
    pristine_user: PathBuf,
}

fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

fn setup() -> Setup {
    let fx = Fx::new("test");
    let cache = tempfile::tempdir().unwrap();
    let pristine_index = cache.path().join("pristine-index.db");
    let pristine_user = cache.path().join("pristine-user.db");
    {
        let mut idx = Index::open(&cache.path().join("idx")).unwrap();
        let cancel = AtomicBool::new(false);
        idx.build_with(&fx.profile(), &fx.overlay(), &mut |_| {}, &cancel, fake_run)
            .unwrap();
        idx.conn
            .execute("VACUUM INTO ?1", [pristine_index.to_str().unwrap()])
            .unwrap();
    }
    {
        let user = UserDb::open(&cache.path().join("u").join("user.db")).unwrap();
        user.set_alias(
            "test",
            "data\\global\\tiles\\act1\\town\\towne1.ds1",
            Some("Town"),
        )
        .unwrap();
        user.add_tag(
            "test",
            "data\\global\\tiles\\act1\\town\\towne1.ds1",
            "mine",
        )
        .unwrap();
        user.set_note("test", "k", Some("note")).unwrap();
        let g = user.create_group("g").unwrap();
        user.add_favorite(g.id, "test", "k").unwrap();
        user.add_favorite(g.id, "test", "k2").unwrap();
        user.create_group("h").unwrap();
        user.record_use("test", "k").unwrap();
        user.record_use("test", "k2").unwrap();
        user.save_search("s", "type:floor act:1").unwrap();
        user.save_search("t", "grass").unwrap();
        user.set_sprite_palette("test", r"data\global\ui\a.dc6", Some("act1"))
            .unwrap();
        user.set_sprite_palette("test", r"data\global\ui\c.dc6", Some("units"))
            .unwrap();
        let raw = rusqlite::Connection::open(cache.path().join("u").join("user.db")).unwrap();
        raw.execute("VACUUM INTO ?1", [pristine_user.to_str().unwrap()])
            .unwrap();
    }
    Setup {
        fx,
        cache,
        pristine_index,
        pristine_user,
    }
}

impl Setup {
    fn index_dir(&self) -> PathBuf {
        self.cache.path().join("work-index")
    }

    fn user_path(&self) -> PathBuf {
        self.cache.path().join("work-user").join("user.db")
    }

    /// A fresh copy of `bytes` as the index; `None` when the (damaged) file
    /// does not open, which is a fine outcome. Earlier handles must be dropped.
    fn open_index(&self, bytes: &[u8]) -> Option<Index> {
        let dir = self.index_dir();
        std::fs::create_dir_all(&dir).unwrap();
        remove_db(&dir.join("index.db"));
        std::fs::write(dir.join("index.db"), bytes).unwrap();
        Index::open(&dir).ok()
    }

    /// As [`Setup::open_index`], for user.db.
    fn open_user(&self, bytes: &[u8]) -> Option<UserDb> {
        let path = self.user_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        remove_db(&path);
        std::fs::write(&path, bytes).unwrap();
        UserDb::open(&path).ok()
    }
}

#[test]
fn damaged_rows_in_index_and_user_db_never_panic() {
    let s = setup();
    let index_bytes = std::fs::read(&s.pristine_index).unwrap();
    let user_bytes = std::fs::read(&s.pristine_user).unwrap();
    let mut rng = Rng(0x00C0_FFEE_1234_5678);
    for _ in 0..iters() {
        let mut idx = s.open_index(&index_bytes).unwrap();
        let user = s.open_user(&user_bytes).unwrap();
        {
            let raw_user = rusqlite::Connection::open(s.user_path()).unwrap();
            raw_user
                .busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            for _ in 0..1 + rng.below(4) {
                if rng.below(3) == 0 {
                    damage(&mut rng, &raw_user, USER_COLUMNS);
                } else {
                    damage(&mut rng, &idx.conn, INDEX_COLUMNS);
                }
            }
        }
        exercise_index(&idx, &user, &s.fx);
        exercise_user(&user);
        let _ = idx.refresh_saves(&s.fx.profile(), &s.fx.overlay());
        // Reopening runs the schema checks and migrations on the damage.
        drop(user);
        if let Ok(user) = UserDb::open(&s.user_path()) {
            exercise_user(&user);
            exercise_index(&idx, &user, &s.fx);
        }
        drop(idx);
        let _ = Index::open(&s.index_dir());
    }
}

#[test]
fn damaged_database_files_never_panic() {
    let s = setup();
    let index_bytes = std::fs::read(&s.pristine_index).unwrap();
    let user_bytes = std::fs::read(&s.pristine_user).unwrap();
    let mut rng = Rng(0xBAD0_DA7A_0000_0001);
    let flip = |rng: &mut Rng, bytes: &[u8]| -> Vec<u8> {
        let mut out = bytes.to_vec();
        match rng.below(4) {
            0 => {
                let n = rng.below(out.len() + 1);
                out.truncate(n);
            }
            _ => {
                for _ in 0..1 + rng.below(8) {
                    // Mostly past the 100-byte header, so SQLite opens the file
                    // and meets the damage while reading pages.
                    let i = if rng.below(4) == 0 {
                        rng.below(100.min(out.len()))
                    } else {
                        rng.below(out.len())
                    };
                    if let Some(b) = out.get_mut(i) {
                        *b ^= 1 << rng.below(8);
                    }
                }
            }
        }
        out
    };
    for _ in 0..iters() {
        let bad_index = flip(&mut rng, &index_bytes);
        let bad_user = flip(&mut rng, &user_bytes);
        let user = match s.open_user(&bad_user) {
            Some(u) => u,
            None => s.open_user(&user_bytes).unwrap(),
        };
        exercise_user(&user);
        if let Some(mut idx) = s.open_index(&bad_index) {
            exercise_index(&idx, &user, &s.fx);
            let _ = idx.refresh_saves(&s.fx.profile(), &s.fx.overlay());
        }
    }
}
