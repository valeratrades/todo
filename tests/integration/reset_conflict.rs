//! Test for issue #46 bug: --reset should not trigger conflicts when marking sub-issues.
//!
//! The root cause: when format_issue runs during --reset, if sub-issue files already
//! exist locally (from a previous fetch), it embeds the LOCAL content into the parent
//! file instead of the GitHub API content. This causes the consensus to differ from
//! what fetch_full_issue_tree returns, triggering false conflicts.
//!
//! Scenario that triggers the bug:
//! 1. Fetch issue once (creates sub-issue files with original content)
//! 2. Locally edit sub-issue files (add blockers, expand body, etc.)
//! 3. Run `--reset` on the parent issue
//! 4. format_issue embeds LOCAL sub-issue content (not GitHub API content)
//! 5. User makes a small change (e.g., mark sub-issue as closed)
//! 6. Post-editor sync fetches remote (gets GitHub API content for sub-issues)
//! 7. Consensus (with local content) != Remote (with API content) → FALSE CONFLICT

use todo::{CloseState, Issue, ParseContext};

use crate::common::{TestContext, git::GitExt};

const OWNER: &str = "testowner";
const REPO: &str = "testrepo";

fn parse(content: &str) -> Issue {
	let ctx = ParseContext::new(content.to_string(), "test.md".to_string());
	Issue::parse(content, &ctx).expect("failed to parse test issue")
}

/// Scenario from issue #46:
/// 1. Run `todo open --reset <url>` to fetch issue with sub-issues
/// 2. Mark one sub-issue as closed
/// 3. Post-editor sync should succeed without conflict
///
/// The bug was that even though we just did --reset (making consensus = remote),
/// the post-editor sync was still detecting divergence and triggering a merge conflict.
#[test]
fn test_reset_then_mark_subissue_closed_no_conflict() {
	let ctx = TestContext::new("");
	ctx.init_git();

	// Set up remote with parent issue and one open sub-issue
	let parent = parse("- [ ] Parent Issue <!-- https://github.com/testowner/testrepo/issues/1 -->\n\tparent body\n");
	let sub = parse("- [ ] Sub Issue <!-- https://github.com/testowner/testrepo/issues/2 -->\n\tsub body\n");

	ctx.remote().issue(OWNER, REPO, 1, &parent).sub_issue(OWNER, REPO, 1, 2, &sub).build();

	// First: open via URL with --reset (this simulates `todo open --reset <url>`)
	// This fetches from remote, stores locally, and commits as consensus
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, 1).args(&["--reset"]).run();
	eprintln!("First open stdout: {stdout}");
	eprintln!("First open stderr: {stderr}");
	assert!(status.success(), "First open with --reset should succeed. stderr: {stderr}");

	// Now simulate user editing: mark the sub-issue as closed
	// The user would do this by changing `- [ ]` to `- [x]` in the editor
	let issue_path = ctx.dir_issue_path(OWNER, REPO, 1, "Parent Issue");

	let mut closed_sub = sub.clone();
	closed_sub.meta.close_state = CloseState::Closed;

	// Build the modified parent with closed sub-issue
	let mut modified_parent = parent.clone();
	modified_parent.children = vec![closed_sub];

	// Second open: this simulates opening the file, making the edit, then closing editor
	// The post-editor sync should NOT trigger a conflict because:
	// - Consensus (from --reset) = remote state
	// - Local = modified (sub-issue closed)
	// - Remote = unchanged (sub-issue still open)
	// This should be detected as LocalOnly change (only local changed since consensus)
	let (status, stdout, stderr) = ctx.open(&issue_path).edit(&modified_parent).run();
	eprintln!("Second open stdout: {stdout}");
	eprintln!("Second open stderr: {stderr}");

	// THE BUG: This was triggering "Conflict detected" even though it should be a simple LocalOnly change
	assert!(status.success(), "Second open (marking sub-issue closed) should succeed without conflict. stderr: {stderr}");
	assert!(!stderr.contains("Conflict"), "Should not mention conflict. stderr: {stderr}");
	assert!(!stdout.contains("Merging remote"), "Should not attempt merge. stdout: {stdout}");
}

/// Simpler version: just body change on a single issue (no sub-issues)
/// This establishes baseline behavior.
#[test]
fn test_reset_then_edit_body_no_conflict() {
	let ctx = TestContext::new("");
	ctx.init_git();

	let issue = parse("- [ ] Test Issue <!-- https://github.com/testowner/testrepo/issues/1 -->\n\toriginal body\n");
	ctx.remote().issue(OWNER, REPO, 1, &issue).build();

	// First: open via URL with --reset
	let (status, _stdout, stderr) = ctx.open_url(OWNER, REPO, 1).args(&["--reset"]).run();
	assert!(status.success(), "First open should succeed. stderr: {stderr}");

	// Now edit the body
	let issue_path = ctx.flat_issue_path(OWNER, REPO, 1, "Test Issue");
	let mut modified = issue.clone();
	modified.comments[0].body = "modified body".to_string();

	// Second open: edit the body
	let (status, stdout, stderr) = ctx.open(&issue_path).edit(&modified).run();
	eprintln!("Second open stdout: {stdout}");
	eprintln!("Second open stderr: {stderr}");

	assert!(status.success(), "Edit should succeed. stderr: {stderr}");
	assert!(!stderr.contains("Conflict"), "Should not have conflict. stderr: {stderr}");
}

/// THE ACTUAL BUG SCENARIO:
/// When a sub-issue has its own sub-sub-issues, it gets its own file (directory format).
/// If user modifies that sub-issue file, then does --reset on the grandparent,
/// format_issue will embed the MODIFIED local content.
///
/// 1. Fetch grandparent (creates hierarchy with sub-issue files for issues with children)
/// 2. User MODIFIES a sub-issue file (adds blockers, expands content)
/// 3. Later, user runs --reset on the GRANDPARENT issue
/// 4. format_issue embeds the MODIFIED local sub-issue content
/// 5. User makes small edit
/// 6. Post-editor sync fetches remote - gets ORIGINAL API content
/// 7. Consensus != Remote → FALSE CONFLICT
#[test]
fn test_reset_with_preexisting_modified_subissue_files() {
	let ctx = TestContext::new("");
	ctx.init_git();

	// Set up remote with 3-level hierarchy:
	// grandparent (#1) -> parent (#2) -> child (#3)
	// This ensures parent (#2) gets its own file because it has children
	let grandparent = parse("- [ ] Grandparent Issue <!-- https://github.com/testowner/testrepo/issues/1 -->\n\tgrandparent body\n");
	let parent = parse("- [ ] Parent Issue <!-- https://github.com/testowner/testrepo/issues/2 -->\n\toriginal parent body\n");
	let child = parse("- [ ] Child Issue <!-- https://github.com/testowner/testrepo/issues/3 -->\n\tchild body\n");

	ctx.remote()
		.issue(OWNER, REPO, 1, &grandparent)
		.sub_issue(OWNER, REPO, 1, 2, &parent)
		.sub_issue(OWNER, REPO, 2, 3, &child)
		.build();

	// Step 1: First fetch - creates local files
	// grandparent (#1) is at: 1_-_Grandparent_Issue/__main__.md
	// parent (#2) gets its own dir because it has children: 1_-_Grandparent_Issue/2_-_Parent_Issue/__main__.md
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, 1).run();
	eprintln!("First fetch stdout: {stdout}");
	eprintln!("First fetch stderr: {stderr}");
	assert!(status.success(), "First fetch should succeed. stderr: {stderr}");

	// Step 2: User modifies the parent sub-issue file locally (adds blockers, expands content)
	// This simulates what happens when user works on issues over time
	// Parent file is at: issues/{owner}/{repo}/1_-_Grandparent_Issue/2_-_Parent_Issue/__main__.md
	let parent_subissue_path = ctx.data_dir().join("issues/testowner/testrepo/1_-_Grandparent_Issue/2_-_Parent_Issue/__main__.md");
	eprintln!("Looking for parent file at: {parent_subissue_path:?}");

	// Verify the file exists
	if !parent_subissue_path.exists() {
		// List what files were created
		let issues_dir = ctx.data_dir().join("issues/testowner/testrepo");
		if issues_dir.exists() {
			eprintln!("Files in issues dir:");
			for entry in walkdir::WalkDir::new(&issues_dir).into_iter().filter_map(|e| e.ok()) {
				eprintln!("  {}", entry.path().display());
			}
		}
		panic!("Parent sub-issue file not found at expected path");
	}

	let modified_parent_content = "- [ ] Parent Issue <!-- https://github.com/testowner/testrepo/issues/2 -->\n\toriginal parent body\n\tADDED LOCAL CONTENT - this is only local\n\t\n\t# Blockers\n\t- local blocker task\n\t\n\t- [ ] Child Issue <!--sub https://github.com/testowner/testrepo/issues/3 -->\n\t\tchild body\n";
	std::fs::write(&parent_subissue_path, modified_parent_content).unwrap();

	// Commit the local modifications
	let issues_dir = ctx.data_dir().join("issues");
	std::process::Command::new("git").args(["-C", issues_dir.to_str().unwrap(), "add", "-A"]).status().unwrap();
	std::process::Command::new("git")
		.args(["-C", issues_dir.to_str().unwrap(), "commit", "-m", "local modifications"])
		.status()
		.unwrap();

	// Step 3: User runs --reset on the GRANDPARENT issue
	// BUG: format_issue will read the MODIFIED parent file and embed that content
	let (status, stdout, stderr) = ctx.open_url(OWNER, REPO, 1).args(&["--reset"]).run();
	eprintln!("Reset stdout: {stdout}");
	eprintln!("Reset stderr: {stderr}");
	assert!(status.success(), "Reset should succeed. stderr: {stderr}");

	// Step 4: User makes a small edit (mark child issue as closed)
	let grandparent_path = ctx.data_dir().join("issues/testowner/testrepo/1_-_Grandparent_Issue/__main__.md");

	let mut closed_child = child.clone();
	closed_child.meta.close_state = CloseState::Closed;
	let mut modified_parent_issue = parent.clone();
	modified_parent_issue.children = vec![closed_child];
	let mut modified_grandparent = grandparent.clone();
	modified_grandparent.children = vec![modified_parent_issue];

	// Step 5: Post-editor sync
	// BUG: This triggers divergence because:
	// - Consensus has parent with MODIFIED local content (from format_issue reading local file)
	// - Remote has parent with ORIGINAL API content
	// - These don't match → divergence detected → merge initiated
	let (status, stdout, stderr) = ctx.open(&grandparent_path).edit(&modified_grandparent).run();
	eprintln!("Edit stdout: {stdout}");
	eprintln!("Edit stderr: {stderr}");

	// After --reset, there should be NO merging. The consensus should match remote exactly.
	// Any change user makes should be detected as LocalOnly (user changed, remote unchanged from consensus).
	// If we see "Merging" it means --reset didn't properly reset to remote state.
	assert!(status.success(), "Edit after reset should succeed. stderr: {stderr}");
	assert!(!stderr.contains("Conflict"), "Should not have conflict. stderr: {stderr}");
	assert!(
		!stdout.contains("Merging"),
		"BUG: --reset should make consensus match remote, so no merge should be needed. stdout: {stdout}"
	);
}
