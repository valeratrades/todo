//! Sync local issue changes back to GitHub.

use std::path::Path;

use chrono::Utc;
use v_utils::prelude::*;

use super::{
	conflict::{ConflictState, save_conflict},
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

/// Check that required CLI tools are available
fn check_required_tools() -> Result<()> {
	use std::process::Command;

	// Check git
	let git_check = Command::new("git").arg("--version").output();
	if git_check.is_err() {
		return Err(eyre!("'git' is required but not found in PATH"));
	}

	// Check gh
	let gh_check = Command::new("gh").arg("--version").output();
	if gh_check.is_err() {
		return Err(eyre!("'gh' (GitHub CLI) is required but not found in PATH"));
	}

	Ok(())
}

/// Handle divergence: remote changed since we last fetched.
/// Creates a PR to merge remote changes into our local state.
async fn handle_divergence(_gh: &BoxedGitHubClient, issue_file_path: &Path, owner: &str, repo: &str, meta: &IssueMetaEntry, _local_issue: &Issue, remote_issue: &Issue) -> Result<()> {
	use std::process::Command;

	use super::files::issues_dir;

	check_required_tools()?;

	let data_dir = issues_dir();
	let data_dir_str = data_dir.to_str().ok_or_else(|| eyre!("Invalid data directory path"))?;

	// Verify data dir is a git repo
	if !data_dir.join(".git").exists() {
		return Err(eyre!(
			"Issue data directory is not a git repository.\n\
			 To enable merge conflict handling, initialize git in: {}\n\
			 Then configure a remote to push PRs to.",
			data_dir.display()
		));
	}

	// Check if remote exists
	let remote_output = Command::new("git").args(["-C", data_dir_str, "remote", "-v"]).output()?;

	if remote_output.stdout.is_empty() {
		return Err(eyre!(
			"No git remote configured for issue data directory.\n\
			 Add a remote to enable PR creation:\n\
			 cd {} && git remote add origin <your-repo-url>",
			data_dir.display()
		));
	}

	// Get current branch name
	let current_branch = Command::new("git").args(["-C", data_dir_str, "rev-parse", "--abbrev-ref", "HEAD"]).output()?;
	let current_branch = String::from_utf8_lossy(&current_branch.stdout).trim().to_string();

	// Auto-commit local changes
	let _ = Command::new("git").args(["-C", data_dir_str, "add", "-A"]).status()?;

	let commit_status = Command::new("git")
		.args(["-C", data_dir_str, "commit", "-m", &format!("Local changes for issue #{}", meta.issue_number)])
		.status()?;

	if commit_status.success() {
		println!("Committed local changes.");
	}

	// Push current branch to ensure local changes are on remote
	let push_status = Command::new("git").args(["-C", data_dir_str, "push", "-u", "origin", &current_branch]).status()?;

	if !push_status.success() {
		return Err(eyre!("Failed to push local changes to remote"));
	}

	// Create branch for remote state
	let branch_name = format!("remote-sync-{}-{}-{}", owner, repo, meta.issue_number);

	// Delete branch if it exists (from previous failed attempt)
	let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).output();

	// Create and checkout new branch
	let branch_status = Command::new("git").args(["-C", data_dir_str, "checkout", "-b", &branch_name]).status()?;

	if !branch_status.success() {
		return Err(eyre!("Failed to create branch for remote state"));
	}

	// Write remote state to the issue file
	let remote_content = remote_issue.serialize();
	std::fs::write(issue_file_path, &remote_content)?;

	// Commit remote state
	let _ = Command::new("git").args(["-C", data_dir_str, "add", "-A"]).status()?;

	let commit_status = Command::new("git")
		.args(["-C", data_dir_str, "commit", "-m", &format!("Remote state for issue #{} (to be merged)", meta.issue_number)])
		.status()?;

	if !commit_status.success() {
		// Restore original branch
		let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status();
		return Err(eyre!("Failed to commit remote state"));
	}

	// Push branch
	let push_status = Command::new("git").args(["-C", data_dir_str, "push", "-u", "origin", &branch_name, "--force"]).status()?;

	if !push_status.success() {
		// Restore original branch
		let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status();
		return Err(eyre!("Failed to push remote state branch"));
	}

	// Create PR using gh CLI
	let pr_title = format!("Sync remote changes for issue #{}", meta.issue_number);
	let pr_body = format!(
		"Remote issue #{} on {}/{} changed since last fetch.\n\n\
		 This PR contains the remote state that needs to be merged into your local changes.\n\n\
		 Review the changes and merge to resolve the conflict.",
		meta.issue_number, owner, repo
	);

	let pr_output = Command::new("gh")
		.args(["pr", "create", "--title", &pr_title, "--body", &pr_body, "--base", &current_branch, "--head", &branch_name])
		.current_dir(&data_dir)
		.output()?;

	// Restore original branch
	let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status();

	let pr_url = if pr_output.status.success() {
		String::from_utf8_lossy(&pr_output.stdout).trim().to_string()
	} else {
		let stderr = String::from_utf8_lossy(&pr_output.stderr);
		// PR might already exist
		if stderr.contains("already exists") {
			format!("(PR already exists for branch {})", branch_name)
		} else {
			return Err(eyre!("Failed to create PR: {}", stderr));
		}
	};

	// Save conflict state
	let conflict = ConflictState {
		issue_number: meta.issue_number,
		detected_at: Utc::now(),
		pr_url: pr_url.clone(),
		reason: "Remote issue changed since last fetch".to_string(),
	};

	save_conflict(owner, repo, &conflict)?;

	Err(eyre!(
		"Divergence detected: remote issue #{} changed since you last fetched.\n\
		 \n\
		 A PR has been created to merge the remote changes: {}\n\
		 \n\
		 To resolve:\n\
		 1. Review and merge the PR\n\
		 2. Pull the merged changes\n\
		 3. The conflict marker will be cleared automatically on next successful sync",
		meta.issue_number,
		pr_url
	))
}

/// Open a local issue file, let user edit, then sync changes back to GitHub.
pub async fn open_local_issue(gh: &BoxedGitHubClient, issue_file_path: &Path, offline: bool) -> Result<()> {
	use super::{conflict::check_conflict, files::extract_owner_repo_from_path};

	// Extract owner and repo from path
	let (owner, repo) = extract_owner_repo_from_path(issue_file_path)?;

	// Load metadata
	let meta = load_issue_meta_from_path(issue_file_path)?;

	// Check for unresolved conflicts before allowing edits
	check_conflict(&owner, &repo, meta.issue_number)?;

	// NOTE: We intentionally do NOT fetch from GitHub before opening.
	// This avoids the network roundtrip delay. Divergence is detected during sync.

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

	// Serialize the (potentially updated) Issue back to markdown
	let serialized = issue.serialize();

	// Write the normalized/updated content back
	if serialized != edited_content {
		std::fs::write(issue_file_path, &serialized)?;
	}

	// If offline mode, skip all network operations
	if offline {
		println!("Offline mode: changes saved locally only.");
		return Ok(());
	}

	// Fetch current GitHub state to check for divergence
	let (current_user, gh_issue, gh_comments, gh_sub_issues) = tokio::try_join!(
		gh.fetch_authenticated_user(),
		gh.fetch_issue(&owner, &repo, meta.issue_number),
		gh.fetch_comments(&owner, &repo, meta.issue_number),
		gh.fetch_sub_issues(&owner, &repo, meta.issue_number),
	)?;

	// Build Issue from current GitHub state
	let remote_issue = Issue::from_github(&gh_issue, &gh_comments, &gh_sub_issues, &owner, &repo, &current_user);

	// Build Issue from what we originally saw (stored in meta)
	let original_issue = Issue::from_meta(&meta, &owner, &repo);

	// Check if remote diverged from what we originally saw
	if remote_issue != original_issue {
		// Remote changed since we last fetched - divergence detected!
		return handle_divergence(gh, issue_file_path, &owner, &repo, &meta, &issue, &remote_issue).await;
	}

	// No divergence - proceed with sync

	// Collect required GitHub actions (e.g., new sub-issues without URLs)
	let actions = issue.collect_actions(&meta.original_sub_issues);
	let has_actions = actions.iter().any(|level| !level.is_empty());

	// Execute actions and update Issue struct with new URLs
	let actions_executed = if has_actions {
		execute_issue_actions(gh, &owner, &repo, &mut issue, actions).await?
	} else {
		0
	};

	// Re-serialize if actions added URLs
	if actions_executed > 0 {
		let serialized = issue.serialize();
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

	// Clear any conflict state on successful sync
	let _ = super::conflict::clear_conflict(&owner, &repo, meta.issue_number);

	Ok(())
}
