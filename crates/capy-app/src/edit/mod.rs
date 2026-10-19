//! Editing: the tools, operations, clipboard, saving and scripting that sit
//! on top of `capy-edit` and `capy-history`. Everything here except the
//! egui drawing (in [`crate::ui`]) is plain data and logic, tested without
//! a window.

pub mod actions;
pub mod clip;
pub mod ops;
pub mod paste;
pub mod prompt;
pub mod save;
pub mod script;
pub mod state;
pub mod tools;
