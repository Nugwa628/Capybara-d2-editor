//! Background asset index (`index.db`) and user data (`user.db`).
//!
//! The contract, schemas and naming rules are in `docs/api/capy-index.md`.

mod build;
pub mod error;
pub mod index;
pub mod naming;
mod par;
pub mod query;
mod schema;
mod search;
#[cfg(test)]
mod tests;
mod tiles;
pub mod user;

pub use error::{IndexError, Result};
pub use index::{AssetRow, BuildPhase, BuildProgress, BuildReport, Hit, Index, IndexStatus};
pub use naming::{color_name, generated_tile_name, place_from_path, texture_class};
pub use query::{AssetKind, Filter, Query, QueryError, parse_query};
pub use user::{FavoriteGroup, SavedSearch, UserDb};
