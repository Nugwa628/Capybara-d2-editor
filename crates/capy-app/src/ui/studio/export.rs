//! Export ▾ (ds1-studio `exportImage.ts`): the map, or only the selected
//! cells, as a PNG drawn by `capy_render` (tiles and objects at 1:1, the
//! layers that are shown), and PrintScreen: the map view on screen copied to
//! the clipboard.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::input::ButtonInput;
use bevy::prelude::{Commands, KeyCode, Res, ResMut};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use capy_render::{ObjectSprites, RenderOptions, RgbaImage};

use crate::config::AppMode;
use crate::state::AppState;

#[derive(Default)]
pub struct ExportState {
    job: Option<Mutex<Receiver<Result<String, String>>>>,
    /// PrintScreen was asked for: capture the window next frame.
    pub copy_view: bool,
    /// The captured window, waiting to be cropped to the map.
    shot: Arc<Mutex<Option<image::RgbaImage>>>,
    waiting: bool,
    /// Size of the last view copied (scripts check it).
    pub last_copy: Option<(u32, u32)>,
    /// A script waits for an export or a capture.
    pub script_wait: bool,
    /// The last export finished (scripts wait for it).
    pub last_export: Option<Result<String, String>>,
}

impl ExportState {
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.copy_view || self.waiting
    }
}

/// A copy of `ds1` with every cell position outside `keep` emptied (on all
/// layers) and the objects outside it dropped.
fn masked(ds1: &d2_formats::ds1::Ds1, keep: &capy_edit::Selection) -> d2_formats::ds1::Ds1 {
    use d2_formats::ds1::Cell;
    let mut out = ds1.clone();
    let cells: std::collections::HashSet<(u32, u32)> =
        keep.iter().map(|(_, x, y)| (x, y)).collect();
    for y in 0..ds1.height {
        for x in 0..ds1.width {
            if cells.contains(&(x, y)) {
                continue;
            }
            for f in &mut out.floors {
                f.set(x, y, Cell::EMPTY);
            }
            for w in &mut out.walls {
                w.tiles.set(x, y, Cell::EMPTY);
                w.orientations.set(x, y, 0);
            }
            out.shadow.set(x, y, Cell::EMPTY);
            if let Some(t) = &mut out.tag {
                t.set(x, y, 0);
            }
        }
    }
    out.objects.retain(|o| {
        let (x, y) = (o.x.div_euclid(5), o.y.div_euclid(5));
        x >= 0 && y >= 0 && cells.contains(&(x as u32, y as u32))
    });
    out
}

/// The smallest rectangle holding every pixel with some alpha.
fn crop_to_content(img: &RgbaImage) -> image::RgbaImage {
    let (w, h) = (img.width, img.height);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if img.pixels[((y * w + x) * 4 + 3) as usize] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    let full = image::RgbaImage::from_raw(w, h, img.pixels.clone())
        .unwrap_or_else(|| image::RgbaImage::new(1, 1));
    if x1 <= x0 || y1 <= y0 {
        return full;
    }
    image::imageops::crop_imm(&full, x0, y0, x1 - x0, y1 - y0).to_image()
}

/// Export ▾ > Map as PNG / Selection as PNG. `path` = `None` asks where.
pub fn export_png(state: &mut AppState, selection: bool, path: Option<PathBuf>) {
    let Some(index) = state.active else { return };
    let doc = &state.docs[index];
    let (Some(tiles), Some(info), Some(session)) =
        (doc.tiles.clone(), doc.info.as_ref(), state.session.as_ref())
    else {
        state.notify("The map is not drawn yet");
        return;
    };
    if selection && doc.selection.is_empty() {
        state.notify("Select some cells to export first");
        return;
    }
    let stem = doc.path.file_name().trim_end_matches(".ds1").to_owned();
    let path = match path {
        Some(p) => p,
        None => {
            let name = if selection {
                format!("{stem}-selection.png")
            } else {
                format!("{stem}.png")
            };
            match rfd::FileDialog::new()
                .add_filter("PNG image", &["png"])
                .set_file_name(name)
                .save_file()
            {
                Some(p) => p,
                None => return,
            }
        }
    };
    let palette = match crate::render::job::load_palette(&session.overlay, info.palette_act) {
        Ok(p) => p,
        Err(e) => {
            state.error(format!("Export: {e}"));
            return;
        }
    };
    let ds1 = if selection {
        Arc::new(masked(&doc.ds1, &doc.selection))
    } else {
        doc.ds1.clone()
    };
    let opts = RenderOptions {
        zoom_div: 1,
        gamma: state.config.gamma,
        visibility: doc.visibility(),
        ..RenderOptions::default()
    };
    let overlay = session.overlay.clone();
    let catalog = state.object_catalog();
    let (tx, rx) = channel();
    state.studio.export.job = Some(Mutex::new(rx));
    state.studio.export.last_export = None;
    state.info(format!("Exporting {} to {}...", doc.path, path.display()));
    std::thread::spawn(move || {
        let sprites = catalog.as_ref().map(|c| ObjectSprites::new(&overlay, c));
        let result =
            capy_render::render_with_objects(&ds1, &tiles, sprites.as_ref(), &palette, &opts)
                .map_err(|e| e.to_string())
                .and_then(|out| {
                    let img = if selection {
                        crop_to_content(&out.image)
                    } else {
                        image::RgbaImage::from_raw(
                            out.image.width,
                            out.image.height,
                            out.image.pixels,
                        )
                        .ok_or_else(|| "bad image".to_owned())?
                    };
                    img.save(&path).map_err(|e| e.to_string())?;
                    Ok(format!(
                        "Exported {} x {} to {}",
                        img.width(),
                        img.height(),
                        path.display()
                    ))
                });
        let _ = tx.send(result);
    });
}

/// Once per frame: finished exports and captured views.
pub fn poll(state: &mut AppState) {
    let finished = state
        .studio
        .export
        .job
        .as_ref()
        .and_then(|rx| rx.lock().ok().and_then(|rx| rx.try_recv().ok()));
    if let Some(result) = finished {
        state.studio.export.job = None;
        match &result {
            Ok(text) => state.notify(text.clone()),
            Err(e) => state.error(format!("Export failed: {e}")),
        }
        state.studio.export.last_export = Some(result);
    }
    if !state.studio.export.waiting {
        return;
    }
    let shot = state
        .studio
        .export
        .shot
        .lock()
        .ok()
        .and_then(|mut s| s.take());
    let Some(full) = shot else { return };
    state.studio.export.waiting = false;
    let Some(rect) = state.map_rect else { return };
    let ppp = state.pixels_per_point;
    let x0 = (rect.left() * ppp).max(0.0) as u32;
    let y0 = (rect.top() * ppp).max(0.0) as u32;
    let w = ((rect.width() * ppp) as u32).min(full.width().saturating_sub(x0));
    let h = ((rect.height() * ppp) as u32).min(full.height().saturating_sub(y0));
    if w == 0 || h == 0 {
        return;
    }
    let view = image::imageops::crop_imm(&full, x0, y0, w, h).to_image();
    state.studio.export.last_copy = Some((w, h));
    if !state.edit.use_os_clipboard {
        state.notify(format!(
            "Map view captured ({w} x {h}); clipboard left alone in scripts"
        ));
        return;
    }
    let data = arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: std::borrow::Cow::Owned(view.into_raw()),
    };
    match arboard::Clipboard::new().and_then(|mut c| c.set_image(data)) {
        Ok(()) => state.notify(format!("Copied the map view ({w} x {h}) to the clipboard")),
        Err(e) => state.error(format!("Copy the view: {e}")),
    }
}

/// PrintScreen (Windows sends only its release) and the capture itself.
pub fn capture_view(
    mut commands: Commands,
    mut state: ResMut<AppState>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_released(KeyCode::PrintScreen)
        && state.config.mode == AppMode::Map
        && super::super::command_enabled(&state, "view.snapshot")
    {
        super::super::commands::run(&mut state, "view.snapshot");
        state.shell.ran.push("view.snapshot");
    }
    if !std::mem::take(&mut state.studio.export.copy_view) {
        return;
    }
    let slot = state.studio.export.shot.clone();
    state.studio.export.waiting = true;
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: bevy::prelude::On<ScreenshotCaptured>| {
            if let Ok(img) = captured.image.clone().try_into_dynamic()
                && let Ok(mut s) = slot.lock()
            {
                *s = Some(img.to_rgba8());
            }
        },
    );
}
