//! The map editor's Sprite preview box: the animated composite of the one
//! selected object, through `capy_render::objects::ObjectSprites`. The
//! object catalog (game tables) is `AppState::object_catalog`; the frames
//! are built when the selection or direction changes.

use capy_render::objects::ObjectSprites;
use d2_assets::objects::{ObjectCatalog, ObjectInfo, SpriteBase};
use d2_assets::{GamePath, Overlay};
use d2_formats::palette::Palette;

use super::viewer::Tex;

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
    pub key: Option<PreviewKey>,
    pub built: Option<Built>,
    /// User direction (mapped per COF like the map renderer does).
    pub direction: u8,
    pub playing: bool,
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState {
            key: None,
            built: None,
            direction: 0,
            playing: true,
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
    cof_path(info).ok_or_else(|| format!("{} has no sprite", info.name))?;
    let sprites = ObjectSprites::new(overlay, catalog);
    // capy-render's delta rule: FrameDelta, else the AnimData speed, else 256.
    let (frames, delta) = sprites
        .frames_of_info(info, direction)
        .map_err(|r| r.to_string())?;
    let raw = |frame: &capy_render::ObjectFrame| {
        frame
            .composite(palette)
            .map(|(img, offset)| (img.width, img.height, img.pixels, offset))
    };
    let directions = frames.first().map_or(1, |f| f.directions.max(1));
    let raw_frames = frames.iter().map(|f| raw(f)).collect();
    // The COF's direction count, for the direction picker.
    Ok((directions, raw_frames, Some(delta)))
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
