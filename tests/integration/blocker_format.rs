//! Integration tests for blocker file formatting.

use std::{fs, thread, time::Duration};

use rstest::rstest;

use crate::fixtures::{BlockerFormatContext, TestContext, blocker_md_ctx, blocker_typ_ctx, ctx};

// ============================================================================
// Markdown (.md) file tests
// ============================================================================

#[rstest]
fn test_blocker_format_adds_spaces_md(blocker_md_ctx: BlockerFormatContext) {
	blocker_md_ctx.run_format().expect("Format command should succeed");

	let formatted = blocker_md_ctx.read_blocker_file();
	insta::assert_snapshot!(formatted, @"
	- move these todos over into a persisted directory
		comment
	- move all typst projects
	- rewrite custom.sh
		comment

	# marketmonkey
	- go in-depth on possibilities

	# SocialNetworks in rust
	- test twitter

	## yt
	- test

	# math tools
	## gauss
	- finish it
			a space-indented comment comment
	- move gaussian pivot over in there
		   another space-indented comment

	# git lfs: docs, music, etc

	# eww: don't restore if outdated

	# todo: blocker: doesn't add spaces between same level headers
	");
}

#[rstest]
fn test_blocker_format_idempotent_md(blocker_md_ctx: BlockerFormatContext) {
	// Run format command first time
	blocker_md_ctx.run_format().expect("First format command should succeed");
	let formatted_once = blocker_md_ctx.read_blocker_file();

	// Run format command second time (simulating open and close)
	blocker_md_ctx.run_format().expect("Second format command should succeed");
	let formatted_twice = blocker_md_ctx.read_blocker_file();

	// Should be idempotent
	assert_eq!(formatted_once, formatted_twice, "Formatting should be idempotent");
}

// ============================================================================
// Typst (.typ) file tests
// ============================================================================

#[rstest]
fn test_blocker_format_typst_headings(blocker_typ_ctx: BlockerFormatContext) {
	blocker_typ_ctx.run_format().expect("Format command should succeed");

	let formatted = blocker_typ_ctx.read_formatted_file();
	insta::assert_snapshot!(formatted, @"
	# marketmonkey
	- go in-depth on possibilities

	# SocialNetworks in rust
	- test twitter

	## yt
	- test

	# math tools
	## gauss
	- finish it
	- move gaussian pivot over in there

	# git lfs: docs, music, etc

	# eww: don't restore if outdated

	# todo: blocker: test typst support
	");
}

#[rstest]
fn test_blocker_format_converts_typst_to_md(blocker_typ_ctx: BlockerFormatContext) {
	// Run format command - should convert .typ to .md
	blocker_typ_ctx.run_format().expect("First format command should succeed");

	// The original .typ file should no longer exist
	assert!(!blocker_typ_ctx.blocker_file.exists(), "Original .typ file should be removed");

	// A new .md file should exist
	let md_file = blocker_typ_ctx.blocker_file.with_extension("md");
	assert!(md_file.exists(), "Converted .md file should exist");

	let formatted = fs::read_to_string(&md_file).unwrap();
	insta::assert_snapshot!(formatted, @"
	# marketmonkey
	- go in-depth on possibilities

	# SocialNetworks in rust
	- test twitter

	## yt
	- test

	# math tools
	## gauss
	- finish it
	- move gaussian pivot over in there

	# git lfs: docs, music, etc

	# eww: don't restore if outdated

	# todo: blocker: test typst support
	");
}

/// Test that sub-issues with NOTE: lines in their body are preserved correctly.
/// This reproduces a bug where adding sub-issues with NOTE: lines gets mangled.
#[rstest]
fn test_sub_issue_with_note_line_preserved(ctx: TestContext) {
	// This is the exact content from issue 46 that was causing problems
	let content = "- [ ] v2_interface <!--https://github.com/owner/repo/issues/46-->
\tDescription of the v2 interface.

\t- [ ] `!s` shortcut to have blockers set on the block we added it in <!--sub https://github.com/owner/repo/issues/57-->
\t\tNOTE: error if outside (tell the user what's wrong with miette, wait for him to fix, - we don't try to be smart and \"fix\" it)
\t- [ ] --last flag on open
\t\tgoes to last accessed
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
      "original_issue_body": "Description of the v2 interface.",
      "original_comments": [],
      "original_sub_issues": [
        {"number": 57, "state": "open"}
      ],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	ctx.write_meta("owner", "repo", meta_content);

	// Set up mock GitHub state - including sub_issues relationships
	let mock_state = r#"{
  "issues": [
    {
      "owner": "owner",
      "repo": "repo",
      "number": 46,
      "title": "v2_interface",
      "body": "Description of the v2 interface.",
      "state": "open"
    },
    {
      "owner": "owner",
      "repo": "repo",
      "number": 57,
      "title": "`!s` shortcut to have blockers set on the block we added it in",
      "body": "NOTE: error if outside (tell the user what's wrong with miette, wait for him to fix, - we don't try to be smart and \"fix\" it)",
      "state": "open"
    }
  ],
  "sub_issues": [
    {
      "owner": "owner",
      "repo": "repo",
      "parent": 46,
      "children": [57]
    }
  ]
}"#;
	ctx.write_mock_state(mock_state);

	let child = ctx.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	ctx.signal_editor_close();

	let (stdout, stderr, success) = ctx.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = ctx.read_issue_file(&issue_file);

	// The NOTE: line MUST be preserved in the sub-issue body
	assert!(final_content.contains("NOTE: error if outside"), "NOTE: line was lost! Final content:\n{final_content}");

	// The second sub-issue should only appear once (not duplicated)
	// Note: "--last flag on open" has no URL, so it's a NEW sub-issue that should be created
	let count = final_content.matches("--last flag on open").count();
	assert_eq!(count, 1, "Sub-issue '--last flag on open' was duplicated {count} times! Final content:\n{final_content}");
}

/// Test that new sub-issues (without URLs) are created exactly once.
/// This reproduces a bug where sub-issues without URLs get created multiple times.
#[rstest]
fn test_new_sub_issue_created_once(ctx: TestContext) {
	// Start with parent issue and one new sub-issue (no URL)
	let content = "- [ ] v2_interface <!--https://github.com/owner/repo/issues/46-->
\tDescription of the v2 interface.

\t- [ ] `!s` shortcut
\t\tSome description
\t- [ ] --last flag on open
\t\tgoes to last accessed
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
      "original_issue_body": "Description of the v2 interface.",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	ctx.write_meta("owner", "repo", meta_content);

	// Set up mock GitHub state with parent issue
	let mock_state = r#"{
  "issues": [
    {
      "owner": "owner",
      "repo": "repo",
      "number": 46,
      "title": "v2_interface",
      "body": "Description of the v2 interface.",
      "state": "open"
    }
  ]
}"#;
	ctx.write_mock_state(mock_state);

	let child = ctx.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	ctx.signal_editor_close();

	let (stdout, stderr, success) = ctx.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = ctx.read_issue_file(&issue_file);

	// Both sub-issues should have URLs now (they were created)
	assert!(
		final_content.contains("<!--sub https://github.com/owner/repo/issues/"),
		"Sub-issues should have been created with URLs. Final content:\n{final_content}"
	);

	// Each sub-issue should appear exactly once
	let count_s = final_content.matches("`!s` shortcut").count();
	assert_eq!(count_s, 1, "Sub-issue '!s shortcut' appears {count_s} times. Final content:\n{final_content}");

	let count_last = final_content.matches("--last flag on open").count();
	assert_eq!(count_last, 1, "Sub-issue '--last flag on open' appears {count_last} times. Final content:\n{final_content}");
}

#[rstest]
#[case("= Project\n- task 1\n- task 2\n+ numbered item 1\n+ numbered item 2", "test_lists.typ")]
fn test_blocker_format_typst_lists(#[case] content: &str, #[case] filename: &str) {
	let ctx = crate::fixtures::blocker_format_ctx_custom(content, filename);
	ctx.run_format().expect("Format command should succeed");

	let formatted = ctx.read_formatted_file();
	insta::assert_snapshot!(formatted, @"
	# Project
	- task 1
	- task 2
	- numbered item 1
	- numbered item 2
	");
}

#[rstest]
#[case("= Main Project\n- first task\n\n== Subproject\n- subtask 1\n- subtask 2\n\n= Another Project\n- another task", "test_mixed.typ")]
fn test_blocker_format_typst_mixed_content(#[case] content: &str, #[case] filename: &str) {
	let ctx = crate::fixtures::blocker_format_ctx_custom(content, filename);
	ctx.run_format().expect("Format command should succeed");

	let formatted = ctx.read_formatted_file();
	insta::assert_snapshot!(formatted, @"
	# Main Project
	- first task

	## Subproject
	- subtask 1
	- subtask 2

	# Another Project
	- another task
	");
}

#[rstest]
#[case(
	r#"- switch to `curswant` and `curspos` for setting the jump target, with preservation of `curswant`
	```rs
	 // getcurpos() returns [bufnum, lnum, col, off, curswant]
	  let curpos: Vec<i64> = api::call_function("getcurpos", ()).unwrap_or_default();
	  let curswant = curpos.get(4).copied().unwrap_or(0);

	  // cursor(line, col, off, curswant) - col=0 means use curswant
	  let _ = api::call_function::<_, ()>("cursor", (target_line, 0, 0, curswant));
	```"#,
	"test_code_block.md"
)]
fn test_blocker_format_comment_with_code_block(#[case] content: &str, #[case] filename: &str) {
	// Test that code blocks within comments can contain blank lines
	let ctx = crate::fixtures::blocker_format_ctx_custom(content, filename);
	ctx.run_format().expect("Format command should succeed - code blocks can have blank lines");
}
