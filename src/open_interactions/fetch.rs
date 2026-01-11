//! Fetch issues from GitHub and store locally.

use std::path::PathBuf;

use todo::{CloseState, Extension};
use v_utils::prelude::*;

use super::{
	files::get_issue_file_path,
	format::format_issue,
	meta::{IssueMetaEntry, save_issue_meta},
};
use crate::github::{BoxedGitHubClient, GitHubIssue, OriginalComment, OriginalSubIssue};

/// Represents a node in the issue ancestry chain (from root to target)
#[derive(Debug)]
struct AncestryNode {
	number: u64,
	title: String,
}

/// Traverse up the parent chain to find the root issue and build the ancestry path.
/// Returns a list of ancestors from root to the immediate parent (not including the target issue).
async fn find_ancestry_chain(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<AncestryNode>> {
	let mut ancestry = Vec::new();
	let mut current_issue_number = issue_number;

	loop {
		match gh.fetch_parent_issue(owner, repo, current_issue_number).await? {
			Some(parent) => {
				// Found a parent, add to the front of the ancestry chain
				ancestry.push(AncestryNode {
					number: parent.number,
					title: parent.title.clone(),
				});
				current_issue_number = parent.number;
			}
			None => {
				// No more parents, we've reached the root
				break;
			}
		}
	}

	// Reverse so it goes from root to immediate parent
	ancestry.reverse();
	Ok(ancestry)
}

/// Fetch an issue and all its sub-issues recursively, writing them to XDG_DATA.
/// If the issue is a sub-issue, first traverses up to find the root and stores
/// the entire hierarchy from root down.
/// Returns the path to the requested issue file.
pub async fn fetch_and_store_issue(
	gh: &BoxedGitHubClient,
	owner: &str,
	repo: &str,
	issue_number: u64,
	extension: &Extension,
	render_closed: bool,
	parent_issue: Option<(u64, String)>,
) -> Result<PathBuf> {
	// If we already have parent info, this is a recursive call - just fetch normally
	if parent_issue.is_some() {
		return fetch_single_issue(gh, owner, repo, issue_number, extension, render_closed, parent_issue).await;
	}

	// First, check if this issue has any parents (is a sub-issue)
	let ancestry = find_ancestry_chain(gh, owner, repo, issue_number).await?;

	if ancestry.is_empty() {
		// This is a root issue, fetch normally (and recursively fetch sub-issues)
		return fetch_issue_tree(gh, owner, repo, issue_number, extension, render_closed, None).await;
	}

	// This is a sub-issue - fetch from the root down
	println!("Issue #{issue_number} is a sub-issue. Fetching from root issue #{}...", ancestry[0].number);

	// Fetch the entire tree starting from root
	let root_number = ancestry[0].number;
	fetch_issue_tree(gh, owner, repo, root_number, extension, render_closed, None).await?;

	// Now find and return the path to the originally requested issue
	// Build the parent info for this specific issue
	let parent_info = if ancestry.len() > 1 {
		// Has a non-root parent
		let immediate_parent = &ancestry[ancestry.len() - 1];
		Some((immediate_parent.number, immediate_parent.title.as_str()))
	} else {
		// Root is the immediate parent
		Some((ancestry[0].number, ancestry[0].title.as_str()))
	};

	// Get the issue info to determine file path
	let issue = gh.fetch_issue(owner, repo, issue_number).await?;
	let issue_closed = issue.state == "closed";
	let issue_file_path = get_issue_file_path(owner, repo, Some(issue_number), &issue.title, extension, issue_closed, parent_info);

	Ok(issue_file_path)
}

/// Fetch a single issue and store it (without checking ancestry).
async fn fetch_single_issue(
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

/// Fetch an issue tree starting from a root issue, recursively fetching all sub-issues.
/// Returns the path to the root issue file.
async fn fetch_issue_tree(
	gh: &BoxedGitHubClient,
	owner: &str,
	repo: &str,
	issue_number: u64,
	extension: &Extension,
	render_closed: bool,
	parent_issue: Option<(u64, String)>,
) -> Result<PathBuf> {
	// Fetch the issue data
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

	// Recursively fetch all sub-issues
	let this_issue_parent_info = (issue_number, issue.title.clone());
	for sub_issue in &sub_issues {
		// Only recurse into open issues, or all if render_closed is true
		if render_closed || sub_issue.state != "closed" {
			if let Err(e) = fetch_sub_issue_tree(gh, owner, repo, sub_issue, extension, render_closed, this_issue_parent_info.clone()).await {
				eprintln!("Warning: Failed to fetch sub-issue #{}: {}", sub_issue.number, e);
			}
		}
	}

	Ok(issue_file_path)
}

/// Fetch a sub-issue and its descendants recursively.
fn fetch_sub_issue_tree<'a>(
	gh: &'a BoxedGitHubClient,
	owner: &'a str,
	repo: &'a str,
	issue: &'a GitHubIssue,
	extension: &'a Extension,
	render_closed: bool,
	parent_issue: (u64, String),
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send + 'a>> {
	Box::pin(async move {
		// Fetch additional data for this sub-issue
		let (current_user, comments, sub_issues) = tokio::try_join!(
			gh.fetch_authenticated_user(),
			gh.fetch_comments(owner, repo, issue.number),
			gh.fetch_sub_issues(owner, repo, issue.number),
		)?;

		// Determine file path
		let parent_info = Some((parent_issue.0, parent_issue.1.as_str()));
		let issue_closed = issue.state == "closed";
		let issue_file_path = get_issue_file_path(owner, repo, Some(issue.number), &issue.title, extension, issue_closed, parent_info);

		// Create parent directories
		if let Some(parent) = issue_file_path.parent() {
			std::fs::create_dir_all(parent)?;
		}

		// Format content
		let content = format_issue(issue, &comments, &sub_issues, owner, repo, &current_user, render_closed, *extension);

		// Write issue file
		std::fs::write(&issue_file_path, &content)?;

		// Save metadata for syncing
		let meta_entry = IssueMetaEntry {
			issue_number: issue.number,
			title: issue.title.clone(),
			extension: extension.as_str().to_string(),
			original_issue_body: issue.body.clone(),
			original_comments: comments.iter().map(OriginalComment::from).collect(),
			original_sub_issues: sub_issues.iter().map(OriginalSubIssue::from).collect(),
			parent_issue: Some(parent_issue.0),
			original_close_state: if issue_closed { CloseState::Closed } else { CloseState::Open },
		};
		save_issue_meta(owner, repo, meta_entry)?;

		// Recursively fetch all sub-issues
		let this_issue_parent_info = (issue.number, issue.title.clone());
		for sub_issue in &sub_issues {
			if render_closed || sub_issue.state != "closed" {
				if let Err(e) = fetch_sub_issue_tree(gh, owner, repo, sub_issue, extension, render_closed, this_issue_parent_info.clone()).await {
					eprintln!("Warning: Failed to fetch sub-issue #{}: {}", sub_issue.number, e);
				}
			}
		}

		Ok(issue_file_path)
	})
}
