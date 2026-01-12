//! Fetch issues from GitHub and store locally.

use std::path::PathBuf;

use todo::{CloseState, Extension, FetchedIssue};
use v_utils::prelude::*;

use super::{
	files::get_issue_file_path,
	format::format_issue,
	meta::{IssueMetaEntry, save_issue_meta},
};
use crate::github::{BoxedGitHubClient, GitHubIssue, OriginalComment, OriginalSubIssue};

/// Traverse up the parent chain to find the root issue and build the ancestry path.
/// Returns a list of ancestors from root to the immediate parent (not including the target issue).
async fn find_ancestry_chain(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<FetchedIssue>> {
	let mut ancestry = Vec::new();
	let mut current_issue_number = issue_number;

	while let Some(parent) = gh.fetch_parent_issue(owner, repo, current_issue_number).await? {
		// Found a parent, add to the chain
		let fetched = FetchedIssue::from_parts(owner, repo, parent.number, &parent.title).ok_or_else(|| eyre!("Failed to construct FetchedIssue for parent #{}", parent.number))?;
		ancestry.push(fetched);
		current_issue_number = parent.number;
	}

	// Reverse so it goes from root to immediate parent
	ancestry.reverse();
	Ok(ancestry)
}

/// Fetch an issue and all its sub-issues recursively, writing them to XDG_DATA.
/// If the issue is a sub-issue, first traverses up to find the root and stores
/// the entire hierarchy from root down.
/// Returns the path to the requested issue file.
pub async fn fetch_and_store_issue(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue_number: u64, extension: &Extension, ancestors: Option<Vec<FetchedIssue>>) -> Result<PathBuf> {
	// If we already have ancestor info, this is a recursive call - use it directly
	if let Some(ancestors) = ancestors {
		return fetch_issue_with_ancestors(gh, owner, repo, issue_number, extension, ancestors).await;
	}

	// First, check if this issue has any parents (is a sub-issue)
	let ancestry = find_ancestry_chain(gh, owner, repo, issue_number).await?;

	if ancestry.is_empty() {
		// This is a root issue, fetch normally (and recursively fetch sub-issues)
		return fetch_issue_with_ancestors(gh, owner, repo, issue_number, extension, vec![]).await;
	}

	// This is a sub-issue - fetch from the root down
	println!("Issue #{issue_number} is a sub-issue. Fetching from root issue #{}...", ancestry[0].number());

	// Fetch the entire tree starting from root
	let root_number = ancestry[0].number();
	fetch_issue_with_ancestors(gh, owner, repo, root_number, extension, vec![]).await?;

	// Now find and return the path to the originally requested issue
	// Get the issue info to determine file path
	let issue = gh.fetch_issue(owner, repo, issue_number).await?;
	let issue_closed = issue.state == "closed";
	let issue_file_path = get_issue_file_path(owner, repo, Some(issue_number), &issue.title, extension, issue_closed, &ancestry);

	Ok(issue_file_path)
}

/// Fetch an issue with known ancestors, storing it and recursively fetching sub-issues.
async fn fetch_issue_with_ancestors(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue_number: u64, extension: &Extension, ancestors: Vec<FetchedIssue>) -> Result<PathBuf> {
	// Fetch the issue data
	let (current_user, issue, comments, sub_issues) = tokio::try_join!(
		gh.fetch_authenticated_user(),
		gh.fetch_issue(owner, repo, issue_number),
		gh.fetch_comments(owner, repo, issue_number),
		gh.fetch_sub_issues(owner, repo, issue_number),
	)?;

	// Determine file path
	let issue_closed = issue.state == "closed";
	let issue_file_path = get_issue_file_path(owner, repo, Some(issue_number), &issue.title, extension, issue_closed, &ancestors);

	// Create parent directories
	if let Some(parent) = issue_file_path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	// Format content
	let content = format_issue(&issue, &comments, &sub_issues, owner, repo, &current_user, *extension, &ancestors);

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
		parent_issue: ancestors.last().map(|p| p.number()),
		original_close_state: if issue_closed { CloseState::Closed } else { CloseState::Open },
	};
	save_issue_meta(owner, repo, meta_entry)?;

	// Build ancestors for children (current issue becomes part of ancestors)
	let mut child_ancestors = ancestors;
	let this_issue = FetchedIssue::from_parts(owner, repo, issue_number, &issue.title).ok_or_else(|| eyre!("Failed to construct FetchedIssue for #{}", issue_number))?;
	child_ancestors.push(this_issue);

	// Recursively fetch all sub-issues
	for sub_issue in &sub_issues {
		if let Err(e) = fetch_sub_issue_tree(gh, owner, repo, sub_issue, extension, child_ancestors.clone()).await {
			eprintln!("Warning: Failed to fetch sub-issue #{}: {e}", sub_issue.number);
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
	ancestors: Vec<FetchedIssue>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send + 'a>> {
	Box::pin(async move {
		// Fetch additional data for this sub-issue
		let (current_user, comments, sub_issues) = tokio::try_join!(
			gh.fetch_authenticated_user(),
			gh.fetch_comments(owner, repo, issue.number),
			gh.fetch_sub_issues(owner, repo, issue.number),
		)?;

		// Determine file path
		let issue_closed = issue.state == "closed";
		let issue_file_path = get_issue_file_path(owner, repo, Some(issue.number), &issue.title, extension, issue_closed, &ancestors);

		// Create parent directories
		if let Some(parent) = issue_file_path.parent() {
			std::fs::create_dir_all(parent)?;
		}

		// Format content
		let content = format_issue(issue, &comments, &sub_issues, owner, repo, &current_user, *extension, &ancestors);

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
			parent_issue: ancestors.last().map(|p| p.number()),
			original_close_state: if issue_closed { CloseState::Closed } else { CloseState::Open },
		};
		save_issue_meta(owner, repo, meta_entry)?;

		// Build ancestors for children (current issue becomes part of ancestors)
		let mut child_ancestors = ancestors;
		let this_issue = FetchedIssue::from_parts(owner, repo, issue.number, &issue.title).ok_or_else(|| eyre!("Failed to construct FetchedIssue for #{}", issue.number))?;
		child_ancestors.push(this_issue);

		// Recursively fetch all sub-issues
		for sub_issue in &sub_issues {
			if let Err(e) = fetch_sub_issue_tree(gh, owner, repo, sub_issue, extension, child_ancestors.clone()).await {
				eprintln!("Warning: Failed to fetch sub-issue #{}: {e}", sub_issue.number);
			}
		}

		Ok(issue_file_path)
	})
}
