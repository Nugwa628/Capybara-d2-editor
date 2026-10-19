//! `Index::search`: SQL does the filtering, nucleo ranks the free text.
//!
//! Free text is matched fuzzily against the user's alias, the generated name
//! and the path (best of the three, with the alias slightly favoured and the
//! path slightly discounted). Filters are exact. Without free text, results
//! come in path and tile-key order, or nearest first for `similar:`.

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use rusqlite::params_from_iter;
use rusqlite::types::Value;

use crate::error::Result;
use crate::index::{ASSET_COLS, Hit, Index, asset_from_row};
use crate::query::{Filter, Query};
use crate::user::UserDb;

/// Hamming distance (of 64) up to which two tiles count as similar.
pub const SIMILAR_MAX_DISTANCE: i64 = 10;

struct Sql {
    clauses: Vec<String>,
    params: Vec<Value>,
    /// Set by the first `similar:` filter: the SQL expression for the distance.
    similar_key: Option<String>,
}

impl Sql {
    fn bind(&mut self, value: impl Into<Value>) -> String {
        self.params.push(value.into());
        format!("?{}", self.params.len())
    }
}

fn json_array(keys: &[String]) -> String {
    serde_json::Value::from(keys.to_vec()).to_string()
}

fn clause(f: &Filter, profile: &str, user: &UserDb, sql: &mut Sql) -> Result<String> {
    Ok(match f {
        Filter::Type(kind) => {
            let p = sql.bind(kind.as_str().to_owned());
            format!("a.kind = {p}")
        }
        Filter::Class(class) => {
            let p = sql.bind(class.to_ascii_lowercase());
            format!("lower(a.class) = {p}")
        }
        Filter::Act(n) => {
            let p = sql.bind(i64::from(n.saturating_sub(1)));
            format!("a.act = {p}")
        }
        Filter::Color(color) => {
            let p = sql.bind(color.to_ascii_lowercase());
            format!("lower(a.color) = {p}")
        }
        Filter::Tag(tag) => {
            let keys = user.keys_with_tag(profile, tag)?;
            let p = sql.bind(json_array(&keys));
            format!("a.key IN (SELECT value FROM json_each({p}))")
        }
        Filter::Idx {
            main,
            sub,
            orientation,
        } => {
            let mut parts = vec![format!("a.main = {}", sql.bind(i64::from(*main)))];
            if let Some(sub) = sub {
                parts.push(format!("a.sub = {}", sql.bind(i64::from(*sub))));
            }
            if let Some(orientation) = orientation {
                parts.push(format!(
                    "a.orientation = {}",
                    sql.bind(i64::from(*orientation))
                ));
            }
            format!("({})", parts.join(" AND "))
        }
        Filter::File(text) => {
            let p = sql.bind(text.to_ascii_lowercase());
            format!("instr(lower(a.path), {p}) > 0")
        }
        Filter::Similar(key) => {
            let key = key.to_ascii_lowercase();
            let p = sql.bind(key);
            let distance = format!(
                "hamming(a.phash, (SELECT s.phash FROM assets s WHERE s.profile = ?1 AND s.key = {p}))"
            );
            if sql.similar_key.is_none() {
                sql.similar_key = Some(distance.clone());
            }
            format!("({distance} <= {SIMILAR_MAX_DISTANCE} AND a.key <> {p})")
        }
        Filter::UsedIn(text) => {
            let p = sql.bind(text.to_ascii_lowercase());
            format!(
                "EXISTS (SELECT 1 FROM used_in u WHERE u.asset_id = a.id AND instr(lower(u.ds1_path), {p}) > 0)"
            )
        }
        Filter::Mirrored => "(a.phash IS NOT NULL AND a.phash_mirror IS NOT NULL \
             AND a.phash <> a.phash_mirror \
             AND EXISTS (SELECT 1 FROM assets b WHERE b.profile = a.profile \
                         AND b.phash = a.phash_mirror AND b.id <> a.id))"
            .to_owned(),
        Filter::Src(text) => {
            let text = text.to_ascii_lowercase();
            let tag = sql.bind(text.clone());
            let name = sql.bind(text);
            format!("(a.src_tag = {tag} OR instr(lower(a.src), {name}) > 0)")
        }
        Filter::Not(inner) => {
            let inner = clause(inner, profile, user, sql)?;
            format!("NOT COALESCE(({inner}), 0)")
        }
    })
}

impl Index {
    /// Search one profile.
    ///
    /// `user` supplies tags (for `tag:`) and aliases (shown and searched
    /// instead of the generated name). At most `limit` hits come back, best
    /// first.
    pub fn search(
        &self,
        profile_id: &str,
        query: &Query,
        user: &UserDb,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut sql = Sql {
            clauses: vec!["a.profile = ?1".to_owned()],
            params: vec![Value::from(profile_id.to_owned())],
            similar_key: None,
        };
        for filter in &query.filters {
            let c = clause(filter, profile_id, user, &mut sql)?;
            sql.clauses.push(c);
        }
        let aliases = user.aliases(profile_id)?;
        let text = query.text.trim();
        let where_clause = sql.clauses.join(" AND ");

        if text.is_empty() {
            let order = match &sql.similar_key {
                Some(distance) => format!("{distance}, a.path, a.id"),
                None => "a.path, a.orientation, a.main, a.sub, a.variant, a.id".to_owned(),
            };
            let score = sql
                .similar_key
                .as_ref()
                .map_or_else(|| "0.0".to_owned(), |d| format!("(1.0 - {d} / 64.0)"));
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let statement = format!(
                "SELECT {ASSET_COLS}, {score} FROM assets a WHERE {where_clause} ORDER BY {order} LIMIT {limit}"
            );
            let mut stmt = self.conn.prepare(&statement).map_err(|e| self.sql(e))?;
            let rows = stmt
                .query_map(params_from_iter(sql.params.iter()), |r| {
                    Ok((asset_from_row(r)?, r.get::<_, f64>(19)?))
                })
                .map_err(|e| self.sql(e))?;
            let mut hits = Vec::new();
            for row in rows {
                let (asset, score) = row.map_err(|e| self.sql(e))?;
                let alias = aliases.get(&asset.key).cloned();
                #[allow(clippy::cast_possible_truncation)]
                hits.push(Hit {
                    asset,
                    score: score as f32,
                    alias,
                });
            }
            return Ok(hits);
        }

        // Free text: fetch the filtered candidates' names, rank, then load
        // full rows for the winners only.
        let statement =
            format!("SELECT a.id, a.key, a.gen_name, a.path FROM assets a WHERE {where_clause}");
        let mut stmt = self.conn.prepare(&statement).map_err(|e| self.sql(e))?;
        let rows = stmt
            .query_map(params_from_iter(sql.params.iter()), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| self.sql(e))?;

        let pattern = Pattern::parse(text, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf: Vec<char> = Vec::new();
        let mut scored: Vec<(f32, String, i64)> = Vec::new();
        for row in rows {
            let (id, key, gen_name, path) = row.map_err(|e| self.sql(e))?;
            let mut best: Option<f32> = None;
            let mut consider = |hay: &str, weight: f32| {
                if let Some(s) = pattern.score(Utf32Str::new(hay, &mut buf), &mut matcher) {
                    #[allow(clippy::cast_precision_loss)]
                    let s = s as f32 * weight;
                    if best.is_none_or(|b| s > b) {
                        best = Some(s);
                    }
                }
            };
            if let Some(alias) = aliases.get(&key) {
                consider(alias, 1.1);
            }
            consider(&gen_name, 1.0);
            consider(&path, 0.8);
            if let Some(score) = best {
                scored.push((score, gen_name, id));
            }
        }
        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        scored.truncate(limit);

        let by_id = format!("SELECT {ASSET_COLS} FROM assets a WHERE a.id = ?1");
        let mut load = self.conn.prepare_cached(&by_id).map_err(|e| self.sql(e))?;
        let mut hits = Vec::with_capacity(scored.len());
        for (score, _, id) in scored {
            let asset = load
                .query_row([id], asset_from_row)
                .map_err(|e| self.sql(e))?;
            let alias = aliases.get(&asset.key).cloned();
            hits.push(Hit {
                asset,
                score,
                alias,
            });
        }
        Ok(hits)
    }
}
