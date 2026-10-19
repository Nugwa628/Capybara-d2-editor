//! Search query language: free text plus `key:value` filter tokens.
//!
//! ```text
//! type:floor act:1 color:green tag:"my tag" idx:0:5/2 file:floor.dt1
//! similar:<asset key> used-in:towne1 mirrored:yes src:mod -tag:foo grass
//! ```
//!
//! * Tokens are separated by whitespace. A value may be wrapped in double
//!   quotes (`tag:"my tag"`); there is no escape, a quote cannot appear inside.
//! * A leading `-` negates a filter (`-tag:foo`).
//! * `word:` with an unknown `word` stays free text, so `foo:bar` searches for
//!   "foo:bar". A quoted phrase at the start of a token is free text too.
//! * Positions in [`QueryError`] are byte offsets into the input.
//!
//! `Filter::Act` holds the number the user typed (1 to 5); the database
//! stores acts 0-based, the index converts.

/// What kind of thing an asset row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Tile,
    Ds1,
    Dt1,
    Dc6,
    Dcc,
    Cof,
}

impl AssetKind {
    /// The value stored in the `kind` column.
    pub fn as_str(self) -> &'static str {
        match self {
            AssetKind::Tile => "tile",
            AssetKind::Ds1 => "ds1",
            AssetKind::Dt1 => "dt1",
            AssetKind::Dc6 => "dc6",
            AssetKind::Dcc => "dcc",
            AssetKind::Cof => "cof",
        }
    }

    /// Inverse of [`AssetKind::as_str`] (ASCII case-insensitive).
    pub fn parse(text: &str) -> Option<AssetKind> {
        let lower = text.to_ascii_lowercase();
        Some(match lower.as_str() {
            "tile" => AssetKind::Tile,
            "ds1" => AssetKind::Ds1,
            "dt1" => AssetKind::Dt1,
            "dc6" => AssetKind::Dc6,
            "dcc" => AssetKind::Dcc,
            "cof" => AssetKind::Cof,
            _ => return None,
        })
    }

    /// Upper-case label used in generated file names (`DS1`).
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Tile => "Tile",
            AssetKind::Ds1 => "DS1",
            AssetKind::Dt1 => "DT1",
            AssetKind::Dc6 => "DC6",
            AssetKind::Dcc => "DCC",
            AssetKind::Cof => "COF",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    /// `type:tile|ds1|dt1|dc6|dcc|cof`
    Type(AssetKind),
    /// `type:floor|wall|roof|shadow|special|tree|lowerwall` become
    /// `Type(Tile)` plus a lower-case class.
    Class(String),
    /// `act:1..5`, as typed.
    Act(u8),
    /// `color:green` (lower-cased).
    Color(String),
    /// `tag:foo`, `tag:"my tag"`.
    Tag(String),
    /// `idx:5`, `idx:5/2`, `idx:0:5/2` (orientation:main/sub).
    Idx {
        main: u8,
        sub: Option<u8>,
        orientation: Option<u8>,
    },
    /// `file:floor.dt1`: case-insensitive substring of the path
    /// (lower-cased, `/` turned into `\`).
    File(String),
    /// `similar:<asset key>`: pHash Hamming distance <= 10, nearest first.
    Similar(String),
    /// `used-in:towne1`: substring of a DS1 path (lower-cased, `/` to `\`).
    UsedIn(String),
    /// `mirrored:yes`: has a horizontally mirrored twin (`mirrored:no` is
    /// `Not(Mirrored)`).
    Mirrored,
    /// `src:mod` / `src:base` / `src:saves` / `src:<archive substring>`
    /// (lower-cased).
    Src(String),
    /// `-tag:foo`
    Not(Box<Filter>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Free-text words joined by single spaces.
    pub text: String,
    pub filters: Vec<Filter>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("bad query at {position}: {detail}")]
pub struct QueryError {
    pub position: usize,
    pub detail: String,
}

fn err<T>(position: usize, detail: impl Into<String>) -> Result<T, QueryError> {
    Err(QueryError {
        position,
        detail: detail.into(),
    })
}

#[derive(Clone, Copy)]
enum Key {
    Type,
    Act,
    Color,
    Tag,
    Idx,
    File,
    Similar,
    UsedIn,
    Mirrored,
    Src,
}

fn known_key(word: &str) -> Option<Key> {
    Some(match word.to_ascii_lowercase().as_str() {
        "type" => Key::Type,
        "act" => Key::Act,
        "color" | "colour" => Key::Color,
        "tag" => Key::Tag,
        "idx" => Key::Idx,
        "file" => Key::File,
        "similar" => Key::Similar,
        "used-in" | "used_in" | "usedin" => Key::UsedIn,
        "mirrored" => Key::Mirrored,
        "src" => Key::Src,
        _ => return None,
    })
}

enum TypeValue {
    Kind(AssetKind),
    Class(&'static str),
}

fn type_value(value: &str, at: usize) -> Result<TypeValue, QueryError> {
    let lower = value.to_ascii_lowercase();
    Ok(match lower.as_str() {
        "floor" => TypeValue::Class("floor"),
        "wall" => TypeValue::Class("wall"),
        "roof" => TypeValue::Class("roof"),
        "shadow" => TypeValue::Class("shadow"),
        "special" => TypeValue::Class("special"),
        "tree" => TypeValue::Class("tree"),
        "lowerwall" | "lower-wall" | "lower_wall" => TypeValue::Class("lower wall"),
        other => match AssetKind::parse(other) {
            Some(kind) => TypeValue::Kind(kind),
            None => {
                return err(
                    at,
                    format!(
                        "unknown type {value:?} (tile, floor, wall, roof, shadow, special, \
                         tree, lowerwall, ds1, dt1, dc6, dcc, cof)"
                    ),
                );
            }
        },
    })
}

fn number(text: &str, at: usize, what: &str) -> Result<u8, QueryError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return err(at, format!("{what} must be a number, got {text:?}"));
    }
    match text.parse::<u8>() {
        Ok(n) => Ok(n),
        Err(_) => err(at, format!("{what} {text} is out of range (0-255)")),
    }
}

/// `5`, `5/2`, `0:5`, `0:5/2`.
fn parse_idx(value: &str, at: usize) -> Result<Filter, QueryError> {
    let (orientation, rest, rest_at) = match value.split_once(':') {
        Some((o, rest)) => (Some(number(o, at, "orientation")?), rest, at + o.len() + 1),
        None => (None, value, at),
    };
    let (main, sub) = match rest.split_once('/') {
        Some((m, s)) => {
            let sub_at = rest_at + m.len() + 1;
            (
                number(m, rest_at, "main index")?,
                Some(number(s, sub_at, "sub index")?),
            )
        }
        None => (number(rest, rest_at, "main index")?, None),
    };
    Ok(Filter::Idx {
        main,
        sub,
        orientation,
    })
}

fn path_value(value: &str) -> String {
    value.to_ascii_lowercase().replace('/', "\\")
}

/// Filters for one `key:value` token; `negated` wraps them in `Not`.
fn build(key: Key, value: &str, at: usize, negated: bool) -> Result<Vec<Filter>, QueryError> {
    let positive = match key {
        Key::Type => match type_value(value, at)? {
            TypeValue::Kind(kind) => vec![Filter::Type(kind)],
            TypeValue::Class(class) => {
                // `-type:floor` means "not a floor", which also keeps files.
                if negated {
                    return Ok(vec![Filter::Not(Box::new(Filter::Class(class.to_owned())))]);
                }
                vec![
                    Filter::Type(AssetKind::Tile),
                    Filter::Class(class.to_owned()),
                ]
            }
        },
        Key::Act => {
            let n = number(value, at, "act")?;
            if !(1..=5).contains(&n) {
                return err(at, format!("act must be 1 to 5, got {n}"));
            }
            vec![Filter::Act(n)]
        }
        Key::Color => vec![Filter::Color(value.to_ascii_lowercase())],
        Key::Tag => vec![Filter::Tag(value.to_owned())],
        Key::Idx => vec![parse_idx(value, at)?],
        Key::File => vec![Filter::File(path_value(value))],
        Key::Similar => vec![Filter::Similar(value.to_ascii_lowercase())],
        Key::UsedIn => vec![Filter::UsedIn(path_value(value))],
        Key::Mirrored => match value.to_ascii_lowercase().as_str() {
            "yes" | "true" | "1" | "on" => vec![Filter::Mirrored],
            "no" | "false" | "0" | "off" => vec![Filter::Not(Box::new(Filter::Mirrored))],
            _ => return err(at, format!("mirrored expects yes or no, got {value:?}")),
        },
        Key::Src => vec![Filter::Src(value.to_ascii_lowercase())],
    };
    Ok(if negated {
        positive
            .into_iter()
            .map(|f| Filter::Not(Box::new(f)))
            .collect()
    } else {
        positive
    })
}

fn word_end(input: &str, from: usize) -> usize {
    input[from..]
        .find(char::is_whitespace)
        .map_or(input.len(), |p| from + p)
}

/// Parse a search string. Unknown `word:` stays free text.
pub fn parse_query(input: &str) -> Result<Query, QueryError> {
    let mut words: Vec<String> = Vec::new();
    let mut filters: Vec<Filter> = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let Some(c) = input[i..].chars().next() else {
            break;
        };
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if c == '"' {
            // A quoted phrase is free text.
            let Some(close) = input[i + 1..].find('"') else {
                return err(i, "unterminated quote");
            };
            let phrase = input[i + 1..i + 1 + close].trim();
            if !phrase.is_empty() {
                words.push(phrase.to_owned());
            }
            i += close + 2;
            continue;
        }
        let negated = c == '-';
        let key_start = i + usize::from(negated);
        let key_len: usize = input[key_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphabetic() || *ch == '-' || *ch == '_')
            .map(char::len_utf8)
            .sum();
        let key_end = key_start + key_len;
        let key = if key_len > 0 && input[key_end..].starts_with(':') {
            known_key(&input[key_start..key_end])
        } else {
            None
        };
        let Some(key) = key else {
            let end = word_end(input, i);
            words.push(input[i..end].to_owned());
            i = end;
            continue;
        };
        let value_at = key_end + 1;
        let (value, next, value_at) = if input[value_at..].starts_with('"') {
            let Some(close) = input[value_at + 1..].find('"') else {
                return err(value_at, "unterminated quote");
            };
            (
                &input[value_at + 1..value_at + 1 + close],
                value_at + close + 2,
                value_at + 1,
            )
        } else {
            let end = word_end(input, value_at);
            (&input[value_at..end], end, value_at)
        };
        if value.trim().is_empty() {
            return err(
                value_at,
                format!("missing value for `{}:`", &input[key_start..key_end]),
            );
        }
        filters.extend(build(key, value, value_at, negated)?);
        i = next;
    }
    Ok(Query {
        text: words.join(" "),
        filters,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn q(text: &str, filters: Vec<Filter>) -> Query {
        Query {
            text: text.to_owned(),
            filters,
        }
    }

    fn not(f: Filter) -> Filter {
        Filter::Not(Box::new(f))
    }

    fn idx(main: u8, sub: Option<u8>, orientation: Option<u8>) -> Filter {
        Filter::Idx {
            main,
            sub,
            orientation,
        }
    }

    #[test]
    fn parses_table() {
        let tile = Filter::Type(AssetKind::Tile);
        let cases: Vec<(&str, Query)> = vec![
            ("", q("", vec![])),
            ("   \t ", q("", vec![])),
            ("grass", q("grass", vec![])),
            ("grass shire  floor", q("grass shire floor", vec![])),
            ("type:ds1", q("", vec![Filter::Type(AssetKind::Ds1)])),
            ("type:dt1", q("", vec![Filter::Type(AssetKind::Dt1)])),
            ("TYPE:DC6", q("", vec![Filter::Type(AssetKind::Dc6)])),
            ("type:dcc", q("", vec![Filter::Type(AssetKind::Dcc)])),
            ("type:cof", q("", vec![Filter::Type(AssetKind::Cof)])),
            ("type:tile", q("", vec![tile.clone()])),
            (
                "type:floor",
                q("", vec![tile.clone(), Filter::Class("floor".into())]),
            ),
            (
                "type:wall",
                q("", vec![tile.clone(), Filter::Class("wall".into())]),
            ),
            (
                "type:roof",
                q("", vec![tile.clone(), Filter::Class("roof".into())]),
            ),
            (
                "type:shadow",
                q("", vec![tile.clone(), Filter::Class("shadow".into())]),
            ),
            (
                "type:special",
                q("", vec![tile.clone(), Filter::Class("special".into())]),
            ),
            (
                "type:lowerwall",
                q("", vec![tile.clone(), Filter::Class("lower wall".into())]),
            ),
            (
                "-type:floor",
                q("", vec![not(Filter::Class("floor".into()))]),
            ),
            ("-type:ds1", q("", vec![not(Filter::Type(AssetKind::Ds1))])),
            ("act:1", q("", vec![Filter::Act(1)])),
            ("act:5", q("", vec![Filter::Act(5)])),
            ("color:Green", q("", vec![Filter::Color("green".into())])),
            ("colour:red", q("", vec![Filter::Color("red".into())])),
            ("tag:foo", q("", vec![Filter::Tag("foo".into())])),
            ("tag:Foo", q("", vec![Filter::Tag("Foo".into())])),
            ("tag:\"my tag\"", q("", vec![Filter::Tag("my tag".into())])),
            ("-tag:foo", q("", vec![not(Filter::Tag("foo".into()))])),
            ("-tag:\"a b\"", q("", vec![not(Filter::Tag("a b".into()))])),
            ("idx:5", q("", vec![idx(5, None, None)])),
            ("idx:5/2", q("", vec![idx(5, Some(2), None)])),
            ("idx:0:5/2", q("", vec![idx(5, Some(2), Some(0))])),
            ("idx:13:7", q("", vec![idx(7, None, Some(13))])),
            ("idx:05/02", q("", vec![idx(5, Some(2), None)])),
            (
                "file:Floor.DT1",
                q("", vec![Filter::File("floor.dt1".into())]),
            ),
            (
                "file:mxl/zone",
                q("", vec![Filter::File("mxl\\zone".into())]),
            ),
            (
                "file:\"act1\\town\"",
                q("", vec![Filter::File("act1\\town".into())]),
            ),
            (
                "similar:data\\x.dt1#0/5/2/0",
                q("", vec![Filter::Similar("data\\x.dt1#0/5/2/0".into())]),
            ),
            (
                "used-in:TowneS1",
                q("", vec![Filter::UsedIn("townes1".into())]),
            ),
            ("used_in:a", q("", vec![Filter::UsedIn("a".into())])),
            ("mirrored:yes", q("", vec![Filter::Mirrored])),
            ("mirrored:no", q("", vec![not(Filter::Mirrored)])),
            ("-mirrored:yes", q("", vec![not(Filter::Mirrored)])),
            ("src:mod", q("", vec![Filter::Src("mod".into())])),
            ("src:MXL", q("", vec![Filter::Src("mxl".into())])),
            (
                "grass act:2 type:floor",
                q(
                    "grass",
                    vec![Filter::Act(2), tile.clone(), Filter::Class("floor".into())],
                ),
            ),
            (
                "act:1 grass color:green shire",
                q(
                    "grass shire",
                    vec![Filter::Act(1), Filter::Color("green".into())],
                ),
            ),
            // Unknown `word:` stays free text.
            ("foo:bar", q("foo:bar", vec![])),
            ("-foo:bar", q("-foo:bar", vec![])),
            (
                "http://x type:ds1",
                q("http://x", vec![Filter::Type(AssetKind::Ds1)]),
            ),
            ("12:30", q("12:30", vec![])),
            ("-", q("-", vec![])),
            ("-grass", q("-grass", vec![])),
            ("a:b:c", q("a:b:c", vec![])),
            // Quoted free text.
            ("\"dark grass\"", q("dark grass", vec![])),
            (
                "\"dark grass\" act:3",
                q("dark grass", vec![Filter::Act(3)]),
            ),
            ("\"\"", q("", vec![])),
            // A filter keyword that is only a prefix of a longer word is text.
            ("tagged:x", q("tagged:x", vec![])),
            ("5/2", q("5/2", vec![])),
        ];
        for (input, want) in cases {
            assert_eq!(parse_query(input), Ok(want), "input {input:?}");
        }
    }

    #[test]
    fn error_positions() {
        let cases: Vec<(&str, usize)> = vec![
            ("tag:\"unterminated", 4),
            ("grass tag:\"x", 10),
            ("\"free", 0),
            ("a \"free text", 2),
            ("act:9", 4),
            ("act:0", 4),
            ("act:x", 4),
            ("grass act:7", 10),
            ("type:banana", 5),
            ("-type:banana", 6),
            ("idx:5/", 6),
            ("idx:a", 4),
            ("idx:1:2:3", 6),
            ("idx:5/x", 6),
            ("idx:300", 4),
            ("idx:0:300/1", 6),
            ("idx:1/300", 6),
            ("mirrored:maybe", 9),
            ("tag:", 4),
            ("grass file: x", 11),
            ("tag:\"\"", 5),
            ("tag:\"  \"", 5),
        ];
        for (input, position) in cases {
            match parse_query(input) {
                Err(e) => assert_eq!(e.position, position, "input {input:?}: {e}"),
                Ok(ok) => panic!("input {input:?} parsed as {ok:?}"),
            }
        }
    }

    #[test]
    fn error_display_names_position() {
        let e = parse_query("act:9").unwrap_err();
        assert!(e.to_string().starts_with("bad query at 4: "), "{e}");
    }

    #[test]
    fn multibyte_input_is_safe() {
        assert_eq!(
            parse_query("grünes gras · 05/02"),
            Ok(q("grünes gras · 05/02", vec![]))
        );
        assert!(parse_query("tag:é\"").is_ok());
        assert_eq!(
            parse_query("tag:\"日本 語\""),
            Ok(q("", vec![Filter::Tag("日本 語".into())]))
        );
    }

    #[test]
    fn kind_round_trip() {
        for k in [
            AssetKind::Tile,
            AssetKind::Ds1,
            AssetKind::Dt1,
            AssetKind::Dc6,
            AssetKind::Dcc,
            AssetKind::Cof,
        ] {
            assert_eq!(AssetKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(AssetKind::parse("nope"), None);
        assert_eq!(AssetKind::Ds1.label(), "DS1");
    }
}
