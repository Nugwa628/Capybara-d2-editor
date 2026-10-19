//! `ObjectCatalog` on small synthetic tables in a temp folder overlay.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use d2_assets::{
    AssetSource, FolderSource, GamePath, GameVersion, ObjectCatalog, ObjectClass, ObjectInfo,
    Overlay, SpriteBase,
};
use d2_formats::ds1::Object;

fn overlay(files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, Overlay) {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in files {
        let p = GamePath::new(name).to_native(dir.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    let layer: Box<dyn AssetSource> = Box::new(FolderSource::new(dir.path(), "t"));
    (dir, Overlay::new(vec![layer]))
}

fn tsv(rows: &[Vec<String>]) -> Vec<u8> {
    let mut out = String::new();
    for row in rows {
        out.push_str(&row.join("\t"));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

fn s(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_owned()).collect()
}

/// Objects.txt with `count` rows named `Obj<n>` with token `T<n>`, plus a few
/// distinctive ones, and an "Expansion" separator after row 2.
fn objects_txt(count: usize) -> Vec<u8> {
    let mut rows = vec![s(&[
        "Name",
        "description - not loaded",
        "Id",
        "Token",
        "SizeX",
        "SizeY",
        "FrameCnt0",
        "FrameCnt1",
        "FrameCnt2",
        "FrameDelta0",
        "FrameDelta1",
        "FrameDelta2",
        "OrderFlag0",
        "OrderFlag1",
        "OrderFlag2",
        "Xoffset",
        "Yoffset",
        "Trans",
        "TR",
        "HD",
    ])];
    let mut data = Vec::new();
    for n in 0..count {
        data.push(vec![
            format!("Obj{n}"),
            format!("object {n}"),
            n.to_string(),
            format!("T{n}"),
            "2".into(),
            "3".into(),
            "10".into(),
            "20".into(),
            "30".into(),
            "128".into(),
            "0".into(),
            "64".into(),
            "1".into(),
            "2".into(),
            "0".into(),
            "-4".into(),
            "7".into(),
            "0".into(),
            "1".into(),
            "0".into(),
        ]);
    }
    // Row 1 is a chest with a real string key; row 2 has a transparency.
    data[1][0] = "chest".into();
    data[1][3] = "CH".into();
    data[2][17] = "3".into();
    // A placeholder name: the description is shown instead.
    if let Some(row) = data.get_mut(385) {
        row[0] = "Dummy".into();
        row[1] = "cain start position".into();
    }
    // A separator the game skips: indices after it shift down by one.
    let marker = {
        let mut m = vec!["Expansion".to_owned()];
        m.resize(20, String::new());
        m
    };
    data.insert(3, marker);
    rows.extend(data);
    tsv(&rows)
}

fn objpreset_txt(entries: &[(u8, u16, u32)]) -> Vec<u8> {
    let mut rows = vec![s(&["Act", "Id", "ObjectsId"])];
    for (act, id, object) in entries {
        rows.push(vec![act.to_string(), id.to_string(), object.to_string()]);
    }
    tsv(&rows)
}

/// MonStats with `count` filler rows and named rows at the front.
fn monstats_txt(count: usize) -> Vec<u8> {
    let mut rows = vec![s(&["Id", "hcIdx", "NameStr", "MonStatsEx", "Code"])];
    let named = [
        ("gheed", "Gheed", "GH"),
        ("cain1", "DeckardCain", "DC"),
        ("akara", "Akara", "PS"),
        ("chicken", "dummy", "CK"),
        ("fallen1", "Fallen", "FA"),
    ];
    for n in 0..count {
        let (id, name, code) = named.get(n).map_or_else(
            || (format!("mon{n}"), format!("Mon{n}"), format!("M{}", n % 90)),
            |(i, nm, c)| ((*i).to_owned(), (*nm).to_owned(), (*c).to_owned()),
        );
        rows.push(vec![id.clone(), n.to_string(), name, id, code]);
    }
    tsv(&rows)
}

fn monstats2_txt() -> Vec<u8> {
    tsv(&[
        s(&["Id", "SizeX", "SizeY", "BaseW", "HD", "TR", "LG"]),
        s(&["gheed", "1", "2", "", "0", "1", "0"]),
        s(&["cain1", "1", "1", "", "0", "1", "0"]),
        s(&["akara", "1", "1", "BOW", "1", "1", "1"]),
        s(&["fallen1", "1", "1", "", "0", "1", "0"]),
    ])
}

fn monpreset_txt() -> Vec<u8> {
    // Act 1 has 12 rows: row 10 is a MonPlace marker (DS1Edit's id 10 is "Fallens").
    let mut rows = vec![s(&["Act", "Place"])];
    for place in [
        "gheed",
        "cain1",
        "akara",
        "chicken",
        "Bishibosh",
        "place_nothing",
        "place_nothing",
        "place_nothing",
        "place_nothing",
        "place_nothing",
        "place_fallen",
        "akara",
    ] {
        rows.push(s(&["1", place]));
    }
    for (act, place) in [
        ("2", "gheed"),
        ("2", "cain1"),
        ("3", "akara"),
        ("3", "akara"),
        ("4", "gheed"),
        ("5", "gheed"),
    ] {
        rows.push(s(&[act, place]));
    }
    tsv(&rows)
}

fn superuniques_txt() -> Vec<u8> {
    tsv(&[
        s(&["Superunique", "Name", "Class"]),
        s(&["Bishibosh", "Bishibosh", "fallen1"]),
    ])
}

/// A `.tbl` holding the given pairs.
fn tbl(entries: &[(&str, &str)]) -> Vec<u8> {
    const HEADER: usize = 0x15;
    const NODE: usize = 17;
    let h = entries.len();
    let n = h.max(1);
    let data_start = HEADER + h * 2 + n * NODE;
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for (key, text) in entries {
        let k = data_start + data.len();
        data.extend_from_slice(key.as_bytes());
        data.push(0);
        let t = data_start + data.len();
        data.extend_from_slice(text.as_bytes());
        data.push(0);
        offsets.push((k, t, text.len() + 1));
    }
    let mut out = vec![0u8; HEADER];
    out[2..4].copy_from_slice(&u16::try_from(h).unwrap().to_le_bytes());
    out[4..8].copy_from_slice(&u32::try_from(n).unwrap().to_le_bytes());
    out[8] = 1;
    out[9..13].copy_from_slice(&u32::try_from(data_start).unwrap().to_le_bytes());
    out[17..21].copy_from_slice(
        &u32::try_from(data_start + data.len())
            .unwrap()
            .to_le_bytes(),
    );
    for i in 0..h {
        out.extend_from_slice(&u16::try_from(i).unwrap().to_le_bytes());
    }
    for (i, (k, t, tl)) in offsets.iter().enumerate() {
        out.push(1);
        out.extend_from_slice(&u16::try_from(i).unwrap().to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(*k).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(*t).unwrap().to_le_bytes());
        out.extend_from_slice(&u16::try_from(*tl).unwrap().to_le_bytes());
    }
    if h == 0 {
        out.extend_from_slice(&[0u8; NODE]);
    }
    out.extend_from_slice(&data);
    out
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

const EXCEL: &str = r"data\global\excel\";

fn excel(name: &str) -> String {
    format!("{EXCEL}{name}")
}

fn build(
    files: &[(&str, Vec<u8>)],
    version: GameVersion,
) -> (tempfile::TempDir, ObjectCatalog, Vec<String>) {
    let (dir, ov) = overlay(files);
    let (catalog, warnings) = ObjectCatalog::build(&ov, version).unwrap();
    (dir, catalog, warnings)
}

fn resolve(c: &ObjectCatalog, act: u8, version: i32, kind: i32, id: i32) -> ObjectInfo {
    c.resolve(act, version, &obj(kind, id))
}

// ---------------------------------------------------------------- type 2

#[test]
fn type2_through_objpreset() {
    let files = [
        (excel("objects.txt"), objects_txt(30)),
        (
            excel("objpreset.txt"),
            objpreset_txt(&[(0, 5, 2), (1, 7, 1), (0, 9, 0), (2, 149, 29)]),
        ),
    ];
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (_d, c, warnings) = build(&files, GameVersion::V114d);
    assert!(
        !warnings.iter().any(|w| w.contains("ObjPreset")),
        "{warnings:?}"
    );

    let a = resolve(&c, 0, 18, 2, 5);
    assert_eq!(a.source, "objpreset");
    assert_eq!(a.class, ObjectClass::Object);
    assert_eq!((a.ds1_kind, a.ds1_id), (2, 5));
    assert_eq!(a.row, Some(2));
    assert_eq!(a.token.as_deref(), Some("T2"));
    assert_eq!(a.sprite_base, Some(SpriteBase::Objects));
    assert_eq!(a.size, (2, 3));
    assert_eq!(a.offset, (-4, 7));
    assert_eq!(a.draw_effect, Some(3));
    assert_eq!(a.parts, ["TR"]);
    // Mode NU: FrameCnt0, FrameDelta0, OrderFlag0.
    assert_eq!(a.mode, "NU");
    assert_eq!(a.frame_count, Some(10));
    assert_eq!(a.frame_delta, Some(128));
    assert_eq!(a.order_flag, 1);

    // The act picks the slice.
    let b = resolve(&c, 1, 18, 2, 7);
    assert_eq!((b.source, b.row), ("objpreset", Some(1)));
    assert_eq!(b.token.as_deref(), Some("CH"));
    assert_eq!(resolve(&c, 0, 18, 2, 7).source, "unresolved");

    // ObjectsId 0 marks an unused slot.
    assert_eq!(resolve(&c, 0, 18, 2, 9).source, "unresolved");
    // Last slot of act 2.
    assert_eq!(resolve(&c, 2, 18, 2, 149).row, Some(29));
    // Act is clamped to 0..=4.
    assert_eq!(resolve(&c, 9, 18, 2, 5).source, "unresolved");
}

#[test]
fn type2_ids_from_150_and_pre_v6_are_direct_rows() {
    let files = [
        (excel("objects.txt"), objects_txt(30)),
        (excel("objpreset.txt"), objpreset_txt(&[(0, 5, 2)])),
    ];
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (_d, c, _w) = build(&files, GameVersion::V114d);

    // id >= 150: Objects row id - 150.
    let hi = resolve(&c, 3, 18, 2, 150 + 17);
    assert_eq!((hi.source, hi.row), ("objpreset", Some(17)));
    assert_eq!(hi.token.as_deref(), Some("T17"));
    // Row 150 - 150 = 0 is test data.
    assert_eq!(resolve(&c, 0, 18, 2, 150).source, "unresolved");
    // Past the table.
    assert_eq!(resolve(&c, 0, 18, 2, 150 + 30).source, "unresolved");

    // Below version 6 the id is the row itself, whatever the act.
    let old = resolve(&c, 4, 5, 2, 17);
    assert_eq!(old.row, Some(17));
    assert_eq!(old.token.as_deref(), Some("T17"));
    // ... and is rejected past the end of Objects.txt.
    assert_eq!(resolve(&c, 0, 5, 2, 30).source, "unresolved");
    assert_eq!(resolve(&c, 0, 2, 2, 5000).source, "unresolved");
    // At version 6 the remap is back (id 5 -> slot (0, 5) -> row 2).
    assert_eq!(resolve(&c, 0, 6, 2, 5).row, Some(2));
}

#[test]
fn type2_negative_ids_read_earlier_acts() {
    let files = [
        (excel("objects.txt"), objects_txt(30)),
        (
            excel("objpreset.txt"),
            objpreset_txt(&[(0, 149, 11), (1, 3, 12)]),
        ),
    ];
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (_d, c, _w) = build(&files, GameVersion::V114d);
    // Act 1, id -1 is slot 149 of act 0.
    assert_eq!(resolve(&c, 1, 18, 2, -1).row, Some(11));
    // Before the first slot of the table.
    assert_eq!(resolve(&c, 0, 18, 2, -1).source, "unresolved");
    assert_eq!(resolve(&c, 0, 18, 2, i32::MIN).source, "unresolved");
}

#[test]
fn type2_through_embedded_obj_txt() {
    // No ObjPreset table: DS1Edit's obj.txt stands in for the hardcoded one.
    let files = [(excel("objects.txt"), objects_txt(600))];
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (_d, c, warnings) = build(&files, GameVersion::V114d);
    assert!(
        warnings.iter().any(|w| w.contains("ObjPreset")),
        "{warnings:?}"
    );

    // Act 0 id 110 is the Cain start position, Objects row 385; the editor draws it as Cain.
    let cain = resolve(&c, 0, 18, 2, 110);
    assert_eq!(cain.source, "obj.txt");
    assert_eq!(cain.row, Some(385));
    assert_eq!(cain.token.as_deref(), Some("DC"));
    assert_eq!(cain.sprite_base, Some(SpriteBase::Monsters));
    assert_eq!(cain.mode, "NU");
    assert!(
        cain.name.starts_with("cain start position"),
        "{}",
        cain.name
    );

    // A slot that DS1Edit knows nothing about.
    assert_eq!(resolve(&c, 0, 18, 2, 140).source, "unresolved");
    // ids >= 150 and old DS1s do not need the table.
    assert_eq!(resolve(&c, 0, 18, 2, 150 + 20).row, Some(20));
    assert_eq!(resolve(&c, 0, 5, 2, 20).row, Some(20));
    // The Expansion separator is not a row: row 3 is the first row after it.
    assert_eq!(resolve(&c, 0, 18, 2, 150 + 3).token.as_deref(), Some("T3"));
}

// ---------------------------------------------------------------- type 1

fn mon_files(monstats_rows: usize) -> Vec<(String, Vec<u8>)> {
    vec![
        (excel("objects.txt"), objects_txt(30)),
        (excel("monpreset.txt"), monpreset_txt()),
        (excel("monstats.txt"), monstats_txt(monstats_rows)),
        (excel("monstats2.txt"), monstats2_txt()),
        (excel("superuniques.txt"), superuniques_txt()),
    ]
}

fn build_owned(
    files: &[(String, Vec<u8>)],
    version: GameVersion,
) -> (tempfile::TempDir, ObjectCatalog, Vec<String>) {
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    build(&files, version)
}

#[test]
fn type1_through_monpreset_and_monstats() {
    let (_d, c, warnings) = build_owned(&mon_files(40), GameVersion::V114d);
    assert!(
        !warnings.iter().any(|w| w.contains("MonPreset")),
        "{warnings:?}"
    );

    let gheed = resolve(&c, 0, 18, 1, 0);
    assert_eq!(gheed.source, "monpreset");
    assert_eq!(gheed.class, ObjectClass::Npc);
    assert_eq!(gheed.row, Some(0));
    assert_eq!(gheed.token.as_deref(), Some("GH"));
    assert_eq!(gheed.sprite_base, Some(SpriteBase::Monsters));
    assert_eq!(gheed.mode, "NU");
    assert_eq!(gheed.weapon_class, "HTH");
    assert_eq!(gheed.order_flag, 0);
    assert_eq!(gheed.parts, ["TR"]);
    assert_eq!(gheed.size, (1, 2));
    assert_eq!(gheed.name, "Gheed");

    // The class weapon comes from MonStats2.
    let akara = resolve(&c, 0, 18, 1, 2);
    assert_eq!(akara.token.as_deref(), Some("PS"));
    assert_eq!(akara.weapon_class, "BOW");
    assert_eq!(akara.parts, ["HD", "TR", "LG"]);

    // Act 1 block row 4 is a superunique: drawn as its class.
    let bish = resolve(&c, 0, 18, 1, 4);
    assert_eq!(bish.token.as_deref(), Some("FA"));
    assert_eq!(bish.row, Some(4));
    assert_eq!(bish.name, "Bishibosh");

    // A MonPlace marker: the preset row is known, DS1Edit's table supplies the sprite.
    let fallens = resolve(&c, 0, 18, 1, 10);
    assert_eq!(fallens.source, "monpreset");
    assert_eq!(fallens.token.as_deref(), Some("FA"));
    assert_eq!(fallens.row, None);

    // The act picks the block: act 3 (index 2) row 1 is akara.
    assert_eq!(resolve(&c, 2, 18, 1, 1).token.as_deref(), Some("PS"));
    // Act 2 block has 2 rows: id 2 falls through to MonStats row 2.
    let direct = resolve(&c, 1, 18, 1, 2);
    assert_eq!((direct.source, direct.row), ("monpreset", Some(2)));
    assert_eq!(direct.token.as_deref(), Some("PS"));
    // Direct rows past the preset blocks.
    let late = resolve(&c, 0, 18, 1, 20);
    assert_eq!(late.row, Some(20));
    assert_eq!(late.token.as_deref(), Some("M20"));
    // A negative id reads backwards into the previous act's block.
    assert_eq!(resolve(&c, 1, 18, 1, -1).token.as_deref(), Some("PS"));
    assert_eq!(resolve(&c, 0, 18, 1, -1).source, "unresolved");
    // Past MonStats.
    assert_eq!(resolve(&c, 0, 18, 1, 40).source, "unresolved");
}

#[test]
fn type1_below_v5_is_a_raw_monstats_row() {
    let (_d, c, _w) = build_owned(&mon_files(40), GameVersion::V114d);
    // Id 10 is the MonPlace marker in the act 1 block but MonStats row 10 when used raw.
    let old = resolve(&c, 0, 4, 1, 10);
    assert_eq!(old.token.as_deref(), Some("M10"));
    assert_eq!(old.row, Some(10));
    let new = resolve(&c, 0, 5, 1, 10);
    assert_eq!(new.token.as_deref(), Some("FA"));
    assert_eq!(new.row, None);
    // Placeholder "dummy" name strings are replaced by the MonStats id.
    assert_eq!(resolve(&c, 0, 5, 1, 3).name, "chicken");
}

#[test]
fn type1_monster_rows_that_become_objects() {
    let (_d, c, _w) = build_owned(&mon_files(600), GameVersion::V114d);
    // Act 3 (index 2), MonStats row 297 is rewritten into Objects row 382. The Objects.txt
    // here has only 30 rows, so the rewrite cannot resolve.
    assert_eq!(resolve(&c, 2, 18, 1, 297).source, "unresolved");
    // Other acts keep the monster.
    let other = resolve(&c, 0, 18, 1, 297);
    assert_eq!(other.class, ObjectClass::Npc);
    assert_eq!(other.row, Some(297));

    // With a big enough Objects.txt the object takes over.
    let mut files = mon_files(600);
    files[0].1 = objects_txt(480);
    let (_d, c, _w) = build_owned(&files, GameVersion::V114d);
    let natalya = resolve(&c, 2, 18, 1, 297);
    assert_eq!(natalya.class, ObjectClass::Object);
    assert_eq!((natalya.ds1_kind, natalya.ds1_id), (1, 297));
    assert_eq!(natalya.row, Some(382));
    assert_eq!(natalya.source, "monpreset");
    // Act 5 (index 4): the statues are permuted.
    assert_eq!(resolve(&c, 4, 18, 1, 537).row, Some(476));
    assert_eq!(resolve(&c, 4, 18, 1, 538).row, Some(475));
    assert_eq!(resolve(&c, 4, 18, 1, 539).row, Some(474));
}

#[test]
fn type1_from_embedded_obj_txt_when_mon_tables_are_missing() {
    let files = [(excel("objects.txt"), objects_txt(30))];
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (_d, c, warnings) = build(&files, GameVersion::V114d);
    assert!(
        warnings.iter().any(|w| w.contains("MonPreset")),
        "{warnings:?}"
    );

    let gheed = resolve(&c, 0, 18, 1, 0);
    assert_eq!(gheed.source, "obj.txt");
    assert_eq!(gheed.token.as_deref(), Some("GH"));
    assert_eq!(gheed.name, "Gheed");
    assert_eq!(gheed.sprite_base, Some(SpriteBase::Monsters));
    // The rogue carries parts.
    let rogue = resolve(&c, 0, 18, 1, 4);
    assert_eq!(rogue.token.as_deref(), Some("RG"));
    assert!(rogue.parts.iter().any(|p| p == "HD"));
    assert!(rogue.parts.iter().any(|p| p == "TR"));
    // A row DS1Edit has no sprite for.
    assert_eq!(resolve(&c, 0, 18, 1, 17).source, "unresolved");
    assert_eq!(resolve(&c, 0, 18, 1, 5000).source, "unresolved");
    // No raw MonStats rows without MonStats.
    assert_eq!(resolve(&c, 0, 4, 1, 0).source, "unresolved");
}

#[test]
fn bin_only_mon_tables_are_not_read_from_stale_txt() {
    // A 1.13c profile with a `.bin` we cannot decode must not fall back to a `.txt`
    // from a lower layer (it belongs to different data).
    let mut files = mon_files(40);
    files.push((excel("monstats.bin"), vec![0u8; 8]));
    files.push((excel("monpreset.bin"), vec![0u8; 8]));
    let (_d, c, warnings) = build_owned(&files, GameVersion::V113c);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("monstats") && w.contains("cannot be loaded")),
        "{warnings:?}"
    );
    // Falls back to DS1Edit's table: Gheed is still found there.
    let gheed = resolve(&c, 0, 18, 1, 0);
    assert_eq!(gheed.source, "obj.txt");
    assert_eq!(gheed.token.as_deref(), Some("GH"));
}

/// A `.bin`: record count, then the records.
fn bin(record: usize, count: usize, fill: impl Fn(usize, &mut [u8])) -> Vec<u8> {
    let mut out = u32::try_from(count).unwrap().to_le_bytes().to_vec();
    for i in 0..count {
        let mut rec = vec![0u8; record];
        fill(i, &mut rec);
        out.extend_from_slice(&rec);
    }
    out
}

fn put(rec: &mut [u8], offset: usize, bytes: &[u8]) {
    rec[offset..offset + bytes.len()].copy_from_slice(bytes);
}

/// The monster tables as Median XL ships them (compiled), over a tiny world:
/// MonStats rows gheed 0, akara 1, fallen1 2; one superunique (class fallen1).
fn mon_bin_files() -> Vec<(String, Vec<u8>)> {
    let stats = bin(0x1A8, 3, |i, r| {
        put(r, 6, &(10_000 + [0u16, 1, 2][i]).to_le_bytes());
        put(r, 16, [b"GH  ", b"PS  ", b"FA  "][i]);
        // MonStatsEx: gheed -> MonStats2 row 0, akara -> row 1, fallen1 blank.
        put(r, 24, &[0u16, 1, 0xFFFF][i].to_le_bytes());
    });
    let stats2 = bin(0x134, 2, |i, r| {
        put(r, 8, &[1, 1]);
        put(r, 16, if i == 1 { b"bow " } else { b"    " });
        // part flags: row 0 TR; row 1 HD TR LG
        put(r, 232, &[0b10u32, 0b111][i].to_le_bytes());
    });
    let supers = bin(0x34, 1, |_, r| {
        put(r, 2, &10_003u16.to_le_bytes());
        put(r, 4, &2u16.to_le_bytes());
    });
    // (act, kind, index)
    let places: [(u8, u8, u16); 7] = [
        (1, 1, 0),
        (1, 1, 1),
        (1, 0, 17),
        (1, 2, 0),
        (1, 1, 99),
        (1, 2, 5),
        (2, 1, 1),
    ];
    let preset = bin(4, places.len(), |i, r| {
        r[0] = places[i].0;
        r[1] = places[i].1;
        put(r, 2, &places[i].2.to_le_bytes());
    });
    vec![
        (excel("objects.txt"), objects_txt(30)),
        (excel("monpreset.bin"), preset),
        (excel("monstats.bin"), stats),
        (excel("monstats2.bin"), stats2),
        (excel("superuniques.bin"), supers),
        (
            r"data\local\lng\eng\patchstring.tbl".to_owned(),
            tbl(&[
                ("Gheed", "Gheed the Trader"),
                ("Akara", "Akara"),
                ("dummy", "an evil force"),
                ("Bishibosh", "Bishibosh"),
            ]),
        ),
    ]
}

#[test]
fn type1_through_compiled_bin_tables() {
    let (_d, c, warnings) = build_owned(&mon_bin_files(), GameVersion::V113c);
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("MonPreset") || w.contains("cannot be loaded")),
        "{warnings:?}"
    );

    // MonStats row 0 through the preset; name from the patchstring id 10000.
    let gheed = resolve(&c, 0, 18, 1, 0);
    assert_eq!(gheed.source, "monpreset");
    assert_eq!((gheed.row, gheed.token.as_deref()), (Some(0), Some("GH")));
    assert_eq!(gheed.name, "Gheed the Trader");
    assert_eq!(gheed.sprite_base, Some(SpriteBase::Monsters));
    assert_eq!(gheed.parts, ["TR"]);
    // MonStats2 row 1: three parts, a weapon class and a size.
    let akara = resolve(&c, 0, 18, 1, 1);
    assert_eq!((akara.row, akara.token.as_deref()), (Some(1), Some("PS")));
    assert_eq!(akara.parts, ["HD", "TR", "LG"]);
    assert_eq!(akara.weapon_class, "BOW");
    assert_eq!(akara.size, (1, 1));
    // A MonPlace marker is named by its monplace.txt code.
    let marker = resolve(&c, 0, 18, 1, 2);
    assert_eq!(marker.source, "monpreset");
    assert_eq!(marker.row, None);
    // A superunique takes the MonStats row of its class and its own name.
    let sup = resolve(&c, 0, 18, 1, 3);
    assert_eq!((sup.row, sup.token.as_deref()), (Some(2), Some("FA")));
    assert_eq!(sup.name, "Bishibosh");
    assert!(
        sup.parts.is_empty(),
        "blank MonStatsEx has no MonStats2 row"
    );
    // Indices past their tables do not resolve.
    assert_eq!(resolve(&c, 0, 18, 1, 4).source, "unresolved");
    assert_eq!(resolve(&c, 0, 18, 1, 5).source, "unresolved");
    // The second act's block starts after the first.
    let akara2 = resolve(&c, 1, 18, 1, 0);
    assert_eq!((akara2.row, akara2.token.as_deref()), (Some(1), Some("PS")));
    // Below version 5 the id is a raw MonStats row; a `dummy` name falls back
    // to the row (the compiled table has no Id text).
    let raw = resolve(&c, 0, 4, 1, 2);
    assert_eq!((raw.row, raw.token.as_deref()), (Some(2), Some("FA")));
    assert_eq!(raw.name, "monster 2");
}

// ---------------------------------------------------------------- robustness

#[test]
fn unknown_ids_and_kinds_never_panic() {
    let (_d, c, _w) = build_owned(&mon_files(40), GameVersion::V114d);
    for version in [0, 1, 4, 5, 6, 18, -1, i32::MAX] {
        for act in [0u8, 4, 5, 255] {
            for kind in [-1, 0, 1, 2, 3, 4, 99, i32::MIN, i32::MAX] {
                for id in [
                    0,
                    1,
                    149,
                    150,
                    151,
                    1000,
                    -1,
                    -150,
                    -151,
                    i32::MIN,
                    i32::MAX,
                ] {
                    let info = resolve(&c, act, version, kind, id);
                    assert_eq!((info.ds1_kind, info.ds1_id), (kind, id));
                    if info.source == "unresolved" {
                        assert_eq!(info.token, None);
                        assert_eq!(info.row, None);
                        assert_eq!(info.name, format!("type {kind} id {id}"));
                    }
                }
            }
        }
    }
    let other = resolve(&c, 0, 18, 4, 7);
    assert_eq!(other.class, ObjectClass::Other(4));
    assert_eq!(other.source, "unresolved");
}

#[test]
fn empty_overlay_builds_with_warnings() {
    let (_d, c, warnings) = build(&[], GameVersion::V113c);
    assert!(!warnings.is_empty());
    // The embedded table still places the Cain start position (no Objects.txt to check).
    let cain = resolve(&c, 0, 18, 2, 110);
    assert_eq!(cain.token.as_deref(), Some("DC"));
    assert!(c.placeable(0).iter().any(|i| i.ds1_kind == 1));
    assert_eq!(resolve(&c, 0, 18, 2, 140).source, "unresolved");
}

#[test]
fn garbage_tables_become_warnings() {
    let files = [
        (excel("objects.txt"), vec![0xFF; 64]),
        (excel("monstats.txt"), b"x".to_vec()),
        (excel("monpreset.txt"), Vec::new()),
        (
            r"data\local\lng\eng\patchstring.tbl".to_owned(),
            vec![1, 2, 3],
        ),
    ];
    let files: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let (_d, c, warnings) = build(&files, GameVersion::V114d);
    assert!(!warnings.is_empty());
    let _ = resolve(&c, 0, 18, 1, 0);
    let _ = resolve(&c, 0, 18, 2, 0);
    let _ = c.placeable(3);
}

// ---------------------------------------------------------------- names and picker

#[test]
fn names_come_from_string_tables() {
    let mut files = mon_files(40);
    // patchstring wins over string.
    files.push((
        r"data\local\lng\eng\patchstring.tbl".to_owned(),
        tbl(&[("chest", "Wooden Chest"), ("Gheed", "Gheed the Gambler")]),
    ));
    files.push((
        r"data\local\lng\eng\string.tbl".to_owned(),
        tbl(&[("chest", "Old Chest"), ("Akara", "Akara the Priestess")]),
    ));
    let mut owned: Vec<(String, Vec<u8>)> = files;
    owned.push((excel("objpreset.txt"), objpreset_txt(&[(0, 5, 1)])));
    let (_d, c, warnings) = build_owned(&owned, GameVersion::V114d);
    assert!(
        !warnings.iter().any(|w| w.contains("string")),
        "{warnings:?}"
    );
    assert_eq!(resolve(&c, 0, 18, 2, 5).name, "Wooden Chest");
    assert_eq!(resolve(&c, 0, 18, 1, 0).name, "Gheed the Gambler");
    assert_eq!(resolve(&c, 0, 18, 1, 2).name, "Akara the Priestess");
    // No string: the key itself.
    assert_eq!(resolve(&c, 0, 18, 1, 1).name, "DeckardCain");
    // Objects without a string use the Name column.
    assert_eq!(resolve(&c, 0, 18, 2, 150 + 6).name, "Obj6");
}

#[test]
fn placeable_lists_the_acts() {
    let mut files = mon_files(40);
    files.push((
        excel("objpreset.txt"),
        objpreset_txt(&[(0, 5, 2), (1, 7, 1)]),
    ));
    let (_d, c, _w) = build_owned(&files, GameVersion::V114d);
    for act in 0..5u8 {
        let list = c.placeable(act);
        assert!(!list.is_empty(), "act {act}");
        assert!(list.iter().all(|i| i.source != "unresolved"));
        // Every entry resolves to itself.
        for info in list.iter().take(50) {
            let again = c.resolve(act, 18, &obj(info.ds1_kind, info.ds1_id));
            assert_eq!(&again, info);
        }
    }
    let act0 = c.placeable(0);
    assert!(
        act0.iter()
            .any(|i| i.ds1_kind == 2 && i.ds1_id == 5 && i.row == Some(2))
    );
    assert!(act0.iter().any(|i| i.ds1_kind == 1 && i.ds1_id == 0));
    // Direct object rows are offered too, except test row 0.
    assert!(act0.iter().any(|i| i.ds1_kind == 2 && i.ds1_id == 150 + 9));
    assert!(!act0.iter().any(|i| i.ds1_kind == 2 && i.ds1_id == 150));
    // Act 1 has its own slot 7.
    assert!(
        c.placeable(1)
            .iter()
            .any(|i| i.ds1_kind == 2 && i.ds1_id == 7)
    );
    assert!(
        !c.placeable(0)
            .iter()
            .any(|i| i.ds1_kind == 2 && i.ds1_id == 7)
    );
    // Out-of-range acts clamp.
    assert_eq!(c.placeable(200).len(), c.placeable(4).len());
}

// ---------------------------------------------------------------- palette shifts

/// MonStats with `count` filler rows whose row `34` has the code `code`
/// (obj.txt act 1 type 1 id 34 is Bishibosh: token FS, RandTransforms row 16).
fn monstats_with(count: usize, code: &str) -> Vec<u8> {
    let mut rows = vec![s(&["Id", "hcIdx", "NameStr", "MonStatsEx", "Code"])];
    for n in 0..count {
        let c = if n == 34 {
            code.to_owned()
        } else {
            format!("M{n}")
        };
        rows.push(vec![
            format!("mon{n}"),
            n.to_string(),
            format!("Mon{n}"),
            String::new(),
            c,
        ]);
    }
    tsv(&rows)
}

#[test]
fn obj_txt_shifts_skins_and_facing_apply_when_the_token_matches() {
    let bishibosh = d2_assets::PaletteShift {
        path: r"Data\Global\Monsters\RandTransforms.dat".to_owned(),
        row: 15,
    };
    // Without monster tables the embedded obj.txt draws the row itself.
    let (_d, bare, _w) = build(&[], GameVersion::V114d);
    let info = resolve(&bare, 0, 18, 1, 34);
    assert_eq!(info.token.as_deref(), Some("FS"));
    assert_eq!(info.colormap.as_ref(), Some(&bishibosh));
    // ... a monster's own Palshift.dat (Blood Raven, Index 3)
    let raven = resolve(&bare, 0, 18, 1, 12).colormap.unwrap();
    assert_eq!(
        (raven.path.as_str(), raven.row),
        (r"Data\Global\Monsters\CR\COF\Palshift.dat", 2)
    );
    // ... and nothing for rows without a Colormap, other acts, or below v5
    assert_eq!(resolve(&bare, 0, 18, 1, 0).colormap, None);
    assert_eq!(resolve(&bare, 1, 18, 1, 34).colormap, None);
    assert_eq!(resolve(&bare, 0, 18, 2, 34).colormap, None);

    // Through MonStats: kept when the resolved token is obj.txt's ...
    let mut files = mon_files(40);
    files.retain(|(name, _)| !name.ends_with("monstats.txt"));
    files.push((excel("monstats.txt"), monstats_with(40, "FS")));
    let (_d, same, _w) = build_owned(&files, GameVersion::V114d);
    let info = resolve(&same, 0, 18, 1, 34);
    assert_eq!(
        (info.source, info.token.as_deref()),
        ("monpreset", Some("FS"))
    );
    assert_eq!(info.colormap.as_ref(), Some(&bishibosh));
    // the same row also supplies the skins and default facing DS1Edit uses
    assert_eq!(info.skins, [("TR".to_owned(), "LIT".to_owned())]);
    assert_eq!(info.direction, Some(2));
    let raw = resolve(&same, 0, 4, 1, 34);
    assert_eq!(raw.colormap, None, "pre-v5 ids are raw rows");
    assert!(raw.skins.is_empty() && raw.direction.is_none());
    // ... dropped when the mod draws something else there
    let (_d, other, _w) = build_owned(&mon_files(40), GameVersion::V114d);
    let info = resolve(&other, 0, 18, 1, 34);
    assert_eq!(info.token.as_deref(), Some("M34"));
    assert_eq!(info.colormap, None);
    assert!(info.skins.is_empty() && info.direction.is_none());
}

#[test]
fn palette_shift_rows_are_256_bytes() {
    let shift = d2_assets::PaletteShift {
        path: "x".to_owned(),
        row: 1,
    };
    let bytes: Vec<u8> = (0..512u32).map(|i| (i / 2) as u8).collect();
    let row = shift.row_in(&bytes).unwrap();
    assert_eq!((row[0], row[255]), (128, 255));
    // the reference refuses a row that runs past the end of the file
    assert!(shift.row_in(&bytes[..511]).is_none());
    let far = d2_assets::PaletteShift {
        path: "x".to_owned(),
        row: u32::MAX,
    };
    assert!(far.row_in(&bytes).is_none());
}
