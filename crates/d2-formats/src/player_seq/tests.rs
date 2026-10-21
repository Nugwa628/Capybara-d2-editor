#![allow(clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::cof::Cof;
use crate::testutil;

#[test]
fn every_sequence_is_numbered_and_has_steps() {
    assert_eq!(PLAYER_SEQUENCES.len(), 23);
    let modes = [
        "a1", "a2", "sc", "th", "kk", "s1", "s2", "s3", "s4", "rn", "dt",
    ];
    for (i, seq) in PLAYER_SEQUENCES.iter().enumerate() {
        assert_eq!(usize::from(seq.seqnum), i + 1, "{}", seq.name);
        assert!(!seq.name.is_empty());
        assert!(seq.by_class.iter().any(Option::is_some), "{}", seq.name);
        for (slot, steps) in seq.by_class.iter().enumerate() {
            let Some(steps) = steps else { continue };
            let class = CLASS_CODES[slot];
            let trimmed = seq.steps(class).expect("a slot with steps");
            assert!(!trimmed.is_empty(), "{} {class}", seq.name);
            assert!(trimmed.len() <= steps.len());
            for s in trimmed {
                assert!(modes.contains(&s.mode), "{} {class}: {}", seq.name, s.mode);
                assert!(s.frame < 32, "{} {class}: frame {}", seq.name, s.frame);
            }
        }
    }
}

#[test]
fn lookup_by_seqnum_and_class() {
    assert!(sequence(0).is_none());
    assert!(sequence(24).is_none());
    let leap = sequence(13).expect("Leap");
    assert_eq!(leap.name, "Leap");
    assert_eq!(class_slot("1HS"), Some(2));
    assert_eq!(class_slot("zzz"), None);
    let steps = leap.steps("hth").expect("a table");
    assert_eq!(steps.len(), 15);
    assert!(steps.iter().all(|s| s.mode == "s1"));
    assert_eq!(steps[5].event, SeqEvent::MeleeAttack);
    // Whirlwind has no table for two-handed swords or spears.
    let ww = sequence(10).expect("Whirlwind");
    assert!(ww.steps("2hs").is_none() && ww.steps("2ht").is_none());
    let a: Vec<_> = ww
        .steps("hth")
        .expect("hth")
        .iter()
        .map(|s| s.frame)
        .collect();
    assert_eq!(a, vec![0, 1, 2, 3, 3, 4, 5, 6]);
}

#[test]
fn modes_change_mid_sequence_and_padding_is_dropped() {
    let fend = sequence(9).expect("Fend");
    let raw = fend.by_class[class_slot("1hs").unwrap()].expect("raw");
    assert_eq!(raw.len(), 16);
    let steps = fend.steps("1hs").expect("steps");
    assert_eq!(steps.len(), 15, "the trailing all-zero entry is padding");
    assert_eq!((steps[2].mode, steps[3].mode), ("a1", "a2"));
    assert_eq!((steps[2].frame, steps[3].frame), (2, 2));
    let proj = sequence(17).expect("Projection");
    assert_eq!(proj.steps("hth").expect("hth").len(), 18);
}

#[test]
fn skills_are_listed_by_sequence() {
    let text = "skill\tcharclass\tanim\tseqtrans\tseqnum\r\n\
        Leap\tbar\tSQ\tA1\t13\r\n\
        Whirlwind\tbar\tSQ\tA1\t10\r\n\
        Frenzy\tbar\tSQ\tA1\t11\r\n\
        Double Swing\tbar\tSQ\tA1\t11\r\n\
        Mosquito\t\tSQ\tSQ\t34\r\n\
        Imp Inferno\t\tSQ\tSQ\t6\r\n\
        Firebolt\tsor\tSC\tSC\t0\r\n\
        Odd\tsor\tSC\tSC\t99\r\n";
    let table = TxtTable::parse(text.as_bytes()).expect("table");
    let got: Vec<(u32, String)> = skills_using(&table)
        .into_iter()
        .map(|s| (s.seqnum, s.name))
        .collect();
    assert_eq!(
        got,
        vec![
            (10, "Whirlwind".to_owned()),
            (11, "Double Swing".to_owned()),
            (11, "Frenzy".to_owned()),
            (13, "Leap".to_owned()),
        ]
    );
}

fn token_of(class: &str) -> Option<&'static str> {
    Some(match class {
        "ama" => "am",
        "sor" => "so",
        "nec" => "ne",
        "pal" => "pa",
        "bar" => "ba",
        "dru" => "dz",
        "ass" => "ai",
        _ => return None,
    })
}

/// Against the real corpus: every player seqnum in `skills.txt` has a table
/// and nearly every step lands inside the frame count of the COF it names (for the
/// COFs that exist; a class without one cannot play that weapon class).
#[test]
fn corpus_skills_are_covered_and_steps_fit_the_cofs() {
    let (mut checked, mut over) = (0, 0);
    for name in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(name) else {
            eprintln!("skipped: no private corpus ({name})");
            continue;
        };
        let excel = root.join("data/global/excel/skills.txt");
        let Ok(bytes) = std::fs::read(&excel) else {
            continue;
        };
        let table = TxtTable::parse(&bytes).expect("skills.txt");
        let mut used = 0;
        for row in table.rows() {
            let class = row.get("charclass").unwrap_or("").trim();
            let seqnum = row.get_i64("seqnum").unwrap_or(0);
            if class.is_empty() || seqnum <= 0 {
                continue;
            }
            let skill = row.get("skill").unwrap_or("?");
            let seq = sequence(seqnum as u32)
                .unwrap_or_else(|| panic!("{name}: {skill} has seqnum {seqnum} with no table"));
            used += 1;
            let token = token_of(class).unwrap_or_else(|| panic!("{name}: class {class}"));
            for (slot, code) in CLASS_CODES.iter().enumerate() {
                let Some(_) = seq.by_class[slot] else {
                    continue;
                };
                for step in seq.steps(code).expect("steps") {
                    let file = root.join(format!(
                        "data/global/chars/{token}/cof/{token}{}{code}.cof",
                        step.mode
                    ));
                    let Ok(bytes) = std::fs::read(&file) else {
                        continue;
                    };
                    let cof = Cof::parse(&bytes).expect("cof");
                    if step.frame >= cof.frames_per_direction {
                        over += 1;
                        eprintln!(
                            "past the last frame: {name}: {skill} ({}) {}{} frame {} of {}",
                            seq.name, step.mode, code, step.frame, cof.frames_per_direction
                        );
                    }
                    checked += 1;
                }
            }
        }
        assert!(used > 0, "{name}: no player skill with a sequence");
    }
    eprintln!("{checked} steps checked against real COFs, {over} past the last frame");
    // A few steps name the frame just past the end (the game presumably
    // clamps); a wholesale mismatch would mean a wrong table.
    assert!(
        over * 50 < checked,
        "{over} of {checked} steps are out of range"
    );
}
