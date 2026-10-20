use super::*;
use crate::testutil;
use proptest::prelude::*;

type LayerSpec = (u8, u8, u8, u8, u8, [u8; 4]);

/// Independent builder: header, layer records, key frames, priority table.
fn build(layers: &[LayerSpec], frames: u8, dirs: u8, keyframes: &[u8], order: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 28];
    out[0] = layers.len() as u8;
    out[1] = frames;
    out[2] = dirs;
    out[3] = 20;
    for l in layers {
        out.extend_from_slice(&[l.0, l.1, l.2, l.3, l.4]);
        out.extend_from_slice(&l.5);
    }
    out.extend_from_slice(keyframes);
    out.extend_from_slice(order);
    out
}

fn sample() -> (Vec<u8>, Vec<u8>) {
    // 3 layers (TR=1, LG=2, HD=0), 2 frames, 2 directions
    let layers = [
        (1u8, 1u8, 0u8, 0u8, 0u8, *b"HTH\0"),
        (2, 0, 1, 1, 3, *b"1HS\0"),
        (0, 0, 0, 0, 0, *b"HTH\0"),
    ];
    let order: Vec<u8> = vec![
        1, 2, 0, // dir 0 frame 0
        0, 1, 2, // dir 0 frame 1
        2, 1, 0, // dir 1 frame 0
        1, 0, 2, // dir 1 frame 1
    ];
    (build(&layers, 2, 2, &[0, 1], &order), order)
}

#[test]
fn parses_layers_keyframes_and_order() {
    let (bytes, order) = sample();
    let cof = Cof::parse(&bytes).unwrap();
    assert_eq!((cof.directions, cof.frames_per_direction), (2, 2));
    assert_eq!(cof.layers.len(), 3);
    assert_eq!(
        cof.layers[1],
        CofLayer {
            component: 2,
            shadow: 0,
            selectable: 1,
            transparent: 1,
            draw_effect: 3,
            weapon_class: *b"1HS\0"
        }
    );
    assert_eq!(cof.keyframes, vec![0, 1]);
    assert_eq!(cof.layer_order, order);
    assert_eq!(cof.missing_bytes, 0);
    assert_eq!(cof.draw_order(0, 0), Some(&[1u8, 2, 0][..]));
    assert_eq!(cof.draw_order(0, 1), Some(&[0u8, 1, 2][..]));
    assert_eq!(cof.draw_order(1, 0), Some(&[2u8, 1, 0][..]));
    assert_eq!(cof.draw_order(1, 1), Some(&[1u8, 0, 2][..]));
    assert_eq!(cof.draw_order(2, 0), None);
    assert_eq!(cof.draw_order(0, 2), None);
    assert_eq!(cof.draw_order(255, 255), None);
}

#[test]
fn trailing_bytes_are_ignored() {
    let (mut bytes, order) = sample();
    bytes.extend_from_slice(&[9, 9, 9]);
    let cof = Cof::parse(&bytes).unwrap();
    assert_eq!(cof.layer_order, order);
    assert_eq!(cof.missing_bytes, 0);
}

#[test]
fn short_priority_table_is_zero_filled() {
    let (mut bytes, order) = sample();
    bytes.truncate(bytes.len() - 4);
    let cof = Cof::parse(&bytes).unwrap();
    assert_eq!(cof.missing_bytes, 4);
    assert_eq!(&cof.layer_order[..8], &order[..8]);
    assert_eq!(&cof.layer_order[8..], &[0, 0, 0, 0]);
    assert_eq!(cof.draw_order(1, 1).map(<[u8]>::len), Some(3));
}

#[test]
fn cut_layer_records_or_huge_gap_is_an_error() {
    let (bytes, _) = sample();
    assert!(Cof::parse(&[]).is_err());
    assert!(Cof::parse(&bytes[..27]).is_err());
    assert!(Cof::parse(&bytes[..28 + 9 * 3 - 1]).is_err());
    // 255 layers x 255 frames x 255 directions would need 16 MB of zeros
    let mut header = vec![0u8; 28];
    header[..3].copy_from_slice(&[1, 255, 255]);
    header.extend_from_slice(&[0; 9]);
    assert!(Cof::parse(&header).is_err());
    // 230 layers in 72 bytes (the shipped amblxbow.cof)
    let mut junk = vec![0x55u8; 72];
    junk[0] = 230;
    assert!(Cof::parse(&junk).is_err());
}

#[test]
fn degenerate_dimensions() {
    let cof = Cof::parse(&build(&[], 3, 2, &[0, 0, 0], &[])).unwrap();
    assert!(cof.layers.is_empty());
    assert_eq!(cof.draw_order(1, 2), Some(&[][..]));
    let zero = Cof::parse(&build(&[(0, 0, 0, 0, 0, [0; 4])], 0, 0, &[], &[])).unwrap();
    assert_eq!(zero.draw_order(0, 0), None);
}

#[test]
fn duplicate_components_are_kept() {
    let layers = [
        (1u8, 0u8, 0u8, 0u8, 0u8, *b"HTH\0"),
        (1, 0, 0, 0, 0, *b"STF\0"),
    ];
    let cof = Cof::parse(&build(&layers, 1, 1, &[0], &[1, 1])).unwrap();
    assert_eq!(cof.layers.len(), 2);
    assert_eq!(cof.layers[1].weapon_class, *b"STF\0");
    // the last record of a component is the one that counts
    assert_eq!(cof.component_layer(1).unwrap().weapon_class, *b"STF\0");
    assert!(cof.component_layer(0).is_none());
}

#[test]
fn layer_names_and_sprite_paths() {
    let layer = CofLayer {
        component: 1,
        shadow: 0,
        selectable: 0,
        transparent: 0,
        draw_effect: 0,
        weapon_class: *b"1HS ",
    };
    assert_eq!(layer.component_code(), Some("TR"));
    assert_eq!(layer.weapon_class_token(), "1hs");
    let bad = CofLayer {
        component: 16,
        ..layer.clone()
    };
    assert_eq!(bad.component_code(), None);
    assert_eq!(path_token(" HTH\0\0"), "hth");

    assert_eq!(
        cof_path(r"data\global\monsters", "BA", "NU", "HTH\0"),
        r"data\global\monsters\ba\cof\banuhth.cof"
    );
    assert_eq!(
        component_paths(r"data\global\objects", "TO", "on", "hth\0", "TR", "LIT"),
        [
            r"data\global\objects\to\tr\totrlitonhth.dcc".to_owned(),
            r"data\global\objects\to\tr\totrlitonhth.dc6".to_owned(),
        ]
    );
    // an empty armour token leaves a gap-free name
    assert_eq!(
        component_paths("x", "ab", "nu", "hth", "s1", "")[0],
        r"x\ab\s1\abs1nuhth.dcc"
    );
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        if let Ok(cof) = Cof::parse(&bytes) {
            for d in 0..=cof.directions {
                for f in 0..=cof.frames_per_direction {
                    let _ = cof.draw_order(d, f);
                }
            }
        }
    }

    #[test]
    fn header_with_random_tail_is_consistent(
        layers in 0u8..6, frames in 0u8..6, dirs in 0u8..6,
        tail in prop::collection::vec(any::<u8>(), 0..300),
    ) {
        let mut bytes = vec![0u8; 28];
        bytes[..3].copy_from_slice(&[layers, frames, dirs]);
        bytes.extend_from_slice(&tail);
        if let Ok(cof) = Cof::parse(&bytes) {
            prop_assert_eq!(cof.layers.len(), usize::from(layers));
            prop_assert_eq!(cof.keyframes.len(), usize::from(frames));
            prop_assert_eq!(
                cof.layer_order.len(),
                usize::from(layers) * usize::from(frames) * usize::from(dirs)
            );
            for d in 0..dirs {
                for f in 0..frames {
                    prop_assert_eq!(cof.draw_order(d, f).map(<[u8]>::len), Some(usize::from(layers)));
                }
            }
        }
    }
}

/// `.cof` files that are not well-formed COFs, with the reason.
const KNOWN_BAD: &[(&str, &str)] = &[(
    "amblxbow.cof",
    "72 bytes of noise declaring 230 layers; not a COF",
)];

#[test]
fn corpus_cof_files_parse() {
    let mut files = 0usize;
    let mut permutation_rows = 0usize;
    let mut rows = 0usize;
    let mut short = Vec::new();
    let mut odd_rows: Vec<String> = Vec::new();
    let mut known_bad_seen = Vec::new();
    let mut failures = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::find_files(&root, "cof") {
            files += 1;
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let bytes = std::fs::read(&path).unwrap();
            let cof = match Cof::parse(&bytes) {
                Ok(c) => c,
                Err(e) => {
                    if let Some((_, why)) = KNOWN_BAD.iter().find(|(n, _)| *n == name) {
                        known_bad_seen.push(format!("{profile}:{name}: {e} ({why})"));
                    } else {
                        failures.push(format!("{profile}:{name}: {e}"));
                    }
                    continue;
                }
            };
            if cof.missing_bytes > 0 {
                short.push(format!("{profile}:{name} (-{} bytes)", cof.missing_bytes));
            }
            let mut components: Vec<u8> = cof.layers.iter().map(|l| l.component).collect();
            components.sort_unstable();
            let mut file_odd = 0usize;
            for d in 0..cof.directions {
                for f in 0..cof.frames_per_direction {
                    let Some(order) = cof.draw_order(d, f) else {
                        failures.push(format!("{profile}:{name}: no draw order at {d},{f}"));
                        continue;
                    };
                    rows += 1;
                    let mut sorted = order.to_vec();
                    sorted.sort_unstable();
                    if sorted == components {
                        permutation_rows += 1;
                    } else {
                        file_odd += 1;
                    }
                }
            }
            if file_odd > 0 {
                odd_rows.push(format!("{profile}:{name} ({file_odd} rows)"));
            }
        }
    }
    println!(
        "cof corpus: {files} files; {} not-a-COF: {known_bad_seen:?}; {} short (zero-filled): {short:?}",
        known_bad_seen.len(),
        short.len()
    );
    println!(
        "draw_order rows: {rows} total, {permutation_rows} are a permutation of the layer components; \
         files with other rows ({}): {odd_rows:?}",
        odd_rows.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
    // all rows are permutations except those of the (documented) odd files
    if files > 0 {
        assert!(odd_rows.len() <= 12, "{odd_rows:?}");
        assert!(permutation_rows * 100 >= rows * 99);
    }
}

// ---------------------------------------------------------------------------
// writer

#[test]
fn sample_writes_back_identically() {
    let (bytes, _) = sample();
    assert_eq!(Cof::parse(&bytes).unwrap().to_bytes().unwrap(), bytes);
}

#[test]
fn short_and_trailing_files_write_back_identically() {
    let (mut bytes, _) = sample();
    bytes.extend_from_slice(b"tail");
    assert_eq!(Cof::parse(&bytes).unwrap().to_bytes().unwrap(), bytes);
    let (mut short, _) = sample();
    short.truncate(short.len() - 5);
    let cof = Cof::parse(&short).unwrap();
    assert_eq!(cof.missing_bytes, 5);
    assert_eq!(cof.to_bytes().unwrap(), short);
}

#[test]
fn object_header_matches_the_games_object_cofs() {
    let mut cof = Cof::parse(&sample().0).unwrap();
    cof.header = Cof::object_header(-10, 20, -30, 40, 256);
    let out = cof.to_bytes().unwrap();
    assert_eq!(&out[3..8], &[0x14, 0, 0, 0x13, 0]);
    assert_eq!(i32::from_le_bytes(out[8..12].try_into().unwrap()), -10);
    assert_eq!(i32::from_le_bytes(out[12..16].try_into().unwrap()), 20);
    assert_eq!(i32::from_le_bytes(out[16..20].try_into().unwrap()), -30);
    assert_eq!(i32::from_le_bytes(out[20..24].try_into().unwrap()), 40);
    assert_eq!(i16::from_le_bytes(out[24..26].try_into().unwrap()), 256);
}

#[test]
fn inconsistent_models_are_refused() {
    let mut cof = Cof::parse(&sample().0).unwrap();
    cof.keyframes.pop();
    assert!(cof.to_bytes().is_err());
    let mut cof = Cof::parse(&sample().0).unwrap();
    cof.layer_order.pop();
    assert!(cof.to_bytes().is_err());
}

proptest! {
    #[test]
    fn any_parsable_bytes_write_back_identically(
        bytes in prop::collection::vec(any::<u8>(), 0..400),
        head in (0u8..6, 0u8..5, 0u8..5),
    ) {
        let mut b = bytes;
        if b.len() >= 3 {
            b[0] = head.0;
            b[1] = head.1;
            b[2] = head.2;
        }
        if let Ok(cof) = Cof::parse(&b) {
            prop_assert_eq!(cof.to_bytes().unwrap(), b);
        }
    }
}

#[test]
fn corpus_cof_files_write_back_identically() {
    let (mut files, mut identical, mut unreadable) = (0usize, 0usize, 0usize);
    let mut failures = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for path in testutil::find_files(&root, "cof") {
            files += 1;
            let bytes = std::fs::read(&path).unwrap();
            match Cof::parse(&bytes) {
                Err(_) => unreadable += 1,
                Ok(c) => match c.to_bytes() {
                    Ok(out) if out == bytes => identical += 1,
                    other => {
                        failures.push(format!("{}: {:?}", path.display(), other.map(|o| o.len())))
                    }
                },
            }
        }
    }
    println!(
        "cof write-back: {files} files, {identical} identical, {unreadable} unreadable, failures {failures:#?}"
    );
    assert!(failures.is_empty());
    assert_eq!(identical + unreadable, files);
}
