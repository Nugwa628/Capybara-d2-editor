//! Loading excel tables through an [`Overlay`], from `.bin` or `.txt`.

use d2_formats::excel::{self, ExcelTableKind};
use d2_formats::txt::TxtTable;

use crate::error::{AssetError, Result};
use crate::overlay::Overlay;
use crate::path::GamePath;
use crate::profile::GameVersion;

const EXCEL_DIR: &str = r"data\global\excel\";

#[derive(Clone, Copy)]
enum Form {
    Bin,
    Txt,
}

fn version_label(version: GameVersion) -> &'static str {
    match version {
        GameVersion::V113c => "1.13c",
        GameVersion::V114d => "1.14d",
        GameVersion::D2r => "d2r",
    }
}

fn excel_path(stem: &str, ext: &str) -> GamePath {
    GamePath::new(&format!("{EXCEL_DIR}{stem}.{ext}"))
}

/// The table in one form: `Ok(None)` when that form is not in the overlay
/// (or, for Levels, when `leveldefs.bin` is missing so the `.bin` is unusable).
fn load_form(
    overlay: &Overlay,
    kind: ExcelTableKind,
    version: GameVersion,
    form: Form,
    notes: &mut Vec<String>,
) -> Result<Option<TxtTable>> {
    let stem = kind.file_stem();
    match form {
        Form::Txt => match overlay.read_bytes(&excel_path(stem, "txt"))? {
            Some(bytes) => Ok(Some(TxtTable::parse(&bytes)?)),
            None => Ok(None),
        },
        Form::Bin => {
            // D2R has no `.bin` tables.
            let Some(excel_version) = version.excel() else {
                return Ok(None);
            };
            let Some(bytes) = overlay.read_bytes(&excel_path(stem, "bin"))? else {
                return Ok(None);
            };
            if kind == ExcelTableKind::Levels {
                let Some(defs) = overlay.read_bytes(&excel_path("leveldefs", "bin"))? else {
                    notes.push("levels.bin is present but leveldefs.bin is missing".to_owned());
                    return Ok(None);
                };
                Ok(Some(excel::decode_levels_bin(
                    excel_version,
                    &bytes,
                    &defs,
                )?))
            } else {
                Ok(Some(excel::decode_bin(kind, excel_version, &bytes)?))
            }
        }
    }
}

/// Loads an excel table from the overlay.
///
/// 1.13c reads `data\global\excel\<stem>.bin` first (decoded with
/// [`excel::decode_bin`]), then `<stem>.txt`. 1.14d reads `.txt` first, then
/// `.bin`. Levels from `.bin` also needs `leveldefs.bin`
/// ([`excel::decode_levels_bin`]). D2R reads `.txt` only.
///
/// A form that exists but cannot be decoded is an error; it is not skipped in
/// favour of the other form. Fails with [`AssetError::Profile`] when neither
/// form exists.
pub fn load_excel(
    overlay: &Overlay,
    kind: ExcelTableKind,
    version: GameVersion,
) -> Result<TxtTable> {
    let order = match version {
        GameVersion::V113c => [Form::Bin, Form::Txt],
        GameVersion::V114d | GameVersion::D2r => [Form::Txt, Form::Bin],
    };
    let mut notes = Vec::new();
    for form in order {
        if let Some(table) = load_form(overlay, kind, version, form, &mut notes)? {
            return Ok(table);
        }
    }
    let stem = kind.file_stem();
    let tried = if version.excel().is_some() {
        format!("{stem}.bin and {stem}.txt")
    } else {
        format!("{stem}.txt")
    };
    let mut detail = format!(r"excel table {stem} not found ({tried} under data\global\excel)");
    for note in notes {
        detail.push_str("; ");
        detail.push_str(&note);
    }
    Err(AssetError::Profile {
        profile: version_label(version).to_owned(),
        detail,
    })
}
