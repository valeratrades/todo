//! Tree operations for issue hierarchies.
//!
//! Handles:
//! - Level-by-level parallel fetching of full issue trees from GitHub
//! - Per-node comparison between local, consensus, and remote states
//! - Timestamp-based auto-resolution of conflicts

use std::collections::HashMap;

use jiff::Timestamp;
use todo::{CloseState, Comment, Issue, IssueMeta};
use v_utils::prelude::*;

use super::github_sync::IssueGitHubExt;
use crate::github::{BoxedGitHubClient, GitHubComment, GitHubIssue};

/// Fetch a complete issue tree from GitHub, level by level.
///
/// Issues at the same nesting level are fetched in parallel.
/// This ensures we get the full tree structure, not just shallow children.
pub async fn fetch_full_issue_tree(gh: &BoxedGitHubClient, owner: &str, repo: &str, root_issue_number: u64) -> Result<Issue> {
	let current_user = gh.fetch_authenticated_user().await?;

	// Fetch root issue with comments and immediate sub-issues
	let (root_issue, root_comments, root_sub_issues) = tokio::try_join!(
		gh.fetch_issue(owner, repo, root_issue_number),
		gh.fetch_comments(owner, repo, root_issue_number),
		gh.fetch_sub_issues(owner, repo, root_issue_number),
	)?;

	// Build root Issue (shallow children for now)
	let mut root = Issue::from_github(&root_issue, &root_comments, &root_sub_issues, owner, repo, &current_user);

	// Now recursively fetch children level by level
	fetch_children_recursive(gh, owner, repo, &current_user, &mut root).await?;

	Ok(root)
}

/// Recursively fetch children for all nodes at the current level in parallel.
fn fetch_children_recursive<'a>(
	gh: &'a BoxedGitHubClient,
	owner: &'a str,
	repo: &'a str,
	current_user: &'a str,
	issue: &'a mut Issue,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
	Box::pin(async move {
		if issue.children.is_empty() {
			return Ok(());
		}

		// Collect all child issue numbers that need fetching
		let child_numbers: Vec<u64> = issue
			.children
			.iter()
			.filter_map(|child| child.meta.url.as_ref().and_then(|url| crate::github::extract_issue_number_from_url(url)))
			.collect();

		if child_numbers.is_empty() {
			return Ok(());
		}

		// Fetch all children's data in parallel
		let futures = child_numbers.iter().map(|&num| {
			let gh = gh.clone();
			async move {
				let (comments, sub_issues) = tokio::try_join!(gh.fetch_comments(owner, repo, num), gh.fetch_sub_issues(owner, repo, num),)?;
				Ok::<_, color_eyre::eyre::Report>((num, comments, sub_issues))
			}
		});
		let results = futures::future::try_join_all(futures).await?;

		// Build a map for quick lookup
		let data_map: HashMap<u64, (Vec<GitHubComment>, Vec<GitHubIssue>)> = results.into_iter().map(|(num, comments, sub_issues)| (num, (comments, sub_issues))).collect();

		// Update each child with full data
		for child in &mut issue.children {
			let Some(child_url) = &child.meta.url else {
				continue;
			};
			let Some(child_num) = crate::github::extract_issue_number_from_url(child_url) else {
				continue;
			};
			let Some((comments, sub_issues)) = data_map.get(&child_num) else {
				continue;
			};

			// Update comments (keep first comment which is body, add actual comments)
			for c in comments {
				let comment_owned = c.user.login == current_user;
				child.comments.push(Comment {
					id: Some(c.id),
					body: c.body.as_deref().unwrap_or("").to_string(),
					owned: comment_owned,
				});
			}

			// Build sub-issue children
			child.children = sub_issues
				.iter()
				.map(|si| {
					let url = format!("https://github.com/{owner}/{repo}/issues/{}", si.number);
					let close_state = if si.state == "closed" { CloseState::Closed } else { CloseState::Open };
					let timestamp = si.updated_at.parse::<Timestamp>().ok();
					Issue {
						meta: IssueMeta {
							title: si.title.clone(),
							url: Some(url),
							close_state,
							owned: si.user.login == current_user,
						},
						labels: si.labels.iter().map(|l| l.name.clone()).collect(),
						comments: vec![Comment {
							id: None,
							body: si.body.as_deref().unwrap_or("").to_string(),
							owned: si.user.login == current_user,
						}],
						children: Vec::new(),
						blockers: Default::default(),
						last_contents_change: timestamp,
					}
				})
				.collect();
		}

		// Recurse into children
		for child in &mut issue.children {
			fetch_children_recursive(gh, owner, repo, current_user, child).await?;
		}

		Ok(())
	})
}

/// Result of comparing a single node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeResolution {
	/// No changes on either side
	NoChange,
	/// Only local changed - push to remote
	LocalOnly,
	/// Only remote changed - take remote
	RemoteOnly,
	/// Both changed, auto-resolved by taking newer (based on timestamp)
	AutoResolved {
		/// True if local is newer, false if remote is newer
		take_local: bool,
	},
	/// Both changed, cannot auto-resolve - needs manual conflict resolution
	Conflict,
}

/// Compare a single Issue node (not including children).
/// Returns the resolution for this node.
pub fn compare_node(local: &Issue, consensus: Option<&Issue>, remote: &Issue) -> NodeResolution {
	// No consensus means first sync - compare local vs remote directly
	let Some(consensus) = consensus else {
		// First sync: if local == remote, no change; otherwise need to pick one
		if node_content_eq(local, remote) {
			return NodeResolution::NoChange;
		}
		// Different content with no consensus - try timestamps, else conflict
		return match (local.last_contents_change, remote.last_contents_change) {
			(Some(local_ts), Some(remote_ts)) if local_ts != remote_ts => NodeResolution::AutoResolved { take_local: local_ts > remote_ts },
			_ => NodeResolution::Conflict,
		};
	};

	// Compare node content (body, comments, close_state) - not children
	let local_matches_consensus = node_content_eq(local, consensus);
	let remote_matches_consensus = node_content_eq(remote, consensus);

	let local_changed = !local_matches_consensus;
	let remote_changed = !remote_matches_consensus;

	match (local_changed, remote_changed) {
		(false, false) => NodeResolution::NoChange,
		(true, false) => NodeResolution::LocalOnly,
		(false, true) => NodeResolution::RemoteOnly,
		(true, true) => {
			// Both changed - try to auto-resolve using timestamps
			match (local.last_contents_change, remote.last_contents_change) {
				(Some(local_ts), Some(remote_ts)) if local_ts != remote_ts => NodeResolution::AutoResolved { take_local: local_ts > remote_ts },
				_ => {
					// Same timestamp or missing timestamps - cannot auto-resolve
					NodeResolution::Conflict
				}
			}
		}
	}
}

/// Check if two Issue nodes have the same content (excluding children).
fn node_content_eq(a: &Issue, b: &Issue) -> bool {
	// Compare close state
	if a.meta.close_state != b.meta.close_state {
		return false;
	}

	// Compare body (first comment)
	let a_body = a.comments.first().map(|c| c.body.as_str()).unwrap_or("");
	let b_body = b.comments.first().map(|c| c.body.as_str()).unwrap_or("");
	if a_body != b_body {
		return false;
	}

	// Compare other comments (by id and body)
	let a_comments: Vec<_> = a.comments.iter().skip(1).map(|c| (c.id, &c.body)).collect();
	let b_comments: Vec<_> = b.comments.iter().skip(1).map(|c| (c.id, &c.body)).collect();
	if a_comments != b_comments {
		return false;
	}

	// Compare labels
	if a.labels != b.labels {
		return false;
	}

	true
}

/// Result of resolving an entire tree.
pub struct TreeResolutionResult {
	/// The resolved issue tree (with auto-resolved changes applied)
	pub resolved: Issue,
	/// Whether any nodes had unresolvable conflicts
	pub has_conflicts: bool,
	/// Paths to nodes that have conflicts (for reporting)
	pub conflict_paths: Vec<Vec<usize>>,
	/// Whether local file needs to be updated
	pub local_needs_update: bool,
	/// Whether remote needs to be updated
	pub remote_needs_update: bool,
}

/// Resolve an entire issue tree by comparing local, consensus, and remote.
///
/// Walks the tree, comparing each node independently.
/// Auto-resolves where possible using timestamps.
/// Returns the resolved tree and conflict information.
pub fn resolve_tree(local: &Issue, consensus: Option<&Issue>, remote: &Issue) -> TreeResolutionResult {
	let mut resolved = local.clone();
	let mut has_conflicts = false;
	let mut conflict_paths = Vec::new();
	let mut local_needs_update = false;
	let mut remote_needs_update = false;

	resolve_tree_recursive(
		&mut resolved,
		local,
		consensus,
		remote,
		&[],
		&mut has_conflicts,
		&mut conflict_paths,
		&mut local_needs_update,
		&mut remote_needs_update,
	);

	TreeResolutionResult {
		resolved,
		has_conflicts,
		conflict_paths,
		local_needs_update,
		remote_needs_update,
	}
}

fn resolve_tree_recursive(
	resolved: &mut Issue,
	local: &Issue,
	consensus: Option<&Issue>,
	remote: &Issue,
	current_path: &[usize],
	has_conflicts: &mut bool,
	conflict_paths: &mut Vec<Vec<usize>>,
	local_needs_update: &mut bool,
	remote_needs_update: &mut bool,
) {
	// Compare this node
	let resolution = compare_node(local, consensus, remote);

	match resolution {
		NodeResolution::NoChange => {
			// Nothing to do for this node
		}
		NodeResolution::LocalOnly => {
			// Local changed, remote didn't - push local (resolved already has local)
			*remote_needs_update = true;
		}
		NodeResolution::RemoteOnly => {
			// Remote changed, local didn't - take remote
			apply_remote_node_content(resolved, remote);
			*local_needs_update = true;
		}
		NodeResolution::AutoResolved { take_local } => {
			if take_local {
				// Local is newer - push to remote
				*remote_needs_update = true;
			} else {
				// Remote is newer - update local
				apply_remote_node_content(resolved, remote);
				*local_needs_update = true;
			}
		}
		NodeResolution::Conflict => {
			*has_conflicts = true;
			conflict_paths.push(current_path.to_vec());
		}
	}

	// Now compare children
	// Build maps by URL for matching
	let local_children_by_url: HashMap<&str, &Issue> = local.children.iter().filter_map(|c| c.meta.url.as_deref().map(|url| (url, c))).collect();

	let consensus_children_by_url: HashMap<&str, &Issue> = consensus
		.map(|c| c.children.iter().filter_map(|child| child.meta.url.as_deref().map(|url| (url, child))).collect())
		.unwrap_or_default();

	let remote_children_by_url: HashMap<&str, &Issue> = remote.children.iter().filter_map(|c| c.meta.url.as_deref().map(|url| (url, c))).collect();

	// Process each child in resolved (which starts as local's children)
	for (i, resolved_child) in resolved.children.iter_mut().enumerate() {
		let Some(url) = resolved_child.meta.url.as_deref() else {
			continue;
		};

		let local_child = local_children_by_url.get(url);
		let consensus_child = consensus_children_by_url.get(url).copied();
		let remote_child = remote_children_by_url.get(url);

		// If both local and remote have this child, recurse
		if let (Some(local_c), Some(remote_c)) = (local_child, remote_child) {
			let mut child_path = current_path.to_vec();
			child_path.push(i);
			resolve_tree_recursive(
				resolved_child, local_c, consensus_child, remote_c, &child_path, has_conflicts, conflict_paths, local_needs_update, remote_needs_update,
			);
		}
	}

	// Handle children that exist in remote but not local (new remote children)
	for (url, remote_child) in &remote_children_by_url {
		if !local_children_by_url.contains_key(url) {
			// New child from remote - add it
			resolved.children.push((*remote_child).clone());
			*local_needs_update = true;
		}
	}
}

/// Apply remote node content to resolved node (excluding children).
fn apply_remote_node_content(resolved: &mut Issue, remote: &Issue) {
	resolved.meta.close_state = remote.meta.close_state.clone();
	resolved.labels = remote.labels.clone();

	// Update comments: keep structure but update content
	if let Some(remote_body) = remote.comments.first()
		&& let Some(resolved_body) = resolved.comments.first_mut()
	{
		resolved_body.body = remote_body.body.clone();
	}

	// Replace other comments with remote's
	resolved.comments.truncate(1);
	resolved.comments.extend(remote.comments.iter().skip(1).cloned());

	// Update timestamp
	resolved.last_contents_change = remote.last_contents_change;
}

#[cfg(test)]
mod tests {
	use insta::assert_snapshot;
	use todo::BlockerSequence;

	use super::*;

	fn make_issue(body: &str, timestamp: Option<i64>) -> Issue {
		Issue {
			meta: IssueMeta {
				title: "Test".to_string(),
				url: Some("https://github.com/o/r/issues/1".to_string()),
				close_state: CloseState::Open,
				owned: true,
			},
			labels: vec![],
			comments: vec![Comment {
				id: None,
				body: body.to_string(),
				owned: true,
			}],
			children: vec![],
			blockers: BlockerSequence::default(),
			last_contents_change: timestamp.map(|ts| Timestamp::from_second(ts).unwrap()),
		}
	}

	#[test]
	fn test_compare_node_no_change() {
		let issue = make_issue("body", Some(1000));
		let consensus = make_issue("body", Some(1000));
		let remote = make_issue("body", Some(1000));

		assert_eq!(compare_node(&issue, Some(&consensus), &remote), NodeResolution::NoChange);
	}

	#[test]
	fn test_compare_node_local_only() {
		let local = make_issue("local changed", Some(2000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("original", Some(1000));

		assert_eq!(compare_node(&local, Some(&consensus), &remote), NodeResolution::LocalOnly);
	}

	#[test]
	fn test_compare_node_remote_only() {
		let local = make_issue("original", Some(1000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(2000));

		assert_eq!(compare_node(&local, Some(&consensus), &remote), NodeResolution::RemoteOnly);
	}

	#[test]
	fn test_compare_node_auto_resolve_local_newer() {
		let local = make_issue("local changed", Some(2000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(1500));

		assert_eq!(compare_node(&local, Some(&consensus), &remote), NodeResolution::AutoResolved { take_local: true });
	}

	#[test]
	fn test_compare_node_auto_resolve_remote_newer() {
		let local = make_issue("local changed", Some(1500));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(2000));

		assert_eq!(compare_node(&local, Some(&consensus), &remote), NodeResolution::AutoResolved { take_local: false });
	}

	#[test]
	fn test_compare_node_conflict_same_timestamp() {
		let local = make_issue("local changed", Some(2000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(2000));

		assert_eq!(compare_node(&local, Some(&consensus), &remote), NodeResolution::Conflict);
	}

	#[test]
	fn test_compare_node_conflict_no_timestamp() {
		let local = make_issue("local changed", None);
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", None);

		assert_eq!(compare_node(&local, Some(&consensus), &remote), NodeResolution::Conflict);
	}

	fn make_issue_with_url(body: &str, timestamp: Option<i64>, url: &str) -> Issue {
		Issue {
			meta: IssueMeta {
				title: "Test".to_string(),
				url: Some(url.to_string()),
				close_state: CloseState::Open,
				owned: true,
			},
			labels: vec![],
			comments: vec![Comment {
				id: None,
				body: body.to_string(),
				owned: true,
			}],
			children: vec![],
			blockers: BlockerSequence::default(),
			last_contents_change: timestamp.map(|ts| Timestamp::from_second(ts).unwrap()),
		}
	}

	#[test]
	fn test_resolve_tree_no_changes() {
		let issue = make_issue("body", Some(1000));
		let consensus = make_issue("body", Some(1000));
		let remote = make_issue("body", Some(1000));

		let result = resolve_tree(&issue, Some(&consensus), &remote);

		assert!(!result.has_conflicts);
		assert!(!result.local_needs_update);
		assert!(!result.remote_needs_update);
	}

	#[test]
	fn test_resolve_tree_local_only() {
		let local = make_issue("local changed", Some(2000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("original", Some(1000));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		assert!(!result.has_conflicts);
		assert!(!result.local_needs_update);
		assert!(result.remote_needs_update);
	}

	#[test]
	fn test_resolve_tree_remote_only() {
		let local = make_issue("original", Some(1000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(2000));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		assert!(!result.has_conflicts);
		assert!(result.local_needs_update);
		assert!(!result.remote_needs_update);
		assert_snapshot!(result.resolved.body(), "remote changed", @"remote changed");
	}

	#[test]
	fn test_resolve_tree_auto_resolve_takes_newer() {
		let local = make_issue("local changed", Some(2000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(1500));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		// Local is newer, so remote needs update
		assert!(!result.has_conflicts);
		assert!(!result.local_needs_update);
		assert!(result.remote_needs_update);
		assert_snapshot!(result.resolved.body(), "local changed", @"local changed");
	}

	#[test]
	fn test_resolve_tree_conflict_same_timestamp() {
		let local = make_issue("local changed", Some(2000));
		let consensus = make_issue("original", Some(1000));
		let remote = make_issue("remote changed", Some(2000));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		assert!(result.has_conflicts);
		assert_eq!(result.conflict_paths.len(), 1);
		assert!(result.conflict_paths[0].is_empty()); // Root node conflict
	}

	#[test]
	fn test_resolve_tree_child_auto_resolves() {
		// Parent is unchanged, child has both-side changes but different timestamps
		let mut local = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		local.children.push(make_issue_with_url("local child body", Some(2000), "https://github.com/o/r/issues/2"));

		let mut consensus = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		consensus.children.push(make_issue_with_url("original child body", Some(1000), "https://github.com/o/r/issues/2"));

		let mut remote = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		remote.children.push(make_issue_with_url("remote child body", Some(1500), "https://github.com/o/r/issues/2"));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		// Local child is newer, so no conflict - remote needs update
		assert!(!result.has_conflicts);
		assert!(!result.local_needs_update);
		assert!(result.remote_needs_update);
	}

	#[test]
	fn test_resolve_tree_child_conflict() {
		// Parent is unchanged, child has both-side changes with same timestamp
		let mut local = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		local.children.push(make_issue_with_url("local child body", Some(2000), "https://github.com/o/r/issues/2"));

		let mut consensus = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		consensus.children.push(make_issue_with_url("original child body", Some(1000), "https://github.com/o/r/issues/2"));

		let mut remote = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		remote.children.push(make_issue_with_url("remote child body", Some(2000), "https://github.com/o/r/issues/2"));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		// Child has conflict (same timestamp)
		assert!(result.has_conflicts);
		assert_eq!(result.conflict_paths.len(), 1);
		assert_eq!(result.conflict_paths[0], vec![0]); // Child at index 0
	}

	#[test]
	fn test_resolve_tree_new_remote_child() {
		// Remote has a new child that local doesn't have
		let local = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");

		let consensus = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");

		let mut remote = make_issue_with_url("parent body", Some(1000), "https://github.com/o/r/issues/1");
		remote.children.push(make_issue_with_url("new remote child", Some(2000), "https://github.com/o/r/issues/2"));

		let result = resolve_tree(&local, Some(&consensus), &remote);

		// Should add new child to local
		assert!(!result.has_conflicts);
		assert!(result.local_needs_update);
		assert_eq!(result.resolved.children.len(), 1);
	}
}
