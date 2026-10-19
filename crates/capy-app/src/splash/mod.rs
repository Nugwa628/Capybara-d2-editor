//! The loading screen: Diablo II's door-opening animation with the capybara
//! in the door and "Capybara's D2 Editor" on the plate, ported from the
//! user's earlier MXL Capybara Editor (see [`art`]).
//!
//! Differences from the old program, by design:
//! * it is an egui overlay in the editor's own window, not a second process;
//!   the profile, catalog and index load on their usual background workers
//!   meanwhile, and the frames are built on a thread of their own;
//! * the 256 px picture is scaled by the largest integer that fits the
//!   window (nearest neighbour), the title by half of that (the old 2x splash
//!   drew it at 1x);
//! * it shows from launch until the start profile's map catalog is ready,
//!   for at least one pass of the animation (850 ms), then fades out like
//!   the old one (alpha -0.15 per 40 ms). A click or Esc skips it.
//!
//! Kept from the old program: the frames play once at 85 ms, then the door
//! light flickers between the last two frames (2.5 Hz); any dialog closes
//! the splash first, so nothing is ever hidden behind it; without the game
//! files there is a plain dark splash with the capybara and the title; it
//! gives up after 90 s.

pub mod art;
pub mod font;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32, Id, Pos2, Rect, Vec2};
use d2_assets::{GamePath, Profile};

use crate::branding::{self, RgbaImage};
use art::{FRAME_MS, SIZE, TITLE, TITLE_FONT, TITLE_GOLD};

pub const DOOR_FILE: &str = r"data\global\ui\loading\loadingscreen.dc6";
pub const DOOR_PAL: &str = r"data\global\palette\loading\pal.dat";

/// At least one pass of the ten-frame animation.
const MIN_SHOW: Duration = Duration::from_millis(850);
/// The old fade: alpha -0.15 every 40 ms, so 7 steps.
const FADE: Duration = Duration::from_millis(280);
/// Black (no picture yet) for at most this long, then the plain splash
/// until the door frames are ready.
const PLAIN_AFTER: Duration = Duration::from_millis(400);
/// The old program's safety net.
const MAX_SHOW: Duration = Duration::from_secs(90);

/// The finished pictures.
pub struct SplashArt {
    pub frames: Vec<RgbaImage>,
    pub title: RgbaImage,
}

/// Build the frames from the game files `read` returns (the active
/// profile's overlay, read-only).
pub fn build_art(read: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<SplashArt, String> {
    let palette = d2_formats::palette::Palette::parse_dat(&read(DOOR_PAL)?)
        .map_err(|e| format!("{DOOR_PAL}: {e}"))?;
    let dc6 =
        d2_formats::dc6::Dc6::parse(&read(DOOR_FILE)?).map_err(|e| format!("{DOOR_FILE}: {e}"))?;
    if dc6.frames.is_empty() {
        return Err(format!("{DOOR_FILE} has no frames"));
    }
    let font_base = format!(r"data\local\font\latin\{TITLE_FONT}");
    let font = font::GameFont::parse(
        &read(&format!("{font_base}.tbl"))?,
        &read(&format!("{font_base}.dc6"))?,
    )?;
    let title = font.render_solid(TITLE, TITLE_GOLD);
    let icon = branding::capybara().ok_or("the app icon has no 256 px image")?;
    let cap = art::prepare_capybara(&icon).ok_or("the app icon is empty")?;
    let frames = dc6
        .frames
        .iter()
        .map(|f| art::door_frame(f, &palette))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SplashArt {
        frames: art::compose_frames(frames, &cap),
        title,
    })
}

/// Which frame shows `elapsed` after the door appeared: one pass at 85 ms,
/// then (for the game's ten frames) the last two alternate at 2.5 Hz.
pub fn frame_at(elapsed: Duration, frames: usize) -> usize {
    let tick = (elapsed.as_millis() / u128::from(FRAME_MS)) as usize;
    if tick < frames {
        return tick;
    }
    if frames == 10 {
        let half_periods = (elapsed.as_secs_f64() * 2.5) as u64;
        if half_periods % 2 == 1 { 8 } else { 9 }
    } else {
        frames.saturating_sub(1)
    }
}

/// What a script asked the splash to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    Frame(usize),
    Plain,
}

struct Textures {
    frames: Vec<egui::TextureHandle>,
    title: Option<(egui::TextureHandle, usize, usize)>,
    icon: Option<egui::TextureHandle>,
}

#[derive(Resource)]
pub struct Splash {
    active: bool,
    started: Instant,
    rx: Option<Mutex<Receiver<Result<SplashArt, String>>>>,
    art: Option<Result<SplashArt, String>>,
    textures: Option<Textures>,
    /// When the door (or the final plain splash) first showed.
    door_since: Option<Instant>,
    plain_since: Option<Instant>,
    closing_since: Option<Instant>,
    pin: Option<Pin>,
}

impl Splash {
    pub fn new(active: bool) -> Splash {
        Splash {
            active,
            started: Instant::now(),
            rx: None,
            art: None,
            textures: None,
            door_since: None,
            plain_since: None,
            closing_since: None,
            pin: None,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Start building the frames from `profile` (once). Without a profile
    /// the plain splash shows.
    pub fn start_build(&mut self, profile: Option<Profile>, saves: PathBuf) {
        if self.rx.is_some() || self.art.is_some() {
            return;
        }
        let Some(profile) = profile else {
            self.art = Some(Err("no game profile".to_owned()));
            return;
        };
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("loading-screen".to_owned())
            .spawn(move || {
                let result = crate::crash::catch(|| {
                    let overlay = profile
                        .open(&saves)
                        .map_err(|e| format!("opening profile {:?}: {e}", profile.id))?;
                    let read = |path: &str| -> Result<Vec<u8>, String> {
                        overlay
                            .read_bytes(&GamePath::new(path))
                            .map_err(|e| format!("{path}: {e}"))?
                            .ok_or_else(|| format!("{path} is not in the game files"))
                    };
                    build_art(&read)
                })
                .unwrap_or_else(|_| Err("building the loading screen panicked".to_owned()));
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => self.rx = Some(Mutex::new(rx)),
            Err(e) => self.art = Some(Err(format!("cannot start a thread: {e}"))),
        }
    }

    /// Script hook: show this frame (or the plain look) until told otherwise.
    pub fn pin(&mut self, pin: Pin) {
        self.pin = Some(pin);
        self.active = true;
        self.closing_since = None;
    }

    /// Script hook: behave as at launch (timers restarted, nothing pinned).
    pub fn restart(&mut self) {
        self.pin = None;
        self.active = true;
        self.started = Instant::now();
        self.door_since = None;
        self.plain_since = None;
        self.closing_since = None;
    }

    pub fn close(&mut self) {
        self.active = false;
        self.pin = None;
        self.textures = None;
    }

    /// `None` while building, else whether the door frames exist.
    pub fn built(&mut self) -> Option<Result<usize, String>> {
        self.poll();
        self.art.as_ref().map(|r| match r {
            Ok(a) => Ok(a.frames.len()),
            Err(e) => Err(e.clone()),
        })
    }

    /// Script hook `splash-save`: the view at scale `n`, as on screen.
    pub fn view(&mut self, frame: usize, n: usize) -> Result<RgbaImage, String> {
        self.poll();
        match &self.art {
            Some(Ok(a)) => {
                let f = a
                    .frames
                    .get(frame)
                    .ok_or_else(|| format!("frame {frame}: there are {}", a.frames.len()))?;
                Ok(art::compose_view(f, Some(&a.title), n))
            }
            Some(Err(e)) => Err(format!("no door frames: {e}")),
            None => Err("the loading screen is still being built".to_owned()),
        }
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let got = match rx.lock() {
            Ok(rx) => match rx.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("the loading screen thread stopped".to_owned()))
                }
            },
            Err(_) => Some(Err("the loading screen thread stopped".to_owned())),
        };
        if let Some(r) = got {
            if let Err(e) = &r {
                bevy::log::info!("Loading screen: plain look ({e})");
            }
            self.art = Some(r);
            self.rx = None;
        }
    }

    fn upload(&mut self, ctx: &egui::Context) {
        if self.textures.is_some() {
            return;
        }
        let image = |name: &str, img: &RgbaImage, options| {
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.data),
                options,
            )
        };
        let mut t = Textures {
            frames: Vec::new(),
            title: None,
            icon: branding::capybara()
                .map(|icon| image("splash-icon", &icon, egui::TextureOptions::LINEAR)),
        };
        if let Some(Ok(a)) = &self.art {
            for (i, f) in a.frames.iter().enumerate() {
                t.frames.push(image(
                    &format!("splash-{i}"),
                    f,
                    egui::TextureOptions::NEAREST,
                ));
            }
            let title = image("splash-title", &a.title, egui::TextureOptions::NEAREST);
            t.title = Some((title, a.title.width, a.title.height));
        }
        self.textures = Some(t);
    }
}

/// Before the UI runs: a click or Esc skips the splash, and the keyboard
/// does not reach the panels underneath while it shows.
pub fn take_input(ctx: &egui::Context, splash: &mut Splash) {
    if !splash.active {
        return;
    }
    let skip = ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.any_pressed());
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            !matches!(
                e,
                egui::Event::Key { .. }
                    | egui::Event::Text(_)
                    | egui::Event::Copy
                    | egui::Event::Cut
                    | egui::Event::Paste(_)
            )
        });
    });
    if skip && splash.pin.is_none() {
        splash.close();
    }
}

/// After the UI: draw the splash on top, or close it when a dialog is up,
/// when the start-up work is done, or after the time limit.
pub fn draw(ctx: &egui::Context, splash: &mut Splash, ready: bool, dialog_open: bool) {
    if !splash.active {
        return;
    }
    if dialog_open || ctx.memory(|m| m.top_modal_layer()).is_some() {
        splash.close();
        return;
    }
    splash.poll();
    let now = Instant::now();
    let door = matches!(splash.art, Some(Ok(_)));
    let failed = matches!(splash.art, Some(Err(_)));
    // Pictures change from nothing to plain to the door; upload per look.
    let had_door = splash
        .textures
        .as_ref()
        .is_some_and(|t| !t.frames.is_empty());
    if door && !had_door {
        splash.textures = None;
    }
    let look = match splash.pin {
        Some(Pin::Frame(n)) if door => Look::Door(n),
        Some(Pin::Plain) => Look::Plain,
        Some(Pin::Frame(_)) => Look::Black, // waiting for the build
        None if door => {
            let since = *splash.door_since.get_or_insert(now);
            let frames = splash
                .art
                .as_ref()
                .map_or(0, |a| a.as_ref().map_or(0, |a| a.frames.len()));
            Look::Door(frame_at(now - since, frames))
        }
        None if failed || now - splash.started >= PLAIN_AFTER => {
            splash.plain_since.get_or_insert(now);
            Look::Plain
        }
        None => Look::Black,
    };
    if splash.pin.is_none() {
        let shown = splash
            .door_since
            .or(if failed { splash.plain_since } else { None });
        let long_enough = shown.is_some_and(|t| now - t >= MIN_SHOW);
        if splash.closing_since.is_none()
            && ((ready && long_enough) || now - splash.started >= MAX_SHOW)
        {
            splash.closing_since = Some(now);
        }
    }
    let alpha = match splash.closing_since {
        Some(t) => 1.0 - (now - t).as_secs_f32() / FADE.as_secs_f32(),
        None => 1.0,
    };
    if alpha <= 0.0 {
        splash.close();
        return;
    }
    if !matches!(look, Look::Black) {
        splash.upload(ctx);
    }
    paint(ctx, splash, look, alpha);
    ctx.request_repaint();
}

#[derive(Clone, Copy)]
enum Look {
    Black,
    Plain,
    Door(usize),
}

fn paint(ctx: &egui::Context, splash: &Splash, look: Look, alpha: f32) {
    let screen = ctx.viewport_rect();
    let ppp = ctx.pixels_per_point();
    let (wpx, hpx) = (screen.width() * ppp, screen.height() * ppp);
    let n = ((wpx.min(hpx) / SIZE as f32).floor() as usize).max(1);
    let edge_px = (SIZE * n) as f32;
    // whole physical pixels, so the nearest-neighbour scale stays crisp
    let left_px = ((wpx - edge_px) / 2.0).floor();
    let top_px = ((hpx - edge_px) / 2.0).floor();
    let at = |x: f32, y: f32| screen.min + Vec2::new((left_px + x) / ppp, (top_px + y) / ppp);
    let tint = Color32::WHITE.gamma_multiply(alpha);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    egui::Area::new(Id::new("loading_screen"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.allocate_rect(screen, egui::Sense::click());
            let painter = ui.painter();
            painter.rect_filled(screen, 0.0, Color32::BLACK.gamma_multiply(alpha));
            let Some(t) = &splash.textures else { return };
            match look {
                Look::Black => {}
                Look::Door(i) => {
                    if let Some(tex) = t.frames.get(i.min(t.frames.len().saturating_sub(1))) {
                        let rect = Rect::from_min_max(at(0.0, 0.0), at(edge_px, edge_px));
                        painter.image(tex.id(), rect, uv, tint);
                    }
                    if let Some((tex, w, h)) = &t.title {
                        let (left, top, ts) = art::title_place(n, *w, *h);
                        let (l, tp) = (left as f32, top as f32);
                        let rect = Rect::from_min_max(
                            at(l, tp),
                            at(l + (w * ts) as f32, tp + (h * ts) as f32),
                        );
                        painter.image(tex.id(), rect, uv, tint);
                    }
                }
                Look::Plain => {
                    // The old fallback on its 2x window: the 256 px icon
                    // centred 30 px above the middle, the title 60 px above
                    // the bottom in 18 pt bold gold.
                    let unit = n as f32 / 2.0;
                    let (cx, cy) = (edge_px / 2.0, edge_px / 2.0 - 30.0 * unit);
                    if let Some(tex) = &t.icon {
                        let half = 128.0 * unit;
                        let rect =
                            Rect::from_min_max(at(cx - half, cy - half), at(cx + half, cy + half));
                        painter.image(tex.id(), rect, uv, tint);
                    }
                    let [r, g, b] = TITLE_GOLD;
                    painter.text(
                        at(cx, edge_px - 60.0 * unit),
                        egui::Align2::CENTER_CENTER,
                        TITLE,
                        egui::FontId::proportional(24.0 * unit / ppp),
                        Color32::from_rgb(r, g, b).gamma_multiply(alpha),
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_play_once_then_the_light_flickers() {
        let ms = Duration::from_millis;
        assert_eq!(frame_at(ms(0), 10), 0);
        assert_eq!(frame_at(ms(84), 10), 0);
        assert_eq!(frame_at(ms(85), 10), 1);
        assert_eq!(frame_at(ms(849), 10), 9);
        // after the pass: 8 and 9 alternate every 0.4 s
        let shown: Vec<usize> = (0..20).map(|k| frame_at(ms(850 + 100 * k), 10)).collect();
        assert!(shown.iter().all(|&f| f == 8 || f == 9), "{shown:?}");
        assert!(shown.contains(&8) && shown.contains(&9), "{shown:?}");
        assert_eq!(frame_at(ms(1300), 10), 8); // 3.25 half periods -> odd
        assert_eq!(frame_at(ms(1700), 10), 9); // 4.25 -> even
        // other frame counts hold the last frame
        assert_eq!(frame_at(ms(5000), 4), 3);
        assert_eq!(frame_at(ms(5000), 0), 0);
    }

    /// Synthetic game files: a 2-frame door, a palette and a font.
    fn files(path: &str) -> Result<Vec<u8>, String> {
        match path {
            DOOR_PAL => Ok((0..768).map(|i| (i % 256) as u8).collect()),
            DOOR_FILE => {
                let frames: Vec<d2_formats::image::IndexedImage> = (0..2)
                    .map(|k| {
                        let mut img =
                            d2_formats::image::IndexedImage::new(SIZE as u32, SIZE as u32, 0, 0);
                        for (i, p) in img.pixels.iter_mut().enumerate() {
                            *p = 1 + ((i + k * 50) % 200) as u8;
                        }
                        img.alpha.iter_mut().for_each(|a| *a = 255);
                        img
                    })
                    .collect();
                d2_formats::dc6::Dc6::from_frames(&[frames])
                    .and_then(|d| d.to_bytes())
                    .map_err(|e| e.to_string())
            }
            p if p.ends_with(".tbl") || p.ends_with(".dc6") => {
                let glyphs: Vec<(u16, u8, u32, u32, i32)> =
                    (32u16..127).map(|c| (c, 6, 5, 8, 0)).collect();
                let (tbl, dc6) = font::tests::synthetic_font(&glyphs);
                Ok(if p.ends_with(".tbl") { tbl } else { dc6 })
            }
            other => Err(format!("{other} is not in the game files")),
        }
    }

    #[test]
    fn art_builds_from_game_files_and_fails_without_them() {
        let art = build_art(&files).expect("builds");
        assert_eq!(art.frames.len(), 2);
        assert!(
            art.frames
                .iter()
                .all(|f| f.width == SIZE && f.height == SIZE)
        );
        // 20 characters at advance 6, plus the outline border
        assert_eq!(art.title.width, 20 * 6 + 2);
        assert_eq!(art.title.height, 8 + 2);
        let missing = |p: &str| -> Result<Vec<u8>, String> {
            if p == DOOR_FILE {
                Err(format!("{p} is not in the game files"))
            } else {
                files(p)
            }
        };
        let e = build_art(&missing).err().expect("no door, no art");
        assert!(e.contains("loadingscreen.dc6"), "{e}");
    }

    #[test]
    fn real_game_files_when_extracted() {
        // Uses the private extracted corpus when it exists (not in CI).
        // The nearest testdata\private (agent worktrees sit inside the checkout).
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(root) = manifest
            .ancestors()
            .map(|a| {
                a.join("testdata")
                    .join("private")
                    .join("extracted")
                    .join("vanilla")
            })
            .find(|r| r.join(DOOR_FILE).is_file())
        else {
            eprintln!("skipped: no extracted vanilla files");
            return;
        };
        let read = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let art = build_art(&read).expect("builds from the real files");
        assert_eq!(art.frames.len(), 10);
        // the door opens: frame 9 is lighter in the door than frame 0
        let l0 = art::lum(art::door_light(&art.frames[0]));
        let l9 = art::lum(art::door_light(&art.frames[9]));
        assert!(l9 > l0 + 20.0, "{l0} {l9}");
        assert!(
            art.title.width > 100 && art.title.height >= 8,
            "{}x{}",
            art.title.width,
            art.title.height
        );
        // For a look by eye: target\tmp\splash_unit\door-<n>.png at 2x.
        let out = manifest.join(r"..\..\target\tmp\splash_unit");
        std::fs::create_dir_all(&out).expect("out dir");
        for (i, f) in art.frames.iter().enumerate() {
            let v = art::compose_view(f, Some(&art.title), 2);
            image::save_buffer(
                out.join(format!("door-{i}.png")),
                &v.data,
                v.width as u32,
                v.height as u32,
                image::ColorType::Rgba8,
            )
            .expect("png");
        }
    }
}
