//! Git and issue setup extensions for TestContext.
//!
//! Consolidates all test setup operations that involve git state and mock GitHub.

use std::path::PathBuf;

use todo::Issue;
use v_fixtures::fs_standards::git::Git;

use super::TestContext;

/// Extension trait for git and issue setup operations.
pub trait GitExt {
	/// Initialize git in the issues directory.
	fn init_git(&self) -> Git;

	/// Write an issue file and commit to git. Returns the path to the issue file.
	///
	/// Uses flat format: `{number}_-_{title}.md`
	fn setup_issue(&self, owner: &str, repo: &str, number: u64, issue: &Issue) -> PathBuf;

	/// Write consensus state, commit, then write local uncommitted changes.
	/// Returns the path to the issue file.
	fn setup_issue_with_local_changes(&self, owner: &str, repo: &str, number: u64, consensus: &Issue, local: &Issue) -> PathBuf;

	/// Get the flat format path for an issue: `{number}_-_{title}.md`
	fn flat_issue_path(&self, owner: &str, repo: &str, number: u64, title: &str) -> PathBuf;

	/// Get the directory format path for an issue: `{number}_-_{title}/__main__.md`
	fn dir_issue_path(&self, owner: &str, repo: &str, number: u64, title: &str) -> PathBuf;

	/// Get the issue path after sync (flat if no children, directory if has children).
	fn issue_path_after_sync(&self, owner: &str, repo: &str, number: u64, title: &str, has_children: bool) -> PathBuf;

	/// Start building mock remote state.
	fn remote(&self) -> RemoteBuilder<'_>;
}

impl GitExt for TestContext {
	fn init_git(&self) -> Git {
		Git::init(self.xdg.data_dir().join("issues"))
	}

	fn setup_issue(&self, owner: &str, repo: &str, number: u64, issue: &Issue) -> PathBuf {
		let issues_dir = format!("issues/{owner}/{repo}");
		let sanitized_title = issue.meta.title.replace(' ', "_");
		let filename = format!("{number}_-_{sanitized_title}.md");
		let path = self.xdg.data_dir().join(&issues_dir).join(&filename);

		self.xdg.write_data(&format!("{issues_dir}/{filename}"), &issue.serialize());

		let git = self.init_git();
		git.add_all();
		git.commit("initial");

		path
	}

	fn setup_issue_with_local_changes(&self, owner: &str, repo: &str, number: u64, consensus: &Issue, local: &Issue) -> PathBuf {
		let issues_dir = format!("issues/{owner}/{repo}");
		let sanitized_title = consensus.meta.title.replace(' ', "_");
		let filename = format!("{number}_-_{sanitized_title}.md");
		let path = self.xdg.data_dir().join(&issues_dir).join(&filename);

		// Write consensus state first
		self.xdg.write_data(&format!("{issues_dir}/{filename}"), &consensus.serialize());

		// Initialize git and commit the consensus state
		let git = self.init_git();
		git.add_all();
		git.commit("Initial sync state");

		// Now write the local changes (uncommitted)
		self.xdg.write_data(&format!("{issues_dir}/{filename}"), &local.serialize());

		path
	}

	fn flat_issue_path(&self, owner: &str, repo: &str, number: u64, title: &str) -> PathBuf {
		let sanitized = title.replace(' ', "_");
		self.xdg.data_dir().join(format!("issues/{owner}/{repo}/{number}_-_{sanitized}.md"))
	}

	fn dir_issue_path(&self, owner: &str, repo: &str, number: u64, title: &str) -> PathBuf {
		let sanitized = title.replace(' ', "_");
		self.xdg.data_dir().join(format!("issues/{owner}/{repo}/{number}_-_{sanitized}/__main__.md"))
	}

	fn issue_path_after_sync(&self, owner: &str, repo: &str, number: u64, title: &str, has_children: bool) -> PathBuf {
		if has_children {
			self.dir_issue_path(owner, repo, number, title)
		} else {
			self.flat_issue_path(owner, repo, number, title)
		}
	}

	fn remote(&self) -> RemoteBuilder<'_> {
		RemoteBuilder::new(self)
	}
}

/// Mock comment for remote setup
struct MockComment {
	owner: String,
	repo: String,
	issue_number: u64,
	comment_id: u64,
	body: String,
	owner_login: String,
}

/// Builder for setting up mock GitHub remote state.
///
/// Usage:
/// ```ignore
/// ctx.remote()
///     .issue("owner", "repo", 1, &parent_issue)
///     .sub_issue("owner", "repo", 1, 2, &child_issue)
///     .build();
/// ```
pub struct RemoteBuilder<'a> {
	ctx: &'a TestContext,
	issues: Vec<MockIssue>,
	sub_issue_relations: Vec<SubIssueRelation>,
	comments: Vec<MockComment>,
}
impl<'a> RemoteBuilder<'a> {
	fn new(ctx: &'a TestContext) -> Self {
		Self {
			ctx,
			issues: Vec::new(),
			sub_issue_relations: Vec::new(),
			comments: Vec::new(),
		}
	}

	/// Add an issue to the remote.
	pub fn issue(mut self, owner: &str, repo: &str, number: u64, issue: &Issue) -> Self {
		self.issues.push(MockIssue {
			owner: owner.to_string(),
			repo: repo.to_string(),
			number,
			title: issue.meta.title.clone(),
			body: issue.body(),
			state: issue.meta.close_state.to_github_state().to_string(),
			state_reason: issue.meta.close_state.to_github_state_reason().map(|s| s.to_string()),
		});

		// Extract comments (skip first which is the body)
		for comment in issue.comments.iter().skip(1) {
			if let Some(id) = comment.id {
				self.comments.push(MockComment {
					owner: owner.to_string(),
					repo: repo.to_string(),
					issue_number: number,
					comment_id: id,
					body: comment.body.clone(),
					owner_login: if comment.owned { "mock_user".to_string() } else { "other_user".to_string() },
				});
			}
		}

		self
	}

	/// Add a sub-issue relationship and the child issue.
	pub fn sub_issue(mut self, owner: &str, repo: &str, parent_number: u64, child_number: u64, child_issue: &Issue) -> Self {
		// Add the child issue
		self.issues.push(MockIssue {
			owner: owner.to_string(),
			repo: repo.to_string(),
			number: child_number,
			title: child_issue.meta.title.clone(),
			body: child_issue.body(),
			state: child_issue.meta.close_state.to_github_state().to_string(),
			state_reason: child_issue.meta.close_state.to_github_state_reason().map(|s| s.to_string()),
		});

		// Add the relationship
		self.sub_issue_relations.push(SubIssueRelation {
			owner: owner.to_string(),
			repo: repo.to_string(),
			parent: parent_number,
			child: child_number,
		});

		self
	}

	/// Build and apply the mock state.
	pub fn build(self) {
		let issues: Vec<serde_json::Value> = self
			.issues
			.into_iter()
			.map(|i| {
				let mut json = serde_json::json!({
					"owner": i.owner,
					"repo": i.repo,
					"number": i.number,
					"title": i.title,
					"body": i.body,
					"state": i.state,
					"owner_login": "mock_user"
				});
				if let Some(reason) = i.state_reason {
					json["state_reason"] = serde_json::Value::String(reason);
				}
				json
			})
			.collect();

		// Group sub-issue relations by (owner, repo, parent)
		let mut sub_issues_map: std::collections::HashMap<(String, String, u64), Vec<u64>> = std::collections::HashMap::new();
		for rel in self.sub_issue_relations {
			sub_issues_map.entry((rel.owner, rel.repo, rel.parent)).or_default().push(rel.child);
		}

		let sub_issues: Vec<serde_json::Value> = sub_issues_map
			.into_iter()
			.map(|((owner, repo, parent), children)| {
				serde_json::json!({
					"owner": owner,
					"repo": repo,
					"parent": parent,
					"children": children
				})
			})
			.collect();

		// Convert comments to JSON
		let comments: Vec<serde_json::Value> = self
			.comments
			.into_iter()
			.map(|c| {
				serde_json::json!({
					"owner": c.owner,
					"repo": c.repo,
					"issue_number": c.issue_number,
					"comment_id": c.comment_id,
					"body": c.body,
					"owner_login": c.owner_login
				})
			})
			.collect();

		let mut state = serde_json::json!({ "issues": issues });
		if !sub_issues.is_empty() {
			state["sub_issues"] = serde_json::Value::Array(sub_issues);
		}
		if !comments.is_empty() {
			state["comments"] = serde_json::Value::Array(comments);
		}

		self.ctx.setup_mock_state(&state);
	}
}

struct MockIssue {
	owner: String,
	repo: String,
	number: u64,
	title: String,
	body: String,
	state: String,
	state_reason: Option<String>,
}

struct SubIssueRelation {
	owner: String,
	repo: String,
	parent: u64,
	child: u64,
}

// Legacy compatibility methods - delegate to RemoteBuilder
impl TestContext {
	/// Set up mock GitHub with a single issue (no sub-issues).
	pub fn setup_remote(&self, owner: &str, repo: &str, number: u64, issue: &Issue) {
		self.remote().issue(owner, repo, number, issue).build();
	}

	/// Set up mock GitHub with an issue and its sub-issues.
	pub fn setup_remote_with_children(&self, owner: &str, repo: &str, number: u64, issue: &Issue, child_numbers: &[u64]) {
		let mut builder = self.remote().issue(owner, repo, number, issue);

		for (child, &child_num) in issue.children.iter().zip(child_numbers.iter()) {
			builder = builder.sub_issue(owner, repo, number, child_num, child);
		}

		builder.build();
	}

	/// Set up mock GitHub with multiple independent issues.
	pub fn setup_remote_issues(&self, issues: &[((&str, &str, u64), &Issue)]) {
		let mut builder = self.remote();
		for ((owner, repo, number), issue) in issues {
			builder = builder.issue(owner, repo, *number, issue);
		}
		builder.build();
	}
}
