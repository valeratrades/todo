//! Integration tests for open command's sub-issue handling.
//!
//! These tests verify that sub-issue content is preserved correctly through
//! the parse → edit → serialize → sync cycle.
//!
//! The tests use a pipe-based mock mechanism:
//! 1. Create a named pipe (FIFO)
//! 2. Spawn the binary with TODO_MOCK_PIPE env var pointing to the pipe
//! 3. The binary waits for a signal on the pipe instead of opening an editor
//! 4. Test modifies the file while binary is waiting
//! 5. Test writes to the pipe to signal "editor closed"
//! 6. Binary continues and syncs changes

use std::{fs, thread, time::Duration};

use rstest::rstest;

use crate::fixtures::{TestContext, ctx};

/// Test that multiple sub-issues (open and closed) are all preserved.
#[rstest]
fn test_multiple_sub_issues_preserved(ctx: TestContext) {
	let content = "- [ ] Complex Parent <!--https://github.com/owner/repo/issues/100-->
\tThe parent body.

\t- [ ] Open sub-issue 1 <!--sub https://github.com/owner/repo/issues/101-->
\t\tContent of open sub 1
\t- [x] Closed sub-issue 1 <!--sub https://github.com/owner/repo/issues/102-->
\t\t<!-- omitted -->
\t- [ ] Open sub-issue 2 <!--sub https://github.com/owner/repo/issues/103-->
\t\tContent of open sub 2
\t- [x] Closed sub-issue 2 <!--sub https://github.com/owner/repo/issues/104-->
\t\t<!-- omitted -->
";

	let issue_file = ctx.write_issue_file("owner", "repo", "100_-_Complex_Parent.md", content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "100": {
      "issue_number": 100,
      "title": "Complex Parent",
      "extension": "md",
      "original_issue_body": "The parent body.",
      "original_comments": [],
      "original_sub_issues": [
        {"number": 101, "state": "open"},
        {"number": 102, "state": "closed"},
        {"number": 103, "state": "open"},
        {"number": 104, "state": "closed"}
      ],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	ctx.write_meta("owner", "repo", meta_content);

	let child = ctx.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	ctx.signal_editor_close();

	let (stdout, stderr, success) = ctx.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = ctx.read_issue_file(&issue_file);
	insta::assert_snapshot!(final_content, @"
	- [ ] Complex Parent <!-- https://github.com/owner/repo/issues/100 -->
		The parent body.
		
		- [ ] Open sub-issue 1 <!--sub https://github.com/owner/repo/issues/101 -->
			Content of open sub 1
		
		- [x] Closed sub-issue 1 <!--sub https://github.com/owner/repo/issues/102 -->
			<!-- omitted -->
		
		- [ ] Open sub-issue 2 <!--sub https://github.com/owner/repo/issues/103 -->
			Content of open sub 2
		
		- [x] Closed sub-issue 2 <!--sub https://github.com/owner/repo/issues/104 -->
			<!-- omitted -->
	");
}

/// Test that adding blockers during edit are preserved.
/// This reproduces a bug where blockers added during editing get lost.
#[rstest]
fn test_adding_blockers_during_edit_are_preserved(ctx: TestContext) {
	// Start with simple issue without blockers
	let initial_content = "- [ ] blocker rewrite <!--https://github.com/owner/repo/issues/49-->
\tget all the present functionality + legacy supported
";

	let issue_file = ctx.write_issue_file("owner", "repo", "49_-_blocker_rewrite.md", initial_content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "49": {
      "issue_number": 49,
      "title": "blocker rewrite",
      "extension": "md",
      "original_issue_body": "get all the present functionality + legacy supported",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	ctx.write_meta("owner", "repo", meta_content);

	// Set up mock GitHub state so the issue exists and can be updated
	let mock_state = r#"{
  "issues": [
    {
      "owner": "owner",
      "repo": "repo",
      "number": 49,
      "title": "blocker rewrite",
      "body": "get all the present functionality + legacy supported",
      "state": "open"
    }
  ]
}"#;
	ctx.write_mock_state(mock_state);

	let child = ctx.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));

	// Add blockers during "editing"
	let modified_content = "- [ ] blocker rewrite <!--https://github.com/owner/repo/issues/49-->
\tget all the present functionality + legacy supported
\t<!--blockers-->
\t- support for virtual blockers
\t- move all primitives into new blocker.rs
";
	fs::write(&issue_file, modified_content).unwrap();

	ctx.signal_editor_close();

	let (stdout, stderr, success) = ctx.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = ctx.read_issue_file(&issue_file);
	insta::assert_snapshot!(final_content, @"
	- [ ] blocker rewrite <!-- https://github.com/owner/repo/issues/49 -->
		get all the present functionality + legacy supported
		
		# Blockers
		- support for virtual blockers
		- move all primitives into new blocker.rs
	");
}

/// Test that closed sub-issues have their content folded to <!-- omitted -->.
#[rstest]
fn test_closed_sub_issues_content_folded(ctx: TestContext) {
	// Start with expanded content for the closed sub-issue
	let content = "- [ ] v2_interface <!--https://github.com/owner/repo/issues/46-->
\tMain issue body here.

\t- [x] Completed task <!--sub https://github.com/owner/repo/issues/77-->
\t\tThis task was done.
\t\tHere are the details.
\t- [ ] In-progress task <!--sub https://github.com/owner/repo/issues/78-->
\t\tDescription of the current task
\t\tWith some implementation notes
";

	let issue_file = ctx.write_issue_file("owner", "repo", "46_-_v2_interface.md", content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "46": {
      "issue_number": 46,
      "title": "v2_interface",
      "extension": "md",
      "original_issue_body": "Main issue body here.",
      "original_comments": [],
      "original_sub_issues": [
        {"number": 77, "state": "closed"},
        {"number": 78, "state": "open"}
      ],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	ctx.write_meta("owner", "repo", meta_content);

	let child = ctx.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	ctx.signal_editor_close();

	let (stdout, stderr, success) = ctx.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = ctx.read_issue_file(&issue_file);
	insta::assert_snapshot!(final_content, @"
	- [ ] v2_interface <!-- https://github.com/owner/repo/issues/46 -->
		Main issue body here.
		
		- [x] Completed task <!--sub https://github.com/owner/repo/issues/77 -->
			<!-- omitted -->
		
		- [ ] In-progress task <!--sub https://github.com/owner/repo/issues/78 -->
			Description of the current task
			With some implementation notes
	");
}
