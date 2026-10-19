//! The browser's scope toggles, as extra query filters.
//!
//! Filters of one [`Query`] are ANDed. "Any of the map's DT1s" is a single
//! query with [`Filter::AnyFile`]; "any sprite kind" becomes one query per
//! kind, which the worker runs and [`merge_hits`] puts together (best score
//! first, ties in the order of the alternatives).

use capy_index::{AssetKind, Filter, Hit, Query};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Tiles of the DT1s the active map loads.
    MapTileSet,
    /// Every tile of the profile (any asset with an explicit `type:`).
    #[default]
    Profile,
    /// DC6, DCC and COF files.
    Sprites,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::MapTileSet, Scope::Profile, Scope::Sprites];

    pub fn label(self) -> &'static str {
        match self {
            Scope::MapTileSet => "This map's tile set",
            Scope::Profile => "Whole profile",
            Scope::Sprites => "Sprites",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Scope::MapTileSet => "map",
            Scope::Profile => "profile",
            Scope::Sprites => "sprites",
        }
    }

    pub fn from_slug(text: &str) -> Option<Scope> {
        Scope::ALL
            .into_iter()
            .find(|s| s.slug().eq_ignore_ascii_case(text.trim()))
    }
}

fn has_type(query: &Query) -> bool {
    query
        .filters
        .iter()
        .any(|f| matches!(f, Filter::Type(_) | Filter::Class(_)))
}

fn with(query: &Query, extra: Filter) -> Query {
    let mut q = query.clone();
    q.filters.push(extra);
    q
}

/// The queries that make up `query` under `scope`. `map_dt1s` are the DT1
/// paths of the active map, in load order. An empty list means "nothing
/// matches" (map scope without a map).
pub fn scoped_queries(query: &Query, scope: Scope, map_dt1s: &[String]) -> Vec<Query> {
    match scope {
        Scope::Profile if has_type(query) => vec![query.clone()],
        Scope::Profile => vec![with(query, Filter::Type(AssetKind::Tile))],
        Scope::MapTileSet if map_dt1s.is_empty() => Vec::new(),
        Scope::MapTileSet => {
            let tiles = if has_type(query) {
                query.clone()
            } else {
                with(query, Filter::Type(AssetKind::Tile))
            };
            vec![with(&tiles, Filter::AnyFile(map_dt1s.to_vec()))]
        }
        Scope::Sprites if has_type(query) => vec![query.clone()],
        Scope::Sprites => [AssetKind::Dc6, AssetKind::Dcc, AssetKind::Cof]
            .into_iter()
            .map(|kind| with(query, Filter::Type(kind)))
            .collect(),
    }
}

/// Answers of several queries as one list: best score first, ties in input
/// order, each asset once, at most `limit`.
pub fn merge_hits(lists: Vec<Vec<Hit>>, limit: usize) -> Vec<Hit> {
    if lists.len() == 1 {
        let mut only = lists.into_iter().next().unwrap_or_default();
        only.truncate(limit);
        return only;
    }
    let mut all: Vec<Hit> = lists.into_iter().flatten().collect();
    all.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen = std::collections::HashSet::new();
    all.retain(|h| seen.insert(h.asset.key.clone()));
    all.truncate(limit);
    all
}

/// Is `path` (a DT1 game path) one of the map's?
pub fn in_tile_set(path: &str, map_dt1s: &[String]) -> bool {
    map_dt1s.iter().any(|p| p.eq_ignore_ascii_case(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capy_index::{AssetRow, parse_query};

    fn hit(key: &str, score: f32) -> Hit {
        Hit {
            asset: AssetRow {
                id: 0,
                key: key.to_owned(),
                kind: AssetKind::Tile,
                path: String::new(),
                act: None,
                orientation: None,
                main: None,
                sub: None,
                variant: None,
                rarity: None,
                width: None,
                height: None,
                class: None,
                gen_name: String::new(),
                color: None,
                texture: None,
                transparency: None,
                src: String::new(),
                src_tag: String::new(),
                directions: None,
                frames: None,
            },
            score,
            alias: None,
        }
    }

    #[test]
    fn profile_scope_defaults_to_tiles() {
        let q = parse_query("grass").expect("q");
        let out = scoped_queries(&q, Scope::Profile, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].filters, vec![Filter::Type(AssetKind::Tile)]);
        assert_eq!(out[0].text, "grass");
        // An explicit type is respected.
        let q = parse_query("type:ds1 town").expect("q");
        assert_eq!(scoped_queries(&q, Scope::Profile, &[]), vec![q.clone()]);
    }

    #[test]
    fn map_scope_is_one_query_over_every_dt1() {
        let dt1s = vec![
            "data\\global\\tiles\\act1\\town\\floor.dt1".to_owned(),
            "data\\global\\tiles\\act1\\town\\fence.dt1".to_owned(),
        ];
        let q = parse_query("type:floor color:green").expect("q");
        let out = scoped_queries(&q, Scope::MapTileSet, &dt1s);
        assert_eq!(out.len(), 1);
        let query = &out[0];
        assert_eq!(query.filters.last(), Some(&Filter::AnyFile(dt1s.clone())));
        assert!(query.filters.contains(&Filter::Color("green".into())));
        let tile_filters = query
            .filters
            .iter()
            .filter(|f| **f == Filter::Type(AssetKind::Tile))
            .count();
        assert_eq!(tile_filters, 1, "type:floor already narrows to tiles");
        // Without a type the scope adds `type:tile`.
        let plain = scoped_queries(&parse_query("grass").expect("q"), Scope::MapTileSet, &dt1s);
        assert_eq!(
            plain[0].filters,
            vec![Filter::Type(AssetKind::Tile), Filter::AnyFile(dt1s.clone())]
        );
        // No map: nothing to search.
        assert!(scoped_queries(&q, Scope::MapTileSet, &[]).is_empty());
        assert!(in_tile_set(
            "DATA\\global\\tiles\\act1\\town\\floor.dt1",
            &dt1s
        ));
        assert!(!in_tile_set("data\\global\\tiles\\act2\\floor.dt1", &dt1s));
    }

    #[test]
    fn sprites_scope_ors_the_three_kinds() {
        let q = parse_query("zombie").expect("q");
        let out = scoped_queries(&q, Scope::Sprites, &[]);
        let kinds: Vec<&Filter> = out.iter().filter_map(|q| q.filters.last()).collect();
        assert_eq!(
            kinds,
            vec![
                &Filter::Type(AssetKind::Dc6),
                &Filter::Type(AssetKind::Dcc),
                &Filter::Type(AssetKind::Cof)
            ]
        );
        let q = parse_query("type:cof").expect("q");
        assert_eq!(scoped_queries(&q, Scope::Sprites, &[]).len(), 1);
        assert_eq!(Scope::from_slug("Sprites"), Some(Scope::Sprites));
        assert_eq!(Scope::from_slug("nope"), None);
    }

    #[test]
    fn merging_ranks_by_score_keeps_ties_in_order_and_dedupes() {
        let merged = merge_hits(
            vec![
                vec![hit("a", 0.0), hit("b", 0.0)],
                vec![hit("c", 0.5), hit("a", 0.0), hit("d", 0.0)],
            ],
            10,
        );
        let keys: Vec<&str> = merged.iter().map(|h| h.asset.key.as_str()).collect();
        assert_eq!(keys, vec!["c", "a", "b", "d"]);
        assert_eq!(
            merge_hits(vec![vec![hit("a", 1.0), hit("b", 0.5)]], 1).len(),
            1
        );
        assert!(merge_hits(Vec::new(), 5).is_empty());
    }
}
