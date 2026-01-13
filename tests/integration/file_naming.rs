//! Integration tests for file naming and placement.
//!
//! Tests the file naming conventions:
//! - Flat format: `{number}_-_{title}.md` for issues without sub-issues
//! - Directory format: `{number}_-_{title}/__main__.md` for issues with sub-issues
//!
//! Also tests that old file placements are automatically cleaned up when the
//! format changes (e.g., when an issue gains sub-issues).

use std::path::PathBuf;

use todo::{Issue, ParseContext};

use crate::common::{TestContext, git::GitExt};

const DEFAULT_OWNER: &str = "testowner";
const DEFAULT_REPO: &str = "testrepo";

/// Parse an Issue from markdown content.
fn parse_issue(content: &str) -> Issue {
	let ctx = ParseContext::new(content.to_string(), "test.md".to_string());
	Issue::parse(content, &ctx).expect("failed to parse test issue")
}

/// Create a simple test issue with given number, title, and body.
fn issue(number: u64, title: &str, body: &str) -> Issue {
	let content = format!("- [ ] {title} <!-- https://github.com/{DEFAULT_OWNER}/{DEFAULT_REPO}/issues/{number} -->\n\t{body}\n");
	parse_issue(&content)
}

/// Extension trait for file naming test setup.
trait FileNamingTestExt {
	/// Set up a local issue file in flat format, committed to git.
	/// Returns the path to the issue file.
	fn setup_flat_issue(&self, issue: &Issue, issue_number: u64) -> PathBuf;

	/// Set up mock GitHub with an issue and optionally its sub-issues.
	fn setup_remote_with_sub_issues(&self, parent: &Issue, parent_number: u64, sub_issues: &[(u64, &str, &str)]);

	/// Set up mock GitHub with just an issue (no sub-issues).
	fn setup_remote_issue(&self, issue: &Issue, issue_number: u64);

	/// Get the flat format path for an issue.
	fn flat_path(&self, issue_number: u64, title: &str) -> PathBuf;

	/// Get the directory format path for an issue.
	fn dir_path(&self, issue_number: u64, title: &str) -> PathBuf;
}

impl FileNamingTestExt for TestContext {
	fn setup_flat_issue(&self, issue: &Issue, issue_number: u64) -> PathBuf {
		let issues_dir = format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}");
		let sanitized_title = issue.meta.title.replace(' ', "_");
		let issue_filename = format!("{issue_number}_-_{sanitized_title}.md");
		let issue_path = self.xdg.data_dir().join(&issues_dir).join(&issue_filename);

		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), &issue.serialize());

		let git = self.init_git();
		git.add_all();
		git.commit("Initial sync state");

		issue_path
	}

	fn setup_remote_with_sub_issues(&self, parent: &Issue, parent_number: u64, sub_issues: &[(u64, &str, &str)]) {
		let body = parent.body();

		let mut issues = vec![serde_json::json!({
			"owner": DEFAULT_OWNER,
			"repo": DEFAULT_REPO,
			"number": parent_number,
			"title": parent.meta.title,
			"body": body,
			"state": "open",
			"owner_login": "mock_user"
		})];

		let children: Vec<u64> = sub_issues.iter().map(|(num, _, _)| *num).collect();

		for (num, title, sub_body) in sub_issues {
			issues.push(serde_json::json!({
				"owner": DEFAULT_OWNER,
				"repo": DEFAULT_REPO,
				"number": num,
				"title": title,
				"body": sub_body,
				"state": "open",
				"owner_login": "mock_user"
			}));
		}

		let state = serde_json::json!({
			"issues": issues,
			"sub_issues": [{
				"owner": DEFAULT_OWNER,
				"repo": DEFAULT_REPO,
				"parent": parent_number,
				"children": children
			}]
		});

		self.setup_mock_state(&state);
	}

	fn setup_remote_issue(&self, issue: &Issue, issue_number: u64) {
		let body = issue.body();
		let state = serde_json::json!({
			"issues": [{
				"owner": DEFAULT_OWNER,
				"repo": DEFAULT_REPO,
				"number": issue_number,
				"title": issue.meta.title,
				"body": body,
				"state": "open",
				"owner_login": "mock_user"
			}]
		});
		self.setup_mock_state(&state);
	}

	fn flat_path(&self, issue_number: u64, title: &str) -> PathBuf {
		let sanitized = title.replace(' ', "_");
		self.xdg.data_dir().join(format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}/{issue_number}_-_{sanitized}.md"))
	}

	fn dir_path(&self, issue_number: u64, title: &str) -> PathBuf {
		let sanitized = title.replace(' ', "_");
		self.xdg
			.data_dir()
			.join(format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}/{issue_number}_-_{sanitized}/__main__.md"))
	}
}

#[test]
fn test_flat_format_preserved_when_no_sub_issues() {
	let ctx = TestContext::new("");

	let parent = issue(1, "Parent Issue", "parent body");
	let issue_path = ctx.setup_flat_issue(&parent, 1);
	ctx.setup_remote_issue(&parent, 1);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed. stderr: {stderr}");

	// Flat file should still exist
	assert!(ctx.flat_path(1, "Parent Issue").exists(), "Flat format file should still exist");

	// Directory format should NOT exist
	assert!(!ctx.dir_path(1, "Parent Issue").exists(), "Directory format should not be created");
}

#[test]
fn test_old_flat_file_removed_when_sub_issues_appear() {
	let ctx = TestContext::new("");

	// Start with a flat issue locally
	let parent = issue(1, "Parent Issue", "parent body");
	let issue_path = ctx.setup_flat_issue(&parent, 1);

	// Remote now has sub-issues
	ctx.setup_remote_with_sub_issues(&parent, 1, &[(2, "Child Issue", "child body")]);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed. stderr: {stderr}");

	// Old flat file should be removed
	assert!(!ctx.flat_path(1, "Parent Issue").exists(), "Old flat format file should be removed");

	// New directory format should exist
	assert!(ctx.dir_path(1, "Parent Issue").exists(), "Directory format file should be created");
}

#[test]
fn test_old_placement_discarded_even_without_local_changes() {
	// This test verifies that when remote gains sub-issues but local has no changes,
	// the old flat file is still cleaned up and replaced with the directory format.

	let ctx = TestContext::new("");

	// Set up a flat issue locally, committed to git
	let parent = issue(1, "Parent Issue", "parent body");
	let issue_path = ctx.setup_flat_issue(&parent, 1);

	// Remote has sub-issues now (simulating someone else adding them)
	ctx.setup_remote_with_sub_issues(&parent, 1, &[(2, "Child Issue", "child body")]);

	// Open the issue (should sync and update format)
	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed. stderr: {stderr}");

	// The critical assertion: old flat file must be gone
	let flat_path = ctx.flat_path(1, "Parent Issue");
	assert!(
		!flat_path.exists(),
		"Old flat format file at {flat_path:?} should be removed even when no local changes were made"
	);

	// New directory format should exist with the main file
	let dir_path = ctx.dir_path(1, "Parent Issue");
	assert!(dir_path.exists(), "Directory format file at {dir_path:?} should be created");

	// Sub-issue file should also exist
	let sub_issue_dir = ctx.xdg.data_dir().join(format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}/1_-_Parent_Issue"));
	assert!(sub_issue_dir.is_dir(), "Sub-issue directory should exist");
}
