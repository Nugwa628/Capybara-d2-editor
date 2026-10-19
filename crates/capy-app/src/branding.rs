//! The capybara app icon (`assets/branding/capybara.ico`, embedded): the
//! window and taskbar icon at run time, and the capybara of the loading
//! screen. The exe's own icon resource comes from the same file (build.rs).

use bevy::prelude::*;

/// The icon file: a 256 px PNG entry plus 64/48/32/24/16 px 32-bit BMP entries.
pub const ICO: &[u8] = include_bytes!("../../../assets/branding/capybara.ico");

/// An RGBA image (row-major, 4 bytes per pixel, not premultiplied).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// One directory entry of an `.ico` file: (edge in px, image bytes).
fn ico_entries(ico: &[u8]) -> Vec<(u32, &[u8])> {
    let u16_at =
        |o: usize| -> Option<u16> { Some(u16::from_le_bytes(ico.get(o..o + 2)?.try_into().ok()?)) };
    let u32_at =
        |o: usize| -> Option<u32> { Some(u32::from_le_bytes(ico.get(o..o + 4)?.try_into().ok()?)) };
    let mut out = Vec::new();
    if u16_at(0) != Some(0) || u16_at(2) != Some(1) {
        return out;
    }
    let count = usize::from(u16_at(4).unwrap_or(0));
    for i in 0..count {
        let at = 6 + 16 * i;
        let (Some(&w), Some(size), Some(offset)) = (ico.get(at), u32_at(at + 8), u32_at(at + 12))
        else {
            break;
        };
        let edge = if w == 0 { 256 } else { u32::from(w) };
        let (start, len) = (offset as usize, size as usize);
        if let Some(bytes) = start.checked_add(len).and_then(|end| ico.get(start..end)) {
            out.push((edge, bytes));
        }
    }
    out
}

/// Decode one entry: PNG, or a 32-bit BMP (BITMAPINFOHEADER, BGRA, bottom-up).
fn decode_entry(bytes: &[u8]) -> Option<RgbaImage> {
    if bytes.starts_with(b"\x89PNG") {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        return Some(RgbaImage {
            width: img.width() as usize,
            height: img.height() as usize,
            data: img.into_raw(),
        });
    }
    let i32_at = |o: usize| -> Option<i32> {
        Some(i32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?))
    };
    let header = usize::try_from(i32_at(0)?).ok()?;
    let width = usize::try_from(i32_at(4)?).ok()?;
    let height = usize::try_from(i32_at(8)?).ok()? / 2; // colour + AND mask
    let bits = u16::from_le_bytes(bytes.get(14..16)?.try_into().ok()?);
    if bits != 32 || i32_at(16)? != 0 || width == 0 || height == 0 || width > 1024 {
        return None;
    }
    let pixels = bytes.get(header..header.checked_add(width * height * 4)?)?;
    let mut data = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = &pixels[(height - 1 - y) * width * 4..][..width * 4];
        let dst = &mut data[y * width * 4..][..width * 4];
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
        }
    }
    Some(RgbaImage {
        width,
        height,
        data,
    })
}

/// The icon at `edge` px (exactly that entry), if the file has it.
pub fn icon(edge: u32) -> Option<RgbaImage> {
    ico_entries(ICO)
        .into_iter()
        .find(|(e, _)| *e == edge)
        .and_then(|(_, bytes)| decode_entry(bytes))
}

/// The 256 px capybara (what the old editor's splash used).
pub fn capybara() -> Option<RgbaImage> {
    icon(256)
}

/// The title-bar (32 px) and taskbar/Alt+Tab (64 px) icons of each new
/// window. winit windows must be touched on the main thread.
pub fn set_window_icons(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut created: MessageReader<bevy::window::WindowCreated>,
) {
    let windows: Vec<Entity> = created.read().map(|c| c.window).collect();
    if windows.is_empty() {
        return;
    }
    let make = |edge: u32| {
        icon(edge).and_then(|i| {
            winit::window::Icon::from_rgba(i.data, i.width as u32, i.height as u32).ok()
        })
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|winit_windows| {
        for &entity in &windows {
            let Some(window) = winit_windows.get_window(entity) else {
                continue;
            };
            window.set_window_icon(make(32));
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowExtWindows;
                window.set_taskbar_icon(make(64));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_size_decodes() {
        for edge in [256, 64, 48, 32, 24, 16] {
            let i = icon(edge).expect("the icon has this size");
            assert_eq!((i.width, i.height), (edge as usize, edge as usize));
            assert_eq!(i.data.len(), i.width * i.height * 4);
            // a real picture: some opaque and some transparent pixels
            assert!(i.data.chunks(4).any(|p| p[3] == 255), "{edge}");
            assert!(i.data.chunks(4).any(|p| p[3] == 0), "{edge}");
        }
        assert!(icon(128).is_none());
    }

    #[test]
    fn broken_files_give_nothing() {
        assert!(ico_entries(b"").is_empty());
        assert!(ico_entries(&[0, 0, 1, 0, 5, 0]).is_empty());
        assert!(decode_entry(&[40, 0, 0, 0]).is_none());
    }
}
