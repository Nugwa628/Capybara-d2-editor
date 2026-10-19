use super::*;
use crate::testutil;

fn sample_dat() -> Vec<u8> {
    // entry i = (b, g, r) = (i, 255 - i, i / 2)
    let mut v = Vec::new();
    for i in 0..=255u8 {
        v.extend_from_slice(&[i, 255 - i, i / 2]);
    }
    v
}

#[test]
fn parse_dat_reads_bgr_into_rgb() {
    let p = Palette::parse_dat(&sample_dat()).unwrap();
    assert_eq!(p.colors[0], [0, 255, 0]);
    assert_eq!(p.colors[200], [100, 55, 200]);
    assert_eq!(p.colors[255], [127, 0, 255]);
}

#[test]
fn parse_dat_short_and_long() {
    assert!(matches!(
        Palette::parse_dat(&[0; 767]),
        Err(FormatError::Truncated { .. })
    ));
    assert!(Palette::parse_dat(&[]).is_err());
    let mut long = sample_dat();
    long.extend_from_slice(&[9; 40]);
    assert_eq!(
        Palette::parse_dat(&long).unwrap(),
        Palette::parse_dat(&sample_dat()).unwrap()
    );
}

#[test]
fn gamma_one_is_identity() {
    let p = Palette::parse_dat(&sample_dat()).unwrap();
    assert_eq!(p.with_gamma(1.0), p);
}

#[test]
fn gamma_matches_formula() {
    let p = Palette::parse_dat(&sample_dat()).unwrap();
    let g = p.with_gamma(1.30);
    let inv = 1.0f64 / 1.3;
    for (a, b) in p.colors.iter().zip(&g.colors) {
        for (&src, &dst) in a.iter().zip(b) {
            let want = if src == 0 {
                0
            } else {
                (255.0 * (f64::from(src) / 255.0).powf(inv) + 0.5) as u8
            };
            assert_eq!(dst, want, "channel {src}");
        }
    }
    // gamma > 1 brightens mid-tones, < 1 darkens them
    assert!(g.colors[128][0] > p.colors[128][0]);
    assert!(p.with_gamma(0.8).colors[128][0] < p.colors[128][0]);
    // black and white are fixed points
    let white = Palette {
        colors: [[255; 3]; 256],
    };
    assert_eq!(white.with_gamma(2.2), white);
    // nonsense gamma values leave the palette alone
    assert_eq!(p.with_gamma(0.0), p);
    assert_eq!(p.with_gamma(-1.0), p);
    assert_eq!(p.with_gamma(f32::NAN), p);
}

#[test]
fn act_paths() {
    assert_eq!(act_palette_path(0), "data\\global\\palette\\act1\\pal.dat");
    assert_eq!(act_palette_path(3), "data\\global\\palette\\act4\\pal.dat");
    assert_eq!(act_palette_path(4), "data\\global\\palette\\act5\\pal.dat");
    assert_eq!(
        act_palette_path(200),
        "data\\global\\palette\\act5\\pal.dat"
    );
}

fn synthetic_pl2(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    for (i, b) in v.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    v
}

#[test]
fn pl2_layout_offsets() {
    let bytes = synthetic_pl2(0x6C327);
    let p = Pl2::parse(&bytes).unwrap();
    assert_eq!(p.base.colors[1], [bytes[4], bytes[5], bytes[6]]);
    assert_eq!(p.light_levels.len(), 32);
    assert_eq!(p.light_levels[1][2], bytes[0x400 + 256 + 2]);
    assert_eq!(p.highlight.len(), 16);
    assert_eq!(p.saturation.len(), 256);
    assert_eq!(p.alpha_blend.len(), 3);
    assert!(p.alpha_blend.iter().all(|t| t.len() == 256));
    assert_eq!(p.alpha_blend[2][3][4], bytes[0x23500 + 3 * 256 + 4]);
    assert_eq!(p.additive[5][6], bytes[0x33500 + 5 * 256 + 6]);
    assert_eq!(p.multiply[7][8], bytes[0x43500 + 7 * 256 + 8]);
    assert_eq!(p.hue_shift.len(), 128);
    assert_eq!(p.screen[9][10], bytes[0x5B500 + 9 * 256 + 10]);
    assert_eq!(p.screen_ramp.len(), 256);
    assert_eq!(p.tint_colors.len(), 295);
    assert_eq!(p.tints.len(), 12);
    assert_eq!(p.tints[11][255], bytes[0x6C327 - 1]);
}

#[test]
fn pl2_short_files() {
    // the shipped `loading` palette is 259 bytes short of the full buffer
    let p = Pl2::parse(&synthetic_pl2(442_916)).unwrap();
    assert_eq!(p.tints.len(), 10);
    assert_eq!(p.multiply.len(), 256);
    // exactly through `multiply` is the minimum
    let p = Pl2::parse(&synthetic_pl2(0x53500)).unwrap();
    assert!(p.hue_shift.is_empty() && p.screen.is_empty() && p.tints.is_empty());
    assert!(p.screen_ramp.is_empty() && p.saturation.len() == 256);
    for cut in [0, 1, 1023, 0x400, 0x53500 - 1] {
        assert!(Pl2::parse(&synthetic_pl2(cut)).is_err(), "cut {cut}");
    }
}

#[test]
fn corpus_palettes() {
    let mut dats = 0;
    let mut pl2s = 0;
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        let base = root.join("data").join("global").join("palette");
        for dat_path in testutil::find_files(&base, "dat") {
            let dat = Palette::parse_dat(&std::fs::read(&dat_path).unwrap()).unwrap();
            dats += 1;
            assert_eq!(dat.colors[0], [0, 0, 0], "{}", dat_path.display());
            assert_eq!(dat.colors[255], [255, 255, 255], "{}", dat_path.display());
            assert_eq!(dat.with_gamma(1.0), dat);
            let g = dat.with_gamma(1.30);
            for (a, b) in dat.colors.iter().zip(&g.colors) {
                for (&s, &d) in a.iter().zip(b) {
                    let want = if s == 0 {
                        0
                    } else {
                        (255.0 * (f64::from(s) / 255.0).powf(1.0 / 1.3) + 0.5) as u8
                    };
                    assert_eq!(d, want);
                }
            }
            let pl2_path = dat_path.with_extension("pl2");
            if !pl2_path.exists() {
                continue;
            }
            let pl2 = Pl2::parse(&std::fs::read(&pl2_path).unwrap()).unwrap();
            pl2s += 1;
            // The byte-order evidence: the PL2 base palette is the BGR
            // pal.dat read as RGB.
            assert_eq!(pl2.base, dat, "{}", pl2_path.display());
            assert_eq!(pl2.light_levels.len(), 32);
            assert_eq!(pl2.alpha_blend.len(), 3);
            // the full-brightness level maps (nearly) every colour to itself
            // (duplicate colours may snap to their twin, `loading` has one)
            let identity = pl2.light_levels[31]
                .iter()
                .enumerate()
                .filter(|&(i, &v)| usize::from(v) == i)
                .count();
            let folder = pl2_path
                .parent()
                .and_then(|p| p.file_name())
                .unwrap()
                .to_string_lossy();
            if folder.starts_with("act") {
                assert!(identity >= 250, "{}", pl2_path.display());
            }
        }
    }
    println!("palette corpus: {dats} pal.dat, {pl2s} pal.pl2 parsed");
}

#[test]
fn corpus_act1_is_warm_under_bgr() {
    // The empirical half of the byte-order decision: the Act 1 palette is
    // earth and foliage, so reading pal.dat as BGR must give more warm
    // entries (R > B) than cool ones.
    let Some(root) = testutil::extracted("vanilla") else {
        println!("skipped: extracted/vanilla missing");
        return;
    };
    let bytes = std::fs::read(root.join(act_palette_path(0).replace('\\', "/"))).unwrap();
    let p = Palette::parse_dat(&bytes).unwrap();
    let warm = p
        .colors
        .iter()
        .filter(|c| c[0] > c[2].saturating_add(10))
        .count();
    let cool = p
        .colors
        .iter()
        .filter(|c| c[2] > c[0].saturating_add(10))
        .count();
    println!(
        "act1 pal.dat as BGR: {warm} warm, {cool} cool; index 150 = {:?}",
        p.colors[150]
    );
    assert!(warm > cool * 2);
    assert_eq!(p.colors[150], [84, 120, 188]);
}

#[test]
fn corpus_alpha_tables_blend_the_base_palette() {
    let Some(root) = testutil::extracted("vanilla") else {
        println!("skipped: extracted/vanilla missing");
        return;
    };
    let path = root.join("data/global/palette/act1/pal.pl2");
    let pl2 = Pl2::parse(&std::fs::read(path).unwrap()).unwrap();
    let col = |i: usize| pl2.base.colors[i];
    let dist = |a: [u8; 3], b: [i32; 3]| -> i32 {
        a.iter()
            .zip(b)
            .map(|(&x, y)| (i32::from(x) - y) * (i32::from(x) - y))
            .sum()
    };
    // For a spread of pairs the chosen entry must be (nearly) the closest
    // palette colour to the weighted mix: no other entry is better by more
    // than rounding noise.
    for (level, w) in [(0usize, 0.25f64), (1, 0.5), (2, 0.75)] {
        let mut checked = 0;
        for s in (1..256).step_by(23) {
            for d in (1..256).step_by(29) {
                let mix: [i32; 3] = std::array::from_fn(|k| {
                    (w * f64::from(col(s)[k]) + (1.0 - w) * f64::from(col(d)[k])) as i32
                });
                let got = usize::from(pl2.alpha_blend[level][s][d]);
                let best = (0..256).map(|i| dist(col(i), mix)).min().unwrap();
                assert!(
                    dist(col(got), mix) <= best + 200,
                    "level {level} s {s} d {d}: got {got}"
                );
                checked += 1;
            }
        }
        assert!(checked > 50);
    }
    // additive row 0 is the identity
    assert!(
        pl2.additive[0]
            .iter()
            .enumerate()
            .all(|(i, &v)| usize::from(v) == i)
    );
}
