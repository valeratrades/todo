//! Integration tests that verify mock calls via tracing output.
//!
//! These tests spawn the binary with TODO_TRACE_FILE set, perform operations,
//! then verify the expected mock methods were called by examining the trace log.

use std::{fs, thread, time::Duration};

use rstest::rstest;

use crate::{
	fixtures::{TracedContext, traced_ctx},
	tracing_utils::TraceLog,
};

/// Test that modifying an issue body traces the update_issue_body call with correct arguments.
/// This test verifies:
/// 1. That tracing captures mock method calls
/// 2. That the correct owner, repo, and issue_number are logged
#[rstest]
fn test_modify_issue_body_traces_update_with_arguments(traced_ctx: TracedContext) {
	let content = "- [ ] Test Issue <!--https://github.com/testowner/testrepo/issues/42-->
\tOriginal body.
";

	let issue_file = traced_ctx.write_issue_file("testowner", "testrepo", "42_-_Test_Issue.md", content);

	let meta_content = r#"{
  "owner": "testowner",
  "repo": "testrepo",
  "issues": {
    "42": {
      "issue_number": 42,
      "title": "Test Issue",
      "extension": "md",
      "original_issue_body": "Original body.",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	traced_ctx.write_meta("testowner", "testrepo", meta_content);

	let child = traced_ctx.spawn_open_with_tracing(&issue_file);
	thread::sleep(Duration::from_millis(100));

	// Modify the issue body during "editing"
	let modified_content = "- [ ] Test Issue <!--https://github.com/testowner/testrepo/issues/42-->
\tModified body content.
";
	fs::write(&issue_file, modified_content).unwrap();

	traced_ctx.signal_editor_close();

	let (_stdout, _stderr, _success) = traced_ctx.wait_for_child(child);
	// Note: The command may fail because the mock doesn't have proper state (no issues pre-added),
	// but we can still verify that tracing captured the attempt to call the mock methods.

	let trace = TraceLog::from_file(&traced_ctx.trace_file);

	// Verify tracing captured the update_issue_body call with correct arguments
	assert_traced!(trace, "update_issue_body", "testowner", "testrepo", 42);
}
