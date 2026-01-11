//! Sync local issue changes back to GitHub.
//!
//! ## Consensus-based sync model
//!
//! The sync logic treats the last synced state as the "consensus". When syncing:
//! - If only local changed since consensus → push local to remote
//! - If only remote changed since consensus → take remote as new local
//! - If both changed since consensus → conflict requiring manual resolution
//! - If neither changed → no action needed

use std::path::Path;

use jiff::Timestamp;
use v_utils::prelude::*;

use super::{
	conflict::{ConflictState, save_conflict},
	fetch::fetch_and_store_issue,
	issue::Issue,
	meta::{IssueMetaEntry, get_issue_meta, load_issue_meta_from_path},
	util::{Extension, expand_blocker_shorthand},
};
use crate::{error::ParseContext, github::BoxedGitHubClient};

//=============================================================================
// Sync implementation
//=============================================================================

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

/// Execute issue actions and update the Issue struct with new URLs.
/// Returns (executed_count, Option<created_issue_number>) - the issue number is set if a root issue was created.
pub async fn execute_issue_actions(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue: &mut Issue, actions: Vec<Vec<crate::github::IssueAction>>) -> Result<(usize, Option<u64>)> {
	use crate::github::IssueAction;

	let mut executed = 0;
	let mut created_root_number = None;

	for level_actions in actions {
		for action in level_actions {
			match action {
				IssueAction::CreateIssue { path, title, body, closed, parent } => {
					let is_root = path.is_empty();
					if is_root {
						println!("Creating issue: {title}");
					} else {
						println!("Creating sub-issue: {title}");
					}

					let created = gh.create_issue(owner, repo, &title, &body).await?;
					println!("Created issue #{}: {}", created.number, created.html_url);

					// Link to parent if this is a sub-issue
					if let Some(parent_num) = parent {
						gh.add_sub_issue(owner, repo, parent_num, created.id).await?;
					}

					// Close if needed
					if closed {
						gh.update_issue_state(owner, repo, created.number, "closed").await?;
					}

					// Update the Issue struct with the new URL
					let url = format!("https://github.com/{owner}/{repo}/issues/{}", created.number);
					if is_root {
						issue.meta.url = Some(url);
						created_root_number = Some(created.number);
					} else if let Some(child) = issue.get_child_mut(&path) {
						child.meta.url = Some(url);
					}

					executed += 1;
				}
				IssueAction::UpdateIssueState { issue_number, closed } => {
					let new_state = if closed { "closed" } else { "open" };
					println!("Updating issue #{issue_number} state to {new_state}...");
					gh.update_issue_state(owner, repo, issue_number, new_state).await?;
					executed += 1;
				}
			}
		}
	}

	Ok((executed, created_root_number))
}

/// Special commit message prefix that indicates a conflict-in-progress commit.
/// These commits are not counted as the "last synced truth" for consensus.
const CONFLICT_COMMIT_PREFIX: &str = "__conflicts:";

/// Handle divergence: both local and remote changed since last sync.
/// Creates a local `remote-state` branch with remote changes and initiates a git merge.
/// No PR is created - conflicts are resolved locally via standard git workflow.
async fn handle_divergence(_gh: &BoxedGitHubClient, issue_file_path: &Path, owner: &str, repo: &str, meta: &IssueMetaEntry, _local_issue: &Issue, remote_issue: &Issue) -> Result<()> {
	use std::process::Command;

	use super::files::issues_dir;

	let data_dir = issues_dir();
	let data_dir_str = data_dir.to_str().ok_or_else(|| eyre!("Invalid data directory path"))?;

	// Check if git is initialized
	let git_check = Command::new("git").args(["-C", data_dir_str, "rev-parse", "--git-dir"]).output()?;
	if !git_check.status.success() {
		return Err(eyre!(
			"Conflict detected: both local and remote have changes since last sync.\n\
			 \n\
			 To enable conflict resolution, initialize git in your issues directory:\n\
			   cd {} && git init\n\
			 \n\
			 Then re-run the command.",
			data_dir.display()
		));
	}

	// Get current branch name
	let branch_output = Command::new("git").args(["-C", data_dir_str, "rev-parse", "--abbrev-ref", "HEAD"]).output()?;
	let current_branch = String::from_utf8_lossy(&branch_output.stdout).trim().to_string();

	// Commit local changes with special prefix (so we know it's a conflict-in-progress)
	let _ = Command::new("git").args(["-C", data_dir_str, "add", "-A"]).status()?;

	let commit_msg = format!("{CONFLICT_COMMIT_PREFIX} local changes for issue #{}", meta.issue_number);
	let commit_output = Command::new("git").args(["-C", data_dir_str, "commit", "-m", &commit_msg]).output()?;

	if commit_output.status.success() {
		println!("Committed local changes.");
	}

	// Create branch for remote state
	let branch_name = "remote-state".to_string();

	// Delete branch if it exists (from previous attempt)
	let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).output();

	// Create new branch from current HEAD (before our conflict commit)
	// First, get the parent commit (before our conflict commit)
	let parent_output = Command::new("git").args(["-C", data_dir_str, "rev-parse", "HEAD~1"]).output();
	let base_commit = if let Ok(output) = parent_output
		&& output.status.success()
	{
		String::from_utf8_lossy(&output.stdout).trim().to_string()
	} else {
		// No parent commit (first commit), use HEAD
		"HEAD".to_string()
	};

	// Create remote-state branch from the base commit
	let branch_status = Command::new("git").args(["-C", data_dir_str, "branch", &branch_name, &base_commit]).status()?;

	if !branch_status.success() {
		return Err(eyre!("Failed to create branch for remote state"));
	}

	// Checkout the remote-state branch
	let checkout_status = Command::new("git").args(["-C", data_dir_str, "checkout", &branch_name]).status()?;

	if !checkout_status.success() {
		return Err(eyre!("Failed to checkout remote-state branch"));
	}

	// Write remote state to the issue file
	let remote_content = remote_issue.serialize();
	std::fs::write(issue_file_path, &remote_content)?;

	// Commit remote state
	let _ = Command::new("git").args(["-C", data_dir_str, "add", "-A"]).status()?;

	let remote_commit_msg = format!("Remote state for {owner}/{repo}#{}", meta.issue_number);
	let commit_status = Command::new("git").args(["-C", data_dir_str, "commit", "-m", &remote_commit_msg]).status()?;

	if !commit_status.success() {
		// Restore original branch
		let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status();
		let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).status();
		return Err(eyre!("Failed to commit remote state"));
	}

	// Switch back to original branch
	let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status()?;

	// Attempt to merge remote-state into current branch
	println!("Merging remote changes...");
	let merge_output = Command::new("git")
		.args(["-C", data_dir_str, "merge", &branch_name, "-m", &format!("Merge remote state for issue #{}", meta.issue_number)])
		.output()?;

	if merge_output.status.success() {
		// Merge succeeded without conflicts - clean up
		let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).status();
		println!("Remote changes merged successfully.");
		// Note: The conflict marker is not saved since merge succeeded
		return Ok(());
	}

	// Merge failed - likely due to conflicts
	let merge_stderr = String::from_utf8_lossy(&merge_output.stderr);
	let merge_stdout = String::from_utf8_lossy(&merge_output.stdout);

	// Check if it's actually a conflict (vs other error)
	if merge_stdout.contains("CONFLICT") || merge_stderr.contains("CONFLICT") || merge_stdout.contains("Automatic merge failed") {
		// Save conflict state
		let conflict = ConflictState {
			issue_number: meta.issue_number,
			detected_at: Timestamp::now(),
			pr_url: String::new(), // No PR in new workflow
			reason: "Both local and remote have changes since last sync".to_string(),
		};

		save_conflict(owner, repo, &conflict)?;

		return Err(eyre!(
			"Conflict detected: both local and remote have changes for issue #{}.\n\
			 \n\
			 Git merge has been initiated. Resolve the conflicts in:\n\
			   {}\n\
			 \n\
			 To resolve:\n\
			 1. Edit the file to resolve conflict markers (<<<<<<< ======= >>>>>>>)\n\
			 2. Run: git add {} && git commit\n\
			 3. Re-run this command to sync your changes\n\
			 \n\
			 To abort the merge: git merge --abort",
			meta.issue_number,
			issue_file_path.display(),
			issue_file_path.display()
		));
	}

	// Some other error during merge
	Err(eyre!("Failed to merge remote changes:\n{}\n{}", merge_stdout.trim(), merge_stderr.trim()))
}

/// Open a local issue file, let user edit, then sync changes back to GitHub.
pub async fn open_local_issue(gh: &BoxedGitHubClient, issue_file_path: &Path, offline: bool) -> Result<()> {
	use super::{conflict::check_conflict, files::extract_owner_repo_from_path, meta::is_virtual_project};

	// Extract owner and repo from path
	let (owner, repo) = extract_owner_repo_from_path(issue_file_path)?;

	// Auto-enable offline mode for virtual projects
	let offline = offline || is_virtual_project(&owner, &repo);

	// Load metadata
	let meta = load_issue_meta_from_path(issue_file_path)?;

	// Check for unresolved conflicts before allowing edits (skip for virtual projects)
	if !offline {
		check_conflict(&owner, &repo, meta.issue_number)?;
	}

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
		println!("Issue marked as duplicate, removing local file...");

		// Close on GitHub (if not already closed and not offline)
		if !offline && !meta.original_close_state.is_closed() {
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

	// Check if this is a pending issue (created via --touch, not yet on GitHub)
	let is_pending = issue.meta.url.is_none();

	// For pending issues, skip divergence check and create on GitHub first
	let (actions_executed, created_issue_number) = if is_pending {
		// Collect actions (will include CreateIssue for the root)
		let actions = issue.collect_actions(&meta.original_sub_issues);
		let has_actions = actions.iter().any(|level| !level.is_empty());

		if has_actions {
			execute_issue_actions(gh, &owner, &repo, &mut issue, actions).await?
		} else {
			(0, None)
		}
	} else {
		// Normal flow: use consensus-based sync
		let (current_user, gh_issue, gh_comments, gh_sub_issues) = tokio::try_join!(
			gh.fetch_authenticated_user(),
			gh.fetch_issue(&owner, &repo, meta.issue_number),
			gh.fetch_comments(&owner, &repo, meta.issue_number),
			gh.fetch_sub_issues(&owner, &repo, meta.issue_number),
		)?;

		// Build Issue from current GitHub state
		let remote_issue = Issue::from_github(&gh_issue, &gh_comments, &gh_sub_issues, &owner, &repo, &current_user);

		// Build Issue from what we originally saw (stored in meta) - this is the consensus
		let original_issue = Issue::from_meta(&meta, &owner, &repo);

		// Use consensus-based sync: compare local and remote against the original (consensus)
		let local_changed = issue != original_issue;
		let remote_changed = remote_issue != original_issue;

		match (local_changed, remote_changed) {
			(false, false) => {
				// Neither changed - nothing to do
				tracing::debug!("[sync] No changes detected");
			}
			(true, false) => {
				// Only local changed - push to remote (normal sync flow)
				tracing::debug!("[sync] Only local changed, pushing to remote");
			}
			(false, true) => {
				// Only remote changed - accept remote as new truth
				// Update local file to match remote state
				tracing::debug!("[sync] Only remote changed, accepting remote state");
				let remote_content = remote_issue.serialize();
				std::fs::write(issue_file_path, &remote_content)?;
				// Update metadata to reflect new original state
				let mut updated_meta = meta.clone();
				updated_meta.original_issue_body = remote_issue.body().into();
				updated_meta.original_close_state = remote_issue.meta.close_state.clone();
				// Note: comments and sub-issues would need similar updates for full implementation
				super::meta::save_issue_meta(&owner, &repo, updated_meta)?;
				println!("Remote changed, local file updated.");
				return Ok(());
			}
			(true, true) => {
				// Both changed - conflict!
				tracing::debug!("[sync] Both local and remote changed, conflict detected");
				return handle_divergence(gh, issue_file_path, &owner, &repo, &meta, &issue, &remote_issue).await;
			}
		}

		// Collect and execute actions (for local changes being pushed)
		let actions = issue.collect_actions(&meta.original_sub_issues);
		let has_actions = actions.iter().any(|level| !level.is_empty());

		if has_actions {
			execute_issue_actions(gh, &owner, &repo, &mut issue, actions).await?
		} else {
			(0, None)
		}
	};

	// Re-serialize if actions added URLs
	if actions_executed > 0 {
		let serialized = issue.serialize();
		std::fs::write(issue_file_path, &serialized)?;
	}

	// Determine the issue number to use for sync/refresh (may have just been created)
	let issue_number = created_issue_number.unwrap_or(meta.issue_number);

	// Sync body/comment/state changes to GitHub (skip for newly created issues - body already set)
	let state_changed = if created_issue_number.is_none() {
		sync_local_issue_to_github(gh, &owner, &repo, &meta, &issue).await?
	} else {
		false
	};

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
		let new_path = fetch_and_store_issue(gh, &owner, &repo, issue_number, &extension, false, parent_issue).await?;

		// If the path changed (title/state changed), delete the old file
		if old_path != new_path && old_path.exists() {
			if created_issue_number.is_some() {
				println!("Issue created on GitHub, updating local file...");
			} else if state_changed {
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
