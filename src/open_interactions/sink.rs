//! Sink trait for pushing issues to sources (GitHub, filesystem).
//!
//! This module provides a unified interface for syncing issues to different
//! destinations. The key insight is that we only want to touch things that
//! have actually changed since the last sync.
//!
//! ## Design Overview
//!
//! The `Sink<L>` trait takes:
//! - `old`: The state of the source we're writing to (preserved after pulling)
//! - `location`: Where to write (e.g., GitHub coordinates or filesystem path)
//!
//! The trait is called on `consensus` - the merged state we want to push.
//!
//! ## Ordering Constraints
//!
//! 1. **Pending issues must be created before** comparing children, as they
//!    need URLs/IDs for deterministic ordering
//! 2. **Pending comments must be created sequentially** (one by one) to
//!    preserve creation order on GitHub
//! 3. **Other operations can run in parallel** per level
//!
//! ## Horizontal-First Iteration
//!
//! The tree is processed level by level (breadth-first), which allows:
//! - Parallel processing of siblings at each level
//! - Proper ordering for pending issue creation (parent before children)

use std::collections::{HashMap, HashSet, VecDeque};

use todo::{Comment, CommentIdentity, Issue, IssueIdentity};

//==============================================================================
// Location Types
//==============================================================================

/// Filesystem location for sinking issues.
#[derive(Clone, Debug)]
pub struct FilesystemLocation<'a> {
	pub owner: &'a str,
	pub repo: &'a str,
}

//==============================================================================
// Diff Results
//==============================================================================

/// Result of comparing an issue node with its old state.
#[derive(Clone, Debug, Default)]
pub struct IssueDiff {
	/// Issue body changed (first comment)
	pub body_changed: bool,
	/// Issue state (open/closed) changed
	pub state_changed: bool,
	/// Issue title changed
	pub title_changed: bool,
	/// Issue labels changed
	pub labels_changed: bool,
	/// Comments to create (pending comments that don't exist in old)
	pub comments_to_create: Vec<Comment>,
	/// Comments to update (existing comments with changed body)
	pub comments_to_update: Vec<(u64, Comment)>,
	/// Comment IDs to delete (exist in old but not in new)
	pub comments_to_delete: Vec<u64>,
	/// Sub-issues to create (pending sub-issues)
	pub children_to_create: Vec<Issue>,
	/// Sub-issue numbers to delete (exist in old but not in new)
	pub children_to_delete: Vec<u64>,
}

impl IssueDiff {
	/// Returns true if there are any changes to sync.
	pub fn has_changes(&self) -> bool {
		self.body_changed
			|| self.state_changed
			|| self.title_changed
			|| self.labels_changed
			|| !self.comments_to_create.is_empty()
			|| !self.comments_to_update.is_empty()
			|| !self.comments_to_delete.is_empty()
			|| !self.children_to_create.is_empty()
			|| !self.children_to_delete.is_empty()
	}
}

//==============================================================================
// Horizontal-First Tree Iterator
//==============================================================================

/// An item in the horizontal iteration, containing the issue and its path.
#[derive(Clone, Debug)]
pub struct TreeNode<'a> {
	/// The issue at this position
	pub issue: &'a Issue,
	/// Path from root (empty for root, [0] for first child, [0, 1] for first child's second child)
	pub path: Vec<usize>,
	/// Parent issue number (for creating sub-issues)
	pub parent_number: Option<u64>,
}

/// Iterator that yields issues in horizontal-first (breadth-first) order.
///
/// This is crucial for proper sync ordering:
/// - Level 0: Root issue
/// - Level 1: All direct children of root
/// - Level 2: All grandchildren
/// - etc.
pub struct HorizontalIter<'a> {
	queue: VecDeque<TreeNode<'a>>,
}

impl<'a> HorizontalIter<'a> {
	/// Create a new horizontal iterator starting from the given issue.
	pub fn new(root: &'a Issue) -> Self {
		let mut queue = VecDeque::new();
		queue.push_back(TreeNode {
			issue: root,
			path: vec![],
			parent_number: None,
		});
		Self { queue }
	}
}

impl<'a> Iterator for HorizontalIter<'a> {
	type Item = TreeNode<'a>;

	fn next(&mut self) -> Option<Self::Item> {
		let node = self.queue.pop_front()?;

		// Queue all children for later processing
		let parent_number = node.issue.meta.identity.number();
		for (i, child) in node.issue.children.iter().enumerate() {
			let mut child_path = node.path.clone();
			child_path.push(i);
			self.queue.push_back(TreeNode {
				issue: child,
				path: child_path,
				parent_number,
			});
		}

		Some(node)
	}
}

/// Extension trait for Issue to get horizontal iteration.
pub trait IssueTreeExt {
	/// Iterate over the issue tree in horizontal-first (breadth-first) order.
	fn iter_horizontal(&self) -> HorizontalIter<'_>;

	/// Get all issues at a specific depth level.
	fn issues_at_level(&self, level: usize) -> Vec<TreeNode<'_>>;

	/// Get the maximum depth of the tree.
	fn max_depth(&self) -> usize;
}

impl IssueTreeExt for Issue {
	fn iter_horizontal(&self) -> HorizontalIter<'_> {
		HorizontalIter::new(self)
	}

	fn issues_at_level(&self, target_level: usize) -> Vec<TreeNode<'_>> {
		self.iter_horizontal().filter(|node| node.path.len() == target_level).collect()
	}

	fn max_depth(&self) -> usize {
		self.iter_horizontal().map(|node| node.path.len()).max().unwrap_or(0)
	}
}

//==============================================================================
// Diff Computation
//==============================================================================

/// Compute the diff between `new` (consensus we're pushing) and `old` (current state of target).
///
/// This identifies what needs to be synced:
/// - Changed content (body, state, title, labels)
/// - Comments to create/update/delete
/// - Children to create/delete
pub fn compute_node_diff(new: &Issue, old: Option<&Issue>) -> IssueDiff {
	let mut diff = IssueDiff::default();

	let Some(old) = old else {
		// No old state - everything is new (but issue itself is handled separately)
		// Collect pending comments and children
		for comment in new.contents.comments.iter().skip(1) {
			if comment.identity.is_pending() {
				diff.comments_to_create.push(comment.clone());
			}
		}
		for child in &new.children {
			if child.meta.identity.is_pending() {
				diff.children_to_create.push(child.clone());
			}
		}
		return diff;
	};

	// Compare body (first comment)
	let new_body = new.contents.comments.first().map(|c| c.body.render()).unwrap_or_default();
	let old_body = old.contents.comments.first().map(|c| c.body.render()).unwrap_or_default();
	diff.body_changed = new_body != old_body;

	// Compare state
	diff.state_changed = new.contents.state != old.contents.state;

	// Compare title
	diff.title_changed = new.contents.title != old.contents.title;

	// Compare labels
	diff.labels_changed = new.contents.labels != old.contents.labels;

	// Compare comments (skip first which is body)
	let old_comments: HashMap<u64, &Comment> = old.contents.comments.iter().skip(1).filter_map(|c| c.identity.id().map(|id| (id, c))).collect();
	let new_comment_ids: HashSet<u64> = new.contents.comments.iter().skip(1).filter_map(|c| c.identity.id()).collect();

	for comment in new.contents.comments.iter().skip(1) {
		match &comment.identity {
			CommentIdentity::Pending | CommentIdentity::Body => {
				// New pending comment to create
				if !comment.body.is_empty() {
					diff.comments_to_create.push(comment.clone());
				}
			}
			CommentIdentity::Created { id, .. } => {
				if let Some(old_comment) = old_comments.get(id) {
					// Existing comment - check if body changed
					if comment.body.render() != old_comment.body.render() {
						diff.comments_to_update.push((*id, comment.clone()));
					}
				}
				// Note: if comment exists in new but not in old, it shouldn't happen
				// (would mean we have an ID for something that doesn't exist)
			}
		}
	}

	// Find comments to delete (in old but not in new)
	for id in old_comments.keys() {
		if !new_comment_ids.contains(id) {
			diff.comments_to_delete.push(*id);
		}
	}

	// Compare children
	let old_children: HashMap<u64, &Issue> = old.children.iter().filter_map(|c| c.meta.identity.number().map(|n| (n, c))).collect();
	let new_child_numbers: HashSet<u64> = new.children.iter().filter_map(|c| c.meta.identity.number()).collect();

	for child in &new.children {
		if child.meta.identity.is_pending() {
			diff.children_to_create.push(child.clone());
		}
	}

	// Find children to delete (in old but not in new)
	for num in old_children.keys() {
		if !new_child_numbers.contains(num) {
			diff.children_to_delete.push(*num);
		}
	}

	diff
}

/// Match old issue to new issue by URL/number for comparison.
pub fn find_old_node<'a>(old_root: &'a Issue, path: &[usize]) -> Option<&'a Issue> {
	if path.is_empty() {
		return Some(old_root);
	}

	let mut current = old_root;
	for &idx in path {
		current = current.children.get(idx)?;
	}
	Some(current)
}

/// Find an issue in the old tree by its URL/number, regardless of path.
/// This handles cases where children might be reordered.
pub fn find_old_by_identity<'a>(old_root: &'a Issue, identity: &IssueIdentity) -> Option<&'a Issue> {
	let target_url = identity.url_str()?;

	for node in old_root.iter_horizontal() {
		if node.issue.meta.identity.url_str() == Some(target_url) {
			return Some(node.issue);
		}
	}
	None
}

//==============================================================================
// Sink Trait
//==============================================================================

/// Trait for sinking (pushing) issues to a destination.
///
/// The trait is implemented for Issue and takes a location type parameter
/// to allow different implementations for different destinations.
///
/// # Type Parameter
/// - `L`: The location type (e.g., `GithubLocation`, `FilesystemLocation`)
#[allow(async_fn_in_trait)]
pub trait Sink<L> {
	/// Sink this issue (consensus) to the given location, comparing against `old` state.
	///
	/// # Arguments
	/// * `old` - The current state at the target location (from last pull)
	/// * `location` - Where to write
	///
	/// # Returns
	/// * `Ok(true)` if any changes were made
	/// * `Ok(false)` if already in sync
	/// * `Err(_)` on failure
	async fn sink(&mut self, old: &Issue, location: L) -> color_eyre::Result<bool>;
}

//==============================================================================
// GitHub Sink Implementation
//==============================================================================

use v_utils::prelude::*;

use crate::github::BoxedGithubClient;

/// GitHub location with client for sinking.
pub struct GithubSink<'a> {
	pub gh: &'a BoxedGithubClient,
	pub owner: &'a str,
	pub repo: &'a str,
}

impl Sink<GithubSink<'_>> for Issue {
	async fn sink(&mut self, old: &Issue, location: GithubSink<'_>) -> color_eyre::Result<bool> {
		let GithubSink { gh, owner, repo } = location;
		let mut changed = false;

		// Phase 1: Create all pending issues level by level (must happen first for deterministic ordering)
		// We need to create issues before we can compare children, as pending issues need URLs
		changed |= create_pending_issues_hierarchical(self, old, gh, owner, repo).await?;

		// Phase 2: Create all pending comments SEQUENTIALLY (to preserve order)
		// This must happen after pending issues are created
		changed |= create_pending_comments_sequential(self, old, gh, owner, repo).await?;

		// Phase 3: Now we can sync the rest (body, state, existing comments) in parallel per level
		changed |= sync_existing_content(self, old, gh, owner, repo).await?;

		// Phase 4: Delete items that exist in old but not in new
		changed |= delete_removed_items(self, old, gh, owner, repo).await?;

		Ok(changed)
	}
}

/// Create all pending issues hierarchically (level by level).
///
/// This function walks the tree level-by-level and creates pending issues.
/// After creation, it updates the local Issue with the new identity so that
/// subsequent iterations can see the correct URLs for deterministic ordering.
async fn create_pending_issues_hierarchical(new: &mut Issue, _old: &Issue, gh: &BoxedGithubClient, owner: &str, repo: &str) -> Result<bool> {
	let mut changed = false;
	let max_depth = new.max_depth();

	for level in 0..=max_depth {
		// Collect indices of pending children at this level
		let pending_at_level = collect_pending_issues_at_level(new, level);

		if pending_at_level.is_empty() {
			continue;
		}

		// Create each pending issue at this level
		// NOTE: We do this sequentially because creation order matters for deterministic numbering
		for path in pending_at_level {
			let parent_number = if path.is_empty() {
				// Root issue is pending
				None
			} else {
				// Get parent's issue number
				let parent_path = &path[..path.len() - 1];
				let parent = get_node_at_path_mut(new, parent_path).expect("parent must exist");
				parent.meta.identity.number()
			};

			let node = get_node_at_path_mut(new, &path).expect("node must exist");

			// Skip if not pending
			if !node.meta.identity.is_pending() {
				continue;
			}

			let title = &node.contents.title;
			let body = node.body();
			let closed = node.contents.state.is_closed();

			println!("Creating issue: {title}");
			let created = gh.create_issue(owner, repo, title, &body).await?;
			println!("Created issue #{}: {}", created.number, created.html_url);

			// Link to parent if sub-issue
			if let Some(parent_num) = parent_number {
				gh.add_sub_issue(owner, repo, parent_num, created.id).await?;
			}

			// Close if needed
			if closed {
				gh.update_issue_state(owner, repo, created.number, "closed").await?;
			}

			// Update the Issue struct with new identity
			let url = format!("https://github.com/{owner}/{repo}/issues/{}", created.number);
			let link = todo::IssueLink::parse(&url).expect("just constructed valid URL");
			let user = gh.fetch_authenticated_user().await?;
			node.meta.identity = todo::IssueIdentity::Created { user, link };

			changed = true;
		}
	}

	Ok(changed)
}

/// Collect paths to all pending issues at a specific level.
fn collect_pending_issues_at_level(issue: &Issue, target_level: usize) -> Vec<Vec<usize>> {
	let mut paths = Vec::new();

	// Helper to recursively collect pending issues
	fn collect(issue: &Issue, current_path: &[usize], target_level: usize, paths: &mut Vec<Vec<usize>>) {
		if current_path.len() == target_level && issue.meta.identity.is_pending() {
			paths.push(current_path.to_vec());
		}

		if current_path.len() < target_level {
			for (i, child) in issue.children.iter().enumerate() {
				let mut child_path = current_path.to_vec();
				child_path.push(i);
				collect(child, &child_path, target_level, paths);
			}
		}
	}

	collect(issue, &[], target_level, &mut paths);
	paths
}

/// Get a mutable reference to a node at a given path.
fn get_node_at_path_mut<'a>(issue: &'a mut Issue, path: &[usize]) -> Option<&'a mut Issue> {
	if path.is_empty() {
		return Some(issue);
	}
	let mut current = issue;
	for &idx in path {
		current = current.children.get_mut(idx)?;
	}
	Some(current)
}

/// Create pending comments sequentially to preserve order.
///
/// Comments must be created one by one because GitHub assigns comment IDs
/// in creation order, and we need deterministic ordering.
async fn create_pending_comments_sequential(new: &mut Issue, _old: &Issue, gh: &BoxedGithubClient, owner: &str, repo: &str) -> Result<bool> {
	let mut changed = false;

	// Process all issues in the tree
	for node_info in new.clone().iter_horizontal() {
		let Some(issue_number) = node_info.issue.meta.identity.number() else {
			continue; // Skip pending issues (already handled)
		};

		// Get mutable reference to this node
		let node = get_node_at_path_mut(new, &node_info.path).expect("node must exist");

		// Find pending comments and create them sequentially
		for comment in node.contents.comments.iter_mut().skip(1) {
			if comment.identity.is_pending() && !comment.body.is_empty() {
				let body_str = comment.body.render();
				println!("Creating comment on issue #{issue_number}...");
				gh.create_comment(owner, repo, issue_number, &body_str).await?;
				// Note: We don't update the comment identity here because we'd need to fetch
				// the created comment ID, which requires another API call. The next pull will
				// sync this properly.
				changed = true;
			}
		}
	}

	Ok(changed)
}

/// Sync existing content (body, state, existing comments).
///
/// This can run operations in parallel since the order doesn't matter
/// for updates to existing content.
async fn sync_existing_content(new: &Issue, old: &Issue, gh: &BoxedGithubClient, owner: &str, repo: &str) -> Result<bool> {
	let mut changed = false;

	// Process all issues
	for node_info in new.iter_horizontal() {
		let Some(issue_number) = node_info.issue.meta.identity.number() else {
			continue;
		};

		// Find corresponding old node
		let old_node = find_old_by_identity(old, &node_info.issue.meta.identity);
		let diff = compute_node_diff(node_info.issue, old_node);

		// Update body if changed
		if diff.body_changed {
			let body = node_info.issue.body();
			println!("Updating issue #{issue_number} body...");
			gh.update_issue_body(owner, repo, issue_number, &body).await?;
			changed = true;
		}

		// Update state if changed
		if diff.state_changed {
			let state = node_info.issue.contents.state.to_github_state();
			println!("Updating issue #{issue_number} state to {state}...");
			gh.update_issue_state(owner, repo, issue_number, state).await?;
			changed = true;
		}

		// Update existing comments
		for (comment_id, comment) in &diff.comments_to_update {
			// Only update comments owned by current user
			if let CommentIdentity::Created { user, .. } = &comment.identity {
				if !todo::current_user::is(user) {
					continue;
				}
			}
			let body_str = comment.body.render();
			println!("Updating comment {comment_id}...");
			gh.update_comment(owner, repo, *comment_id, &body_str).await?;
			changed = true;
		}
	}

	Ok(changed)
}

/// Delete items that exist in old but not in new.
async fn delete_removed_items(new: &Issue, old: &Issue, gh: &BoxedGithubClient, owner: &str, repo: &str) -> Result<bool> {
	let mut changed = false;

	// Process all issues in OLD tree to find deleted comments
	for old_node_info in old.iter_horizontal() {
		let Some(issue_number) = old_node_info.issue.meta.identity.number() else {
			continue;
		};

		// Find corresponding new node
		let new_node = find_old_by_identity(new, &old_node_info.issue.meta.identity);
		let Some(new_node) = new_node else {
			// Issue was deleted - we don't delete issues from GitHub
			// (they might be moved elsewhere or intentionally removed from local)
			continue;
		};

		// Find comments to delete
		let new_comment_ids: HashSet<u64> = new_node.contents.comments.iter().filter_map(|c| c.identity.id()).collect();

		for comment in old_node_info.issue.contents.comments.iter().skip(1) {
			if let Some(id) = comment.identity.id() {
				if !new_comment_ids.contains(&id) {
					println!("Deleting comment {id} from issue #{issue_number}...");
					gh.delete_comment(owner, repo, id).await?;
					changed = true;
				}
			}
		}
	}

	Ok(changed)
}

//==============================================================================
// Filesystem Sink Implementation
//==============================================================================

use super::files::save_issue_tree;

impl Sink<FilesystemLocation<'_>> for Issue {
	async fn sink(&mut self, old: &Issue, location: FilesystemLocation<'_>) -> color_eyre::Result<bool> {
		let FilesystemLocation { owner, repo } = location;

		// For filesystem, we simply check if anything changed and save if so
		// The save_issue_tree function handles creating directories and files

		// Check if there are any differences
		let has_changes = !issues_equal(self, old);

		if has_changes {
			save_issue_tree(self, owner, repo, &[])?;
		}

		Ok(has_changes)
	}
}

/// Deep equality check for issues (including children).
fn issues_equal(a: &Issue, b: &Issue) -> bool {
	// Check node content
	if a.contents != b.contents {
		return false;
	}
	if a.meta.identity != b.meta.identity {
		return false;
	}

	// Check children count
	if a.children.len() != b.children.len() {
		return false;
	}

	// Recursively check children
	for (child_a, child_b) in a.children.iter().zip(b.children.iter()) {
		if !issues_equal(child_a, child_b) {
			return false;
		}
	}

	true
}

#[cfg(test)]
mod tests {
	use todo::{BlockerSequence, CloseState, IssueContents, IssueLink, IssueMeta};

	use super::*;

	fn make_issue(title: &str, number: Option<u64>) -> Issue {
		let identity = match number {
			Some(n) => IssueIdentity::Created {
				user: "testuser".to_string(),
				link: IssueLink::parse(&format!("https://github.com/o/r/issues/{n}")).unwrap(),
			},
			None => IssueIdentity::Pending,
		};

		Issue {
			meta: IssueMeta {
				identity,
				last_contents_change: None,
			},
			contents: IssueContents {
				title: title.to_string(),
				labels: vec![],
				state: CloseState::Open,
				comments: vec![Comment {
					identity: CommentIdentity::Body,
					body: todo::Events::parse("body"),
				}],
				blockers: BlockerSequence::default(),
			},
			children: vec![],
		}
	}

	#[test]
	fn test_horizontal_iter_single_node() {
		let issue = make_issue("Root", Some(1));
		let nodes: Vec<_> = issue.iter_horizontal().collect();

		assert_eq!(nodes.len(), 1);
		assert!(nodes[0].path.is_empty());
		assert_eq!(nodes[0].issue.contents.title, "Root");
	}

	#[test]
	fn test_horizontal_iter_with_children() {
		let mut root = make_issue("Root", Some(1));
		root.children.push(make_issue("Child1", Some(2)));
		root.children.push(make_issue("Child2", Some(3)));

		let nodes: Vec<_> = root.iter_horizontal().collect();

		assert_eq!(nodes.len(), 3);
		// Level 0: Root
		assert!(nodes[0].path.is_empty());
		// Level 1: Children (in order)
		assert_eq!(nodes[1].path, vec![0]);
		assert_eq!(nodes[2].path, vec![1]);
	}

	#[test]
	fn test_horizontal_iter_nested() {
		let mut root = make_issue("Root", Some(1));
		let mut child1 = make_issue("Child1", Some(2));
		child1.children.push(make_issue("Grandchild1", Some(4)));
		child1.children.push(make_issue("Grandchild2", Some(5)));
		root.children.push(child1);
		root.children.push(make_issue("Child2", Some(3)));

		let nodes: Vec<_> = root.iter_horizontal().collect();

		// Should be breadth-first: Root, Child1, Child2, Grandchild1, Grandchild2
		assert_eq!(nodes.len(), 5);
		assert_eq!(nodes[0].issue.contents.title, "Root");
		assert_eq!(nodes[1].issue.contents.title, "Child1");
		assert_eq!(nodes[2].issue.contents.title, "Child2");
		assert_eq!(nodes[3].issue.contents.title, "Grandchild1");
		assert_eq!(nodes[4].issue.contents.title, "Grandchild2");

		// Check paths
		assert!(nodes[0].path.is_empty());
		assert_eq!(nodes[1].path, vec![0]);
		assert_eq!(nodes[2].path, vec![1]);
		assert_eq!(nodes[3].path, vec![0, 0]);
		assert_eq!(nodes[4].path, vec![0, 1]);
	}

	#[test]
	fn test_issues_at_level() {
		let mut root = make_issue("Root", Some(1));
		let mut child1 = make_issue("Child1", Some(2));
		child1.children.push(make_issue("Grandchild1", Some(4)));
		root.children.push(child1);
		root.children.push(make_issue("Child2", Some(3)));

		// Level 0: Root only
		let level0 = root.issues_at_level(0);
		assert_eq!(level0.len(), 1);
		assert_eq!(level0[0].issue.contents.title, "Root");

		// Level 1: Child1, Child2
		let level1 = root.issues_at_level(1);
		assert_eq!(level1.len(), 2);

		// Level 2: Grandchild1
		let level2 = root.issues_at_level(2);
		assert_eq!(level2.len(), 1);
		assert_eq!(level2[0].issue.contents.title, "Grandchild1");
	}

	#[test]
	fn test_max_depth() {
		let issue = make_issue("Root", Some(1));
		assert_eq!(issue.max_depth(), 0);

		let mut root = make_issue("Root", Some(1));
		root.children.push(make_issue("Child", Some(2)));
		assert_eq!(root.max_depth(), 1);

		let mut deep_root = make_issue("Root", Some(1));
		let mut child = make_issue("Child", Some(2));
		child.children.push(make_issue("Grandchild", Some(3)));
		deep_root.children.push(child);
		assert_eq!(deep_root.max_depth(), 2);
	}

	#[test]
	fn test_compute_node_diff_no_changes() {
		let issue = make_issue("Root", Some(1));
		let diff = compute_node_diff(&issue, Some(&issue));

		assert!(!diff.has_changes());
	}

	#[test]
	fn test_compute_node_diff_body_changed() {
		let old = make_issue("Root", Some(1));
		let mut new = make_issue("Root", Some(1));
		new.contents.comments[0].body = todo::Events::parse("new body");

		let diff = compute_node_diff(&new, Some(&old));

		assert!(diff.body_changed);
		assert!(diff.has_changes());
	}

	#[test]
	fn test_compute_node_diff_state_changed() {
		let old = make_issue("Root", Some(1));
		let mut new = make_issue("Root", Some(1));
		new.contents.state = CloseState::Closed;

		let diff = compute_node_diff(&new, Some(&old));

		assert!(diff.state_changed);
		assert!(diff.has_changes());
	}

	#[test]
	fn test_compute_node_diff_pending_comment() {
		let old = make_issue("Root", Some(1));
		let mut new = make_issue("Root", Some(1));
		new.contents.comments.push(Comment {
			identity: CommentIdentity::Pending,
			body: todo::Events::parse("new comment"),
		});

		let diff = compute_node_diff(&new, Some(&old));

		assert_eq!(diff.comments_to_create.len(), 1);
		assert!(diff.has_changes());
	}

	#[test]
	fn test_compute_node_diff_comment_deleted() {
		let mut old = make_issue("Root", Some(1));
		old.contents.comments.push(Comment {
			identity: CommentIdentity::Created { user: "user".to_string(), id: 123 },
			body: todo::Events::parse("old comment"),
		});
		let new = make_issue("Root", Some(1));

		let diff = compute_node_diff(&new, Some(&old));

		assert_eq!(diff.comments_to_delete, vec![123]);
		assert!(diff.has_changes());
	}

	#[test]
	fn test_compute_node_diff_pending_child() {
		let old = make_issue("Root", Some(1));
		let mut new = make_issue("Root", Some(1));
		new.children.push(make_issue("New Child", None)); // Pending

		let diff = compute_node_diff(&new, Some(&old));

		assert_eq!(diff.children_to_create.len(), 1);
		assert!(diff.has_changes());
	}

	#[test]
	fn test_find_old_node() {
		let mut root = make_issue("Root", Some(1));
		let mut child = make_issue("Child", Some(2));
		child.children.push(make_issue("Grandchild", Some(3)));
		root.children.push(child);

		// Find root
		let found = find_old_node(&root, &[]);
		assert_eq!(found.unwrap().contents.title, "Root");

		// Find child
		let found = find_old_node(&root, &[0]);
		assert_eq!(found.unwrap().contents.title, "Child");

		// Find grandchild
		let found = find_old_node(&root, &[0, 0]);
		assert_eq!(found.unwrap().contents.title, "Grandchild");

		// Invalid path
		let found = find_old_node(&root, &[1]);
		assert!(found.is_none());
	}

	#[test]
	fn test_find_old_by_identity() {
		let mut root = make_issue("Root", Some(1));
		let mut child = make_issue("Child", Some(2));
		child.children.push(make_issue("Grandchild", Some(3)));
		root.children.push(child);

		// Find by identity
		let child_identity = IssueIdentity::Created {
			user: "testuser".to_string(),
			link: IssueLink::parse("https://github.com/o/r/issues/2").unwrap(),
		};
		let found = find_old_by_identity(&root, &child_identity);
		assert_eq!(found.unwrap().contents.title, "Child");

		// Find grandchild
		let grandchild_identity = IssueIdentity::Created {
			user: "testuser".to_string(),
			link: IssueLink::parse("https://github.com/o/r/issues/3").unwrap(),
		};
		let found = find_old_by_identity(&root, &grandchild_identity);
		assert_eq!(found.unwrap().contents.title, "Grandchild");

		// Not found
		let missing_identity = IssueIdentity::Created {
			user: "testuser".to_string(),
			link: IssueLink::parse("https://github.com/o/r/issues/999").unwrap(),
		};
		let found = find_old_by_identity(&root, &missing_identity);
		assert!(found.is_none());
	}
}
