//! Compatibility fixes against the real vanilla and Median XL tables
//! (`CAPY_PRIVATE_DIR`): every table fix leaves a state that applying it again
//! does not change, and the `ShortenPaths` renames do the same. Skipped when
//! the corpus is absent.

#![allow(clippy::unwrap_used)]

mod common;

use capy_mod::compat::rename::{apply_renames, rename_problems, suggest_renames};
use capy_mod::compat::tables::Ctx;
use capy_mod::compat::{Env, FixAction, Tables, verify_in_game};
use common::{corpus, read_doc};

#[test]
fn fixes_are_idempotent_on_the_games_own_tables() {
    for profile in ["vanilla", "mxl"] {
        let Some(root) = corpus(profile) else {
            continue;
        };
        let (Ok(prest), Ok(levels), Ok(types)) = (
            read_doc(&root, "lvlprest.txt"),
            read_doc(&root, "levels.txt"),
            read_doc(&root, "lvltypes.txt"),
        ) else {
            continue;
        };
        let mut tables = Tables {
            prest: Some(prest.clone()),
            levels: Some(levels.clone()),
            types: Some(types.clone()),
            automap: read_doc(&root, "automap.txt").ok(),
        };
        let ctx = Ctx {
            prest: &prest,
            levels: &levels,
            types: &types,
        };
        let env = Env::default();
        let mut fixes = Vec::new();
        let mut long = Vec::new();
        for r in 0..prest.row_count() {
            for n in 1..=1 {
                let Some(c) = prest.column(&format!("File{n}")) else {
                    continue;
                };
                let Some(file) = prest.cell(r, c).filter(|f| !f.is_empty() && f != "0") else {
                    continue;
                };
                for f in verify_in_game(&ctx, &file, (8, 8), &env) {
                    for fix in f.fixes {
                        match fix.action {
                            FixAction::Edits(e) => fixes.push(e),
                            FixAction::ShortenPaths { paths } => long.extend(paths),
                            _ => {}
                        }
                    }
                }
            }
        }
        let mut applied = 0usize;
        for edits in &fixes {
            for e in edits {
                applied += usize::from(e.apply_tables(&mut tables).unwrap());
            }
        }
        let snapshot = tables.clone();
        for edits in &fixes {
            for e in edits {
                assert!(!e.apply_tables(&mut tables).unwrap(), "{profile}: {e:?}");
            }
        }
        assert_eq!(tables.prest, snapshot.prest, "{profile}");
        assert_eq!(tables.types, snapshot.types, "{profile}");
        long.sort();
        long.dedup();
        let renames = suggest_renames(&long);
        assert!(
            rename_problems(&renames).iter().all(|p| p.is_none()),
            "{profile}: {:?}",
            rename_problems(&renames)
        );
        let changed = apply_renames(&mut tables, &renames);
        let snapshot = tables.clone();
        assert!(apply_renames(&mut tables, &renames).is_empty(), "{profile}");
        assert_eq!(tables.types, snapshot.types, "{profile}");
        println!(
            "{profile}: {} fix lists ({applied} edits changed something), {} long paths, {} rows renamed",
            fixes.len(),
            long.len(),
            changed.len()
        );
    }
}
