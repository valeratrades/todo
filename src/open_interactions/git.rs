//! Git operations for issue sync.
//!
//! The consensus state (last synced state) is stored in git.
//! This module provides functions to read committed content.

use std::{path::Path, process::Command};

use todo::{Issue, ParseContext};
use v_utils::prelude::*;

use super::files::issues_dir;

/// Read file content from the last commit (HEAD).
/// Returns None if the file doesn't exist in git or git is not initialized.
pub fn read_committed_content(file_path: &Path) -> Option<String> {
	let data_dir = issues_dir();
	let data_dir_str = data_dir.to_str()?;

	// Get the relative path from issues_dir
	let rel_path = file_path.strip_prefix(&data_dir).ok()?;
	let rel_path_str = rel_path.to_str()?;

	// Check if git is initialized
	let git_check = Command::new("git").args(["-C", data_dir_str, "rev-parse", "--git-dir"]).output().ok()?;
	if !git_check.status.success() {
		return None;
	}

	// Check if the file is tracked in git
	let ls_output = Command::new("git").args(["-C", data_dir_str, "ls-files", rel_path_str]).output().ok()?;
	if !ls_output.status.success() || ls_output.stdout.is_empty() {
		// File is not tracked by git
		return None;
	}

	// Read file content from HEAD
	// Note: HEAD:./path is needed because git show HEAD:path expects repo-root-relative paths,
	// but we're running from a subdirectory (issues_dir). The ./ prefix makes it cwd-relative.
	let output = Command::new("git").args(["-C", data_dir_str, "show", &format!("HEAD:./{rel_path_str}")]).output().ok()?;

	if output.status.success() { String::from_utf8(output.stdout).ok() } else { None }
}

/// Load the consensus Issue from git (last committed state).
/// Returns None if the file doesn't exist in git or can't be parsed.
pub fn load_consensus_issue(file_path: &Path) -> Option<Issue> {
	let content = read_committed_content(file_path)?;
	let ctx = ParseContext::new(content.clone(), file_path.display().to_string());
	Issue::parse(&content, &ctx).ok()
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
