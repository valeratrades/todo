//! GitHub issue editing functionality.
//!
//! This module provides functionality for fetching GitHub issues,
//! editing them locally, and syncing changes back to GitHub.

mod command;
mod conflict;
mod fetch;
pub(crate) mod files;
mod format;
mod github_sync;
mod meta;
mod sync;
mod touch;
pub(crate) mod util;

// Re-export the public API
pub use command::{OpenArgs, open_command};
// Re-export Issue from the library crate
pub use todo::Issue;
