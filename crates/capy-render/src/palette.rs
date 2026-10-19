//! Which act palette a map uses.
//!
//! The reference resolves it in two steps:
//!
//! 1. `read_lvltypes_txt` (`core/txtread.c:737-815`) takes the LvlTypes `Act`
//!    of the row, except that when every tileset of the row sits under one
//!    `ActN\` folder and that disagrees with the `Act` column it believes the
//!    artwork (`lvltypes_row_tileset_act`, `txtread.c:688-736`). In the
//!    shipped table exactly one row triggers this: id 35, "Act 5 - Lava",
//!    declares Act 5 but its tilesets are Act 4 art.
//! 2. `palette_resolve_index(txt_act, ds1_act)` (`core/palette.c:64-84`) uses
//!    that act when it is 1..=5, otherwise the DS1's own act, otherwise
//!    Act 1; the palette index is the act minus one.

use d2_formats::tables::LvlType;

/// Act (0-based, `0..=4`) of an `ActN\...` tile path, `None` for other paths.
fn path_act(path: &str) -> Option<u8> {
    let b = path.as_bytes();
    if b.len() < 5 || !b[..3].eq_ignore_ascii_case(b"act") {
        return None;
    }
    if !(b'1'..=b'5').contains(&b[3]) || (b[4] != b'\\' && b[4] != b'/') {
        return None;
    }
    Some(b[3] - b'1')
}

/// The act (0-based) every non-empty tileset of `lvl_type` agrees on, if they
/// all sit under one `ActN\` folder.
pub fn tileset_act(lvl_type: &LvlType) -> Option<u8> {
    let mut found: Option<u8> = None;
    for file in lvl_type.files.iter().flatten() {
        let file = file.trim();
        if file.is_empty() || file == "0" {
            continue;
        }
        let act = path_act(file)?;
        match found {
            None => found = Some(act),
            Some(prev) if prev != act => return None,
            Some(_) => {}
        }
    }
    found
}

/// Palette act (0-based) for a map: the LvlType's act with the tileset
/// override above, falling back to the DS1's act (`ds1_act`, 0-based) when the
/// map has no LvlType.
pub fn palette_act(lvl_type: Option<&LvlType>, ds1_act: u8) -> u8 {
    match lvl_type {
        Some(t) => tileset_act(t).unwrap_or(t.act).min(4),
        None => ds1_act.min(4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl_type(act: u8, files: &[&str]) -> LvlType {
        LvlType {
            row: 0,
            id: 0,
            name: String::new(),
            act,
            files: files
                .iter()
                .map(|f| (!f.is_empty() && *f != "0").then(|| (*f).to_owned()))
                .collect(),
            expansion: false,
        }
    }

    #[test]
    fn act_column_is_used_by_default() {
        let t = lvl_type(1, &["act2\\desert\\floor.dt1", "mixed\\x.dt1"]);
        assert_eq!(palette_act(Some(&t), 0), 1);
    }

    #[test]
    fn unanimous_tileset_folder_overrides_the_column() {
        // LvlTypes id 35: declares Act 5 (index 4), tiles are Act 4 art.
        let t = lvl_type(
            4,
            &[
                "act4\\expansion\\lava\\floor.dt1",
                "act4/expansion/lava/wall.dt1",
                "0",
            ],
        );
        assert_eq!(palette_act(Some(&t), 0), 3);
    }

    #[test]
    fn mixed_acts_defer_to_the_column() {
        let t = lvl_type(2, &["act1\\a.dt1", "act2\\b.dt1"]);
        assert_eq!(palette_act(Some(&t), 0), 2);
    }

    #[test]
    fn no_lvltype_uses_the_ds1_act_clamped() {
        assert_eq!(palette_act(None, 3), 3);
        assert_eq!(palette_act(None, 9), 4);
    }
}
