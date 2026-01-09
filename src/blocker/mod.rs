//! Blocker management module.
//!
//! This module provides functionality for managing "blockers" - a stack-based task tracking system
//! where you work on one thing at a time. The core philosophy is:
//! - Forces prioritization (high leverage)
//! - Solving top 1 thing can often unlock many smaller ones for free
//!
//! # Architecture
//!
//! - `standard`: Pure parsing primitives (strings only)
//! - `operations`: Pure operations on BlockerSequence (no I/O)
//! - `source`: BlockerSource trait for data access abstraction
//! - `io`: File-based source implementation + CLI handling
//! - `integration`: Issue-based source implementation
//! - `clockify`: Time tracking integration

pub mod clockify;
pub(super) mod integration;
mod io;
mod operations;
mod source;
mod standard;

// Re-export the public API
use color_eyre::eyre::Result;
pub use io::BlockerArgs;
pub use operations::{BlockerSequence, DisplayFormat};
pub use standard::{Line, classify_line};

use crate::config::LiveSettings;

/// Main entry point for blocker commands
pub async fn main(settings: &LiveSettings, args: BlockerArgs) -> Result<()> {
	io::main(settings, args).await
}
