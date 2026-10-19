//! File > "Import DS1Edit ini mappings...": DS1Edit's legacy `assets/*.ini`
//! files (`<LvlTypesId> <LvlPrestDef> <path>` per line) become per-profile
//! map overrides, in the map catalog and in `config.toml`.

use std::collections::BTreeSet;

use d2_assets::{GamePath, MapCatalog, parse_legacy_ini};

use crate::config::Config;

/// What an import did, for the toast.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IniImport {
    pub files: usize,
    /// Lines that parsed.
    pub entries: usize,
    /// Distinct maps of the profile that now have an override.
    pub applied: usize,
    /// Lines naming a map the profile does not have.
    pub unknown: usize,
    /// Bad lines and unreadable files, `file: line n: why`.
    pub warnings: Vec<String>,
    /// The maps that got an override (to refresh open tabs).
    pub maps: Vec<GamePath>,
}

impl IniImport {
    /// The one-line toast text.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "Imported {} mapping(s) from {} file(s): {} map(s) now use the ini's LvlType/Def",
            self.entries, self.files, self.applied
        );
        if self.unknown > 0 {
            text.push_str(&format!(
                ", {} line(s) name maps this profile does not have",
                self.unknown
            ));
        }
        if !self.warnings.is_empty() {
            text.push_str(&format!(", {} warning(s) in the log", self.warnings.len()));
        }
        text
    }
}

/// Parses every `(file name, text)`, applies the entries to `catalog`
/// (`MapCatalog::import_legacy`) and records the ones it accepted as
/// overrides of `profile` in `config`. Later lines win over earlier ones,
/// in the catalog and in the config alike.
pub fn import(
    config: &mut Config,
    catalog: &mut MapCatalog,
    profile: &str,
    files: &[(String, String)],
) -> IniImport {
    let mut report = IniImport {
        files: files.len(),
        ..IniImport::default()
    };
    let mut entries = Vec::new();
    for (name, text) in files {
        let (parsed, warnings) = parse_legacy_ini(text);
        report
            .warnings
            .extend(warnings.into_iter().map(|w| format!("{name}: {w}")));
        entries.extend(parsed);
    }
    report.entries = entries.len();
    catalog.import_legacy(&entries);
    // What the catalog accepted is what it now lists as an override with the
    // entry's values (the last line for a map wins in both).
    let overrides = catalog.overrides();
    let mut applied = BTreeSet::new();
    for entry in &entries {
        let accepted = overrides.iter().any(|(ds1, lvl_type, def)| {
            ds1 == &entry.ds1 && *lvl_type == entry.lvl_type && *def == Some(entry.lvlprest_def)
        });
        if accepted {
            config.set_override(
                profile,
                entry.ds1.as_str(),
                entry.lvl_type,
                Some(entry.lvlprest_def),
            );
            applied.insert(entry.ds1.clone());
        } else if !overrides.iter().any(|(ds1, ..)| ds1 == &entry.ds1) {
            report.unknown += 1;
        }
    }
    report.applied = applied.len();
    report.maps = applied.into_iter().collect();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_assets::{GameVersion, Overlay};

    fn catalog() -> MapCatalog {
        let overlay = Overlay::new(Vec::new());
        let known = vec![
            GamePath::new("data\\global\\tiles\\act1\\town\\towne1.ds1"),
            GamePath::new("data\\global\\tiles\\act2\\town\\townn1.ds1"),
        ];
        let (catalog, _) =
            MapCatalog::build(&overlay, GameVersion::V114d, &known).expect("empty catalog");
        catalog
    }

    #[test]
    fn mappings_become_persisted_per_profile_overrides() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("config.toml");
        let mut config = Config::default();
        config.set_override("other", "data\\global\\tiles\\act1\\town\\towne1.ds1", 9, None);
        let mut cat = catalog();
        let files = vec![
            (
                "act1.ini".to_owned(),
                "; DS1Edit act 1\n\
                 1 2 assets/tiles/ACT1/TOWN/townE1.ds1\n\
                 garbage line\n\
                 3 4 assets/tiles/ACT1/NOPE/missing.ds1\n"
                    .to_owned(),
            ),
            (
                "act2.ini".to_owned(),
                "5 6 Assets\\Tiles\\Act2\\Town\\TownN1.ds1\n\
                 1 7 assets/tiles/act1/town/towne1.ds1\n"
                    .to_owned(),
            ),
        ];
        let report = import(&mut config, &mut cat, "vanilla", &files);
        assert_eq!(report.files, 2);
        assert_eq!(report.entries, 4);
        assert_eq!(report.applied, 2);
        assert_eq!(report.unknown, 1);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].starts_with("act1.ini: line 3"), "{:?}", report.warnings);
        assert!(report.summary().contains("4 mapping(s) from 2 file(s)"));

        // The catalog uses them...
        let towne1 = GamePath::new("data\\global\\tiles\\act1\\town\\towne1.ds1");
        assert!(
            cat.overrides()
                .contains(&(towne1.clone(), 1, Some(7))),
            "the later line wins: {:?}",
            cat.overrides()
        );
        // ...and config.toml keeps them for the profile, the other profile's
        // override untouched.
        config.save(&file).expect("save");
        let back = Config::load(&file).expect("load");
        let mut mine: Vec<(String, u32, Option<u32>)> = back
            .overrides_for("vanilla")
            .map(|o| (o.ds1.clone(), o.lvl_type, o.def))
            .collect();
        mine.sort();
        assert_eq!(
            mine,
            vec![
                (towne1.as_str().to_owned(), 1, Some(7)),
                (
                    "data\\global\\tiles\\act2\\town\\townn1.ds1".to_owned(),
                    5,
                    Some(6)
                ),
            ]
        );
        assert_eq!(back.overrides_for("other").count(), 1);
    }

    #[test]
    fn nothing_to_import_changes_nothing() {
        let mut config = Config::default();
        let mut cat = catalog();
        let report = import(&mut config, &mut cat, "vanilla", &[("e.ini".into(), String::new())]);
        assert_eq!(report.entries, 0);
        assert_eq!(report.applied, 0);
        assert!(config.overrides.is_empty());
        assert!(cat.overrides().is_empty());
    }
}
