//! Blocker management module.
//!
//! This module provides functionality for managing "blockers" - a stack-based task tracking system
//! where you work on one thing at a time. The core philosophy is:
//! - Forces prioritization (high leverage)
//! - Solving top 1 thing can often unlock many smaller ones for free
//!
//! # Architecture
//!
//! - `standard`: Extended parsing primitives (typst conversion, formatting)
//! - `operations`: Extended operations on BlockerSequence (pop, move, etc.)
//! - `source`: BlockerSource trait for data access abstraction
//! - `io`: File-based source implementation + CLI handling
//! - `integration`: Issue-based source implementation
//! - `clockify`: Time tracking integration
//!
//! Core types (HeaderLevel, Line, BlockerSequence, classify_line) are defined in the
//! library crate (todo::blocker_types) and re-exported here for convenience.

pub mod clockify;
pub(super) mod integration;
mod io;
mod operations;
mod source;
mod standard;

// Re-export core types from library
// Re-export the CLI API
use color_eyre::eyre::Result;
pub use io::BlockerArgs;
// Re-export extended operations
pub use operations::BlockerSequenceExt;
pub use todo::BlockerSequence;

use crate::config::LiveSettings;

/// Main entry point for blocker commands
pub async fn main(settings: &LiveSettings, args: BlockerArgs) -> Result<()> {
	io::main(settings, args).await
}
