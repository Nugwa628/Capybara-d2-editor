use std::path::{Path, PathBuf};

use d2_formats::cof::Cof;

use crate::cof::{
    COMPONENT_CODES, CofLayerSprite, UnitKind, cof_component_paths, compose_cof_frame,
};
use crate::image_io::export_png;
use crate::model::{RgbaFrame, Sprite, SpriteFormat};
use crate::testutil::{
    act1_palette, extracted, find_files, image, private_root, synthetic_palette,
};

#[test]
fn component_paths_follow_the_spec() {
    let p = cof_component_paths(UnitKind::Object, "2N", "NU", "HTH", "TR", "LIT");
    assert_eq!(
        p,
        vec![
            "data\\global\\objects\\2n\\tr\\2ntrlitnuhth.dcc".to_owned(),
            "data\\global\\objects\\2n\\tr\\2ntrlitnuhth.dc6".to_owned(),
        ]
    );
    let m = cof_component_paths(UnitKind::Monster, "ZM", "A1", "HTH\0", "HD", "");
    assert_eq!(m[0], "data\\global\\monsters\\zm\\hd\\zmhda1hth.dcc");
    let c = cof_component_paths(UnitKind::Character, "am", "wl", "bow ", "rh", "lit");
    assert_eq!(c[0], "data\\global\\chars\\am\\rh\\amrhlitwlbow.dcc");
    assert!(c[1].ends_with(".dc6"));
}

fn opaque_layer(component: u8, x: i32, y: i32, w: u32, h: u32, colour: u8) -> CofLayerSprite {
    CofLayerSprite {
        component,
        sprite: Sprite {
            directions: vec![vec![image(w, h, x, y, vec![colour; (w * h) as usize])]],
        },
        draw_effect: 0,
        transparent: false,
    }
}

/// A two-layer COF: component 1 (TR) is drawn first and component 0 (HD) on
/// top in frame 0; the order is reversed in frame 1.
fn synthetic_cof() -> Cof {
    let mut bytes = vec![0u8; 28];
    bytes[0] = 2; // layers
    bytes[1] = 2; // frames
    bytes[2] = 1; // directions
    for comp in [0u8, 1] {
        bytes.extend_from_slice(&[comp, 0, 0, 0, 0, b'H', b'T', b'H', 0]);
    }
    bytes.extend_from_slice(&[0, 0]); // keyframes
    bytes.extend_from_slice(&[1, 0]); // frame 0: TR then HD
    bytes.extend_from_slice(&[0, 1]); // frame 1: HD then TR
    Cof::parse(&bytes).unwrap()
}

#[test]
fn layers_are_drawn_in_priority_order() {
    let pal = synthetic_palette();
    let cof = synthetic_cof();
    let layers = [
        opaque_layer(0, -2, -2, 4, 4, 10), // HD
        opaque_layer(1, 0, 0, 4, 4, 20),   // TR, overlaps HD at (0..2, 0..2)
    ];
    let f0 = compose_cof_frame(&cof, &layers, 0, 0, &pal).unwrap();
    assert_eq!(
        (f0.width, f0.height, f0.offset_x, f0.offset_y),
        (6, 6, -2, -2)
    );
    // frame 0: TR first, HD on top
    assert_eq!(f0.get(2, 2).unwrap()[..3], pal.colors[10]);
    // frame 1: HD first, TR on top
    let f1 = compose_cof_frame(&cof, &layers, 0, 1, &pal).unwrap();
    assert_eq!(f1.get(2, 2).unwrap()[..3], pal.colors[20]);
    // outside the overlap each layer shows through, and the rest is clear
    assert_eq!(f0.get(0, 0).unwrap()[..3], pal.colors[10]);
    assert_eq!(f0.get(5, 5).unwrap()[..3], pal.colors[20]);
    assert_eq!(f0.get(5, 0).unwrap(), [0, 0, 0, 0]);
}

#[test]
fn blend_modes_come_from_trans_b() {
    let pal = synthetic_palette();
    let cof = synthetic_cof();
    let base = opaque_layer(1, 0, 0, 2, 2, 20);
    for (effect, blend) in [
        (0u8, d2_formats::blend::Blend::Alpha25),
        (1, d2_formats::blend::Blend::Alpha50),
        (2, d2_formats::blend::Blend::Alpha75),
        (3, d2_formats::blend::Blend::Additive),
        (4, d2_formats::blend::Blend::Multiply),
        (6, d2_formats::blend::Blend::Screen),
        (5, d2_formats::blend::Blend::Normal),
        (9, d2_formats::blend::Blend::Normal),
    ] {
        let top = CofLayerSprite {
            transparent: true,
            draw_effect: effect,
            ..opaque_layer(0, 0, 0, 2, 2, 30)
        };
        // frame 1 draws HD (the blended layer) first, so use frame 0: TR then HD
        let out = compose_cof_frame(&cof, &[top.clone(), base.clone()], 0, 0, &pal).unwrap();
        let under = [pal.colors[20][0], pal.colors[20][1], pal.colors[20][2], 255];
        assert_eq!(
            out.get(0, 0).unwrap(),
            blend.apply(under, pal.colors[30]),
            "effect {effect}"
        );
        // without the transparent flag the layer is opaque
        let solid = CofLayerSprite {
            transparent: false,
            ..top
        };
        let out = compose_cof_frame(&cof, &[solid, base.clone()], 0, 0, &pal).unwrap();
        assert_eq!(out.get(0, 0).unwrap()[..3], pal.colors[30]);
    }
}

#[test]
fn missing_layers_and_bad_indices() {
    let pal = synthetic_palette();
    let cof = synthetic_cof();
    // only one of the two layers supplied
    let only = [opaque_layer(1, 3, 4, 2, 2, 20)];
    let f = compose_cof_frame(&cof, &only, 0, 0, &pal).unwrap();
    assert_eq!((f.width, f.height, f.offset_x, f.offset_y), (2, 2, 3, 4));
    // none supplied: an empty frame, not an error
    let none = compose_cof_frame(&cof, &[], 0, 0, &pal).unwrap();
    assert_eq!((none.width, none.height), (0, 0));
    // direction / frame outside the COF
    assert!(compose_cof_frame(&cof, &only, 1, 0, &pal).is_err());
    assert!(compose_cof_frame(&cof, &only, 0, 2, &pal).is_err());
    // a layer sprite with fewer frames repeats its last one; fewer directions skip it
    let short = opaque_layer(0, 0, 0, 1, 1, 9);
    assert!(compose_cof_frame(&cof, std::slice::from_ref(&short), 0, 1, &pal).is_ok());
    let mut no_dirs = short;
    no_dirs.sprite.directions.clear();
    assert_eq!(
        compose_cof_frame(&cof, &[no_dirs], 0, 0, &pal)
            .unwrap()
            .width,
        0
    );
}

// ---------------------------------------------------------------------------
// a real object COF from the corpus

struct Resolved {
    token: String,
    mode: String,
    cof: Cof,
    layers: Vec<CofLayerSprite>,
}

fn load_sprite(objects: &Path, mpq_path: &str) -> Option<(Sprite, SpriteFormat)> {
    let rel = mpq_path
        .strip_prefix("data\\global\\objects\\")?
        .replace('\\', "/");
    let path = objects.join(rel);
    let format = SpriteFormat::from_extension(path.extension()?.to_str()?)?;
    let bytes = std::fs::read(path).ok()?;
    Some((Sprite::decode(&bytes, format).ok()?, format))
}

/// Resolves every layer of an object COF through `cof_component_paths`.
/// The armour token is whatever sits between the component code and the
/// mode in the real file name.
fn resolve(objects: &Path, cof_path: &Path) -> Option<Resolved> {
    let token = cof_path
        .parent()?
        .parent()?
        .file_name()?
        .to_str()?
        .to_owned();
    let stem = cof_path.file_stem()?.to_str()?.to_owned();
    let tail = stem.strip_prefix(&token)?;
    let (mode, class) = (tail.get(..2)?.to_owned(), tail.get(2..)?.to_owned());
    let cof = Cof::parse(&std::fs::read(cof_path).ok()?).ok()?;
    if cof.layers.is_empty() || cof.layers.len() > 4 {
        return None;
    }
    let mut layers = Vec::new();
    for layer in &cof.layers {
        let code = COMPONENT_CODES
            .get(usize::from(layer.component))?
            .to_ascii_lowercase();
        let wclass: String = layer
            .weapon_class
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| char::from(b))
            .collect();
        let dir = objects.join(&token).join(&code);
        let prefix = format!("{token}{code}");
        let suffix = format!("{mode}{}", wclass.to_ascii_lowercase());
        let armor = std::fs::read_dir(&dir).ok()?.flatten().find_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let base = name
                .strip_suffix(".dcc")
                .or_else(|| name.strip_suffix(".dc6"))?;
            base.strip_prefix(&prefix)?
                .strip_suffix(&suffix)
                .map(str::to_owned)
        })?;
        let candidates =
            cof_component_paths(UnitKind::Object, &token, &mode, &wclass, &code, &armor);
        let (sprite, _) = candidates.iter().find_map(|p| load_sprite(objects, p))?;
        layers.push(CofLayerSprite {
            component: layer.component,
            sprite,
            draw_effect: layer.draw_effect,
            transparent: layer.transparent != 0,
        });
    }
    let _ = class;
    Some(Resolved {
        token,
        mode,
        cof,
        layers,
    })
}

fn on_grey(f: &RgbaFrame) -> RgbaFrame {
    let mut out = RgbaFrame::new(f.width, f.height, f.offset_x, f.offset_y);
    for y in 0..f.height {
        for x in 0..f.width {
            let [r, g, b, a] = f.get(x, y).unwrap();
            let bg = if (x / 8 + y / 8) % 2 == 0 { 70u16 } else { 90 };
            let mix = |c: u8, bgc: u16| {
                ((u16::from(c) * u16::from(a) + bgc * (255 - u16::from(a))) / 255) as u8
            };
            out.put(x, y, [mix(r, bg), mix(g, bg), mix(b, bg), 255]);
        }
    }
    out
}

/// Frames of one direction side by side, bottoms aligned.
fn strip(frames: &[RgbaFrame]) -> RgbaFrame {
    let w: u32 = frames.iter().map(|f| f.width + 4).sum();
    let h = frames.iter().map(|f| f.height).max().unwrap_or(0);
    let mut out = RgbaFrame::new(w, h, 0, 0);
    let mut x0 = 0;
    for f in frames {
        for y in 0..f.height {
            for x in 0..f.width {
                out.put(x0 + x, h - f.height + y, f.get(x, y).unwrap());
            }
        }
        x0 += f.width + 4;
    }
    out
}

fn out_dir() -> Option<PathBuf> {
    let dir = private_root()?.join("sprite-tests");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[test]
fn compose_real_object_cofs() {
    let (Some(root), Some(pal)) = (extracted("vanilla"), act1_palette()) else {
        println!("skipped: no corpus");
        return;
    };
    let objects = root.join("data/global/objects");
    let cofs = find_files(&objects, "cof");
    let mut resolved = Vec::new();
    for path in &cofs {
        if let Some(r) = resolve(&objects, path) {
            resolved.push(r);
        }
    }
    println!(
        "object COFs: {} found, {} fully resolved",
        cofs.len(),
        resolved.len()
    );
    assert!(resolved.len() >= 50, "only {} resolved", resolved.len());

    let mut best: Option<usize> = None;
    let mut empty = Vec::new();
    for (i, r) in resolved.iter().enumerate() {
        let frame = compose_cof_frame(&r.cof, &r.layers, 0, 0, &pal).unwrap();
        if !frame.pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0) {
            // e.g. an animation whose first frame is blank
            empty.push(format!("{}{}", r.token, r.mode));
            continue;
        }
        let more = best.is_none_or(|b| {
            let key = |r: &Resolved| (r.cof.layers.len() > 1, r.cof.frames_per_direction);
            key(r) > key(&resolved[b])
        });
        if more {
            best = Some(i);
        }
    }
    println!(
        "composed frame 0 of {} object COFs: {} non-empty, {} blank ({:?})",
        resolved.len(),
        resolved.len() - empty.len(),
        empty.len(),
        &empty[..empty.len().min(8)]
    );
    assert!(
        empty.len() * 10 <= resolved.len(),
        "too many blank composites"
    );

    let Some(r) = best.map(|i| &resolved[i]) else {
        return;
    };
    let frames: Vec<RgbaFrame> = (0..r.cof.frames_per_direction)
        .map(|f| compose_cof_frame(&r.cof, &r.layers, 0, f, &pal).unwrap())
        .collect();
    println!(
        "best: token {} mode {} ({} layers, {} frames, frame size {}x{})",
        r.token,
        r.mode,
        r.cof.layers.len(),
        frames.len(),
        frames[0].width,
        frames[0].height
    );
    if let Some(dir) = out_dir() {
        let name = format!("cof-{}-{}", r.token, r.mode);
        let write = |suffix: &str, f: &RgbaFrame| {
            let path = dir.join(format!("{name}-{suffix}.png"));
            std::fs::write(&path, export_png(f).unwrap()).unwrap();
            println!("wrote {}", path.display());
        };
        write("frame0", &frames[0]);
        write("frame0-onbg", &on_grey(&frames[0]));
        write("anim-onbg", &on_grey(&strip(&frames)));
    }
}
