//! Data source abstraction for blocker storage.
//!
//! This module defines the `BlockerSource` trait which abstracts over
//! where blockers are stored (standalone files vs issue files).

use std::path::PathBuf;

use color_eyre::eyre::Result;

use super::operations::BlockerSequence;

/// Trait for blocker data sources.
/// Implementations handle reading/writing blocker content from different backends.
pub trait BlockerSource {
	/// Load the blocker sequence
	fn load(&self) -> Result<BlockerSequence>;

	/// Save the blocker sequence
	fn save(&self, blockers: &BlockerSequence) -> Result<()>;

	/// Get a display name for this source (for user messages)
	fn display_name(&self) -> String;

	/// Get the path for building ownership hierarchy (project name extraction)
	fn path_for_hierarchy(&self) -> Option<PathBuf>;
}
