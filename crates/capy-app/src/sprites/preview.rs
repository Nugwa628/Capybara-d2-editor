//! The map editor's Sprite preview box: the animated composite of the one
//! selected object, through `capy_render::objects::ObjectSprites`. The
//! object catalog (game tables) is built once per profile on a worker
//! thread; the frames are built when the selection or direction changes.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use capy_render::objects::{ObjectOptions, ObjectSprites};
use d2_assets::objects::{ObjectCatalog, ObjectInfo, SpriteBase};
use d2_assets::{GamePath, Overlay};
use d2_formats::cof::Cof;
use d2_formats::palette::Palette;

use super::viewer::Tex;

pub enum CatalogSlot {
    None,
    Building {
        epoch: u64,
        rx: Mutex<Receiver<Result<ObjectCatalog, String>>>,
    },
    Ready {
        epoch: u64,
        catalog: Arc<ObjectCatalog>,
    },
    Failed {
        epoch: u64,
        error: String,
    },
}

/// What the preview shows, to know when to rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewKey {
    pub epoch: u64,
    pub doc: u64,
    pub index: usize,
    pub kind: i32,
    pub id: i32,
    pub act: u8,
    pub direction: u8,
    pub palette: u64,
}

/// One built preview.
pub struct Built {
    pub info: ObjectInfo,
    pub cof_path: Option<GamePath>,
    pub directions: u8,
    pub frames: Vec<Option<Tex>>,
    /// Frames advanced per 25 Hz tick, times 256.
    pub delta: u32,
    pub error: Option<String>,
}

pub struct PreviewState {
    pub catalog: CatalogSlot,
    pub key: Option<PreviewKey>,
    pub built: Option<Built>,
    /// User direction (mapped per COF like the map renderer does).
    pub direction: u8,
    pub playing: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState {
            catalog: CatalogSlot::None,
            key: None,
            built: None,
            direction: 0,
            playing: true,
        }
    }
}

impl PreviewState {
    /// The catalog for `epoch`, starting its build when needed.
    pub fn catalog(
        &mut self,
        epoch: u64,
        overlay: &Arc<Overlay>,
        version: d2_assets::GameVersion,
    ) -> Result<Option<Arc<ObjectCatalog>>, String> {
        let current = match &self.catalog {
            CatalogSlot::Building { epoch: e, .. }
            | CatalogSlot::Ready { epoch: e, .. }
            | CatalogSlot::Failed { epoch: e, .. } => Some(*e),
            CatalogSlot::None => None,
        };
        if current != Some(epoch) {
            let (tx, rx) = channel();
            let overlay = overlay.clone();
            let spawned = std::thread::Builder::new()
                .name("object-catalog".to_owned())
                .spawn(move || {
                    let r = ObjectCatalog::build(&overlay, version)
                        .map(|(c, _)| c)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(r);
                });
            self.catalog = match spawned {
                Ok(_) => CatalogSlot::Building {
                    epoch,
                    rx: Mutex::new(rx),
                },
                Err(e) => CatalogSlot::Failed {
                    epoch,
                    error: e.to_string(),
                },
            };
        }
        if let CatalogSlot::Building { rx, .. } = &self.catalog {
            let got = rx.lock().ok().and_then(|r| r.try_recv().ok());
            match got {
                Some(Ok(catalog)) => {
                    self.catalog = CatalogSlot::Ready {
                        epoch,
                        catalog: Arc::new(catalog),
                    }
                }
                Some(Err(error)) => self.catalog = CatalogSlot::Failed { epoch, error },
                None => {}
            }
        }
        match &self.catalog {
            CatalogSlot::Ready { catalog, .. } => Ok(Some(catalog.clone())),
            CatalogSlot::Failed { error, .. } => Err(error.clone()),
            _ => Ok(None),
        }
    }
}

pub fn base_folder(base: SpriteBase) -> &'static str {
    match base {
        SpriteBase::Objects => "data\\global\\objects",
        SpriteBase::Monsters => "data\\global\\monsters",
    }
}

fn clean(s: &str, default: &str) -> String {
    let t = s
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_ascii_lowercase();
    if t.is_empty() { default.to_owned() } else { t }
}

/// The COF an object draws with (`{base}\{token}\cof\{token}{mode}{class}.cof`).
pub fn cof_path(info: &ObjectInfo) -> Option<GamePath> {
    let base = info.sprite_base?;
    let token = clean(info.token.as_deref()?, "");
    if token.is_empty() {
        return None;
    }
    let mode = clean(&info.mode, "nu");
    let class = clean(&info.weapon_class, "hth");
    Some(GamePath::new(&format!(
        "{}\\{token}\\cof\\{token}{mode}{class}.cof",
        base_folder(base)
    )))
}

/// RGBA frames of one object at a user direction: `(info, cof, frames)`
/// where each frame is `(width, height, rgba, offset)` or `None` when empty.
pub type RawFrame = Option<(u32, u32, Vec<u8>, (i32, i32))>;

pub fn build_frames(
    overlay: &Overlay,
    catalog: &ObjectCatalog,
    info: &ObjectInfo,
    direction: u8,
    palette: &Palette,
) -> Result<(u8, Vec<RawFrame>, Option<u32>), String> {
    let path = cof_path(info).ok_or_else(|| format!("{} has no sprite", info.name))?;
    // Only the direction count for the picker; frames come from the renderer.
    let bytes = overlay
        .read_bytes(&path)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{path} not found"))?;
    let cof = Cof::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    if cof.directions == 0 {
        return Err(format!("{path} has no directions"));
    }
    let sprites = ObjectSprites::new(overlay, catalog);
    let opts = ObjectOptions::default();
    let first = sprites
        .frame_of_info(info, &opts, Some(0), direction)
        .map_err(|r| format!("{r:?}"))?;
    let count = first.frames_per_direction.max(1);
    let mut frames = Vec::with_capacity(usize::from(count));
    let raw = |frame: &capy_render::ObjectFrame| {
        frame
            .composite(palette)
            .map(|(img, offset)| (img.width, img.height, img.pixels, offset))
    };
    frames.push(raw(&first));
    for f in 1..count {
        let frame = sprites
            .frame_of_info(info, &opts, Some(f), direction)
            .map_err(|r| format!("{r:?}"))?;
        frames.push(raw(&frame));
    }
    // Objects animate at their FrameDelta; monsters at the AnimData speed.
    let delta = info.frame_delta.or_else(|| {
        let anim = overlay
            .read_bytes(&GamePath::new("data\\global\\animdata.d2"))
            .ok()
            .flatten()
            .and_then(|b| d2_formats::animdata::AnimData::parse(&b).ok())?;
        let key = path.file_name().to_owned();
        anim.get(&key).map(|r| r.speed)
    });
    Ok((cof.directions, frames, delta))
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn cof_paths_follow_token_mode_class() {
        let mut info = ObjectInfo {
            class: d2_assets::objects::ObjectClass::Object,
            ds1_kind: 2,
            ds1_id: 1,
            row: None,
            name: "x".into(),
            sprite_base: Some(SpriteBase::Objects),
            token: Some("TO".into()),
            mode: "ON".into(),
            weapon_class: String::new(),
            order_flag: 0,
            parts: Vec::new(),
            skins: Vec::new(),
            direction: None,
            size: (0, 0),
            offset: (0, 0),
            draw_effect: None,
            frame_count: None,
            frame_delta: None,
            source: "obj.txt",
            colormap: None,
        };
        assert_eq!(
            cof_path(&info).map(|p| p.as_str().to_owned()),
            Some("data\\global\\objects\\to\\cof\\toonhth.cof".to_owned())
        );
        info.token = None;
        assert!(cof_path(&info).is_none());
    }
}
