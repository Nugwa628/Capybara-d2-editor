use super::*;
use crate::testutil;
use proptest::prelude::*;

fn record(name: &[u8], fpd: u32, speed: u32, flag_seed: u8) -> Vec<u8> {
    let mut r = vec![0u8; RECORD_LEN];
    r[..name.len()].copy_from_slice(name);
    r[8..12].copy_from_slice(&fpd.to_le_bytes());
    r[12..16].copy_from_slice(&speed.to_le_bytes());
    for (i, b) in r[16..].iter_mut().enumerate() {
        *b = flag_seed.wrapping_add(i as u8);
    }
    r
}

fn block(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (records.len() as i32).to_le_bytes().to_vec();
    for r in records {
        out.extend_from_slice(r);
    }
    out
}

fn sample() -> Vec<u8> {
    let mut f = Vec::new();
    f.extend(block(&[
        record(b"ANNUHTH", 16, 256, 1),
        record(b"ANWLHTH", 8, 128, 2),
    ]));
    f.extend(block(&[]));
    f.extend(block(&[record(b"A1NUHTH", 12, 200, 3)]));
    // a duplicate name later in the file: the first one wins
    f.extend(block(&[record(b"ANNUHTH", 99, 99, 4)]));
    f
}

#[test]
fn parses_blocks_and_records() {
    let a = AnimData::parse(&sample()).unwrap();
    assert_eq!(a.records().len(), 4);
    let r = a.get("ANNUHTH").unwrap();
    assert_eq!(r.cof_name, "ANNUHTH");
    assert_eq!((r.frames_per_direction, r.speed), (16, 256));
    assert_eq!(r.flags[0], 1);
    assert_eq!(r.flags[143], 1u8.wrapping_add(143));
    assert_eq!(a.get("ANWLHTH").unwrap().speed, 128);
    assert_eq!(a.get("A1NUHTH").unwrap().frames_per_direction, 12);
    assert_eq!(a.records()[3].speed, 99, "duplicates stay in records()");
    assert_eq!(a.get("ANNUHTH").unwrap().speed, 256, "first duplicate wins");
}

#[test]
fn lookup_ignores_case_and_cof_extension() {
    let a = AnimData::parse(&sample()).unwrap();
    for name in [
        "AnNuHTH",
        "annuhth",
        "ANNUHTH.COF",
        "annuhth.cof",
        "AnNuHth.CoF",
    ] {
        assert_eq!(a.get(name), a.get("ANNUHTH"), "{name}");
        assert!(a.get(name).is_some());
    }
    assert!(a.get("annuhth.cof.cof").is_none());
    assert!(a.get("annuhth.dcc").is_none());
    assert!(a.get("").is_none());
    assert!(a.get(".cof").is_none());
    assert!(a.get("ANNUHT").is_none());
    assert!(a.get("ÄNNUHTH").is_none());
}

#[test]
fn full_width_names_and_latin1() {
    let mut f = block(&[record(b"ABCDEFGH", 1, 2, 0), record(b"\xE9x", 1, 2, 0)]);
    f.extend_from_slice(&[7, 7]); // fewer than 4 trailing bytes are ignored
    let a = AnimData::parse(&f).unwrap();
    assert!(a.get("abcdefgh").is_some());
    assert_eq!(a.records()[1].cof_name, "\u{e9}x");
}

#[test]
fn malformed_files_error() {
    assert!(AnimData::parse(&[]).is_err());
    assert!(AnimData::parse(&[1, 0]).is_err());
    // negative count
    assert!(AnimData::parse(&(-1i32).to_le_bytes()).is_err());
    // count larger than the data
    let mut f = 5i32.to_le_bytes().to_vec();
    f.extend_from_slice(&record(b"X", 1, 1, 0));
    assert!(AnimData::parse(&f).is_err());
    // enormous count
    assert!(AnimData::parse(&i32::MAX.to_le_bytes()).is_err());
    // a zero-record file is empty but valid
    let empty = AnimData::parse(&0i32.to_le_bytes()).unwrap();
    assert!(empty.records().is_empty());
    assert!(empty.get("X").is_none());
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..700)) {
        if let Ok(a) = AnimData::parse(&bytes) {
            for r in a.records() {
                prop_assert_eq!(a.get(&r.cof_name).map(|x| x.cof_name.to_ascii_uppercase()),
                                Some(r.cof_name.to_ascii_uppercase()));
            }
        }
    }
}

#[test]
fn corpus_animdata() {
    for (profile, min_records) in [("vanilla", 3558usize), ("mxl", 6000)] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        let bytes = std::fs::read(root.join("data/global/animdata.d2")).unwrap();
        let a = AnimData::parse(&bytes).unwrap();
        let unique: std::collections::HashSet<String> = a
            .records()
            .iter()
            .map(|r| r.cof_name.to_ascii_uppercase())
            .collect();
        println!(
            "animdata {profile}: {} records, {} distinct names",
            a.records().len(),
            unique.len()
        );
        assert!(a.records().len() >= min_records);
        let upper = a.get("AnNuHTH").expect("record");
        let lower = a.get("annuhth.cof").expect("record");
        assert_eq!(upper, lower);
        assert_eq!(upper.frames_per_direction, 16);
        assert_eq!(upper.speed, 160);
        for r in a.records() {
            assert!(a.get(&r.cof_name).is_some());
        }
        if profile == "vanilla" {
            assert_eq!(a.records().len(), 3558);
            assert_eq!(unique.len(), 3558 - 29, "29 duplicate names in vanilla");
        }
    }
}
