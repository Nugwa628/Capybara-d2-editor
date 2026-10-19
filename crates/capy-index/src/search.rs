//! `Index::search`: SQL does the filtering, nucleo ranks the free text.
//!
//! Free text is matched fuzzily against the user's alias, the generated name
//! and the path (best of the three, with the alias slightly favoured and the
//! path slightly discounted). Filters are exact. Without free text, results
//! come in path and tile-key order, or nearest first for `similar:`.

use nucleo::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo::{Config, Matcher, Utf32Str};
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use std::cell::RefCell;

use crate::error::Result;
use crate::index::{ASSET_COLS, Hit, Index, asset_from_row};
use crate::query::{Filter, Query};
use crate::user::UserDb;

/// Hamming distance (of 64) up to which two tiles count as similar.
pub const SIMILAR_MAX_DISTANCE: i64 = 10;

struct Sql {
    clauses: Vec<String>,
    params: Vec<Value>,
    /// Set by the first `similar:` filter: the SQL expressions for the
    /// distance and for "is the reference asset itself".
    similar_key: Option<(String, String)>,
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
                sql.similar_key = Some((distance.clone(), format!("a.key = {p}")));
            }
            format!("{distance} <= {SIMILAR_MAX_DISTANCE}")
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

/// Bonuses added to the nucleo score of a token, per field and match kind.
/// Alias beats generated name beats path beats plain fuzzy.
const ALIAS_PREFIX: f32 = 600.0;
const ALIAS_SUBSTRING: f32 = 500.0;
const NAME_PREFIX: f32 = 400.0;
const NAME_SUBSTRING: f32 = 300.0;
const PATH_SEGMENT: f32 = 150.0;
const PATH_SUBSTRING: f32 = 60.0;
/// A fuzzy-only path match counts for less than the same name match.
const PATH_FUZZY_WEIGHT: f32 = 0.6;
/// A fuzzy match with no alias/name/path hit counts only when its nucleo
/// score is at least this share of the token's score against itself.
pub(crate) const FUZZY_FLOOR: f32 = 0.7;

fn word_prefix(haystack_lower: &str, token: &str) -> bool {
    haystack_lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(token))
}

fn segment_prefix(path_lower: &str, token: &str) -> bool {
    path_lower
        .split(['\\', '/', '.', '#'])
        .any(|s| s.starts_with(token))
}

/// Scores free text against an asset: whitespace-separated tokens, every one
/// of which must match (AND). A token's score is the best over alias, name
/// and path of its fuzzy score plus a tier bonus; the asset's score is the sum.
struct Ranker {
    /// Per token: the fuzzy atom, the lower-cased token and the atom's score
    /// against the token itself (the best a fuzzy match can do).
    tokens: Vec<(Atom, String, f32)>,
    scratch: RefCell<(Matcher, Vec<char>, Vec<char>)>,
}

impl Ranker {
    fn new(text: &str) -> Ranker {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();
        let tokens = text
            .split_whitespace()
            .map(|t| {
                let atom = Atom::new(
                    t,
                    CaseMatching::Smart,
                    Normalization::Smart,
                    AtomKind::Fuzzy,
                    false,
                );
                let own = atom
                    .score(Utf32Str::new(t, &mut buf), &mut matcher)
                    .map_or(0.0, f32::from);
                (atom, t.to_lowercase(), own)
            })
            .collect();
        Ranker {
            tokens,
            scratch: RefCell::new((matcher, Vec::new(), Vec::new())),
        }
    }

    fn score(&self, alias: Option<&str>, name: &str, path: &str) -> Option<f32> {
        let mut guard = self.scratch.borrow_mut();
        let (matcher, buf, _) = &mut *guard;
        let name_lower = name.to_lowercase();
        let path_lower = path.to_lowercase();
        let alias_lower = alias.map(str::to_lowercase);
        let mut total = 0.0f32;
        for (atom, token, own) in &self.tokens {
            // Fuzzy score of a field; `hit` says the token is already a
            // prefix, substring or segment hit there. Without a hit the
            // score must reach the floor, so loose subsequences never match.
            let mut fuzzy = |hay: &str, hit: bool| -> Option<f32> {
                let score = atom
                    .score(Utf32Str::new(hay, buf), matcher)
                    .map(f32::from)?;
                (hit || score >= FUZZY_FLOOR * own).then_some(score)
            };
            let mut best: Option<f32> = None;
            let mut offer = |s: Option<f32>| {
                if let Some(s) = s
                    && best.is_none_or(|b| s > b)
                {
                    best = Some(s);
                }
            };
            if let (Some(alias), Some(lower)) = (alias, alias_lower.as_deref()) {
                let bonus = if word_prefix(lower, token) {
                    ALIAS_PREFIX
                } else if lower.contains(token.as_str()) {
                    ALIAS_SUBSTRING
                } else {
                    0.0
                };
                offer(fuzzy(alias, bonus > 0.0).map(|f| f + bonus));
            }
            let bonus = if word_prefix(&name_lower, token) {
                NAME_PREFIX
            } else if name_lower.contains(token.as_str()) {
                NAME_SUBSTRING
            } else {
                0.0
            };
            offer(fuzzy(name, bonus > 0.0).map(|f| f + bonus));
            let bonus = if segment_prefix(&path_lower, token) {
                PATH_SEGMENT
            } else if path_lower.contains(token.as_str()) {
                PATH_SUBSTRING
            } else {
                0.0
            };
            offer(fuzzy(path, bonus > 0.0).map(|f| f * PATH_FUZZY_WEIGHT + bonus));
            total += best?;
        }
        Some(total)
    }
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
                // The reference asset itself first, then nearest first.
                Some((distance, itself)) => format!("{itself} DESC, {distance}, a.path, a.id"),
                None => "a.path, a.orientation, a.main, a.sub, a.variant, a.id".to_owned(),
            };
            let score = sql
                .similar_key
                .as_ref()
                .map_or_else(|| "0.0".to_owned(), |(d, _)| format!("(1.0 - {d} / 64.0)"));
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let statement = format!(
                "SELECT {ASSET_COLS}, {score} FROM assets a WHERE {where_clause} ORDER BY {order} LIMIT {limit}"
            );
            let mut stmt = self.conn.prepare(&statement).map_err(|e| self.sql(e))?;
            let rows = stmt
                .query_map(params_from_iter(sql.params.iter()), |r| {
                    Ok((asset_from_row(r)?, r.get::<_, f64>(21)?))
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

        let ranker = Ranker::new(text);
        let mut scored: Vec<(f32, String, i64)> = Vec::new();
        for row in rows {
            let (id, key, gen_name, path) = row.map_err(|e| self.sql(e))?;
            let alias = aliases.get(&key).map(String::as_str);
            if let Some(score) = ranker.score(alias, &gen_name, &path) {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const YSHARI_NAME: &str = "Mxl Quest Yshari Trees · Floor · dark · 04/55";
    const YSHARI_PATH: &str = "data\\global\\tiles\\mxl\\quest\\yshari\\trees.dt1#0/4/55/0";
    const SHIRE_NAME: &str = "Mxl Zone Whimsy Shire · Floor · green · 01/02";
    const SHIRE_PATH: &str = "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.dt1#0/1/2/0";
    const FLOOR_NAME: &str = "Mxl Zone Whimsy Whimsyfloor · Floor · green · 23/05";
    const FLOOR_PATH: &str = "data\\global\\tiles\\mxl\\zone\\whimsy\\whimsyfloor.dt1#0/23/5/0";

    #[test]
    fn loose_fuzzy_hits_do_not_match() {
        let ranker = Ranker::new("shire floor");
        assert!(
            ranker.score(None, YSHARI_NAME, YSHARI_PATH).is_none(),
            "`shire` is only a loose subsequence of the Yshari names"
        );
        assert!(ranker.score(None, SHIRE_NAME, SHIRE_PATH).is_some());
        // Any subsequence used to match; now only close ones do.
        assert!(
            Ranker::new("yre")
                .score(None, SHIRE_NAME, SHIRE_PATH)
                .is_none()
        );
    }

    #[test]
    fn prefix_substring_and_segment_hits_always_match() {
        assert!(
            Ranker::new("shi")
                .score(None, SHIRE_NAME, SHIRE_PATH)
                .is_some()
        );
        assert!(
            Ranker::new("hire")
                .score(None, SHIRE_NAME, SHIRE_PATH)
                .is_some(),
            "substring of the name"
        );
        assert!(
            Ranker::new("shire.dt1")
                .score(None, "unrelated", SHIRE_PATH)
                .is_some(),
            "path substring"
        );
        assert!(
            Ranker::new("meadow")
                .score(Some("Meadow mosaic"), "unrelated", "x")
                .is_some(),
            "alias prefix"
        );
    }

    /// The share of its self-match score the typo `whimsyflor` reaches on the
    /// real name and path; the floor is [`FUZZY_FLOOR`].
    #[test]
    fn typo_ratio_is_above_the_floor() {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();
        let atom = Atom::new(
            "whimsyflor",
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );
        let own = f32::from(
            atom.score(Utf32Str::new("whimsyflor", &mut buf), &mut matcher)
                .unwrap(),
        );
        let on_name = f32::from(
            atom.score(
                Utf32Str::new(&FLOOR_NAME.to_lowercase(), &mut buf),
                &mut matcher,
            )
            .unwrap(),
        );
        let on_path = f32::from(
            atom.score(
                Utf32Str::new(&FLOOR_PATH.to_lowercase(), &mut buf),
                &mut matcher,
            )
            .unwrap(),
        );
        println!(
            "whimsyflor: self {own}, name {on_name} ({:.2}), path {on_path} ({:.2})",
            on_name / own,
            on_path / own
        );
        assert!(on_name / own >= FUZZY_FLOOR || on_path / own >= FUZZY_FLOOR);
        assert!(
            Ranker::new("whimsyflor 23/05")
                .score(None, FLOOR_NAME, FLOOR_PATH)
                .is_some()
        );
    }
}
