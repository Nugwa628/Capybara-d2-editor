//! `KnownNames::harvest` on small synthetic folders: table and DS1 chains,
//! unresolved candidates, and robustness against bad files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use d2_assets::{AssetSource, FolderSource, GamePath, GameVersion, KnownNames, Overlay};
use d2_formats::ds1::Ds1;

fn ds1_bytes(refs: &[&str]) -> Vec<u8> {
    let mut ds1 = Ds1::new(4, 4, 0);
    ds1.files = refs.iter().map(|r| r.as_bytes().to_vec()).collect();
    ds1.to_bytes().unwrap()
}

/// A folder that cannot list its names, like `Patch_D2.mpq`.
struct Unlisted(FolderSource);

impl AssetSource for Unlisted {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn kind(&self) -> d2_assets::SourceKind {
        self.0.kind()
    }
    fn contains(&self, path: &GamePath) -> bool {
        self.0.contains(path)
    }
    fn read(&self, path: &GamePath) -> d2_assets::Result<Option<Vec<u8>>> {
        self.0.read(path)
    }
    fn list(&self) -> d2_assets::Result<Vec<GamePath>> {
        Ok(Vec::new())
    }
    fn fingerprint(&self) -> d2_assets::Result<d2_assets::SourceFingerprint> {
        self.0.fingerprint()
    }
}

/// Folder overlay whose layer lists its files (`listed`) or does not.
fn overlay(files: &[(&str, Vec<u8>)], listed: bool) -> (tempfile::TempDir, Overlay) {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in files {
        let p = GamePath::new(name).to_native(dir.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    let folder = FolderSource::new(dir.path(), "t");
    let layer: Box<dyn AssetSource> = if listed {
        Box::new(folder)
    } else {
        Box::new(Unlisted(folder))
    };
    (dir, Overlay::new(vec![layer]))
}

fn has(names: &KnownNames, path: &str) -> bool {
    names.contains(&GamePath::new(path))
}

const TILES: &str = r"data\global\tiles\";

#[test]
fn follows_tables_and_ds1_references_and_drops_unresolved() {
    let lvltypes =
        b"Name\tId\tAct\tFile 1\tFile 2\r\nTown\t1\t1\tAct1\\Floor.dt1\tact1\\missing.dt1\r\n"
            .to_vec();
    let lvlprest =
        b"Def\tLevelId\tDt1Mask\tFile1\tFile2\r\n1\t1\t0\tact1\\x.ds1\tact1\\gone.ds1\r\n".to_vec();
    let files = [
        (r"data\global\excel\lvltypes.txt", lvltypes),
        (r"data\global\excel\lvlprest.txt", lvlprest),
        (r"data\global\tiles\act1\floor.dt1", vec![0]),
        (
            r"data\global\tiles\act1\x.ds1",
            ds1_bytes(&[
                r"C:\d2\data\global\tiles\ACT1\Walls.tg1",
                r"C:\d2\data\global\tiles\ACT1\Ghost.tg1",
                "DS1EDIT_WRKSPC_PATH=elsewhere",
            ]),
        ),
        (r"data\global\tiles\act1\walls.dt1", vec![0]),
    ];
    let (_dir, overlay) = overlay(&files, false);
    let (names, stats) = KnownNames::harvest(&overlay, GameVersion::V114d).unwrap();

    assert!(has(&names, &format!("{TILES}act1\\floor.dt1")));
    assert!(has(&names, &format!("{TILES}act1\\walls.dt1")));
    assert!(has(&names, &format!("{TILES}act1\\x.ds1")));
    for dropped in ["act1\\missing.dt1", "act1\\gone.ds1", "act1\\ghost.dt1"] {
        assert!(!has(&names, &format!("{TILES}{dropped}")), "{dropped}");
    }
    // every kept name resolves
    assert!(names.iter().all(|p| overlay.resolve(p).is_some()));
    // missing.dt1, gone.ds1, ghost.dt1 at least (the seed adds more)
    assert!(stats.unresolved >= 3, "{stats:?}");
    // nothing is listed: x.ds1 and floor.dt1 come from the tables, walls.dt1 from x.ds1
    assert_eq!(stats.from_listfiles, 0);
    assert!(stats.from_tables >= 2 && stats.from_ds1 >= 1, "{stats:?}");
    assert_eq!(stats.from_cof, 0);
    assert!(stats.rounds >= 1);
    assert!(stats.warnings.is_empty(), "{:#?}", stats.warnings);
}

#[test]
fn chain_through_two_rounds() {
    // a.ds1 is only a name in lvlprest and exists; its dt1 is found via the DS1.
    let lvlprest = b"Def\tLevelId\tDt1Mask\tFile1\r\n1\t1\t0\tdeep\\a.ds1\r\n".to_vec();
    let lvltypes = b"Name\tId\tAct\tFile 1\r\nT\t1\t1\t0\r\n".to_vec();
    let files = [
        (r"data\global\excel\lvlprest.txt", lvlprest),
        (r"data\global\excel\lvltypes.txt", lvltypes),
        (
            r"data\global\tiles\deep\a.ds1",
            ds1_bytes(&[r"deep\tiles.tg1"]),
        ),
        (r"data\global\tiles\deep\tiles.dt1", vec![0]),
    ];
    let (_dir, overlay) = overlay(&files, false);
    let (names, stats) = KnownNames::harvest(&overlay, GameVersion::V114d).unwrap();
    assert!(has(&names, &format!("{TILES}deep\\tiles.dt1")));
    assert!(stats.warnings.is_empty(), "{:#?}", stats.warnings);
}

#[test]
fn bad_files_are_warnings_not_failures() {
    let files = [
        // a lvltypes.txt that is not a table
        (
            r"data\global\excel\lvltypes.txt",
            b"nonsense\r\n1\r\n".to_vec(),
        ),
        // lvlprest.bin of the wrong size, preferred by 1.13c
        (r"data\global\excel\lvlprest.bin", vec![7, 0, 0, 0, 1, 2, 3]),
        (r"data\global\tiles\bad.ds1", vec![1, 2, 3]),
        (r"data\global\tiles\empty.ds1", Vec::new()),
        (r"data\global\tiles\good.ds1", ds1_bytes(&[r"good.tg1"])),
        (r"data\global\tiles\good.dt1", vec![0]),
    ];
    let (_dir, overlay) = overlay(&files, true);
    for version in [GameVersion::V113c, GameVersion::V114d, GameVersion::D2r] {
        let (names, stats) = KnownNames::harvest(&overlay, version).unwrap();
        assert!(has(&names, r"data\global\tiles\good.dt1"), "{version:?}");
        assert!(has(&names, r"data\global\tiles\bad.ds1"));
        let joined = stats.warnings.join("\n");
        assert!(joined.contains("bad.ds1"), "{joined}");
        assert!(joined.contains("empty.ds1"), "{joined}");
        assert!(joined.contains("lvlprest"), "{joined}");
    }
}

#[test]
fn excel_table_names_get_both_forms() {
    let files = [
        (r"data\global\excel\monstats.txt", b"Id\r\n".to_vec()),
        (r"data\global\excel\skills.bin", vec![0, 0, 0, 0]),
        (r"data\global\excel\armor.txt", b"x\r\n".to_vec()),
        (r"data\global\excel\armor.bin", vec![0, 0, 0, 0]),
    ];
    let (_dir, overlay) = overlay(&files, true);
    let (names, stats) = KnownNames::harvest(&overlay, GameVersion::V114d).unwrap();
    // listed names plus nothing invented: absent siblings are dropped, not kept
    assert!(has(&names, r"data\global\excel\monstats.txt"));
    assert!(!has(&names, r"data\global\excel\monstats.bin"));
    assert!(!has(&names, r"data\global\excel\skills.txt"));
    assert!(has(&names, r"data\global\excel\armor.bin"));
    assert!(stats.unresolved >= 2, "{stats:?}");
}

#[test]
fn empty_overlay_harvests_nothing() {
    let (names, stats) =
        KnownNames::harvest(&Overlay::new(Vec::new()), GameVersion::V113c).unwrap();
    assert!(names.is_empty());
    assert!(stats.unresolved > 0, "the seed names are all unresolved");
    assert_eq!(stats.from_listfiles + stats.from_seed + stats.from_ds1, 0);
}
