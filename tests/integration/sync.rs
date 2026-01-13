//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Git commit state = last synced truth (consensus)
//! - Only conflict if BOTH local and remote changed since last sync
//! - Single-side changes auto-resolve
//!
//! Tests work with `Issue` directly - our canonical representation.
//! The mock GitHub layer translates to API format at the boundary.

use todo::{Issue, ParseContext};

use crate::common::{TestContext, git::GitExt};

const OWNER: &str = "testowner";
const REPO: &str = "testrepo";
const NUMBER: u64 = 1;

fn parse(content: &str) -> Issue {
	let ctx = ParseContext::new(content.to_string(), "test.md".to_string());
	Issue::parse(content, &ctx).expect("failed to parse test issue")
}

fn issue(title: &str, body: &str) -> Issue {
	let content = format!("- [ ] {title} <!-- https://github.com/{OWNER}/{REPO}/issues/{NUMBER} -->\n\t{body}\n");
	parse(&content)
}

#[test]
fn test_both_diverged_triggers_conflict() {
	let ctx = TestContext::new("");

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	// Set up: consensus committed to git, local is uncommitted changes
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

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
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

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
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &consensus);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

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
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &local);
	// Remote still matches consensus
	ctx.setup_remote(OWNER, REPO, NUMBER, &consensus);

	let (status, stdout, stderr) = ctx.run_open(&issue_path);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	assert!(status.success(), "Should succeed when only local changed. stderr: {stderr}");
	// Should push local changes to remote
	assert!(
		stdout.contains("Updating") || stdout.contains("Synced") || stdout.contains("body"),
		"Expected push activity. stdout: {stdout}, stderr: {stderr}"
	);
}

#[test]
fn test_reset_with_local_source_skips_sync() {
	let ctx = TestContext::new("");

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	// Set up: consensus committed to git, local is uncommitted changes
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

	// Run with --reset flag
	let (status, stdout, stderr) = ctx.open(&issue_path).args(&["--reset"]).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {status:?}");

	// With --reset, should reset to local state without sync
	assert!(status.success(), "Should succeed with --reset. stderr: {stderr}");

	// Local file should still have local changes (not overwritten by remote)
	let content = std::fs::read_to_string(&issue_path).unwrap();
	assert!(content.contains("local body"), "Local changes should be preserved with --reset");
}

#[test]
fn test_closing_issue_syncs_state_change() {
	let ctx = TestContext::new("");

	// Start with open issue
	let open_issue = issue("Test Issue", "body");
	let issue_path = ctx.setup_issue(OWNER, REPO, NUMBER, &open_issue);
	ctx.setup_remote(OWNER, REPO, NUMBER, &open_issue);

	// Edit to close the issue
	let mut closed_issue = open_issue.clone();
	closed_issue.meta.close_state = todo::CloseState::Closed;

	let (status, stdout, stderr) = ctx.open(&issue_path).edit(&closed_issue).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed. stderr: {stderr}");
	assert!(
		stdout.contains("state") || stdout.contains("closed") || stdout.contains("Updating"),
		"Expected state change sync. stdout: {stdout}"
	);
}
