//! Sync local issue changes back to GitHub.

use std::path::Path;

use v_utils::prelude::*;

use super::{
	fetch::fetch_and_store_issue,
	issue::Issue,
	meta::{IssueMetaEntry, get_issue_meta, load_issue_meta_from_path},
	util::{Extension, expand_blocker_shorthand},
};
use crate::{error::ParseContext, github::BoxedGitHubClient};

/// Sync changes from a local issue file back to GitHub using stored metadata.
/// Returns whether the issue state changed (for file renaming).
pub async fn sync_local_issue_to_github(gh: &BoxedGitHubClient, owner: &str, repo: &str, meta: &IssueMetaEntry, issue: &Issue) -> Result<bool> {
	let mut updates = 0;
	let mut creates = 0;
	let mut deletes = 0;
	let mut state_changed = false;

	// Step 0: Check if issue state (open/closed) changed
	// Note: GitHub only supports binary open/closed, so NotPlanned and Duplicate both map to closed
	let current_closed = issue.meta.close_state.is_closed();
	let original_closed = meta.original_close_state.is_closed();
	if current_closed != original_closed {
		let new_state = issue.meta.close_state.to_github_state();
		println!("Updating issue state to {new_state}...");
		gh.update_issue_state(owner, repo, meta.issue_number, new_state).await?;
		state_changed = true;
		updates += 1;
	}

	// Step 1: Check issue body (includes blockers section)
	let issue_body = issue.body();
	let original_body = meta.original_issue_body.as_deref().unwrap_or("");
	tracing::debug!("[sync] issue.blockers.len() = {}", issue.blockers.len());
	tracing::debug!("[sync] issue_body:\n{issue_body}");
	tracing::debug!("[sync] original_body:\n{original_body}");
	if issue_body != original_body {
		println!("Updating issue body...");
		gh.update_issue_body(owner, repo, meta.issue_number, &issue_body).await?;
		updates += 1;
	}

	// Step 2: Sync comments (skip first which is body)
	let target_ids: std::collections::HashSet<u64> = issue.comments.iter().skip(1).filter_map(|c| c.id).collect();
	let original_ids: std::collections::HashSet<u64> = meta.original_comments.iter().map(|c| c.id).collect();

	// Delete comments that were removed
	for orig in &meta.original_comments {
		if !target_ids.contains(&orig.id) {
			println!("Deleting comment {}...", orig.id);
			gh.delete_comment(owner, repo, orig.id).await?;
			deletes += 1;
		}
	}

	// Update existing comments and create new ones
	for comment in issue.comments.iter().skip(1) {
		if !comment.owned {
			continue; // Skip immutable comments
		}
		match comment.id {
			Some(id) if original_ids.contains(&id) => {
				let original = meta.original_comments.iter().find(|c| c.id == id).and_then(|c| c.body.as_deref()).unwrap_or("");
				if comment.body != original {
					println!("Updating comment {id}...");
					gh.update_comment(owner, repo, id, &comment.body).await?;
					updates += 1;
				}
			}
			Some(id) => {
				eprintln!("Warning: comment {id} not found in original, skipping");
			}
			None =>
				if !comment.body.is_empty() {
					println!("Creating new comment...");
					gh.create_comment(owner, repo, meta.issue_number, &comment.body).await?;
					creates += 1;
				},
		}
	}

	let total = updates + creates + deletes;
	if total > 0 {
		let mut parts = Vec::new();
		if updates > 0 {
			parts.push(format!("{updates} updated"));
		}
		if creates > 0 {
			parts.push(format!("{creates} created"));
		}
		if deletes > 0 {
			parts.push(format!("{deletes} deleted"));
		}
		println!("Synced to GitHub: {}", parts.join(", "));
	}

	Ok(state_changed)
}

/// Execute issue actions (create sub-issues, etc.) and update the Issue struct with new URLs.
pub async fn execute_issue_actions(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue: &mut Issue, actions: Vec<Vec<crate::github::IssueAction>>) -> Result<usize> {
	use crate::github::IssueAction;

	let mut executed = 0;

	for level_actions in actions {
		for action in level_actions {
			match action {
				IssueAction::CreateSubIssue {
					child_path,
					title,
					closed,
					parent_issue_number,
				} => {
					// Create the issue on GitHub
					println!("Creating sub-issue: {title}");
					let created = gh.create_issue(owner, repo, &title, "").await?;

					// Add as sub-issue to parent
					gh.add_sub_issue(owner, repo, parent_issue_number, created.id).await?;

					// If created as closed, close it
					if closed {
						gh.update_issue_state(owner, repo, created.number, "closed").await?;
					}

					// Update the Issue struct with the new URL
					if let Some(child) = issue.get_child_mut(&child_path) {
						child.meta.url = Some(format!("https://github.com/{owner}/{repo}/issues/{}", created.number));
					}

					executed += 1;
				}
				IssueAction::UpdateSubIssueState { issue_number, closed } => {
					let new_state = if closed { "closed" } else { "open" };
					println!("Updating sub-issue #{issue_number} state to {new_state}...");
					gh.update_issue_state(owner, repo, issue_number, new_state).await?;
					executed += 1;
				}
			}
		}
	}

	Ok(executed)
}

/// Reconstruct an issue file with current sub-issue contents.
/// This reads local sub-issue files and embeds their content back into the parent.
/// IMPORTANT: Preserves local sub-issues that don't have URLs yet (not created on GitHub).
pub async fn reconstruct_issue_with_sub_issues(gh: &BoxedGitHubClient, issue_file_path: &Path, owner: &str, repo: &str) -> Result<()> {
	use super::{
		files::find_sub_issue_file,
		format::format_issue,
		meta::{IssueMetaEntry, save_issue_meta},
	};
	use crate::github::OriginalSubIssue;

	let meta = load_issue_meta_from_path(issue_file_path)?;
	let extension = match meta.extension.as_str() {
		"typ" => Extension::Typ,
		_ => Extension::Md,
	};

	// First, parse the current local file to find any sub-issues without URLs
	// These are new sub-issues that haven't been created on GitHub yet
	let local_content = std::fs::read_to_string(issue_file_path)?;
	let ctx = ParseContext::new(local_content.clone(), issue_file_path.display().to_string());
	let local_issue = Issue::parse(&local_content, &ctx)?;
	let local_only_children: Vec<_> = local_issue.children.iter().filter(|c| c.meta.url.is_none()).cloned().collect();

	// Fetch fresh data from GitHub
	let (current_user, issue, comments, sub_issues) = tokio::try_join!(
		gh.fetch_authenticated_user(),
		gh.fetch_issue(owner, repo, meta.issue_number),
		gh.fetch_comments(owner, repo, meta.issue_number),
		gh.fetch_sub_issues(owner, repo, meta.issue_number),
	)?;

	// Check for sub-issues that exist on GitHub but not locally
	for sub in &sub_issues {
		let local_path = find_sub_issue_file(owner, repo, meta.issue_number, &meta.title, sub.number);
		if local_path.is_none() {
			// Sub-issue not found locally - fetch and store it
			println!("Fetching sub-issue #{}: {}...", sub.number, sub.title);
			let parent_info = Some((meta.issue_number, meta.title.clone()));
			let sub_path = fetch_and_store_issue(gh, owner, repo, sub.number, &extension, false, parent_info).await?;
			println!("Stored sub-issue at: {:?}", sub_path);
		}
	}

	// Re-format the parent issue with updated sub-issue contents
	let content = format_issue(&issue, &comments, &sub_issues, owner, repo, &current_user, false, extension);

	// If there are local-only sub-issues, we need to append them to the formatted content
	let final_content = if local_only_children.is_empty() {
		content
	} else {
		// Parse the formatted content and add the local children back
		let formatted_ctx = ParseContext::new(content.clone(), issue_file_path.display().to_string());
		let mut formatted_issue = Issue::parse(&content, &formatted_ctx)?;
		formatted_issue.children.extend(local_only_children);
		formatted_issue.serialize()
	};

	// Write the updated content
	std::fs::write(issue_file_path, &final_content)?;

	// Update metadata with current sub-issue state
	let meta_entry = IssueMetaEntry {
		issue_number: meta.issue_number,
		title: issue.title.clone(),
		extension: meta.extension.clone(),
		original_issue_body: issue.body.clone(),
		original_comments: comments.iter().map(|c| c.into()).collect(),
		original_sub_issues: sub_issues.iter().map(OriginalSubIssue::from).collect(),
		parent_issue: meta.parent_issue,
		original_close_state: if issue.state == "closed" {
			super::issue::CloseState::Closed
		} else {
			super::issue::CloseState::Open
		},
	};
	save_issue_meta(owner, repo, meta_entry)?;

	Ok(())
}

/// Open a local issue file, let user edit, then sync changes back to GitHub.
pub async fn open_local_issue(gh: &BoxedGitHubClient, issue_file_path: &Path) -> Result<()> {
	use super::files::extract_owner_repo_from_path;

	// Extract owner and repo from path
	let (owner, repo) = extract_owner_repo_from_path(issue_file_path)?;

	// Load metadata
	let meta = load_issue_meta_from_path(issue_file_path)?;

	// Reconstruct the file with current sub-issue contents before opening
	// This ensures we show the latest state of all sub-issue files
	// Also fetches any missing sub-issues from GitHub
	if let Err(e) = reconstruct_issue_with_sub_issues(gh, issue_file_path, &owner, &repo).await {
		eprintln!("Warning: could not reconstruct sub-issues: {e}");
	}

	// Open in editor (blocks until editor closes)
	// If TODO_MOCK_PIPE env var is set, waits for pipe signal instead
	crate::utils::open_file(issue_file_path).await?;

	// Read edited content, expand !b shorthand, and parse into Issue struct
	let raw_content = std::fs::read_to_string(issue_file_path)?;
	let extension = match meta.extension.as_str() {
		"typ" => Extension::Typ,
		_ => Extension::Md,
	};
	let edited_content = expand_blocker_shorthand(&raw_content, &extension);

	// Write back if !b was expanded
	if edited_content != raw_content {
		std::fs::write(issue_file_path, &edited_content)?;
	}

	let ctx = ParseContext::new(edited_content.clone(), issue_file_path.display().to_string());
	let mut issue = Issue::parse(&edited_content, &ctx)?;

	// Handle duplicate close type: remove from local storage entirely
	if issue.meta.close_state.should_remove() {
		println!("Issue marked as duplicate, closing on GitHub and removing local file...");

		// Close on GitHub (if not already closed)
		if !meta.original_close_state.is_closed() {
			gh.update_issue_state(&owner, &repo, meta.issue_number, "closed").await?;
		}

		// Remove local file
		std::fs::remove_file(issue_file_path)?;

		// Remove sub-issues directory if it exists
		let sub_dir = issue_file_path.with_extension("");
		let sub_dir = if sub_dir.extension().is_some() {
			// Handle .md.bak case - strip .md too
			sub_dir.with_extension("")
		} else {
			sub_dir
		};
		if sub_dir.is_dir() {
			std::fs::remove_dir_all(&sub_dir)?;
		}

		// Remove from metadata
		let mut project_meta = super::meta::load_project_meta(&owner, &repo);
		project_meta.issues.remove(&meta.issue_number);
		super::meta::save_project_meta(&project_meta)?;

		println!("Duplicate issue removed from local storage.");
		return Ok(());
	}

	// Collect required GitHub actions (e.g., new sub-issues without URLs)
	let actions = issue.collect_actions(&meta.original_sub_issues);
	let has_actions = actions.iter().any(|level| !level.is_empty());

	// Execute actions and update Issue struct with new URLs
	let actions_executed = if has_actions {
		execute_issue_actions(gh, &owner, &repo, &mut issue, actions).await?
	} else {
		0
	};

	// Serialize the (potentially updated) Issue back to markdown
	let serialized = issue.serialize();

	// Write the normalized/updated content back
	if serialized != edited_content {
		std::fs::write(issue_file_path, &serialized)?;
	}

	// Sync body/comment/state changes to GitHub
	let state_changed = sync_local_issue_to_github(gh, &owner, &repo, &meta, &issue).await?;

	// If we executed actions or state changed, refresh from GitHub
	if actions_executed > 0 || state_changed {
		// Re-fetch and update local file and metadata to reflect the synced state
		println!("Refreshing local issue file from GitHub...");
		let extension = match meta.extension.as_str() {
			"typ" => Extension::Typ,
			_ => Extension::Md,
		};

		// Determine parent issue info if this is a sub-issue
		let parent_issue = meta
			.parent_issue
			.and_then(|parent_num| get_issue_meta(&owner, &repo, parent_num).map(|parent_meta| (parent_num, parent_meta.title)));

		// Store the old path before re-fetching
		let old_path = issue_file_path.to_path_buf();

		// Re-fetch creates file with potentially new title/state (affects .bak suffix)
		let new_path = fetch_and_store_issue(gh, &owner, &repo, meta.issue_number, &extension, false, parent_issue).await?;

		// If the path changed (title/state changed), delete the old file
		if old_path != new_path && old_path.exists() {
			if state_changed {
				println!("Issue state changed, renaming file...");
			} else {
				println!("Issue renamed, removing old file: {:?}", old_path);
			}
			std::fs::remove_file(&old_path)?;

			// For state changes, we might also need to rename the sub-issues directory
			// The directory name doesn't include .bak, so this only matters for title changes
			let old_sub_dir = old_path.with_extension("");
			if old_sub_dir.is_dir()
				&& let Err(e) = std::fs::remove_dir_all(&old_sub_dir)
			{
				eprintln!("Warning: could not remove old sub-issues directory: {e}");
			}
		}

		if actions_executed > 0 {
			println!("Synced {actions_executed} actions to GitHub.");
		}
	} else {
		println!("No changes made.");
	}

	Ok(())
}
