//! Game-data editing for mods: registering maps with the game's level tables,
//! level types, automap, cube recipes, entering text, custom objects, a
//! compatibility checker and the crash-log reader. Pure edits over
//! [`d2_formats::txt::TxtDoc`] and [`d2_formats::tbl::TblDoc`]; no I/O, no UI.
//!
//! The public API is specified in `docs/api/capy-mod.md`.

pub mod add_map;
pub mod automap;
pub mod change_type;
pub mod compat;
pub mod crash_log;
pub mod cube;
pub mod custom_object;
mod doc_util;
pub mod entry_text;
pub mod level_tables;
pub mod lvl_error;
mod txt_util;
pub mod type_package;

// Tile tools (P11): DT1 collision flags, DT1 manager, tile settings, act-safe,
// reviews and the unused-asset report.
pub mod tiles_actsafe;
pub mod tiles_dt1;
pub mod tiles_flags;
pub mod tiles_review;
pub mod tiles_settings;
pub mod tiles_usage;

mod tiles_pixels;
