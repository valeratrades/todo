//! Conflict state management for merge sync semantics.
//!
//! When remote GitHub state diverges from what we last saw, we create a PR
//! and record a conflict state. Until resolved, the issue cannot be edited.

// False positive: fields ARE used via thiserror's #[error] format string expansion
#![allow(unused_assignments)]

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use v_utils::prelude::*;

/// Get the conflicts directory
fn conflicts_dir() -> PathBuf {
	v_utils::xdg_state_dir!("todo/conflicts")
}

/// Get the conflict state file path for a specific issue
fn conflict_file_path(owner: &str, repo: &str, issue_number: u64) -> PathBuf {
	conflicts_dir().join(owner).join(repo).join(format!("{issue_number}.json"))
}

/// State of a conflict that needs resolution
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConflictState {
	/// Issue number on GitHub
	pub issue_number: u64,
	/// When the conflict was detected
	pub detected_at: DateTime<Utc>,
	/// URL of the PR created to merge remote changes
	pub pr_url: String,
	/// Brief description of what diverged
	pub reason: String,
}

/// Error returned when trying to open an issue with unresolved conflicts
#[derive(Debug, Diagnostic, Error)]
#[error("Issue #{issue_number} has unresolved merge conflicts (detected {detected_at})")]
#[diagnostic(
	code(todo::conflict::unresolved),
	help("Resolve the conflict by reviewing and merging the PR, then delete the conflict marker.\nPR: {pr_url}")
)]
pub struct ConflictError {
	pub issue_number: u64,
	pub pr_url: String,
	pub detected_at: DateTime<Utc>,
}

impl From<ConflictState> for ConflictError {
	fn from(state: ConflictState) -> Self {
		Self {
			issue_number: state.issue_number,
			pr_url: state.pr_url,
			detected_at: state.detected_at,
		}
	}
}

/// Load conflict state for an issue, if any exists
pub fn load_conflict(owner: &str, repo: &str, issue_number: u64) -> Option<ConflictState> {
	let path = conflict_file_path(owner, repo, issue_number);
	std::fs::read_to_string(&path).ok().and_then(|content| serde_json::from_str(&content).ok())
}

/// Save conflict state for an issue
pub fn save_conflict(owner: &str, repo: &str, state: &ConflictState) -> Result<()> {
	let path = conflict_file_path(owner, repo, state.issue_number);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let content = serde_json::to_string_pretty(state)?;
	std::fs::write(&path, content)?;
	Ok(())
}

/// Clear conflict state for an issue (after resolution)
pub fn clear_conflict(owner: &str, repo: &str, issue_number: u64) -> Result<()> {
	let path = conflict_file_path(owner, repo, issue_number);
	if path.exists() {
		std::fs::remove_file(&path)?;
	}
	Ok(())
}

/// Check if an issue has unresolved conflicts
/// Returns Err with miette diagnostic if conflict exists
pub fn check_conflict(owner: &str, repo: &str, issue_number: u64) -> Result<(), ConflictError> {
	if let Some(state) = load_conflict(owner, repo, issue_number) {
		Err(state.into())
	} else {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_conflict_state_roundtrip() {
		let state = ConflictState {
			issue_number: 42,
			detected_at: Utc::now(),
			pr_url: "https://github.com/owner/repo/pull/123".to_string(),
			reason: "Remote body changed".to_string(),
		};

		let json = serde_json::to_string(&state).unwrap();
		let parsed: ConflictState = serde_json::from_str(&json).unwrap();

		assert_eq!(parsed.issue_number, 42);
		assert_eq!(parsed.pr_url, "https://github.com/owner/repo/pull/123");
	}
}
