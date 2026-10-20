//! Corpus checks of the GIF export: a real DCC and a real COF composite are
//! exported with a frame range, decoded again and compared pixel by pixel.
//! Skipped without the private corpus (`CAPY_PRIVATE_DIR`).
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::path::{Path, PathBuf};

use capy_sprite::{Sprite, SpriteFormat};
use d2_assets::{FolderSource, GamePath, Overlay};
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use super::cof::{CofView, parse_cof_path};
use super::gif_export::{DirChoice, GifSettings, GifSource, build};

type Rgb = Option<[u8; 3]>;
type Want = (u32, u32, i32, i32, Vec<Rgb>);

fn corpus() -> Option<(PathBuf, Palette)> {
    let root = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .map(|a| a.join("testdata").join("private"))
                .find(|p| p.is_dir())
        })?;
    let root = root.join("extracted").join("vanilla");
    let bytes = std::fs::read(root.join("data/global/palette/act1/pal.dat")).ok()?;
    Some((root, Palette::parse_dat(&bytes).ok()?))
}

/// Decodes `gif_bytes` and checks that frame `i` is the picture `want[i]`
/// at `scale` on the union canvas, with `delay` centiseconds.
fn check_gif(gif_bytes: &[u8], want: &[Want], scale: u32, delay: u16) {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = opts.read_info(std::io::Cursor::new(gif_bytes)).unwrap();
    let table = dec.global_palette().unwrap().to_vec();
    let (cw, ch) = (u32::from(dec.width()), u32::from(dec.height()));
    let live: Vec<_> = want.iter().filter(|f| f.0 > 0 && f.1 > 0).collect();
    let x0 = live.iter().map(|f| f.2).min().unwrap();
    let y0 = live.iter().map(|f| f.3).min().unwrap();
    let x1 = live.iter().map(|f| f.2 + f.0 as i32).max().unwrap();
    let y1 = live.iter().map(|f| f.3 + f.1 as i32).max().unwrap();
    assert_eq!(
        (cw, ch),
        ((x1 - x0) as u32 * scale, (y1 - y0) as u32 * scale)
    );
    let mut n = 0;
    while let Some(f) = dec.read_next_frame().unwrap() {
        let (w, h, ox, oy, px) = &want[n];
        assert_eq!(f.delay, delay, "frame {n} delay");
        assert_eq!(f.transparent, Some(0));
        let mut wrong = 0;
        for y in 0..ch {
            for x in 0..cw {
                let got = f.buffer[(y * cw + x) as usize];
                let (sx, sy) = ((x / scale) as i32 + x0 - ox, (y / scale) as i32 + y0 - oy);
                let expect = if sx >= 0 && sy >= 0 && (sx as u32) < *w && (sy as u32) < *h {
                    px[(sy as u32 * w + sx as u32) as usize]
                } else {
                    None
                };
                let rgb = (got != 0).then(|| {
                    let i = usize::from(got) * 3;
                    [table[i], table[i + 1], table[i + 2]]
                });
                if rgb != expect {
                    wrong += 1;
                }
            }
        }
        assert_eq!(wrong, 0, "frame {n}: {wrong} pixels differ");
        n += 1;
    }
    assert_eq!(n, want.len(), "frame count");
}

fn indexed_want(img: &IndexedImage, pal: &Palette) -> Want {
    let px = img
        .pixels
        .iter()
        .zip(&img.alpha)
        .map(|(&p, &a)| (a != 0 && p != 0).then(|| pal.colors[usize::from(p)]))
        .collect();
    (img.width, img.height, img.offset_x, img.offset_y, px)
}

#[test]
fn real_dcc_range_scale_and_order_survive_the_gif() {
    let Some((root, pal)) = corpus() else {
        eprintln!("skipped: no private corpus");
        return;
    };
    let bytes = std::fs::read(root.join("data/global/objects/12/tr/12trlitnuhth.dcc")).unwrap();
    let sprite = Sprite::decode(&bytes, SpriteFormat::Dcc).unwrap();
    let total = sprite.frames_per_direction();
    assert!(total >= 10, "a real animation: {total} frames");
    let mut s = GifSettings::new(total, sprite.directions.len(), 12.5);
    (s.start, s.end) = (2, 8);
    s.scale = 2;
    let files = build(GifSource::Sprite(&sprite), &s, 0, &pal).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].frames, 7);
    let want: Vec<_> = sprite.directions[0][2..=8]
        .iter()
        .map(|i| indexed_want(i, &pal))
        .collect();
    check_gif(&files[0].bytes, &want, 2, 8);
    // Not a single frame, and not all the same picture.
    assert!(want.windows(2).any(|w| w[0].4 != w[1].4));
}

#[test]
fn real_cof_composite_exports_with_a_frame_range() {
    let Some((root, pal)) = corpus() else {
        eprintln!("skipped: no private corpus");
        return;
    };
    let path = "data\\global\\monsters\\fa\\cof\\fanuhth.cof";
    let game = GamePath::new(path);
    let overlay = Overlay::new(vec![Box::new(FolderSource::new(&root, "corpus"))]);
    let bytes = overlay.read_bytes(&game).unwrap().expect("the COF");
    let cof = d2_formats::cof::Cof::parse(&bytes).unwrap();
    let name = parse_cof_path(path).unwrap();
    let view = CofView::load(cof, name, Some(game), Some(&overlay), &[], &[]);
    assert!(view.layers.iter().filter(|l| l.sprite.is_some()).count() >= 2);
    let total = view.frames();
    assert!(total >= 8, "{total} frames");
    let mut s = GifSettings::new(total, view.directions(), 25.0);
    (s.start, s.end) = (1, 5);
    s.dirs = DirChoice::Ticked;
    s.ticked = vec![false; view.directions()];
    s.ticked[2] = true;
    let files = build(GifSource::Cof(&view), &s, 0, &pal).unwrap();
    assert_eq!(
        (files.len(), files[0].dir, files[0].frames),
        (1, Some(2), 5)
    );
    // Expected: the composite the Viewer shows, mapped to the palette.
    let want: Vec<Want> = (1..=5)
        .map(|f| {
            let r = view.compose(2, f, &pal).unwrap();
            let px = r
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    (p[3] >= 128).then(|| {
                        pal.colors[usize::from(capy_paint::nearest_index(&pal, [p[0], p[1], p[2]]))]
                    })
                })
                .collect();
            (r.width, r.height, r.offset_x, r.offset_y, px)
        })
        .collect();
    assert!(want.iter().any(|f| f.4.iter().any(Option::is_some)));
    check_gif(&files[0].bytes, &want, 1, 4);
    // One part alone is a plain sprite export of that layer.
    let first = view.layers.iter().position(|l| l.sprite.is_some()).unwrap();
    let single = view.layers[first].sprite.as_ref().unwrap();
    s.part = Some(first);
    s.dirs = DirChoice::Current;
    let part = build(GifSource::Sprite(single), &s, 2, &pal).unwrap();
    assert_eq!(part[0].frames, 5);
}
