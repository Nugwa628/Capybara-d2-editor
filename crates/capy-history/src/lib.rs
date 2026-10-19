//! capy-history: everything the editor writes about maps.
//!
//! - [`atomic_write`]: crash-safe file replacement.
//! - [`SavesDir`]: loose saves mirroring game paths, plus `CHANGED.txt`.
//! - [`SnapshotStore`]: deduplicated, zlib-compressed snapshots with a timeline.
//! - [`Milestones`] (feature `milestones`): a git repository over the saves folder.
//!
//! Every write goes to a path derived from a root the caller passes in.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::panic))]

pub mod atomic;
pub mod error;
pub mod saves;
pub mod store;

#[cfg(feature = "milestones")]
pub mod milestone;

pub use atomic::atomic_write;
pub use error::{HistoryError, Result};
#[cfg(feature = "milestones")]
pub use milestone::{MilestoneInfo, Milestones};
pub use saves::SavesDir;
pub use store::{
    ContentHash, PruneReport, Retention, Snapshot, SnapshotId, SnapshotKind, SnapshotStore,
};
