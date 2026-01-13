//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Git commit state = last synced truth (consensus)
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
	/// Set up a local issue file with consensus committed to git, then local uncommitted changes.
	/// Returns the path to the issue file.
	fn setup_local_with_git(&self, consensus: &Issue, local: &Issue) -> PathBuf;

	/// Set up mock GitHub to return an issue.
	fn setup_remote(&self, issue: &Issue);
}

impl SyncTestExt for TestContext {
	fn setup_local_with_git(&self, consensus: &Issue, local: &Issue) -> PathBuf {
		let issues_dir = format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}");
		let sanitized_title = consensus.meta.title.replace(' ', "_");
		let issue_filename = format!("{DEFAULT_NUMBER}_-_{sanitized_title}.md");
		let issue_path = self.xdg.data_dir().join(&issues_dir).join(&issue_filename);

		// Write consensus state first
		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), &consensus.serialize());

		// Initialize git and commit the consensus state
		let git = self.init_git();
		git.add_all();
		git.commit("Initial sync state");

		// Now write the local changes (uncommitted)
		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), &local.serialize());

		issue_path
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

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	// Set up: consensus committed to git, local is uncommitted changes
	let issue_path = ctx.setup_local_with_git(&consensus, &local);
	ctx.setup_remote(&remote);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(
		!status.success() || stderr.contains("Conflict") || stdout.contains("Merging"),
		"Should trigger conflict or merge when both diverged. stdout: {stdout}, stderr: {stderr}"
	);
}

#[test]
fn test_both_diverged_with_git_initiates_merge() {
	let ctx = TestContext::new("");

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	// Set up: consensus committed to git, local is uncommitted changes
	let issue_path = ctx.setup_local_with_git(&consensus, &local);
	ctx.setup_remote(&remote);

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

	let consensus = issue("Test Issue", "consensus body");
	let remote = issue("Test Issue", "remote changed body");

	// Local matches consensus (no uncommitted changes)
	let issue_path = ctx.setup_local_with_git(&consensus, &consensus);
	ctx.setup_remote(&remote);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(status.success(), "Should succeed when only remote changed. stderr: {stderr}");
	assert!(
		stdout.contains("Remote changed") || stdout.contains("updated"),
		"Expected remote update message. stdout: {stdout}, stderr: {stderr}"
	);
}

#[test]
fn test_only_local_changed_pushes_local() {
	let ctx = TestContext::new("");

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local changed body");

	// Set up: consensus committed to git, local is uncommitted changes
	let issue_path = ctx.setup_local_with_git(&consensus, &local);
	ctx.setup_remote(&consensus); // Remote still has consensus

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

#[test]
fn test_closing_issue_syncs_state_change() {
	let ctx = TestContext::new("");

	// Start with open issue in consensus
	let consensus = issue("Test Issue", "body");
	// Local: user closed the issue (simulating editor edit)
	let local = {
		let content = format!("- [x] Test Issue <!-- https://github.com/{DEFAULT_OWNER}/{DEFAULT_REPO}/issues/{DEFAULT_NUMBER} -->\n\tbody\n");
		parse_issue(&content)
	};

	// Set up: consensus committed to git, local has the close change
	let issue_path = ctx.setup_local_with_git(&consensus, &local);
	ctx.setup_remote(&consensus); // Remote still has consensus (open)

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(status.success(), "Should succeed when closing issue. stderr: {stderr}");
	// The sync should update the issue state on GitHub
	assert!(
		stdout.contains("Updating issue state") || stdout.contains("closed"),
		"Expected state change sync. stdout: {stdout}, stderr: {stderr}"
	);
}

#[test]
fn test_reset_with_local_source_skips_sync() {
	let ctx = TestContext::new("");

	// State A: local file before reset
	let local_a = issue("Test Issue", "local body A");
	// State B: remote state (different from local)
	let remote_b = issue("Test Issue", "remote body B");
	// State C: what user edits to while in editor
	let edited_c = issue("Test Issue", "user edited body C");

	// Set up: local file exists with state A, committed to git
	let issue_path = ctx.setup_local_with_git(&local_a, &local_a);
	ctx.setup_remote(&remote_b);

	// Run with --reset via local path: user edits to C, but sync is skipped
	let (status, stdout, stderr) = ctx.open(&issue_path).args(&["--reset"]).edit(&edited_c).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(status.success(), "Should succeed. stderr: {stderr}");

	// Read the final file state
	let final_content = std::fs::read_to_string(&issue_path).unwrap();
	eprintln!("final_content: {final_content}");

	// User's edit should be preserved locally
	assert!(final_content.contains("user edited body C"), "User's edit should be preserved. Final content: {final_content}");

	// With --reset and local source, sync is intentionally skipped
	assert!(stdout.contains("Reset: keeping local state"), "Should skip sync with --reset and local source. stdout: {stdout}");
}
