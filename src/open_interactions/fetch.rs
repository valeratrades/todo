//! Fetch issues from GitHub and store locally.

use std::path::PathBuf;

use todo::{CloseState, Extension};
use v_utils::prelude::*;

use super::{
	files::get_issue_file_path,
	format::format_issue,
	meta::{IssueMetaEntry, save_issue_meta},
};
use crate::github::{BoxedGitHubClient, OriginalComment, OriginalSubIssue};

/// Fetch an issue and all its sub-issues recursively, writing them to XDG_DATA.
/// Returns the path to the main issue file.
pub async fn fetch_and_store_issue(
	gh: &BoxedGitHubClient,
	owner: &str,
	repo: &str,
	issue_number: u64,
	extension: &Extension,
	render_closed: bool,
	parent_issue: Option<(u64, String)>,
) -> Result<PathBuf> {
	// Fetch issue data in parallel
	let (current_user, issue, comments, sub_issues) = tokio::try_join!(
		gh.fetch_authenticated_user(),
		gh.fetch_issue(owner, repo, issue_number),
		gh.fetch_comments(owner, repo, issue_number),
		gh.fetch_sub_issues(owner, repo, issue_number),
	)?;

	// Determine file path
	let parent_info = parent_issue.as_ref().map(|(num, title)| (*num, title.as_str()));
	let issue_closed = issue.state == "closed";
	let issue_file_path = get_issue_file_path(owner, repo, Some(issue_number), &issue.title, extension, issue_closed, parent_info);

	// Create parent directories
	if let Some(parent) = issue_file_path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	// Format content
	let content = format_issue(&issue, &comments, &sub_issues, owner, repo, &current_user, render_closed, *extension);

	// Write issue file
	std::fs::write(&issue_file_path, &content)?;

	// Save metadata for syncing
	let meta_entry = IssueMetaEntry {
		issue_number,
		title: issue.title.clone(),
		extension: extension.as_str().to_string(),
		original_issue_body: issue.body.clone(),
		original_comments: comments.iter().map(OriginalComment::from).collect(),
		original_sub_issues: sub_issues.iter().map(OriginalSubIssue::from).collect(),
		parent_issue: parent_issue.as_ref().map(|(num, _)| *num),
		original_close_state: if issue_closed { CloseState::Closed } else { CloseState::Open },
	};
	save_issue_meta(owner, repo, meta_entry)?;

	Ok(issue_file_path)
}
