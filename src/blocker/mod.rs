//! Blocker management module.
//!
//! This module provides functionality for managing "blockers" - a stack-based task tracking system
//! where you work on one thing at a time. The core philosophy is:
//! - Forces prioritization (high leverage)
//! - Solving top 1 thing can often unlock many smaller ones for free
//!
//! # Module Structure
//!
//! - `standard`: Parsing primitives and formatting (HeaderLevel, LineType, BlockerSequence)
//! - `operations`: Core stack operations (add, pop, list, current)
//! - `io`: File/project resolution, clockify integration (halt/resume)
//! - `integration`: Bridges for working with issue files and the `open` module

mod integration;
mod io;
mod operations;
mod standard;

// Re-export the public API
use color_eyre::eyre::Result;
pub use integration::{IntegrationArgs, IntegrationCommand};
pub use io::{BlockerArgs, Command, HaltArgs, ResumeArgs};
pub use operations::BlockerSequence;
pub use standard::{HeaderLevel, LineType, classify_line, format_blocker_content, parse_parent_headers, strip_blocker_prefix};

use crate::config::LiveSettings;

/// Main entry point for blocker commands
pub async fn main(settings: &LiveSettings, args: BlockerArgs) -> Result<()> {
	io::main(settings, args).await
}

/// Entry point for integrated blocker commands (works with issue files)
pub async fn main_integrated(settings: &LiveSettings, args: IntegrationArgs) -> Result<()> {
	integration::main(settings, args).await
}
