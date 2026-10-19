//! Palette conversion planner: property tests on random palettes and
//! settings, plus checks against the real vanilla palettes when the private
//! corpus is present (skipped otherwise).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use capy_paint::remap::{
    ACT_SPECIFIC, Adjust, ColorDistance, MatchKind, RemapPlan, RemapSettings, TargetSet,
    candidates, plan,
};
use d2_formats::palette::Palette;
use proptest::prelude::*;

/// `PROPTEST_CASES` when set, else `default`.
fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn palette() -> impl Strategy<Value = Palette> {
    // few distinct channel values so exact duplicates and ties happen
    let channel = prop_oneof![any::<u8>(), (0u8..4).prop_map(|v| v * 85)];
    prop::collection::vec([channel.clone(), channel.clone(), channel], 256).prop_map(|v| {
        let mut colors = [[0u8; 3]; 256];
        colors.copy_from_slice(&v);
        Palette { colors }
    })
}

fn used() -> impl Strategy<Value = [u64; 256]> {
    prop::collection::vec(prop_oneof![Just(0u64), 1u64..1000], 256).prop_map(|v| {
        let mut u = [0u64; 256];
        u.copy_from_slice(&v);
        u
    })
}

fn targets() -> impl Strategy<Value = TargetSet> {
    prop_oneof![
        Just(TargetSet::All),
        Just(TargetSet::SharedActColors),
        prop::collection::vec(any::<u8>(), 0..12).prop_map(TargetSet::Only),
    ]
}

fn adjust() -> impl Strategy<Value = Adjust> {
    prop_oneof![
        Just(Adjust::default()),
        (-1.2f32..1.2, -1.2f32..1.2, -1.2f32..1.2, -400f32..400.0).prop_map(
            |(brightness, contrast, saturation, hue_degrees)| Adjust {
                brightness,
                contrast,
                saturation,
                hue_degrees,
            }
        ),
    ]
}

fn settings() -> impl Strategy<Value = RemapSettings> {
    (
        0u8..3,
        targets(),
        any::<bool>(),
        prop_oneof![Just(1.0f32), 0.0f32..4.0],
        adjust(),
        prop::collection::btree_map(any::<u8>(), any::<u8>(), 0..8),
    )
        .prop_map(
            |(d, targets, keep_exact, lightness_weight, adjust, overrides)| RemapSettings {
                distance: [
                    ColorDistance::OkLab,
                    ColorDistance::Redmean,
                    ColorDistance::Rgb,
                ][usize::from(d)],
                targets,
                keep_exact,
                lightness_weight,
                adjust,
                overrides,
            },
        )
}

fn is_override(s: &RemapSettings, i: u8) -> Option<u8> {
    s.overrides.get(&i).copied().filter(|&t| i != 0 && t != 0)
}

fn check_valid(r: &RemapPlan) {
    assert_eq!(r.table[0], 0);
    assert!(r.table[1..].iter().all(|&t| t != 0));
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::with_cases(cases(256)) })]

    #[test]
    fn plan_invariants(src in palette(), dst in palette(), used in used(), s in settings()) {
        let r = plan(&src, &dst, &used, &s);
        check_valid(&r);
        let allowed = s.targets.effective();
        prop_assert!(!allowed.is_empty() && !allowed.contains(&0));
        for i in 1..=255u8 {
            let t = r.table[usize::from(i)];
            match is_override(&s, i) {
                // overrides always honoured
                Some(o) => prop_assert_eq!(t, o),
                // TargetSet respected
                None => prop_assert!(allowed.contains(&t)),
            }
        }
        // entries: exactly the used non-zero indices, worst first
        let n_used = used[1..].iter().filter(|&&p| p > 0).count();
        prop_assert_eq!(r.entries.len(), n_used);
        for w in r.entries.windows(2) {
            prop_assert!(w[0].error >= w[1].error);
        }
        for e in &r.entries {
            prop_assert_eq!(r.table[usize::from(e.source)], e.target);
            prop_assert_eq!(e.pixels, used[usize::from(e.source)]);
            prop_assert_eq!(e.source_rgb, src.colors[usize::from(e.source)]);
            prop_assert_eq!(e.target_rgb, dst.colors[usize::from(e.target)]);
            prop_assert!(e.error.is_finite() && e.error >= 0.0);
            let kind = if is_override(&s, e.source).is_some() {
                MatchKind::Override
            } else if s.keep_exact && allowed.contains(&e.source)
                && src.colors[usize::from(e.source)] == dst.colors[usize::from(e.source)]
            {
                MatchKind::Kept
            } else {
                MatchKind::Nearest
            };
            prop_assert_eq!(e.how, kind);
            if kind == MatchKind::Nearest {
                // the plan's choice is the picker's first candidate
                let c = candidates(&src, &dst, e.source, &s, 1);
                prop_assert_eq!(c[0].0, e.target);
                prop_assert_eq!(c[0].1, e.error);
            }
        }
        prop_assert!(r.worst_error >= r.mean_error - 1e-3);
        // deterministic
        prop_assert_eq!(&plan(&src, &dst, &used, &s), &r);
    }

    #[test]
    fn identity_when_palettes_match(p in palette(), used in used(), mut s in settings()) {
        s.keep_exact = true;
        s.overrides.clear();
        let r = plan(&p, &p, &used, &s);
        check_valid(&r);
        let allowed = s.targets.effective();
        for &i in &allowed {
            prop_assert_eq!(r.table[usize::from(i)], i);
        }
        if s.targets == TargetSet::All {
            prop_assert!(r.entries.iter().all(|e| e.how == MatchKind::Kept && e.error == 0.0));
        }
    }

    #[test]
    fn candidates_are_allowed_sorted_and_unique(
        src in palette(), dst in palette(), source in any::<u8>(), s in settings(), k in 0usize..300
    ) {
        let c = candidates(&src, &dst, source, &s, k);
        let allowed = s.targets.effective();
        if source == 0 {
            prop_assert!(c.is_empty());
        } else {
            prop_assert_eq!(c.len(), k.min(allowed.len()));
        }
        let mut seen = [false; 256];
        for &(t, e) in &c {
            prop_assert!(allowed.contains(&t));
            prop_assert!(!seen[usize::from(t)]);
            seen[usize::from(t)] = true;
            prop_assert!(e.is_finite());
        }
    }
}

// ---------------------------------------------------------------------------
// real palettes

/// The vanilla palette `name` (act1, menu0, ...) from the private corpus.
fn real_palette(name: &str) -> Option<Palette> {
    let rel = format!("testdata/private/extracted/vanilla/data/global/palette/{name}/pal.dat");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // the corpus lives in the main checkout, not in agent worktrees
    let paths = [
        manifest.join("../..").join(&rel),
        PathBuf::from(r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor").join(&rel),
    ];
    let bytes = paths.iter().find_map(|p| std::fs::read(p).ok())?;
    Some(Palette::parse_dat(&bytes).unwrap())
}

fn real(names: &[&str]) -> Option<Vec<Palette>> {
    let v: Option<Vec<Palette>> = names.iter().map(|n| real_palette(n)).collect();
    if v.is_none() {
        eprintln!("private palettes not present, skipping");
    }
    v
}

fn print_act_specific(title: &str, r: &RemapPlan) {
    println!(
        "\n{title}: mean {:.2}, worst {:.2}",
        r.mean_error, r.worst_error
    );
    println!("  src  source rgb      ->  dst  target rgb       error  how");
    let mut rows: Vec<_> = r
        .entries
        .iter()
        .filter(|e| ACT_SPECIFIC.contains(&e.source))
        .collect();
    rows.sort_by_key(|e| e.source);
    for e in rows {
        println!(
            "  {:3}  {:3?}  -> {:3}  {:3?}  {:6.2}  {:?}",
            e.source, e.source_rgb, e.target, e.target_rgb, e.error, e.how
        );
    }
}

#[test]
fn real_act_palettes_differ_only_in_act_specific() {
    let Some(p) = real(&["act1", "act2", "act3", "act4", "act5", "units", "sky"]) else {
        return;
    };
    for (k, q) in p.iter().enumerate().skip(1) {
        let differ: Vec<u8> = (0..=255u8)
            .filter(|&i| p[0].colors[usize::from(i)] != q.colors[usize::from(i)])
            .collect();
        println!("act1 vs palette #{k}: {} indices differ", differ.len());
        assert!(differ.iter().all(|i| ACT_SPECIFIC.contains(i)));
    }
}

#[test]
fn real_act5_to_act1() {
    let Some(p) = real(&["act5", "act1"]) else {
        return;
    };
    let (act5, act1) = (&p[0], &p[1]);
    let used = [1u64; 256];

    let start = std::time::Instant::now();
    let r = plan(act5, act1, &used, &RemapSettings::default());
    println!("plan of 255 indices took {:?}", start.elapsed());
    check_valid(&r);
    for i in 1..=255u8 {
        if !ACT_SPECIFIC.contains(&i) {
            assert_eq!(r.table[usize::from(i)], i, "index {i} must be kept");
        }
    }
    let changed = (1..=255u8)
        .filter(|&i| r.table[usize::from(i)] != i)
        .count();
    println!("default: {changed} indices change");
    print_act_specific("act5 -> act1, default (OkLab, All, keep_exact)", &r);

    let shared = RemapSettings {
        targets: TargetSet::SharedActColors,
        ..RemapSettings::default()
    };
    let r = plan(act5, act1, &used, &shared);
    check_valid(&r);
    for i in 1..=255u8 {
        let t = r.table[usize::from(i)];
        assert!(!ACT_SPECIFIC.contains(&t), "{i} -> {t}");
        if !ACT_SPECIFIC.contains(&i) {
            assert_eq!(t, i);
        }
    }
    print_act_specific("act5 -> act1, SharedActColors", &r);

    let heavy_l = RemapSettings {
        lightness_weight: 2.0,
        ..RemapSettings::default()
    };
    let r = plan(act5, act1, &used, &heavy_l);
    check_valid(&r);
    print_act_specific("act5 -> act1, lightness_weight 2.0", &r);

    for distance in [ColorDistance::Redmean, ColorDistance::Rgb] {
        let s = RemapSettings {
            distance,
            ..RemapSettings::default()
        };
        let r = plan(act5, act1, &used, &s);
        check_valid(&r);
        println!(
            "act5 -> act1, {distance:?}: mean {:.2}, worst {:.2}",
            r.mean_error, r.worst_error
        );
    }

    // overrides honoured on real data
    let s = RemapSettings {
        overrides: BTreeMap::from([(230, 12), (10, 99)]),
        ..RemapSettings::default()
    };
    let r = plan(act5, act1, &used, &s);
    assert_eq!((r.table[230], r.table[10]), (12, 99));

    let start = std::time::Instant::now();
    let c = candidates(act5, act1, 240, &RemapSettings::default(), 8);
    println!(
        "\ncandidates for act5 #240 {:?} (took {:?}):",
        act5.colors[240],
        start.elapsed()
    );
    for (t, e) in &c {
        println!("  {t:3} {:3?} {e:6.2}", act1.colors[usize::from(*t)]);
    }
    assert_eq!(c.len(), 8);
}

#[test]
fn real_act1_to_menu0() {
    let Some(p) = real(&["act1", "menu0"]) else {
        return;
    };
    let (act1, menu0) = (&p[0], &p[1]);
    let differ = (0..=255usize)
        .filter(|&i| act1.colors[i] != menu0.colors[i])
        .count();
    let used = [1u64; 256];
    let r = plan(act1, menu0, &used, &RemapSettings::default());
    check_valid(&r);
    let kept = r
        .entries
        .iter()
        .filter(|e| e.how == MatchKind::Kept)
        .count();
    let over5 = r.entries.iter().filter(|e| e.error > 5.0).count();
    println!(
        "\nact1 -> menu0: {differ} indices differ, {kept} kept, mean {:.2}, worst {:.2}, {over5} entries > 5",
        r.mean_error, r.worst_error
    );
    for e in r.entries.iter().take(10) {
        println!(
            "  {:3} {:3?} -> {:3} {:3?} {:6.2} {:?}",
            e.source, e.source_rgb, e.target, e.target_rgb, e.error, e.how
        );
    }
    // reasonable: identical colours kept, near matches on average (menu0
    // has few saturated greens/blues, so those are the worst entries), and
    // nothing wildly off
    assert_eq!(kept, 255 - differ);
    assert!(r.mean_error < 5.0, "mean {}", r.mean_error);
    assert!(r.worst_error < 30.0, "worst {}", r.worst_error);
    let close = r.entries.iter().filter(|e| e.error <= 3.0).count();
    println!("  {close} of 255 entries within 3");
    assert!(close >= 128, "{close} entries within 3");
}
