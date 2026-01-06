//! Blocker management module.
//!
//! This module provides functionality for managing "blockers" - a stack-based task tracking system
//! where you work on one thing at a time. The core philosophy is:
//! - Forces prioritization (high leverage)
//! - Solving top 1 thing can often unlock many smaller ones for free
//!
//! # Module Structure
//!
//! - `standard`: Parsing primitives and formatting (HeaderLevel, LineType)
//! - `operations`: Core stack operations (BlockerSequence with add, pop, list, current)
//! - `clockify`: Time tracking integration (protocol + tracking state)
//! - `io`: File/project resolution for standalone blocker files
//! - `integration`: Helpers for working with issue files (used via --integrated flag)

pub mod clockify;
pub(super) mod integration;
mod io;
mod operations;
mod standard;

// Re-export the public API
use color_eyre::eyre::Result;
pub use io::BlockerArgs;
pub use standard::{LineType, classify_line};

use crate::config::LiveSettings;

/// Main entry point for blocker commands
pub async fn main(settings: &LiveSettings, args: BlockerArgs) -> Result<()> {
	io::main(settings, args).await
}
