//! Sync local issue changes back to GitHub.
//!
#![allow(unused_assignments)] // Fields in PushError are read by miette's derive macro via attributes
//!
//! ## Unified Sync Workflow
//!
//! All sync operations go through `sync_issue`, which:
//! 1. Takes local state, remote state, consensus (last committed), and a `MergeMode`
//! 2. Produces a merged state according to the mode
//! 3. Returns the merged issue for the caller to commit
//!
//! ## MergeMode semantics
//!
//! - `Normal`: Auto-resolve where only one side changed since consensus.
//!   If both changed the same thing → create git merge conflict.
//! - `Force(side)`: On conflicts, take the preferred side. Non-conflicting
//!   parts still merge normally.
//! - `Reset(side)`: Take preferred side entirely. Content only in the
//!   non-preferred side is deleted.
//!
//! ## Consensus-based comparison
//!
//! The git-committed state serves as "consensus" (last synced truth).
//! For each field/node:
//! - If only local changed since consensus → use local
//! - If only remote changed since consensus → use remote
//! - If both changed → apply MergeMode rules

/// Which side to prefer in merge operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
	/// Prefer local file state
	Local,
	/// Prefer remote GitHub state
	Remote,
}

/// How to merge local and remote states.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MergeMode {
	/// Auto-resolve conflicts where possible, create merge conflict otherwise.
	/// - Only local changed → use local
	/// - Only remote changed → use remote
	/// - Both changed → git merge conflict
	#[default]
	Normal,
	/// Force preferred side on conflicts, but keep non-conflicting parts from both.
	Force { prefer: Side },
	/// Reset to preferred side entirely. Content not in preferred side is deleted.
	Reset { prefer: Side },
}

/// Options for controlling sync behavior.
///
/// The `merge_mode` is consumed after first use (for pre-editor sync),
/// so post-editor sync runs with `MergeMode::Normal`.
#[derive(Debug, Default)]
pub struct SyncOptions {
	/// Merge mode for conflict resolution. Consumed after first use. For reasons, refer to https://github.com/valeratrades/todo/issues/83#issuecomment-3746995182
	merge_mode: std::cell::Cell<Option<MergeMode>>,
	/// Fetch and sync from remote before opening editor
	pub pull: bool,
}

impl SyncOptions {
	/// Create new sync options with the given merge mode and pull flag.
	pub fn new(merge_mode: Option<MergeMode>, pull: bool) -> Self {
		Self {
			merge_mode: std::cell::Cell::new(merge_mode),
			pull,
		}
	}

	/// Take the merge mode, returning Normal if already taken or not set.
	/// This ensures non-Normal modes are only used once.
	pub fn take_merge_mode(&self) -> MergeMode {
		self.merge_mode.take().unwrap_or_default()
	}

	/// Peek at the merge mode without consuming it.
	pub fn peek_merge_mode(&self) -> MergeMode {
		self.merge_mode.get().unwrap_or_default()
	}
}

use std::path::Path;

use miette::Diagnostic;
use thiserror::Error;
use todo::{CloseState, Extension, FetchedIssue, Issue, ParseContext};
use v_utils::prelude::*;

use super::{
	conflict::mark_conflict,
	fetch::fetch_and_store_issue,
	git::{commit_issue_changes, is_git_initialized, load_consensus_issue},
	github_sync::IssueGitHubExt,
	meta::load_issue_meta_from_path,
	tree::{fetch_full_issue_tree, resolve_tree},
};
use crate::{blocker_interactions::BlockerSequenceExt, github::BoxedGitHubClient};

//=============================================================================
// Error types
//=============================================================================

/// Errors that can occur when pushing changes to GitHub.
#[derive(Debug, Diagnostic, Error)]
pub enum PushError {
	/// Local file references a comment ID that doesn't exist on GitHub.
	/// This typically happens when comment IDs are manually edited in the local file.
	#[error("comment {comment_id} not found in consensus")]
	#[diagnostic(
		code(todo::sync::id_mismatch),
		help(
			"The local file references a comment ID that doesn't exist on GitHub.\nThis can happen if comment IDs were manually edited.\nTry re-fetching the issue with `--pull --reset=remote`."
		)
	)]
	IdMismatch { comment_id: u64 },
}

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
		if let Some(id) = comment.id
			&& !target_ids.contains(&id)
		{
			println!("Deleting comment {id}...");
			gh.delete_comment(owner, repo, id).await?;
			deletes += 1;
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
				return Err(PushError::IdMismatch { comment_id: id }.into());
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

/// Apply the merge mode to produce a merged issue.
///
/// Returns (merged_issue, local_needs_update, remote_needs_update).
/// - local_needs_update: true if local file should be rewritten with merged result
/// - remote_needs_update: true if changes should be pushed to GitHub
async fn apply_merge_mode(
	local: &Issue,
	consensus: Option<&Issue>,
	remote: &Issue,
	mode: MergeMode,
	issue_file_path: &Path,
	owner: &str,
	repo: &str,
	issue_number: u64,
) -> Result<(Issue, bool, bool)> {
	// First, do the standard tree resolution to detect conflicts
	let resolution = resolve_tree(local, consensus, remote);

	match mode {
		MergeMode::Normal => {
			if resolution.has_conflicts {
				// Normal mode with conflicts: trigger git merge
				tracing::debug!("[sync] Unresolvable conflicts detected at paths: {:?}", resolution.conflict_paths);
				handle_divergence(issue_file_path, owner, repo, issue_number, remote).await?;
				// handle_divergence bails on conflict, so if we reach here it means merge succeeded
				// Re-read the merged file
				let merged_content = std::fs::read_to_string(issue_file_path)?;
				let ctx = ParseContext::new(merged_content.clone(), issue_file_path.display().to_string());
				let merged = Issue::parse(&merged_content, &ctx)?;
				Ok((merged, false, true)) // File already written by merge, push to remote
			} else {
				// No conflicts, use resolution results
				if resolution.local_needs_update {
					tracing::debug!("[sync] Applying auto-resolved remote changes to local");
				}
				Ok((resolution.resolved, resolution.local_needs_update, resolution.remote_needs_update))
			}
		}
		MergeMode::Force { prefer } => {
			if resolution.has_conflicts {
				match prefer {
					Side::Local => {
						// Force local: keep local version, push to remote
						tracing::debug!("[sync] Force mode: taking local version, pushing to remote");
						println!("Force: taking local version (conflicts overwritten)");
						Ok((local.clone(), false, true))
					}
					Side::Remote => {
						// Force remote: take remote version, don't push
						tracing::debug!("[sync] Force mode: taking remote version");
						println!("Force: taking remote version (local overwritten)");
						Ok((remote.clone(), true, false))
					}
				}
			} else {
				// No conflicts, use normal resolution
				Ok((resolution.resolved, resolution.local_needs_update, resolution.remote_needs_update))
			}
		}
		MergeMode::Reset { prefer } => {
			// Reset takes preferred side entirely, regardless of conflicts
			match prefer {
				Side::Local => {
					// Reset to local: keep local as-is, push everything to remote
					tracing::debug!("[sync] Reset mode: taking local version entirely");
					println!("Reset: taking local version (remote will be overwritten)");
					Ok((local.clone(), false, true))
				}
				Side::Remote => {
					// Reset to remote: take remote entirely, don't push
					tracing::debug!("[sync] Reset mode: taking remote version entirely");
					println!("Reset: taking remote version (local replaced)");
					Ok((remote.clone(), true, false))
				}
			}
		}
	}
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
		bail!(
			"Conflict detected: both local and remote have changes since last sync.\n\
			 \n\
			 To enable conflict resolution, initialize git in your issues directory:\n\
			   cd {} && git init\n\
			 \n\
			 Then re-run the command.",
			data_dir.display()
		);
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
		bail!("Failed to create branch for remote state");
	}

	// Checkout the remote-state branch
	let checkout_status = Command::new("git").args(["-C", data_dir_str, "checkout", &branch_name]).status()?;

	if !checkout_status.success() {
		bail!("Failed to checkout remote-state branch");
	}

	// Write remote state to the issue file
	let remote_content = remote_issue.serialize();
	std::fs::write(issue_file_path, &remote_content)?;

	// Commit remote state (if there are any changes)
	let _ = Command::new("git").args(["-C", data_dir_str, "add", "-A"]).status()?;

	// Check if there are changes to commit
	let diff_status = Command::new("git").args(["-C", data_dir_str, "diff", "--cached", "--quiet"]).status()?;

	if !diff_status.success() {
		// There are staged changes, commit them
		let remote_commit_msg = format!("Remote state for {owner}/{repo}#{issue_number}");
		let commit_status = Command::new("git").args(["-C", data_dir_str, "commit", "-m", &remote_commit_msg]).status()?;

		if !commit_status.success() {
			// Restore original branch
			let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status();
			let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).status();
			bail!("Failed to commit remote state");
		}
	} else {
		// No changes - remote state matches local, no merge needed
		// Just switch back and clean up
		let _ = Command::new("git").args(["-C", data_dir_str, "checkout", &current_branch]).status();
		let _ = Command::new("git").args(["-C", data_dir_str, "branch", "-D", &branch_name]).status();
		println!("Remote state matches local, no changes needed.");
		return Ok(());
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

		bail!(
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
		);
	}

	// Some other error during merge
	Err(eyre!("Failed to merge remote changes:\n{}\n{}", merge_stdout.trim(), merge_stderr.trim()))
}

/// Result of applying a modifier to an issue.
pub struct ModifyResult {
	/// The text to display after the operation (e.g., "Popped: task name")
	pub output: Option<String>,
	/// Whether the file was modified by the user.
	/// When false (and not in integration test mode), skip all sync operations.
	pub file_modified: bool,
}

/// A modifier that can be applied to an issue file.
#[derive(Debug)]
pub enum Modifier {
	/// Open the file in an editor and wait for user to close it.
	/// If `open_at_blocker` is true, opens at the position of the last blocker item.
	Editor { open_at_blocker: bool },
	/// Pop the last blocker from the stack
	BlockerPop,
	/// Add a blocker to the stack
	BlockerAdd { text: String },
}

impl Modifier {
	/// Apply this modifier to an issue. Returns output to display.
	#[tracing::instrument(level = "debug", skip(issue, extension))]
	async fn apply(&self, issue: &mut Issue, issue_file_path: &Path, extension: &Extension) -> Result<ModifyResult> {
		match self {
			Modifier::Editor { open_at_blocker } => {
				// Serialize current state
				let content = issue.serialize();
				std::fs::write(issue_file_path, &content)?;

				// Record file modification time before opening editor
				let mtime_before = std::fs::metadata(issue_file_path)?.modified()?;

				// Calculate position if opening at blocker
				let position = if *open_at_blocker {
					issue.find_last_blocker_position().map(|(line, col)| crate::utils::Position::new(line, Some(col)))
				} else {
					None
				};

				// Open in editor (blocks until editor closes)
				crate::utils::open_file(issue_file_path, position).await?;

				// Check if file was modified by comparing modification times
				let mtime_after = std::fs::metadata(issue_file_path)?.modified()?;
				let file_modified = mtime_after != mtime_before;

				// Read edited content, expand shorthands (!b, !c), and re-parse
				let raw_content = std::fs::read_to_string(issue_file_path)?;
				let edited_content = expand_blocker_shorthand(&raw_content, extension);
				let edited_content = expand_comment_shorthand(&edited_content, extension);

				// Write back if shorthands were expanded
				if edited_content != raw_content {
					std::fs::write(issue_file_path, &edited_content)?;
				}

				let ctx = ParseContext::new(edited_content.clone(), issue_file_path.display().to_string());
				*issue = Issue::parse(&edited_content, &ctx)?;

				Ok(ModifyResult { output: None, file_modified })
			}
			Modifier::BlockerPop => {
				use crate::blocker_interactions::BlockerSequenceExt;

				let popped = issue.blockers.pop();
				let output = popped.map(|text| format!("Popped: {text}"));

				Ok(ModifyResult { output, file_modified: true })
			}
			Modifier::BlockerAdd { text } => {
				use crate::blocker_interactions::BlockerSequenceExt;

				issue.blockers.add(text);
				let output = None; // will repeat it when printing the current

				Ok(ModifyResult { output, file_modified: true })
			}
		}
	}
}

/// Inner sync logic shared by open_local_issue and sync_issue_file.
///
/// This is the unified sync entry point. All sync operations go through here.
async fn sync_issue_to_github_inner(
	gh: &BoxedGitHubClient,
	issue_file_path: &Path,
	owner: &str,
	repo: &str,
	issue_number: u64,
	issue: &mut Issue,
	extension: Extension,
	merge_mode: MergeMode,
) -> Result<()> {
	// Load consensus from git (last committed state)
	let consensus = load_consensus_issue(issue_file_path);

	//=========================================================================
	// PRE-SYNC: Create new issues so they have URLs for comparison
	//=========================================================================
	// This must happen BEFORE merge because issues without URLs can't be compared.
	// Includes: root issue if pending (--touch), and any new `- [ ]` sub-issues.
	let create_actions = issue.collect_create_actions();
	let has_creates = create_actions.iter().any(|level| !level.is_empty());
	let mut created_issue_number = None;

	if has_creates {
		let (executed, created) = execute_issue_actions(gh, owner, repo, issue, create_actions).await?;
		created_issue_number = created;

		// Re-serialize to save the new URLs
		if executed > 0 {
			let serialized = issue.serialize();
			std::fs::write(issue_file_path, &serialized)?;
			tracing::debug!("[sync] Pre-sync: created {executed} new issue(s)");
		}
	}

	//=========================================================================
	// SYNC: Merge local and remote state
	//=========================================================================
	// Now that all issues have URLs, we can compare and merge.
	let local_needs_update = if issue.meta.url.is_some() {
		// Normal flow: fetch remote and merge
		let remote_issue = fetch_full_issue_tree(gh, owner, repo, issue_number).await?;

		// Apply merge mode to get the merged result
		let (merged, local_needs_update, remote_needs_update) = apply_merge_mode(issue, consensus.as_ref(), &remote_issue, merge_mode, issue_file_path, owner, repo, issue_number).await?;

		// Update issue with merged result
		*issue = merged;

		// Write local file if it needs updating
		if local_needs_update {
			let content = issue.serialize();
			std::fs::write(issue_file_path, &content)?;
		}

		if !local_needs_update && !remote_needs_update {
			tracing::debug!("[sync] No changes detected");
		} else if remote_needs_update {
			tracing::debug!("[sync] Will push local changes to remote");
		}

		local_needs_update
	} else {
		// Issue was just created, no merge needed
		false
	};

	//=========================================================================
	// POST-SYNC: Push differences to remote
	//=========================================================================
	// Compare merged state against consensus and push all differences.
	// This handles: body/comments/state for root, state changes for sub-issues.
	let final_issue_number = created_issue_number.unwrap_or(issue_number);

	// Push root issue changes (body, comments, state) - skip for newly created issues
	let state_changed = if created_issue_number.is_none() && final_issue_number != 0 {
		let consensus_for_sync = consensus.as_ref().cloned().unwrap_or_else(|| Issue {
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

	// Push sub-issue state updates
	let consensus_sub_issues: Vec<_> = consensus
		.as_ref()
		.map(|c| {
			c.children
				.iter()
				.filter_map(|child| {
					let number = child.meta.url.as_ref().and_then(|u| u.rsplit('/').next()).and_then(|n| n.parse().ok())?;
					Some(crate::github::OriginalSubIssue {
						number,
						state: child.meta.close_state.to_github_state().to_string(),
					})
				})
				.collect()
		})
		.unwrap_or_default();

	let update_actions = issue.collect_update_actions(&consensus_sub_issues);
	let has_updates = update_actions.iter().any(|level| !level.is_empty());
	let updates_executed = if has_updates {
		let (executed, _) = execute_issue_actions(gh, owner, repo, issue, update_actions).await?;
		executed
	} else {
		0
	};

	// Determine if we need to refresh from GitHub
	let needs_refresh = has_creates || state_changed || local_needs_update || updates_executed > 0;

	if needs_refresh {
		// Re-fetch and update local file to reflect the synced state
		println!("Refreshing local issue file from GitHub...");

		// Determine parent issue info if this is a sub-issue
		let meta = load_issue_meta_from_path(issue_file_path)?;
		let ancestors: Option<Vec<FetchedIssue>> = meta.parent_issue.and_then(|parent_num| {
			let parent_meta = load_issue_meta_from_path(issue_file_path.parent()?.join("..").as_path()).ok()?;
			let fetched = FetchedIssue::from_parts(owner, repo, parent_num, &parent_meta.title)?;
			Some(vec![fetched])
		});

		// Store the old path before re-fetching
		let old_path = issue_file_path.to_path_buf();

		// Re-fetch creates file with potentially new title/state
		let new_path = fetch_and_store_issue(gh, owner, repo, final_issue_number, &extension, ancestors).await?;

		// If the path changed, delete the old file
		if old_path != new_path && old_path.exists() {
			if created_issue_number.is_some() {
				println!("Issue created on GitHub, updating local file...");
			} else if state_changed {
				println!("Issue state changed, renaming file...");
			} else {
				println!("Issue renamed/moved, removing old file: {old_path:?}");
			}
			std::fs::remove_file(&old_path)?;

			// Handle old sub-issues directory cleanup
			let old_sub_dir = old_path.with_extension("");
			let old_sub_dir = if old_sub_dir.extension().is_some() { old_sub_dir.with_extension("") } else { old_sub_dir };

			let new_parent = new_path.parent();
			if old_sub_dir.is_dir()
				&& new_parent != Some(old_sub_dir.as_path())
				&& let Err(e) = std::fs::remove_dir_all(&old_sub_dir)
			{
				eprintln!("Warning: could not remove old sub-issues directory: {e}");
			}
		}

		let total_actions = if has_creates { 1 } else { 0 } + updates_executed;
		if total_actions > 0 {
			println!("Synced {total_actions} actions to GitHub.");
		}

		// Commit the synced changes to local git
		commit_issue_changes(issue_file_path, owner, repo, final_issue_number, None)?;
	} else {
		println!("No changes made.");
	}

	Ok(())
}

/// Open a local issue file with the default editor modifier.
/// If `open_at_blocker` is true, opens the editor at the position of the last blocker item.
#[tracing::instrument(level = "debug", skip(gh, sync_opts), target = "todo::open_interactions::sync")]
pub async fn open_local_issue(gh: &BoxedGitHubClient, issue_file_path: &Path, offline: bool, sync_opts: SyncOptions, open_at_blocker: bool) -> Result<()> {
	modify_and_sync_issue(gh, issue_file_path, offline, Modifier::Editor { open_at_blocker }, sync_opts).await?;
	Ok(())
}

/// Modify a local issue file using the given modifier, then sync changes back to GitHub.
#[tracing::instrument(level = "debug", skip(gh), target = "todo::open_interactions::sync")]
pub async fn modify_and_sync_issue(gh: &BoxedGitHubClient, issue_file_path: &Path, offline: bool, modifier: Modifier, sync_opts: SyncOptions) -> Result<ModifyResult> {
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
	let content = expand_comment_shorthand(&content, &extension);
	if content != raw_content {
		std::fs::write(issue_file_path, &content)?;
	}

	let ctx = ParseContext::new(content.clone(), issue_file_path.display().to_string());
	let mut issue = Issue::parse(&content, &ctx)?;

	// Handle --pull: fetch and sync from remote BEFORE opening editor
	// This uses the merge mode (which is consumed, so post-editor sync uses Normal)
	if sync_opts.pull && !offline && meta.issue_number != 0 {
		println!("Pulling latest from GitHub...");

		// Fetch remote state
		let remote_issue = fetch_full_issue_tree(gh, &owner, &repo, meta.issue_number).await?;

		// Load consensus from git
		let consensus = load_consensus_issue(issue_file_path);

		// Take merge mode (consumes it, so post-editor sync will use Normal)
		let merge_mode = sync_opts.take_merge_mode();

		// Apply merge mode through unified sync logic
		let (merged, local_needs_update, _remote_needs_update) =
			apply_merge_mode(&issue, consensus.as_ref(), &remote_issue, merge_mode, issue_file_path, &owner, &repo, meta.issue_number).await?;

		if local_needs_update {
			// Write merged result to local file
			issue = merged;
			let content = issue.serialize();
			std::fs::write(issue_file_path, &content)?;
			commit_issue_changes(issue_file_path, &owner, &repo, meta.issue_number, None)?;
		} else if issue != merged {
			// Issue changed but doesn't need file update (keeping local)
			issue = merged;
		} else {
			println!("Already up to date.");
		}
	}

	// Apply the modifier (editor, blocker command, etc.)
	let result = modifier.apply(&mut issue, issue_file_path, &extension).await?;

	// If file was not modified by the user, skip all sync operations and exit early.
	// Skip this check in integration tests to ensure they always run the full sync path.
	if !result.file_modified && std::env::var("__IS_INTEGRATION_TEST").is_err() {
		v_utils::log!("Aborted (no changes made)");
		return Ok(result);
	}

	// Handle duplicate close type: remove from local storage entirely
	if let CloseState::Duplicate(dup_number) = issue.meta.close_state {
		// Validate that the referenced duplicate issue exists
		if !offline {
			let exists = gh.fetch_issue(&owner, &repo, dup_number).await.is_ok();
			if !exists {
				bail!(
					"Cannot mark issue as duplicate of #{dup_number}: issue #{dup_number} does not exist in {owner}/{repo}.\n\
					 \n\
					 Check that the issue number is correct. The duplicate issue must exist in the same repository."
				);
			}
		}

		println!("Issue marked as duplicate of #{dup_number}, removing local file...");

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

	// Post-editor sync: take merge mode (will be Normal if already consumed by pre-editor sync)
	let merge_mode = sync_opts.take_merge_mode();

	// Use shared sync logic
	sync_issue_to_github_inner(gh, issue_file_path, &owner, &repo, meta.issue_number, &mut issue, extension, merge_mode).await?;

	Ok(result)
}

/// Modify a local issue file offline (no GitHub sync).
/// Use this when you know you're in offline mode and don't want to require a GitHub client.
#[tracing::instrument(level = "debug", target = "todo::open_interactions::sync")]
pub async fn modify_issue_offline(issue_file_path: &Path, modifier: Modifier) -> Result<ModifyResult> {
	use super::meta::load_issue_meta_from_path;

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
	let content = expand_comment_shorthand(&content, &extension);
	if content != raw_content {
		std::fs::write(issue_file_path, &content)?;
	}

	let ctx = ParseContext::new(content.clone(), issue_file_path.display().to_string());
	let mut issue = Issue::parse(&content, &ctx)?;

	// Apply the modifier (blocker command)
	let result = modifier.apply(&mut issue, issue_file_path, &extension).await?;

	// Serialize the (potentially updated) Issue back to markdown
	let serialized = issue.serialize();

	// Write the normalized/updated content back
	std::fs::write(issue_file_path, &serialized)?;

	println!("Offline mode: changes saved locally only.");

	Ok(result)
}
