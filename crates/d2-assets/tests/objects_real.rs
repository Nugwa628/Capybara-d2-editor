//! `ObjectCatalog` against every object of every DS1 in the real installs.
//!
//! Directories come from `CAPY_VANILLA_DIR` / `CAPY_MXL_DIR`; a missing
//! install prints `skipped: <path> missing`. Everything is opened read-only.
//! Run with `--nocapture` to see the resolution report.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{dir_available, mxl_dir, vanilla_dir};
use d2_assets::{
    AssetSource, GamePath, GameVersion, KnownNames, ObjectCatalog, ObjectClass, ObjectInfo,
    Overlay, Profile,
};
use d2_formats::ds1::{Ds1, Object};

struct Fixture {
    label: &'static str,
    _saves: tempfile::TempDir,
    overlay: Overlay,
    version: GameVersion,
    catalog: ObjectCatalog,
    warnings: Vec<String>,
    names: KnownNames,
}

fn fixture(label: &'static str, dir: PathBuf, profile: fn(&Path) -> Profile) -> Option<Fixture> {
    if !dir_available(&dir) {
        return None;
    }
    let profile = profile(&dir);
    let saves = tempfile::tempdir().expect("temp saves dir");
    let overlay = profile.open(saves.path()).expect("open profile");
    let version = profile.game_version;
    let (catalog, warnings) = ObjectCatalog::build(&overlay, version).expect("build");
    let (names, _) = KnownNames::harvest(&overlay, version).expect("harvest");
    Some(Fixture {
        label,
        _saves: saves,
        overlay,
        version,
        catalog,
        warnings,
        names,
    })
}

fn vanilla() -> Option<&'static Fixture> {
    static F: std::sync::OnceLock<Option<Fixture>> = std::sync::OnceLock::new();
    F.get_or_init(|| fixture("vanilla", vanilla_dir(), Profile::vanilla))
        .as_ref()
}

fn mxl() -> Option<&'static Fixture> {
    static F: std::sync::OnceLock<Option<Fixture>> = std::sync::OnceLock::new();
    F.get_or_init(|| fixture("median-xl", mxl_dir(), Profile::median_xl))
        .as_ref()
}

#[derive(Default)]
struct Report {
    files: usize,
    unparsed: Vec<String>,
    total: usize,
    by_source: BTreeMap<&'static str, usize>,
    by_class: BTreeMap<String, usize>,
    /// `(kind, id, act)` of unresolved objects with their occurrences.
    unresolved: BTreeMap<(i32, i32, u8), usize>,
    /// Per kind: (objects, with a token, resolved).
    per_kind: BTreeMap<i32, (usize, usize, usize)>,
    /// Type 1 objects that resolved but have no token, by `(act, id, name)`.
    tokenless: BTreeMap<(u8, i32, String), usize>,
    /// Type 1 objects that resolved to a MonStats row but only got the
    /// `monster <row>` fallback name, and those whose parts list is empty.
    fallback_named: usize,
    no_parts: usize,
}

fn ds1_names(f: &Fixture) -> Vec<GamePath> {
    f.names
        .iter()
        .filter(|p| p.extension() == Some("ds1"))
        .cloned()
        .collect()
}

fn report(f: &Fixture) -> Report {
    let mut r = Report::default();
    for path in ds1_names(f) {
        let Some(bytes) = f.overlay.read_bytes(&path).expect("read") else {
            continue;
        };
        let ds1 = match Ds1::parse(&bytes) {
            Ok(ds1) => ds1,
            Err(e) => {
                r.unparsed.push(format!("{path}: {e}"));
                continue;
            }
        };
        r.files += 1;
        let act = ds1.effective_act();
        for object in &ds1.objects {
            let info = f.catalog.resolve(act, ds1.version, object);
            r.total += 1;
            *r.by_source.entry(info.source).or_default() += 1;
            let class = match info.class {
                ObjectClass::Npc => "npc (type 1)".to_owned(),
                ObjectClass::Object => "object".to_owned(),
                ObjectClass::Other(k) => format!("other({k})"),
            };
            *r.by_class.entry(class).or_default() += 1;
            let kind = r.per_kind.entry(object.kind).or_default();
            kind.0 += 1;
            kind.1 += usize::from(info.token.is_some());
            kind.2 += usize::from(info.source != "unresolved");
            if info.source == "unresolved" {
                *r.unresolved
                    .entry((object.kind, object.id, act))
                    .or_default() += 1;
            } else if object.kind == 1 && info.token.is_some() {
                r.fallback_named += usize::from(info.name.starts_with("monster "));
                r.no_parts += usize::from(info.parts.is_empty());
            }
            if info.source != "unresolved" && info.token.is_none() && object.kind == 1 {
                *r.tokenless
                    .entry((act, object.id, info.name.clone()))
                    .or_default() += 1;
            }
        }
    }
    r
}

fn print_report(f: &Fixture, r: &Report) {
    println!("== {} ==", f.label);
    for w in &f.warnings {
        println!("warning: {w}");
    }
    println!(
        "{} ds1 files parsed ({} failed), {} objects",
        r.files,
        r.unparsed.len(),
        r.total
    );
    for u in &r.unparsed {
        println!("unparsed: {u}");
    }
    println!(
        "type 1 with a token: {} named by the row fallback, {} without MonStats2 parts",
        r.fallback_named, r.no_parts
    );
    println!("by source: {:?}", r.by_source);
    println!("by class: {:?}", r.by_class);
    for (kind, (n, tokens, resolved)) in &r.per_kind {
        println!(
            "kind {kind}: {n} objects, {resolved} resolved ({:.2}%), {tokens} with a token ({:.2}%)",
            100.0 * *resolved as f64 / *n as f64,
            100.0 * *tokens as f64 / *n as f64,
        );
    }
    let mut top: Vec<_> = r.unresolved.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("top unresolved (kind, id, act) x count:");
    for ((kind, id, act), n) in top.iter().take(10) {
        println!("  ({kind}, {id}, {act}) x {n}");
    }
    let mut tokenless: Vec<_> = r.tokenless.iter().collect();
    tokenless.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("top type 1 without a token (act, id, name) x count:");
    for ((act, id, name), n) in tokenless.iter().take(12) {
        println!("  ({act}, {id}, {name}) x {n}");
    }
}

/// The Objects row DS1Edit's table gives for type 2 `(act, id)`.
fn embedded_objects_id(act: u8, id: i32) -> Option<u32> {
    const OBJ_TXT: &str = include_str!("../../../assets/editor/obj.txt");
    OBJ_TXT.lines().skip(1).find_map(|line| {
        let cells: Vec<&str> = line.split('\t').collect();
        let matches = cells.first().and_then(|c| c.parse::<u8>().ok()) == Some(act + 1)
            && cells.get(1) == Some(&"2")
            && cells.get(2).and_then(|c| c.parse::<i32>().ok()) == Some(id);
        if matches {
            cells.get(4).and_then(|c| c.trim().parse().ok())
        } else {
            None
        }
    })
}

fn obj(kind: i32, id: i32) -> Object {
    Object {
        kind,
        id,
        x: 0,
        y: 0,
        flags: 0,
    }
}

fn load_ds1(f: &Fixture, path: &str) -> Ds1 {
    let bytes = f
        .overlay
        .read_bytes(&GamePath::new(path))
        .unwrap()
        .unwrap_or_else(|| panic!("{path} missing"));
    Ds1::parse(&bytes).unwrap()
}

#[test]
fn vanilla_resolves_every_object() {
    let Some(f) = vanilla() else { return };
    let r = report(f);
    print_report(f, &r);
    assert!(r.total > 10_000, "only {} objects", r.total);
    assert!(r.unparsed.is_empty(), "{:?}", r.unparsed);

    let (n2, tokens2, _) = r.per_kind.get(&2).copied().unwrap_or_default();
    assert!(n2 > 0);
    // The hardcoded table sends some slots to Objects rows 580-582, which the
    // 573-row vanilla Objects.txt does not have (the game cannot create them
    // either; DS1Edit lists them as "-580", "-581", "-582").
    let beyond_table: usize = r
        .unresolved
        .iter()
        .filter(|((kind, id, act), _)| {
            *kind == 2 && embedded_objects_id(*act, *id).is_some_and(|row| row >= 573)
        })
        .map(|(_, n)| *n)
        .sum();
    let unexplained = n2 - tokens2 - beyond_table;
    let raw = 100.0 * tokens2 as f64 / n2 as f64;
    let adjusted = 100.0 * tokens2 as f64 / (n2 - beyond_table) as f64;
    println!(
        "vanilla type 2 token rate: {raw:.3}% ({beyond_table} point past Objects.txt, \
         {unexplained} unexplained); without those {adjusted:.3}%"
    );
    assert_eq!(unexplained, 0, "{:?}", r.unresolved);
    assert!(raw >= 98.0, "type 2 token rate {raw:.3}%");
    assert!(adjusted >= 99.0, "type 2 token rate {adjusted:.3}%");
    let (n1, tokens1, resolved1) = r.per_kind.get(&1).copied().unwrap_or_default();
    println!(
        "vanilla type 1: {resolved1}/{n1} resolved, {tokens1}/{n1} with a token ({:.2}%)",
        100.0 * tokens1 as f64 / n1.max(1) as f64
    );
    // The rest are MonPlace spawn markers (groups, corpses, "?") that DS1Edit has no sprite for.
    assert_eq!(resolved1, n1);
    assert!(tokens1 * 100 >= n1 * 70, "type 1 token rate too low");
}

#[test]
fn median_xl_resolves_every_object() {
    let Some(f) = mxl() else { return };
    let r = report(f);
    print_report(f, &r);
    assert!(r.total > 0);
    // Type 2 goes through objpreset.bin and objects.bin.
    let (n2, tokens2, _) = r.per_kind.get(&2).copied().unwrap_or_default();
    println!(
        "median-xl type 2: {tokens2}/{n2} with a token ({:.2}%)",
        100.0 * tokens2 as f64 / n2.max(1) as f64
    );
    assert!(*r.by_source.get("objpreset").unwrap_or(&0) > 0);
    // Type 1 goes through monpreset.bin, monstats.bin, monstats2.bin and
    // superuniques.bin.
    assert!(
        !f.warnings
            .iter()
            .any(|w| w.contains("MonPreset") || w.contains("cannot be loaded")),
        "{:?}",
        f.warnings
    );
    let (n1, tokens1, resolved1) = r.per_kind.get(&1).copied().unwrap_or_default();
    println!(
        "median-xl type 1: {resolved1}/{n1} resolved ({:.2}%), {tokens1}/{n1} with a token ({:.2}%)",
        100.0 * resolved1 as f64 / n1.max(1) as f64,
        100.0 * tokens1 as f64 / n1.max(1) as f64,
    );
    assert!(*r.by_source.get("monpreset").unwrap_or(&0) > 0);
    // What is left without a token are MonPlace markers (groups, corpses),
    // exactly as in vanilla.
    assert_eq!(resolved1, n1);
    assert!(tokens1 * 100 >= n1 * 85, "type 1 token rate too low");
}

#[test]
fn rogue_encampment_spot_check() {
    let Some(f) = vanilla() else { return };
    let ds1 = load_ds1(f, r"data\global\tiles\act1\town\towne1.ds1");
    let act = ds1.effective_act();
    assert_eq!(act, 0);
    let expected = [
        (0, "GH", "gheed"),
        (2, "PS", "akara"),
        (3, "CK", "chicken"),
        (4, "RG", "rogue"),
        (5, "RC", "kashya"),
        (7, "WA", "warriv"),
        (8, "CI", "charsi"),
        (13, "CW", "cow"),
    ];
    for (id, token, who) in expected {
        let info = f.catalog.resolve(act, ds1.version, &obj(1, id));
        println!("type 1 id {id}: {who} -> {info:?}");
        assert_eq!(info.source, "monpreset", "{who}");
        assert_eq!(info.token.as_deref(), Some(token), "{who}");
        assert_eq!(info.class, ObjectClass::Npc);
        assert!(
            info.name.to_ascii_lowercase().contains(who),
            "{who}: name {:?}",
            info.name
        );
    }
    // The ids that are really in the file resolve too.
    let present: Vec<&Object> = ds1.objects.iter().filter(|o| o.kind == 1).collect();
    assert!(!present.is_empty());
    for o in present {
        let info = f.catalog.resolve(act, ds1.version, o);
        assert_ne!(info.source, "unresolved", "type 1 id {}", o.id);
    }

    // Type 2 id 110 in act 0 is the Cain start position, Objects row 385.
    let cain = f.catalog.resolve(act, ds1.version, &obj(2, 110));
    println!("type 2 id 110: {cain:?}");
    assert_eq!(cain.source, "obj.txt");
    assert_eq!(cain.row, Some(385));
    assert_eq!(cain.class, ObjectClass::Object);
    assert!(cain.name.contains("cain"), "{:?}", cain.name);
    // Skins come from obj.txt: every skinned component is an enabled part,
    // and skin tokens are 3-letter codes such as LIT.
    assert!(!cain.skins.is_empty(), "cain should carry obj.txt skins");
    for (component, skin) in &cain.skins {
        assert!(cain.parts.contains(component), "{component} not in parts");
        assert_eq!(skin.len(), 3, "{component}={skin}");
    }
    // Same answer through any DS1 version that remaps.
    assert_eq!(f.catalog.resolve(0, 6, &obj(2, 110)).row, Some(385));
    // Below version 6 the id is a direct row.
    assert_eq!(f.catalog.resolve(0, 5, &obj(2, 385)).row, Some(385));
}

#[test]
fn placeable_lists() {
    for f in [vanilla(), mxl()].into_iter().flatten() {
        for act in 0..5u8 {
            let list = f.catalog.placeable(act);
            println!(
                "{} act {act}: {} placeable ({} type 1, {} type 2)",
                f.label,
                list.len(),
                list.iter().filter(|i| i.ds1_kind == 1).count(),
                list.iter().filter(|i| i.ds1_kind == 2).count()
            );
            if f.label == "vanilla" {
                assert!(!list.is_empty(), "act {act}");
                assert!(list.iter().any(|i| i.ds1_kind == 1), "act {act} type 1");
                assert!(list.iter().any(|i| i.ds1_kind == 2), "act {act} type 2");
            }
            assert!(list.iter().all(|i| i.source != "unresolved"));
            let again: Vec<ObjectInfo> = list
                .iter()
                .take(20)
                .map(|i| f.catalog.resolve(act, 18, &obj(i.ds1_kind, i.ds1_id)))
                .collect();
            assert_eq!(&again[..], &list[..again.len()]);
        }
    }
}

/// A whole overlay as one layer, with the monster tables hidden so that type 1
/// has to come from the embedded obj.txt.
struct HideMonsters(Overlay);

impl AssetSource for HideMonsters {
    fn name(&self) -> &str {
        "hide-monsters"
    }
    fn kind(&self) -> d2_assets::SourceKind {
        d2_assets::SourceKind::Folder
    }
    fn contains(&self, path: &GamePath) -> bool {
        !hidden(path) && self.0.resolve(path).is_some()
    }
    fn read(&self, path: &GamePath) -> d2_assets::Result<Option<Vec<u8>>> {
        if hidden(path) {
            Ok(None)
        } else {
            self.0.read_bytes(path)
        }
    }
    fn list(&self) -> d2_assets::Result<Vec<GamePath>> {
        Ok(Vec::new())
    }
    fn fingerprint(&self) -> d2_assets::Result<d2_assets::SourceFingerprint> {
        Ok(d2_assets::SourceFingerprint {
            path: PathBuf::new(),
            size: 0,
            modified_unix_ms: 0,
            entries: 0,
        })
    }
}

fn hidden(path: &GamePath) -> bool {
    let name = path.file_name();
    path.as_str().starts_with(r"data\global\excel\")
        && (name.starts_with("monpreset")
            || name.starts_with("monstats")
            || name.starts_with("superuniques"))
}

/// The MonPreset route and DS1Edit's obj.txt describe the same game table:
/// they have to agree on the sprite token wherever both know one.
#[test]
fn monpreset_route_agrees_with_embedded_obj_txt() {
    let Some(f) = vanilla() else { return };
    let saves = tempfile::tempdir().unwrap();
    let inner = Profile::vanilla(&vanilla_dir()).open(saves.path()).unwrap();
    let overlay = Overlay::new(vec![Box::new(HideMonsters(inner))]);
    let (catalog, warnings) = ObjectCatalog::build(&overlay, f.version).unwrap();
    assert!(
        warnings.iter().any(|w| w.contains("MonPreset")),
        "{warnings:?}"
    );

    let (mut both, mut agree, mut only_a, mut only_b) = (0, 0, 0, 0);
    let mut disagreements = Vec::new();
    for act in 0..5u8 {
        for id in 0..60 {
            let a = f.catalog.resolve(act, 18, &obj(1, id));
            let b = catalog.resolve(act, 18, &obj(1, id));
            match (&a.token, &b.token) {
                (Some(x), Some(y)) => {
                    both += 1;
                    if x.eq_ignore_ascii_case(y) {
                        agree += 1;
                    } else {
                        disagreements.push(format!(
                            "act {act} id {id}: monpreset {x} ({}) vs obj.txt {y} ({})",
                            a.name, b.name
                        ));
                    }
                }
                (Some(_), None) => only_a += 1,
                (None, Some(_)) => only_b += 1,
                (None, None) => {}
            }
        }
    }
    println!(
        "type 1 token agreement: {agree}/{both}; only monpreset {only_a}, only obj.txt {only_b}"
    );
    for d in &disagreements {
        println!("  {d}");
    }
    assert!(both > 100);
    assert!(agree * 100 >= both * 90, "{disagreements:#?}");
}
