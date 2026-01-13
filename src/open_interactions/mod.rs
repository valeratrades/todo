//! GitHub issue editing functionality.
//!
//! This module provides functionality for fetching GitHub issues,
//! editing them locally, and syncing changes back to GitHub.

mod command;
mod conflict;
mod fetch;
pub(crate) mod files;
mod format;
mod git;
mod github_sync;
mod meta;
mod sync;
mod touch;
pub(crate) mod util;

// Re-export the public API
pub use command::{OpenArgs, open_command};
// Re-export sync types for blocker integration
pub use sync::{Modifier, ModifyResult, modify_and_sync_issue};
// Re-export Issue from the library crate
pub use todo::Issue;
