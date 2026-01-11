//! Issue file format standard.
//!
//! This module contains the canonical representation of issue files,
//! including parsing, serialization, and all pure types.
//!
//! The issue format is designed for local-first issue tracking with
//! optional GitHub synchronization.

mod blocker;
mod error;
mod marker;
mod types;
mod util;

pub use blocker::{BlockerItem, BlockerSequence, HeaderLevel, Line, classify_line};
pub use error::{ParseContext, ParseError};
pub use marker::Marker;
pub use types::{CloseState, Comment, FetchedIssue, Issue, IssueLink, IssueMeta};
pub use util::{is_blockers_marker, normalize_issue_indentation};

// Re-export Extension and Header from parent
pub use crate::{Extension, Header};
