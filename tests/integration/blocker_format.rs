//! Integration tests for blocker file formatting.

use std::fs;

use rstest::rstest;

use crate::fixtures::{BlockerFormatContext, blocker_md_ctx, blocker_typ_ctx};

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
