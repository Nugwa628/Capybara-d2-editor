//! The `ShortenPaths` fix: give over-long tile paths shorter names. Each file
//! is copied to its new path by the caller; this module checks the new names
//! and points every `LvlTypes.txt` (`File 1`..`File 32`) and `LvlPrest.txt`
//! (`File1`..`File6`) cell that names an old path at the new one.
//!
//! Applying the same renames twice changes nothing the second time: the old
//! name is no longer in the tables.

use super::tables::{MAX_TILE_PATH, norm_path};
use super::{TableName, Tables};
use crate::doc_util::{get, set_if_different};

/// One file to move: both paths are relative to `data/global/tiles`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    /// The path now.
    pub from: String,
    /// The path it gets.
    pub to: String,
}

/// A suggested short name for each path (see
/// [`crate::tiles_dt1::suggest_short_path`]), as editable rename rows.
pub fn suggest_renames(paths: &[String]) -> Vec<Rename> {
    paths
        .iter()
        .map(|p| Rename {
            from: p.clone(),
            to: crate::tiles_dt1::suggest_short_path(&p.replace('\\', "/"), MAX_TILE_PATH),
        })
        .collect()
}

/// Why each new name cannot be used (`None` = fine): empty, still too long,
/// a different extension, or the same path as another row's.
pub fn rename_problems(renames: &[Rename]) -> Vec<Option<String>> {
    let clean: Vec<String> = renames
        .iter()
        .map(|r| {
            r.to.trim()
                .replace('\\', "/")
                .trim_start_matches('/')
                .to_owned()
        })
        .collect();
    let ext = |p: &str| {
        p.rfind('.')
            .map(|i| p[i..].to_ascii_lowercase())
            .unwrap_or_default()
    };
    renames
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let n = &clean[i];
            if n.is_empty() {
                Some("Give it a path.".to_owned())
            } else if n.len() > MAX_TILE_PATH {
                Some(format!("{} too many", n.len() - MAX_TILE_PATH))
            } else if !n.to_ascii_lowercase().ends_with(&ext(&r.from)) {
                Some(format!("Keep the {} ending", ext(&r.from)))
            } else if clean
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.eq_ignore_ascii_case(n))
            {
                Some("Two files would share this path".to_owned())
            } else {
                None
            }
        })
        .collect()
}

/// Points the table cells that name `from` at `to`, for every rename whose
/// name actually changes. Returns the rows it changed ("id name").
pub fn apply_renames(tables: &mut Tables, renames: &[Rename]) -> Vec<String> {
    let mut changed = Vec::new();
    for r in renames {
        let (from, to) = (norm_path(&r.from), r.to.trim().replace('\\', "/"));
        if from == norm_path(&to) {
            continue;
        }
        for (table, prefix, count, id_col) in [
            (TableName::LvlTypes, "File ", 32usize, "Id"),
            (TableName::LvlPrest, "File", 6, "Def"),
        ] {
            let Some(doc) = tables.doc_mut(table) else {
                continue;
            };
            for row in 0..doc.row_count() {
                for n in 1..=count {
                    let col = format!("{prefix}{n}");
                    let v = get(doc, row, &col);
                    if v.is_empty() || v == "0" || norm_path(&v) != from {
                        continue;
                    }
                    if set_if_different(doc, row, &col, &to).is_some() {
                        let name = format!("{} {}", get(doc, row, id_col), get(doc, row, "Name"));
                        let name = name.trim().to_owned();
                        if !changed.contains(&name) {
                            changed.push(name);
                        }
                    }
                }
            }
        }
    }
    changed
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use d2_formats::txt::TxtDoc;

    fn long() -> String {
        "act1/outdoors/a_really_long_folder_name/some very long tile file name.dt1".to_owned()
    }

    #[test]
    fn suggestions_fit_and_keep_the_extension() {
        let r = suggest_renames(&[long()]);
        assert!(r[0].to.len() <= MAX_TILE_PATH, "{}", r[0].to);
        assert!(r[0].to.ends_with(".dt1"));
        assert_eq!(rename_problems(&r), vec![None]);
    }

    #[test]
    fn problems_are_named() {
        let mk = |to: &str| Rename {
            from: "a/b.dt1".into(),
            to: to.into(),
        };
        let p = rename_problems(&[
            mk(""),
            mk(&"x".repeat(50)),
            mk("c.ds1"),
            mk("d.dt1"),
            mk("D.DT1"),
        ]);
        assert!(p[0].is_some() && p[1].is_some() && p[2].is_some());
        assert!(p[3].is_some() && p[4].is_some(), "two share a path");
    }

    #[test]
    fn renames_update_both_tables_and_are_idempotent() {
        let from = long();
        let types = TxtDoc::parse(
            format!("Name\tId\tFile 1\tFile 2\nTown\t1\t{from}\t0\nOther\t2\tx/y.dt1\t{from}\n")
                .as_bytes(),
        );
        let prest = TxtDoc::parse(
            format!(
                "Name\tDef\tFile1\tFile2\nA\t7\t{}\t0\n",
                from.to_uppercase().replace("/", "\\")
            )
            .as_bytes(),
        );
        let mut t = Tables {
            types: Some(types),
            prest: Some(prest),
            ..Tables::default()
        };
        let r = vec![Rename {
            from: from.clone(),
            to: "act1/short.dt1".into(),
        }];
        let changed = apply_renames(&mut t, &r);
        assert_eq!(changed.len(), 3, "{changed:?}");
        let ty = t.types.as_ref().unwrap();
        assert_eq!(get(ty, 0, "File 1"), "act1/short.dt1");
        assert_eq!(get(ty, 1, "File 2"), "act1/short.dt1");
        assert_eq!(get(t.prest.as_ref().unwrap(), 0, "File1"), "act1/short.dt1");
        let before = t.clone();
        assert!(apply_renames(&mut t, &r).is_empty());
        assert_eq!(t.types, before.types);
        assert_eq!(t.prest, before.prest);
    }
}
