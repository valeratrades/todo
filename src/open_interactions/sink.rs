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

use std::collections::{HashMap, HashSet};

use todo::{Comment, CommentIdentity, Issue, IssueIdentity, IssueLink, LinkedIssueMeta};

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
	#[cfg(test)]
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
		queue.push_back(TreeNode { issue: root, path: vec![] });
		Self { queue }
	}
}

impl<'a> Iterator for HorizontalIter<'a> {
	type Item = TreeNode<'a>;

	fn next(&mut self) -> Option<Self::Item> {
		let node = self.queue.pop_front()?;

		// Queue all children for later processing
		for (i, child) in node.issue.children.iter().enumerate() {
			let mut child_path = node.path.clone();
			child_path.push(i);
			self.queue.push_back(TreeNode { issue: child, path: child_path });
		}

		Some(node)
	}
}

/// Extension trait for Issue to get horizontal iteration.
pub trait IssueTreeExt {
	/// Iterate over the issue tree in horizontal-first (breadth-first) order.
	fn iter_horizontal(&self) -> HorizontalIter<'_>;

	/// Get all issues at a specific depth level.
	#[allow(dead_code)]
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
			if child.is_local() {
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
	let old_children: HashMap<u64, &Issue> = old.children.iter().filter_map(|c| c.number().map(|n| (n, c))).collect();
	let new_child_numbers: HashSet<u64> = new.children.iter().filter_map(|c| c.number()).collect();

	for child in &new.children {
		if child.is_local() {
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

//==============================================================================
// Sink Trait
//==============================================================================

/// Trait for sinking (pushing) issues to a destination.
///
/// The trait is implemented for Issue and takes a location type parameter
/// to allow different implementations for different destinations.
///
/// # Type Parameter
/// - `L`: The location type (e.g., `GithubSink`, `&Path`)
#[allow(async_fn_in_trait)]
pub trait Sink<L> {
	/// Sink this issue (consensus) to the given location, comparing against `old` state.
	///
	/// # Arguments
	/// * `old` - The current state at the target location (from last pull), or None if no previous state exists
	/// * `location` - Where to write
	///
	/// # Returns
	/// * `Ok(true)` if any changes were made
	/// * `Ok(false)` if already in sync
	/// * `Err(_)` on failure
	async fn sink(&mut self, old: Option<&Issue>, location: L) -> color_eyre::Result<bool>;
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
	async fn sink(&mut self, old: Option<&Issue>, location: GithubSink<'_>) -> color_eyre::Result<bool> {
		let GithubSink { gh, owner, repo } = location;
		let mut changed = false;

		// If this is a pending (local) issue, create it first
		if self.is_local() {
			let title = &self.contents.title;
			let body = self.body();
			let closed = self.contents.state.is_closed();

			println!("Creating issue: {title}");
			let created = gh.create_issue(owner, repo, title, &body).await?;
			println!("Created issue #{}: {}", created.number, created.html_url);

			// Close if needed
			if closed {
				gh.update_issue_state(owner, repo, created.number, "closed").await?;
			}

			// Update identity
			let url = format!("https://github.com/{owner}/{repo}/issues/{}", created.number);
			let link = IssueLink::parse(&url).expect("just constructed valid URL");
			let user = gh.fetch_authenticated_user().await?;
			self.identity = IssueIdentity::Linked(LinkedIssueMeta {
				user,
				link,
				ts: None,
				lineage: vec![],
			});
			changed = true;
		}

		let issue_number = self.number().expect("issue must have number after creation");

		// Sync content against old (if we have old state)
		let diff = compute_node_diff(self, old);

		if diff.body_changed {
			let body = self.body();
			println!("Updating issue #{issue_number} body...");
			gh.update_issue_body(owner, repo, issue_number, &body).await?;
			changed = true;
		}

		if diff.state_changed {
			let state = self.contents.state.to_github_state();
			println!("Updating issue #{issue_number} state to {state}...");
			gh.update_issue_state(owner, repo, issue_number, state).await?;
			changed = true;
		}

		// Create pending comments sequentially (order matters)
		for comment in self.contents.comments.iter_mut().skip(1) {
			if comment.identity.is_pending() && !comment.body.is_empty() {
				let body_str = comment.body.render();
				println!("Creating new comment on issue #{issue_number}...");
				gh.create_comment(owner, repo, issue_number, &body_str).await?;
				changed = true;
			}
		}

		// Update existing comments
		for (comment_id, comment) in &diff.comments_to_update {
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

		// Delete removed comments
		for comment_id in &diff.comments_to_delete {
			println!("Deleting comment {comment_id} from issue #{issue_number}...");
			gh.delete_comment(owner, repo, *comment_id).await?;
			changed = true;
		}

		// Recursively sink children
		// Match children by position when we have old, otherwise sink with None
		for (i, child) in self.children.iter_mut().enumerate() {
			let old_child = old.and_then(|o| o.children.get(i));

			// If child is pending, it needs to be linked to parent after creation
			let was_pending = child.is_local();

			let child_sink = GithubSink { gh, owner, repo };
			changed |= Box::pin(child.sink(old_child, child_sink)).await?;

			// Link newly created child to parent
			if was_pending {
				let child_id = gh.fetch_issue(owner, repo, child.number().unwrap()).await?.id;
				gh.add_sub_issue(owner, repo, issue_number, child_id).await?;
			}
		}

		Ok(changed)
	}
}

#[cfg(test)]
mod tests {
	use todo::{BlockerSequence, CloseState, IssueContents, IssueLink, LocalIssueMeta};

	use super::*;

	fn make_issue(title: &str, number: Option<u64>) -> Issue {
		let identity = match number {
			Some(n) => IssueIdentity::Linked(LinkedIssueMeta {
				user: "testuser".to_string(),
				link: IssueLink::parse(&format!("https://github.com/o/r/issues/{n}")).unwrap(),
				ts: None,
				lineage: vec![],
			}),
			None => IssueIdentity::Local(LocalIssueMeta { path: "test".into() }),
		};

		Issue {
			identity,
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
	}
}
