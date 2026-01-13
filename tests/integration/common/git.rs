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

	/// Set up mock GitHub with a single issue (no sub-issues).
	fn setup_remote(&self, owner: &str, repo: &str, number: u64, issue: &Issue);

	/// Set up mock GitHub with an issue and its sub-issues.
	fn setup_remote_with_children(&self, owner: &str, repo: &str, number: u64, issue: &Issue, child_numbers: &[u64]);

	/// Set up mock GitHub with multiple independent issues.
	fn setup_remote_issues(&self, issues: &[((&str, &str, u64), &Issue)]);

	/// Get the flat format path for an issue: `{number}_-_{title}.md`
	fn flat_issue_path(&self, owner: &str, repo: &str, number: u64, title: &str) -> PathBuf;

	/// Get the directory format path for an issue: `{number}_-_{title}/__main__.md`
	fn dir_issue_path(&self, owner: &str, repo: &str, number: u64, title: &str) -> PathBuf;

	/// Get the issue path after sync (flat if no children, directory if has children).
	fn issue_path_after_sync(&self, owner: &str, repo: &str, number: u64, title: &str, has_children: bool) -> PathBuf;
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

	fn setup_remote(&self, owner: &str, repo: &str, number: u64, issue: &Issue) {
		let state = serde_json::json!({
			"issues": [{
				"owner": owner,
				"repo": repo,
				"number": number,
				"title": issue.meta.title,
				"body": issue.body(),
				"state": if issue.meta.close_state.is_closed() { "closed" } else { "open" },
				"owner_login": "mock_user"
			}]
		});
		self.setup_mock_state(&state);
	}

	fn setup_remote_with_children(&self, owner: &str, repo: &str, number: u64, issue: &Issue, child_numbers: &[u64]) {
		let mut issues = vec![serde_json::json!({
			"owner": owner,
			"repo": repo,
			"number": number,
			"title": issue.meta.title,
			"body": issue.body(),
			"state": if issue.meta.close_state.is_closed() { "closed" } else { "open" },
			"owner_login": "mock_user"
		})];

		for (child, &child_num) in issue.children.iter().zip(child_numbers.iter()) {
			issues.push(serde_json::json!({
				"owner": owner,
				"repo": repo,
				"number": child_num,
				"title": child.meta.title,
				"body": child.comments.first().map(|c| c.body.as_str()).unwrap_or(""),
				"state": if child.meta.close_state.is_closed() { "closed" } else { "open" },
				"owner_login": "mock_user"
			}));
		}

		let state = serde_json::json!({
			"issues": issues,
			"sub_issues": [{
				"owner": owner,
				"repo": repo,
				"parent": number,
				"children": child_numbers
			}]
		});
		self.setup_mock_state(&state);
	}

	fn setup_remote_issues(&self, issues: &[((&str, &str, u64), &Issue)]) {
		let json_issues: Vec<_> = issues
			.iter()
			.map(|((owner, repo, number), issue)| {
				serde_json::json!({
					"owner": owner,
					"repo": repo,
					"number": number,
					"title": issue.meta.title,
					"body": issue.body(),
					"state": if issue.meta.close_state.is_closed() { "closed" } else { "open" },
					"owner_login": "mock_user"
				})
			})
			.collect();

		let state = serde_json::json!({ "issues": json_issues });
		self.setup_mock_state(&state);
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
}
