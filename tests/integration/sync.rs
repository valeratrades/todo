//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Commit state on main = last synced truth
//! - Only conflict if BOTH local and remote changed since last sync
//! - Single-side changes auto-resolve
//!
//! Tests work with `Issue` directly - our canonical representation.
//! The mock GitHub layer translates to API format at the boundary.

use std::path::PathBuf;

use todo::{Issue, ParseContext};

use crate::common::{TestContext, git::GitExt};

/// Default test repository coordinates (only needed for file paths and mock setup)
const DEFAULT_OWNER: &str = "testowner";
const DEFAULT_REPO: &str = "testrepo";
const DEFAULT_NUMBER: u64 = 1;

/// Parse an Issue from markdown content.
fn parse_issue(content: &str) -> Issue {
	let ctx = ParseContext::new(content.to_string(), "test.md".to_string());
	Issue::parse(content, &ctx).expect("failed to parse test issue")
}

/// Create a simple test issue with given title and body.
fn issue(title: &str, body: &str) -> Issue {
	let content = format!("- [ ] {title} <!-- https://github.com/{DEFAULT_OWNER}/{DEFAULT_REPO}/issues/{DEFAULT_NUMBER} -->\n\t{body}\n");
	parse_issue(&content)
}

/// Extension trait for sync-specific test setup.
trait SyncTestExt {
	/// Set up a local issue file and metadata.
	/// `local` is the current local state, `original` is the last synced state.
	fn setup_local(&self, local: &Issue, original: &Issue) -> PathBuf;

	/// Set up mock GitHub to return an issue.
	fn setup_remote(&self, issue: &Issue);
}

impl SyncTestExt for TestContext {
	fn setup_local(&self, local: &Issue, original: &Issue) -> PathBuf {
		let issues_dir = format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}");
		let sanitized_title = local.meta.title.replace(' ', "_");
		let issue_filename = format!("{DEFAULT_NUMBER}_-_{sanitized_title}.md");

		// Write the local issue file
		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), &local.serialize());

		// Write metadata with original as the consensus state
		self.write_meta(DEFAULT_OWNER, DEFAULT_REPO, DEFAULT_NUMBER, original);

		self.xdg.data_dir().join(&issues_dir).join(&issue_filename)
	}

	fn setup_remote(&self, issue: &Issue) {
		let body = issue.body();
		let state = serde_json::json!({
			"issues": [{
				"owner": DEFAULT_OWNER,
				"repo": DEFAULT_REPO,
				"number": DEFAULT_NUMBER,
				"title": issue.meta.title,
				"body": body,
				"state": "open",
				"owner_login": "mock_user"
			}]
		});
		self.setup_mock_state(&state);
	}
}

#[test]
fn test_both_diverged_triggers_conflict() {
	let ctx = TestContext::new("");

	let original = issue("Test Issue", "original body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	let issue_path = ctx.setup_local(&local, &original);
	ctx.setup_remote(&remote);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(!status.success(), "Should fail when both diverged");
	assert!(
		stderr.contains("Conflict detected") || stderr.contains("both local and remote have changes"),
		"Expected conflict message. stdout: {stdout}, stderr: {stderr}"
	);
}

#[test]
fn test_both_diverged_with_git_initiates_merge() {
	let ctx = TestContext::new("");

	let original = issue("Test Issue", "original body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	let issue_path = ctx.setup_local(&local, &original);
	ctx.setup_remote(&remote);

	let git = ctx.init_git();
	git.add_all();
	git.commit("Initial commit");

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(
		stdout.contains("Merging") || stdout.contains("merged") || stderr.contains("CONFLICT") || stderr.contains("Conflict"),
		"Expected merge activity or conflict. stdout: {stdout}, stderr: {stderr}"
	);
}

#[test]
fn test_only_remote_changed_takes_remote() {
	let ctx = TestContext::new("");

	let original = issue("Test Issue", "original body");
	let remote = issue("Test Issue", "remote changed body");

	// Local matches original
	let issue_path = ctx.setup_local(&original, &original);
	ctx.setup_remote(&remote);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(status.success(), "Should succeed when only remote changed. stderr: {stderr}");
}

#[test]
fn test_only_local_changed_pushes_local() {
	let ctx = TestContext::new("");

	let original = issue("Test Issue", "original body");
	let local = issue("Test Issue", "local changed body");

	let issue_path = ctx.setup_local(&local, &original);
	ctx.setup_remote(&original); // Remote still has original

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(status.success(), "Should succeed when only local changed. stderr: {stderr}");
	assert!(
		stdout.contains("Updating") || stdout.contains("Synced") || stdout.contains("No changes"),
		"Expected sync activity. stdout: {stdout}, stderr: {stderr}"
	);
}
