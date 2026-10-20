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
