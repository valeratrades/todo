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

/// Opening via URL when no local file exists should create the file from remote.
#[test]
fn test_url_open_creates_local_file_from_remote() {
	let ctx = TestContext::new("");
	ctx.init_git(); // Need git initialized for commits

	let remote = issue("Test Issue", "remote body content");
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

	// No local file exists - URL open should create it
	let expected_path = ctx.flat_issue_path(OWNER, REPO, NUMBER, "Test Issue");
	assert!(!expected_path.exists(), "Local file should not exist before open");

	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, NUMBER).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed creating from URL. stderr: {stderr}");

	// File should now exist with remote content
	assert!(expected_path.exists(), "Local file should be created");
	let content = std::fs::read_to_string(&expected_path).unwrap();
	assert!(content.contains("remote body content"), "Should have remote content. Got: {content}");
}

/// When opening via URL with --reset, local state should be completely replaced with remote.
/// No merge conflicts, no prompts - just nuke and replace.
#[test]
fn test_reset_with_remote_url_nukes_local_state() {
	let ctx = TestContext::new("");

	let local = issue("Test Issue", "local body that should be nuked");
	let remote = issue("Test Issue", "remote body wins");

	// Set up: local file exists with different content
	let issue_path = ctx.setup_issue(OWNER, REPO, NUMBER, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

	// Open via URL with --reset should nuke local and use remote
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, NUMBER).args(&["--reset"]).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed with --reset via URL. stderr: {stderr}");

	// Local file should now have remote content
	let content = std::fs::read_to_string(&issue_path).unwrap();
	assert!(content.contains("remote body wins"), "Local should be replaced with remote. Got: {content}");
	assert!(!content.contains("local body that should be nuked"), "Local content should be gone");
}

/// When opening via URL with --reset and there's divergence, should NOT trigger merge conflict.
#[test]
fn test_reset_with_remote_url_skips_merge_on_divergence() {
	let ctx = TestContext::new("");

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local diverged body");
	let remote = issue("Test Issue", "remote diverged body");

	// Set up: consensus committed, local has uncommitted changes, remote is different
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

	// Open via URL with --reset should NOT trigger merge conflict
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, NUMBER).args(&["--reset"]).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	// Should succeed without merge conflict
	assert!(status.success(), "Should succeed without merge conflict. stderr: {stderr}");
	assert!(!stderr.contains("Conflict"), "Should not mention conflict with --reset");
	assert!(!stdout.contains("Merging"), "Should not attempt merge with --reset");

	// Local should have remote content
	let content = std::fs::read_to_string(&issue_path).unwrap();
	assert!(content.contains("remote diverged body"), "Should have remote content. Got: {content}");
}

/// --pull flag should fetch and sync BEFORE opening editor.
/// This test verifies the fetch actually happens by checking stdout for fetch message.
#[test]
fn test_pull_fetches_before_editor() {
	let ctx = TestContext::new("");

	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote body from github");

	// Set up: local file exists, remote has different content
	let issue_path = ctx.setup_issue(OWNER, REPO, NUMBER, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

	// --pull should fetch from GitHub before opening editor
	let (status, stdout, stderr) = ctx.open(&issue_path).args(&["--pull"]).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed with --pull. stderr: {stderr}");

	// Should show fetch activity
	assert!(
		stdout.contains("Fetching") || stdout.contains("Pulling"),
		"Should show fetch/pull activity with --pull. stdout: {stdout}"
	);
}

/// --pull with diverged state should trigger conflict resolution (or auto-resolve).
#[test]
fn test_pull_with_divergence_runs_sync_before_editor() {
	let ctx = TestContext::new("");

	let consensus = issue("Test Issue", "consensus body");
	let local = issue("Test Issue", "local diverged body");
	let remote = issue("Test Issue", "remote diverged body");

	// Set up: both local and remote diverged from consensus
	let issue_path = ctx.setup_issue_with_local_changes(OWNER, REPO, NUMBER, &consensus, &local);
	ctx.setup_remote(OWNER, REPO, NUMBER, &remote);

	// --pull should attempt to sync/merge BEFORE editor opens
	let (_status, stdout, stderr) = ctx.open(&issue_path).args(&["--pull"]).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	// Should either succeed (auto-resolved) or fail with conflict
	// But importantly, it should attempt sync BEFORE editor
	assert!(
		stdout.contains("Merging") || stdout.contains("Conflict") || stderr.contains("Conflict") || stdout.contains("Pulling"),
		"Should attempt sync/merge with --pull before editor. stdout: {stdout}, stderr: {stderr}"
	);
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

/// Sub-issues closed as duplicates should NOT appear in the pulled remote state.
/// GitHub marks these with state_reason="duplicate" - they should be filtered out entirely.
#[test]
fn test_duplicate_sub_issues_filtered_from_remote() {
	let ctx = TestContext::new("");
	ctx.init_git();

	// Create issues with proper CloseState
	let parent = parse("- [ ] Parent Issue <!-- https://github.com/testowner/testrepo/issues/1 -->\n\tparent body\n");

	let mut normal_closed = parse("- [x] Normal Closed Sub <!-- https://github.com/testowner/testrepo/issues/2 -->\n\tsub body\n");
	normal_closed.meta.close_state = todo::CloseState::Closed;

	let mut duplicate = parse("- [x] Duplicate Sub <!-- https://github.com/testowner/testrepo/issues/3 -->\n\tduplicate body\n");
	duplicate.meta.close_state = todo::CloseState::Duplicate(2); // duplicate of #2

	// Set up remote with parent and two sub-issues
	ctx.remote()
		.issue(OWNER, REPO, 1, &parent)
		.sub_issue(OWNER, REPO, 1, 2, &normal_closed)
		.sub_issue(OWNER, REPO, 1, 3, &duplicate)
		.build();

	// Open via URL to fetch from remote
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, 1).run();

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(status.success(), "Should succeed. stderr: {stderr}");

	// Check the created file - duplicate sub-issue should NOT be present
	let issue_path = ctx.dir_issue_path(OWNER, REPO, 1, "Parent Issue");
	let content = std::fs::read_to_string(&issue_path).unwrap();

	eprintln!("File content:\n{content}");

	// Normal closed sub-issue should appear
	assert!(content.contains("Normal Closed Sub"), "Normal closed sub-issue should appear. Got: {content}");
	assert!(content.contains("[x]"), "Normal closed sub should show as [x]. Got: {content}");

	// Duplicate sub-issue should NOT appear at all
	assert!(
		!content.contains("Duplicate Sub"),
		"Duplicate sub-issue should NOT appear in local representation. Got: {content}"
	);
}

/// Opening an issue twice when local matches remote should succeed (no-op).
/// This tests the case where you:
/// 1. Open an issue from URL (fetches remote)
/// 2. Open again without making changes
/// The second open should succeed, not fail with "Failed to commit remote state".
#[test]
fn test_open_unchanged_succeeds() {
	let ctx = TestContext::new("");
	ctx.init_git();

	let issue = parse("- [ ] Test Issue <!-- https://github.com/testowner/testrepo/issues/1 -->\n\tissue body\n");

	// Set up remote with just the issue
	ctx.remote().issue(OWNER, REPO, 1, &issue).build();

	// First open via URL
	let (status, _stdout, stderr) = ctx.open_url(OWNER, REPO, 1).run();
	assert!(status.success(), "First open should succeed. stderr: {stderr}");

	// Second open - should also succeed (no-op since nothing changed)
	let issue_path = ctx.flat_issue_path(OWNER, REPO, 1, "Test Issue");
	let (status, _stdout, stderr) = ctx.open(&issue_path).run();
	assert!(status.success(), "Second open (unchanged) should succeed. stderr: {stderr}");
}

/// Opening an issue by number when remote state matches local should succeed.
/// Reproduces: https://github.com/valeratrades/todo/issues/83
/// The issue happens when:
/// 1. `todo open --reset <url>` fetches and stores remote state
/// 2. `todo open <number>` is called (by number, not path)
/// 3. Remote state hasn't changed, but the merge machinery still runs
/// 4. Git commit fails because there's nothing to commit
#[test]
fn test_open_by_number_unchanged_succeeds() {
	let ctx = TestContext::new("");
	ctx.init_git();

	let issue = parse("- [ ] Test Issue <!-- https://github.com/testowner/testrepo/issues/1 -->\n\tissue body\n");

	// Set up remote
	ctx.remote().issue(OWNER, REPO, 1, &issue).build();

	// First open via URL with --reset
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, 1).args(&["--reset"]).run();
	eprintln!("First open stdout: {stdout}");
	eprintln!("First open stderr: {stderr}");
	assert!(status.success(), "First open should succeed. stderr: {stderr}");

	// Second open by number (simulating the failing case)
	// This uses the mock, so remote state is the same
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, 1).run();
	eprintln!("Second open stdout: {stdout}");
	eprintln!("Second open stderr: {stderr}");
	assert!(status.success(), "Second open (unchanged) should succeed. stderr: {stderr}");
}
