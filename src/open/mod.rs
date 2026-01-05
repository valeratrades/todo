//! GitHub issue editing functionality.
//!
//! This module provides functionality for fetching GitHub issues,
//! editing them locally, and syncing changes back to GitHub.

mod command;
mod conflict;
mod fetch;
mod files;
mod format;
mod issue;
mod meta;
mod sync;
mod touch;
mod util;

// Re-export the public API
pub use command::{OpenArgs, open_command};
// Re-export for tests that need access to internal types
