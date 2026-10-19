//! Game data access for the Capybara Map Editor.
//!
//! Everything here is read-only with respect to game folders: archives are
//! opened with plain `File::open` and read with positional reads, nothing is
//! ever written, renamed or locked.
//!
//! * [`MpqArchive`] reads Blizzard MPQ archives (own implementation).
//! * [`AssetSource`] abstracts one layer of game data (MPQ, loose folder, CASC).
//! * [`Overlay`] stacks sources by priority.
//! * [`Profile`] describes a game setup as a TOML-serialisable layer list.
//! * [`KnownNames`] collects file names for archives without a listfile.

pub mod error;
pub mod mpq;
pub mod names;
pub mod overlay;
pub mod path;
pub mod profile;
pub mod source;

pub use error::{AssetError, Result};
pub use mpq::{MpqArchive, MpqCoverage, MpqHeader};
pub use names::KnownNames;
pub use overlay::{LayerId, Overlay};
pub use path::GamePath;
pub use profile::{GameVersion, LayerSpec, Profile};
pub use source::{AssetSource, CascSource, FolderSource, MpqSource, SourceFingerprint, SourceKind};
