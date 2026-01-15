//! Git operations for issue sync.
//!
//! The consensus state (last synced state) is stored in git.
//! This module provides functions to read committed content.

use std::{path::Path, process::Command};

use todo::Issue;
use v_utils::prelude::*;

use super::files::issues_dir;

/// Result of checking if a file is tracked in git.
pub enum GitTrackingStatus {
	/// File is tracked and we have its committed content
	Tracked(String),
	/// File is not tracked in git (new file)
	Untracked,
	/// Git is not initialized in the issues directory
	NoGit,
}

/// Check if a file is tracked in git and read its committed content.
pub fn read_committed_content(file_path: &Path) -> GitTrackingStatus {
	let data_dir = issues_dir();
	let Some(data_dir_str) = data_dir.to_str() else {
		return GitTrackingStatus::NoGit;
	};

	// Get the relative path from issues_dir
	let Some(rel_path) = file_path.strip_prefix(&data_dir).ok() else {
		return GitTrackingStatus::NoGit;
	};
	let Some(rel_path_str) = rel_path.to_str() else {
		return GitTrackingStatus::NoGit;
	};

	// Check if git is initialized
	let Ok(git_check) = Command::new("git").args(["-C", data_dir_str, "rev-parse", "--git-dir"]).output() else {
		return GitTrackingStatus::NoGit;
	};
	if !git_check.status.success() {
		return GitTrackingStatus::NoGit;
	}

	// Check if the file is tracked in git
	let Ok(ls_output) = Command::new("git").args(["-C", data_dir_str, "ls-files", rel_path_str]).output() else {
		return GitTrackingStatus::NoGit;
	};
	if !ls_output.status.success() || ls_output.stdout.is_empty() {
		// File is not tracked by git - this is valid for new files
		return GitTrackingStatus::Untracked;
	}

	// File IS tracked - we MUST be able to read it. If we can't, that's a bug.
	// Note: HEAD:./path is needed because git show HEAD:path expects repo-root-relative paths,
	// but we're running from a subdirectory (issues_dir). The ./ prefix makes it cwd-relative.
	let output = Command::new("git")
		.args(["-C", data_dir_str, "show", &format!("HEAD:./{rel_path_str}")])
		.output()
		.expect("git show command failed to execute");

	if !output.status.success() {
		panic!(
			"BUG: File is tracked in git but cannot read committed content.\n\
			 File: {}\n\
			 Git error: {}",
			file_path.display(),
			String::from_utf8_lossy(&output.stderr)
		);
	}

	let content = String::from_utf8(output.stdout).expect("git show returned invalid UTF-8");
	GitTrackingStatus::Tracked(content)
}

/// Load the consensus Issue from git (last committed state).
///
/// Returns:
/// - `Some(Issue)` if file is tracked and consensus loaded successfully
/// - `None` if file is not tracked (new file, no consensus yet)
///
/// Panics if file is tracked but consensus cannot be loaded (indicates a bug).
pub fn load_consensus_issue(file_path: &Path) -> Option<Issue> {
	match read_committed_content(file_path) {
		GitTrackingStatus::Tracked(content) => {
			let issue = Issue::parse(&content, file_path).unwrap_or_else(|e| {
				panic!(
					"BUG: Failed to parse committed consensus issue.\n\
					 File: {}\n\
					 Parse error: {e}",
					file_path.display()
				)
			});
			Some(issue)
		}
		GitTrackingStatus::Untracked | GitTrackingStatus::NoGit => None,
	}
}

/// Check if git is initialized in the issues directory.
pub fn is_git_initialized() -> bool {
	let data_dir = issues_dir();
	let Some(data_dir_str) = data_dir.to_str() else {
		return false;
	};

	Command::new("git")
		.args(["-C", data_dir_str, "rev-parse", "--git-dir"])
		.output()
		.map(|o| o.status.success())
		.unwrap_or(false)
}

/// Stage and commit changes for an issue file.
pub fn commit_issue_changes(_file_path: &Path, owner: &str, repo: &str, issue_number: u64, message: Option<&str>) -> Result<()> {
	let data_dir = issues_dir();
	let data_dir_str = data_dir.to_str().ok_or_else(|| eyre!("Invalid data directory path"))?;

	// Stage changes
	let _ = Command::new("git").args(["-C", data_dir_str, "add", "-A"]).status()?;

	// Check if there are staged changes
	let diff_output = Command::new("git").args(["-C", data_dir_str, "diff", "--cached", "--quiet"]).status()?;
	if diff_output.success() {
		// No staged changes
		return Ok(());
	}

	// Commit with provided or default message
	let default_msg = format!("sync: {owner}/{repo}#{issue_number}");
	let commit_msg = message.unwrap_or(&default_msg);
	Command::new("git").args(["-C", data_dir_str, "commit", "-m", commit_msg]).output()?;

	Ok(())
}
