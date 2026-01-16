//! Issue file format standard.
//!
//! This module contains the canonical representation of issue files,
//! including parsing, serialization, and all pure types.
//!
//! The issue format is designed for local-first issue tracking with
//! optional Github synchronization.

mod blocker;
pub use blocker::{BlockerItem, BlockerSequence, DisplayFormat, HeaderLevel, Line, classify_line, join_with_blockers, split_blockers};

mod contents;
pub use contents::Content;

mod error;
pub use error::ParseError;
mod marker;
pub use marker::Marker;

mod types;
pub use types::{CloseState, Comment, CommentIdentity, FetchedIssue, Issue, IssueIdentity, IssueLink, IssueMeta};

mod util;
pub use util::{is_blockers_marker, normalize_issue_indentation};

// Re-export Extension and Header from parent
pub use crate::{Extension, Header};
