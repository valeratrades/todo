//! Sync local issue changes back to GitHub.
//!
//! ## Consensus-based sync model
//!
//! The sync logic uses git's last committed state as the "consensus" (last synced state).
//! When syncing:
//! - If only local changed since consensus → push local to remote
//! - If only remote changed since consensus → take remote as new local
//! - If both changed since consensus → conflict requiring manual resolution
//! - If neither changed → no action needed
//!
//! This eliminates the need for storing consensus state in .meta.json files.

use std::path::Path;

use todo::{Extension, FetchedIssue, Issue, ParseContext};
use v_utils::prelude::*;

use super::{
	conflict::mark_conflict,
	fetch::fetch_and_store_issue,
	git::{commit_issue_changes, is_git_initialized, load_consensus_issue},
	github_sync::IssueGitHubExt,
	meta::load_issue_meta_from_path,
	util::expand_blocker_shorthand,
};
use crate::{blocker_interactions::BlockerSequenceExt, github::BoxedGitHubClient};

//=============================================================================
// Sync implementation
//=============================================================================

/// Sync changes from a local issue to GitHub.
/// Compares local state against remote state using git consensus.
/// Returns whether the issue state changed (for file renaming).
pub async fn sync_local_issue_to_github(gh: &BoxedGitHubClient, owner: &str, repo: &str, issue_number: u64, consensus: &Issue, local: &Issue) -> Result<bool> {
	let mut updates = 0;
	let mut creates = 0;
	let mut deletes = 0;
	let mut state_changed = false;

	// Step 0: Check if issue state (open/closed) changed
	let current_closed = local.meta.close_state.is_closed();
	let consensus_closed = consensus.meta.close_state.is_closed();
	if current_closed != consensus_closed {
		let new_state = local.meta.close_state.to_github_state();
		println!("Updating issue state to {new_state}...");
		gh.update_issue_state(owner, repo, issue_number, new_state).await?;
		state_changed = true;
		updates += 1;
	}

	// Step 1: Check issue body (includes blockers section)
	let issue_body = local.body();
	let consensus_body = consensus.body();
	tracing::debug!("[sync] local.blockers.len() = {}", local.blockers.len());
	tracing::debug!("[sync] issue_body:\n{issue_body}");
	tracing::debug!("[sync] consensus_body:\n{consensus_body}");
	if issue_body != consensus_body {
		println!("Updating issue body...");
		gh.update_issue_body(owner, repo, issue_number, &issue_body).await?;
		updates += 1;
	}

	// Step 2: Sync comments (skip first which is body)
	let target_ids: std::collections::HashSet<u64> = local.comments.iter().skip(1).filter_map(|c| c.id).collect();
	let consensus_ids: std::collections::HashSet<u64> = consensus.comments.iter().skip(1).filter_map(|c| c.id).collect();

	// Delete comments that were removed
	for comment in consensus.comments.iter().skip(1) {
		if let Some(id) = comment.id {
			if !target_ids.contains(&id) {
				println!("Deleting comment {id}...");
				gh.delete_comment(owner, repo, id).await?;
				deletes += 1;
			}
		}
	}

	// Update existing comments and create new ones
	for comment in local.comments.iter().skip(1) {
		if !comment.owned {
			continue; // Skip immutable comments
		}
		match comment.id {
			Some(id) if consensus_ids.contains(&id) => {
				let consensus_body = consensus.comments.iter().skip(1).find(|c| c.id == Some(id)).map(|c| c.body.as_str()).unwrap_or("");
				if comment.body != consensus_body {
					println!("Updating comment {id}...");
					gh.update_comment(owner, repo, id, &comment.body).await?;
					updates += 1;
				}
			}
			Some(id) => {
				eprintln!("Warning: comment {id} not found in consensus, skipping");
			}
			None =>
				if !comment.body.is_empty() {
					println!("Creating new comment...");
					gh.create_comment(owner, repo, issue_number, &comment.body).await?;
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

/// Handle divergence: both local and remote changed since last sync.
/// Creates a local `remote-state` branch with remote changes and initiates a git merge.
async fn handle_divergence(issue_file_path: &Path, owner: &str, repo: &str, issue_number: u64, remote_issue: &Issue) -> Result<()> {
	use std::process::Command;

	use super::files::issues_dir;

	let data_dir = issues_dir();
	let data_dir_str = data_dir.to_str().ok_or_else(|| eyre!("Invalid data directory path"))?;

	// Check if git is initialized
	if !is_git_initialized() {
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

	let commit_msg = format!("__conflicts: local changes for issue #{issue_number}");
	let commit_output = Command::new("git").args(["-C", data_dir_str, "commit", "-m", &commit_msg]).output()?;

	if commit_output.status.success() {
		println!("Committed local changes.");
	}

	// Create branch for remote state
	let branch_name = "remote-state".to_string();

	// Delete branch if it exists (from previous attempt)
	let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).output();

	// Create new branch from current HEAD (before our conflict commit)
	let parent_output = Command::new("git").args(["-C", data_dir_str, "rev-parse", "HEAD~1"]).output();
	let base_commit = if let Ok(output) = parent_output
		&& output.status.success()
	{
		String::from_utf8_lossy(&output.stdout).trim().to_string()
	} else {
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

	let remote_commit_msg = format!("Remote state for {owner}/{repo}#{issue_number}");
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
		.args(["-C", data_dir_str, "merge", &branch_name, "-m", &format!("Merge remote state for issue #{issue_number}")])
		.output()?;

	if merge_output.status.success() {
		// Merge succeeded without conflicts - clean up
		let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).status();
		println!("Remote changes merged successfully.");
		return Ok(());
	}

	// Merge failed - likely due to conflicts
	let merge_stderr = String::from_utf8_lossy(&merge_output.stderr);
	let merge_stdout = String::from_utf8_lossy(&merge_output.stdout);

	// Check if it's actually a conflict (vs other error)
	if merge_stdout.contains("CONFLICT") || merge_stderr.contains("CONFLICT") || merge_stdout.contains("Automatic merge failed") {
		// Mark this file as having conflicts - blocks all operations until resolved
		let _ = mark_conflict(issue_file_path);

		return Err(eyre!(
			"Conflict detected: both local and remote have changes for issue #{issue_number}.\n\
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
			issue_file_path.display(),
			issue_file_path.display()
		));
	}

	// Some other error during merge
	Err(eyre!("Failed to merge remote changes:\n{}\n{}", merge_stdout.trim(), merge_stderr.trim()))
}

/// Result of applying a modifier to an issue.
pub struct ModifyResult {
	/// The text to display after the operation (e.g., "Popped: task name")
	pub output: Option<String>,
}

/// A modifier that can be applied to an issue file.
pub enum Modifier {
	/// Open the file in an editor and wait for user to close it
	Editor,
	/// Apply a blocker operation programmatically
	BlockerPop,
}

impl Modifier {
	/// Apply this modifier to an issue. Returns output to display.
	async fn apply(&self, issue: &mut Issue, issue_file_path: &Path, extension: &Extension) -> Result<ModifyResult> {
		match self {
			Modifier::Editor => {
				// Serialize current state
				let content = issue.serialize();
				std::fs::write(issue_file_path, &content)?;

				// Open in editor (blocks until editor closes)
				crate::utils::open_file(issue_file_path).await?;

				// Read edited content, expand !b shorthand, and re-parse
				let raw_content = std::fs::read_to_string(issue_file_path)?;
				let edited_content = expand_blocker_shorthand(&raw_content, extension);

				// Write back if !b was expanded
				if edited_content != raw_content {
					std::fs::write(issue_file_path, &edited_content)?;
				}

				let ctx = ParseContext::new(edited_content.clone(), issue_file_path.display().to_string());
				*issue = Issue::parse(&edited_content, &ctx)?;

				Ok(ModifyResult { output: None })
			}
			Modifier::BlockerPop => {
				use crate::blocker_interactions::BlockerSequenceExt;

				let popped = issue.blockers.pop();
				let output = popped.map(|text| format!("Popped: {text}"));

				Ok(ModifyResult { output })
			}
		}
	}
}

/// Inner sync logic shared by open_local_issue and sync_issue_file.
async fn sync_issue_to_github_inner(gh: &BoxedGitHubClient, issue_file_path: &Path, owner: &str, repo: &str, issue_number: u64, issue: &mut Issue, extension: Extension) -> Result<()> {
	// Check if this is a pending issue (created via --touch, not yet on GitHub)
	let is_pending = issue.meta.url.is_none();

	// Load consensus from git (last committed state)
	let consensus = load_consensus_issue(issue_file_path);

	// For pending issues, skip divergence check and create on GitHub first
	let (actions_executed, created_issue_number) = if is_pending {
		// Collect actions (will include CreateIssue for the root)
		// For pending issues without consensus, use empty sub-issues list
		let actions = issue.collect_actions(&[]);
		let has_actions = actions.iter().any(|level| !level.is_empty());

		if has_actions {
			execute_issue_actions(gh, owner, repo, issue, actions).await?
		} else {
			(0, None)
		}
	} else {
		// Normal flow: use consensus-based sync
		let (current_user, gh_issue, gh_comments, gh_sub_issues) = tokio::try_join!(
			gh.fetch_authenticated_user(),
			gh.fetch_issue(owner, repo, issue_number),
			gh.fetch_comments(owner, repo, issue_number),
			gh.fetch_sub_issues(owner, repo, issue_number),
		)?;

		// Build Issue from current GitHub state
		let remote_issue = Issue::from_github(&gh_issue, &gh_comments, &gh_sub_issues, owner, repo, &current_user);

		// Consensus is the last committed state in git
		// If no consensus (new file), treat as "only local changed"
		let local_changed = consensus.as_ref().map(|c| *issue != *c).unwrap_or(true);
		let remote_changed = consensus.as_ref().map(|c| remote_issue != *c).unwrap_or(false);

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
				tracing::debug!("[sync] Only remote changed, accepting remote state");
				let remote_content = remote_issue.serialize();
				std::fs::write(issue_file_path, &remote_content)?;
				// Commit the remote state update to git
				commit_issue_changes(issue_file_path, owner, repo, issue_number, None)?;
				println!("Remote changed, local file updated.");
				return Ok(());
			}
			(true, true) => {
				// Both changed - conflict!
				tracing::debug!("[sync] Both local and remote changed, conflict detected");
				return handle_divergence(issue_file_path, owner, repo, issue_number, &remote_issue).await;
			}
		}

		// Collect and execute actions (for local changes being pushed)
		// Use sub-issues from consensus for comparison
		let consensus_sub_issues: Vec<_> = consensus
			.as_ref()
			.map(|c| {
				c.children
					.iter()
					.map(|child| crate::github::OriginalSubIssue {
						number: child.meta.url.as_ref().and_then(|u| u.rsplit('/').next()).and_then(|n| n.parse().ok()).unwrap_or(0),
						state: child.meta.close_state.to_github_state().to_string(),
					})
					.collect()
			})
			.unwrap_or_default();

		let actions = issue.collect_actions(&consensus_sub_issues);
		let has_actions = actions.iter().any(|level| !level.is_empty());

		if has_actions {
			execute_issue_actions(gh, owner, repo, issue, actions).await?
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
	let final_issue_number = created_issue_number.unwrap_or(issue_number);

	// Sync body/comment/state changes to GitHub (skip for newly created issues - body already set)
	let state_changed = if created_issue_number.is_none() {
		// Use consensus for comparison, or create empty Issue if no consensus
		let consensus_for_sync = consensus.unwrap_or_else(|| Issue {
			meta: issue.meta.clone(),
			labels: vec![],
			comments: vec![],
			children: vec![],
			blockers: Default::default(),
			last_contents_change: None,
		});
		sync_local_issue_to_github(gh, owner, repo, final_issue_number, &consensus_for_sync, issue).await?
	} else {
		false
	};

	// If we executed actions or state changed, refresh from GitHub
	if actions_executed > 0 || state_changed {
		// Re-fetch and update local file to reflect the synced state
		println!("Refreshing local issue file from GitHub...");

		// Determine parent issue info if this is a sub-issue
		let meta = load_issue_meta_from_path(issue_file_path)?;
		let ancestors: Option<Vec<FetchedIssue>> = meta.parent_issue.and_then(|parent_num| {
			// Load parent info from filesystem
			// This is simplified - full implementation would traverse the hierarchy
			let parent_meta = load_issue_meta_from_path(issue_file_path.parent()?.join("..").as_path()).ok()?;
			let fetched = FetchedIssue::from_parts(owner, repo, parent_num, &parent_meta.title)?;
			Some(vec![fetched])
		});

		// Store the old path before re-fetching
		let old_path = issue_file_path.to_path_buf();

		// Re-fetch creates file with potentially new title/state (affects .bak suffix)
		let new_path = fetch_and_store_issue(gh, owner, repo, final_issue_number, &extension, ancestors).await?;

		// If the path changed (title/state changed or format changed), delete the old file
		if old_path != new_path && old_path.exists() {
			if created_issue_number.is_some() {
				println!("Issue created on GitHub, updating local file...");
			} else if state_changed {
				println!("Issue state changed, renaming file...");
			} else {
				println!("Issue renamed/moved, removing old file: {:?}", old_path);
			}
			std::fs::remove_file(&old_path)?;

			// Handle old sub-issues directory cleanup
			let old_sub_dir = old_path.with_extension("");
			let old_sub_dir = if old_sub_dir.extension().is_some() { old_sub_dir.with_extension("") } else { old_sub_dir };

			let new_parent = new_path.parent();
			if old_sub_dir.is_dir() && new_parent != Some(old_sub_dir.as_path()) {
				if let Err(e) = std::fs::remove_dir_all(&old_sub_dir) {
					eprintln!("Warning: could not remove old sub-issues directory: {e}");
				}
			}
		}

		if actions_executed > 0 {
			println!("Synced {actions_executed} actions to GitHub.");
		}

		// Commit the synced changes to local git
		commit_issue_changes(issue_file_path, owner, repo, final_issue_number, None)?;
	} else {
		println!("No changes made.");
	}

	Ok(())
}

/// Open a local issue file with the default editor modifier.
pub async fn open_local_issue(gh: &BoxedGitHubClient, issue_file_path: &Path, offline: bool) -> Result<()> {
	modify_and_sync_issue(gh, issue_file_path, offline, Modifier::Editor).await?;
	Ok(())
}

/// Modify a local issue file using the given modifier, then sync changes back to GitHub.
pub async fn modify_and_sync_issue(gh: &BoxedGitHubClient, issue_file_path: &Path, offline: bool, modifier: Modifier) -> Result<ModifyResult> {
	use super::{conflict::check_any_conflicts, files::extract_owner_repo_from_path, meta::is_virtual_project};

	// Check for any unresolved conflicts first
	check_any_conflicts()?;

	// Extract owner and repo from path
	let (owner, repo) = extract_owner_repo_from_path(issue_file_path)?;

	// Auto-enable offline mode for virtual projects
	let offline = offline || is_virtual_project(&owner, &repo);

	// Load metadata from path
	let meta = load_issue_meta_from_path(issue_file_path)?;

	// Determine extension
	let extension = match meta.extension.as_str() {
		"typ" => Extension::Typ,
		_ => Extension::Md,
	};

	// Read and parse the current issue state
	let raw_content = std::fs::read_to_string(issue_file_path)?;
	let content = expand_blocker_shorthand(&raw_content, &extension);
	if content != raw_content {
		std::fs::write(issue_file_path, &content)?;
	}

	let ctx = ParseContext::new(content.clone(), issue_file_path.display().to_string());
	let mut issue = Issue::parse(&content, &ctx)?;

	// Apply the modifier (editor, blocker command, etc.)
	let result = modifier.apply(&mut issue, issue_file_path, &extension).await?;

	// Handle duplicate close type: remove from local storage entirely
	if issue.meta.close_state.should_remove() {
		println!("Issue marked as duplicate, removing local file...");

		// Close on GitHub (if not already closed and not offline)
		let consensus = load_consensus_issue(issue_file_path);
		let consensus_closed = consensus.map(|c| c.meta.close_state.is_closed()).unwrap_or(false);
		if !offline && !consensus_closed {
			gh.update_issue_state(&owner, &repo, meta.issue_number, "closed").await?;
		}

		// Remove local file
		std::fs::remove_file(issue_file_path)?;

		// Remove sub-issues directory if it exists
		let sub_dir = issue_file_path.with_extension("");
		let sub_dir = if sub_dir.extension().is_some() { sub_dir.with_extension("") } else { sub_dir };
		if sub_dir.is_dir() {
			std::fs::remove_dir_all(&sub_dir)?;
		}

		// Commit the removal to git
		commit_issue_changes(issue_file_path, &owner, &repo, meta.issue_number, None)?;

		println!("Duplicate issue removed from local storage.");
		return Ok(result);
	}

	// Serialize the (potentially updated) Issue back to markdown
	let serialized = issue.serialize();

	// Write the normalized/updated content back
	std::fs::write(issue_file_path, &serialized)?;

	// If offline mode, skip all network operations
	if offline {
		println!("Offline mode: changes saved locally only.");
		return Ok(result);
	}

	// Use shared sync logic
	sync_issue_to_github_inner(gh, issue_file_path, &owner, &repo, meta.issue_number, &mut issue, extension).await?;

	Ok(result)
}
