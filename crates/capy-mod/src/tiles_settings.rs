//! Tile settings as a DT1 Tools `.ini` (ds1-studio `dt1Header.ts`,
//! `TileSettings.tsx`): read and write every field the `.ini` holds, and the
//! `.ini` text itself.
//!
//! The `.ini` lists the 5x5 flags as five rows `floor_flag1..5`; row `r` is
//! sub-tile row `y = r` of `Tile::subtile_flags` (`y * 5 + x`). Numbers use
//! the bases the tools write: hex for direction, orientation, indices, frame
//! and unknown, decimal for `roof_y`, `tile_sound` and `animated`.

use d2_formats::dt1::{Dt1, Tile};

/// Everything a DT1 Tools `.ini` holds for one tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileSettings {
    pub direction: i32,
    pub roof_height: i16,
    pub sound: u8,
    pub animated: bool,
    pub orientation: i32,
    pub main_index: i32,
    pub sub_index: i32,
    /// Rarity of a variant, or the frame number of an animated tile.
    pub frame: i32,
    pub unknown: u32,
    /// `y * 5 + x`.
    pub flags: [u8; 25],
}

/// A change to some fields (`None` = keep).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileSettingsPatch {
    pub direction: Option<i32>,
    pub roof_height: Option<i16>,
    pub sound: Option<u8>,
    pub animated: Option<bool>,
    pub orientation: Option<i32>,
    pub main_index: Option<i32>,
    pub sub_index: Option<i32>,
    pub frame: Option<i32>,
    pub unknown: Option<u32>,
    pub flags: Option<[u8; 25]>,
}

pub fn read_settings(tile: &Tile) -> TileSettings {
    TileSettings {
        direction: tile.direction,
        roof_height: tile.roof_height,
        sound: tile.sound,
        animated: tile.animated != 0,
        orientation: tile.orientation,
        main_index: tile.main_index,
        sub_index: tile.sub_index,
        frame: tile.rarity,
        unknown: u32::from_le_bytes(tile.preserved.unknown_36),
        flags: tile.subtile_flags,
    }
}

/// Applies the patch; returns whether anything changed. An `animated` tile
/// written as `true` keeps a non-zero stored byte it already had.
pub fn apply_settings(tile: &mut Tile, patch: &TileSettingsPatch) -> bool {
    let before = (read_settings(tile), tile.animated);
    if let Some(v) = patch.direction {
        tile.direction = v;
    }
    if let Some(v) = patch.roof_height {
        tile.roof_height = v;
    }
    if let Some(v) = patch.sound {
        tile.sound = v;
    }
    if let Some(v) = patch.animated
        && v != (tile.animated != 0)
    {
        tile.animated = u8::from(v);
    }
    if let Some(v) = patch.orientation {
        tile.orientation = v;
    }
    if let Some(v) = patch.main_index {
        tile.main_index = v;
    }
    if let Some(v) = patch.sub_index {
        tile.sub_index = v;
    }
    if let Some(v) = patch.frame {
        tile.rarity = v;
    }
    if let Some(v) = patch.unknown {
        tile.preserved.unknown_36 = v.to_le_bytes();
    }
    if let Some(v) = patch.flags {
        tile.subtile_flags = v;
    }
    (read_settings(tile), tile.animated) != before
}

/// Applies per-tile patches by tile index; returns the indices that changed.
/// An index past the end fails and changes nothing.
pub fn write_settings(
    dt1: &mut Dt1,
    changes: &[(usize, TileSettingsPatch)],
) -> Result<Vec<usize>, usize> {
    if let Some((i, _)) = changes.iter().find(|(i, _)| *i >= dt1.tiles.len()) {
        return Err(*i);
    }
    let mut out = Vec::new();
    for (i, p) in changes {
        if let Some(t) = dt1.tiles.get_mut(*i)
            && apply_settings(t, p)
        {
            out.push(*i);
        }
    }
    Ok(out)
}

/// DT1 Tools' image sheet for a tile kind.
pub fn pcx_sheet(orientation: i32) -> &'static str {
    match orientation {
        0 | 13 => "fs",
        15 => "rf",
        o if o >= 16 => "wd",
        _ => "ws",
    }
}

/// The DT1 as a DT1 Tools `.ini` (CRLF lines). `pcx_x`/`pcx_y` place each
/// tile in its sheet (160-pixel columns, 5 per row; rows as tall as the
/// tallest tile of the row); the sheets themselves are not written.
pub fn dt1_to_ini(dt1: &Dt1) -> String {
    let mut lines = vec![format!("nb_blocks   = {}", dt1.tiles.len()), String::new()];
    // sheet -> (column, y, row height)
    let mut sheets: Vec<(&'static str, usize, i64, i64)> = Vec::new();
    for (i, tile) in dt1.tiles.iter().enumerate() {
        let s = read_settings(tile);
        let name = pcx_sheet(s.orientation);
        let at = match sheets.iter().position(|e| e.0 == name) {
            Some(p) => p,
            None => {
                sheets.push((name, 0, 0, 0));
                sheets.len() - 1
            }
        };
        let h = i64::from(tile.height).abs();
        let h = if h == 0 { 80 } else { h };
        let (x, y) = (sheets[at].1 as i64 * 160, sheets[at].2);
        sheets[at].3 = sheets[at].3.max(h);
        sheets[at].1 += 1;
        if sheets[at].1 == 5 {
            sheets[at].1 = 0;
            sheets[at].2 += sheets[at].3;
            sheets[at].3 = 0;
        }
        lines.push(format!("block       = {i}"));
        lines.push(format!("pcx_file    = {name}"));
        lines.push(format!("pcx_x       = {x}"));
        lines.push(format!("pcx_y       = {y}"));
        lines.push(format!("direction   = {:08X}", s.direction as u32));
        lines.push(format!("roof_y      = {}", s.roof_height));
        lines.push(format!("tile_sound  = {}", s.sound));
        lines.push(format!("animated    = {}", u8::from(s.animated)));
        lines.push(format!("orientation = {:08X}", s.orientation as u32));
        lines.push(format!("main_index  = {:08X}", s.main_index as u32));
        lines.push(format!("sub_index   = {:08X}", s.sub_index as u32));
        lines.push(format!("frame       = {:08X}", s.frame as u32));
        lines.push(format!("unknown     = {:08X}", s.unknown));
        for r in 0..5 {
            let row: Vec<String> = s.flags[r * 5..r * 5 + 5]
                .iter()
                .map(|v| format!("{v:02X}"))
                .collect();
            lines.push(format!("floor_flag{} = {}", r + 1, row.join(" ")));
        }
        lines.push(String::new());
    }
    lines.join("\r\n")
}

/// One `block = n` section of a `.ini`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IniBlock {
    pub block: usize,
    pub settings: TileSettingsPatch,
}

fn hex_i32(v: &str) -> i32 {
    let t = v.trim().trim_start_matches("0x");
    i64::from_str_radix(t, 16).map_or(0, |n| n as i32)
}

fn dec(v: &str) -> i64 {
    v.trim().parse::<i64>().unwrap_or(0)
}

/// Reads a DT1 Tools `.ini`. Unknown keys and lines are ignored; a
/// `floor_flag` section needs all five rows to count.
pub fn parse_dt1_ini(text: &str) -> (Vec<IniBlock>, Option<usize>) {
    let mut blocks: Vec<IniBlock> = Vec::new();
    let mut count = None;
    let mut rows: [Option<[u8; 5]>; 5] = [None; 5];
    let flush = |blocks: &mut Vec<IniBlock>, rows: &mut [Option<[u8; 5]>; 5]| {
        if let Some(cur) = blocks.last_mut()
            && rows.iter().all(Option::is_some)
        {
            let mut f = [0u8; 25];
            for (r, row) in rows.iter().enumerate() {
                if let Some(row) = row {
                    f[r * 5..r * 5 + 5].copy_from_slice(row);
                }
            }
            cur.settings.flags = Some(f);
        }
        *rows = [None; 5];
    };
    for raw in text.lines() {
        let Some((key, value)) = raw.split_once('=') else {
            continue;
        };
        let (k, v) = (key.trim().to_ascii_lowercase(), value.trim());
        if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        if k == "nb_blocks" {
            count = usize::try_from(dec(v)).ok();
        } else if k == "block" {
            flush(&mut blocks, &mut rows);
            blocks.push(IniBlock {
                block: usize::try_from(dec(v)).unwrap_or(0),
                settings: TileSettingsPatch::default(),
            });
        } else if let Some(cur) = blocks.last_mut() {
            let s = &mut cur.settings;
            match k.as_str() {
                "direction" => s.direction = Some(hex_i32(v)),
                "roof_y" => s.roof_height = Some(dec(v) as i16),
                "tile_sound" => s.sound = Some(dec(v) as u8),
                "animated" => s.animated = Some(dec(v) != 0),
                "orientation" => s.orientation = Some(hex_i32(v)),
                "main_index" => s.main_index = Some(hex_i32(v)),
                "sub_index" => s.sub_index = Some(hex_i32(v)),
                "frame" => s.frame = Some(hex_i32(v)),
                "unknown" => s.unknown = Some(hex_i32(v) as u32),
                _ => {
                    if let Some(n) = k
                        .strip_prefix("floor_flag")
                        .and_then(|n| n.parse::<usize>().ok())
                        && (1..=5).contains(&n)
                    {
                        let mut row = [0u8; 5];
                        for (d, part) in row.iter_mut().zip(v.split_whitespace()) {
                            *d = (hex_i32(part) & 0xFF) as u8;
                        }
                        rows[n - 1] = Some(row);
                    }
                }
            }
        }
    }
    flush(&mut blocks, &mut rows);
    (blocks, count)
}

/// Applies an imported `.ini` to the DT1 (blocks matched by index). Returns
/// the tile indices that changed; blocks naming a tile the DT1 lacks are
/// returned separately and skipped.
pub fn apply_ini(dt1: &mut Dt1, blocks: &[IniBlock]) -> (Vec<usize>, Vec<usize>) {
    let mut changed = Vec::new();
    let mut missing = Vec::new();
    for b in blocks {
        match dt1.tiles.get_mut(b.block) {
            Some(t) => {
                if apply_settings(t, &b.settings) {
                    changed.push(b.block);
                }
            }
            None => missing.push(b.block),
        }
    }
    (changed, missing)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn ini_roundtrip_single_tile() {
        let mut tile = Tile::default();
        tile.orientation = 3;
        tile.main_index = 7;
        tile.sub_index = 0x1F;
        tile.direction = -2;
        tile.roof_height = -5;
        tile.sound = 4;
        tile.rarity = 2;
        tile.height = -160;
        tile.preserved.unknown_36 = [0xff, 0, 0xff, 0];
        for (i, f) in tile.subtile_flags.iter_mut().enumerate() {
            *f = i as u8 * 3;
        }
        let dt1 = Dt1 {
            version: (7, 6),
            tiles: vec![tile.clone()],
        };
        let text = dt1_to_ini(&dt1);
        assert!(text.contains("floor_flag1 = 00 03 06 09 0C"));
        let (blocks, count) = parse_dt1_ini(&text);
        assert_eq!(count, Some(1));
        assert_eq!(blocks.len(), 1);
        let mut other = Dt1 {
            version: (7, 6),
            tiles: vec![Tile::default()],
        };
        let (changed, missing) = apply_ini(&mut other, &blocks);
        assert_eq!(changed, vec![0]);
        assert!(missing.is_empty());
        assert_eq!(read_settings(&other.tiles[0]), read_settings(&tile));
    }

    #[test]
    fn patch_reports_no_change() {
        let mut t = Tile::default();
        assert!(!apply_settings(&mut t, &TileSettingsPatch::default()));
        assert!(apply_settings(
            &mut t,
            &TileSettingsPatch {
                sound: Some(9),
                ..Default::default()
            }
        ));
    }
}
