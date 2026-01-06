//! Integration tests for blocker file formatting.

use crate::fixtures::{DEFAULT_BLOCKER_MD, DEFAULT_BLOCKER_TYP, TodoTestContext};

// ============================================================================
// Markdown (.md) file tests
// ============================================================================

#[test]
fn test_blocker_format_adds_spaces_md() {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.md\n{DEFAULT_BLOCKER_MD}"));

	ctx.run_format("test.md").expect("Format command should succeed");

	let formatted = ctx.read_blocker("test.md");
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

#[test]
fn test_blocker_format_idempotent_md() {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.md\n{DEFAULT_BLOCKER_MD}"));

	// Run format command first time
	ctx.run_format("test.md").expect("First format command should succeed");
	let formatted_once = ctx.read_blocker("test.md");

	// Run format command second time
	ctx.run_format("test.md").expect("Second format command should succeed");
	let formatted_twice = ctx.read_blocker("test.md");

	assert_eq!(formatted_once, formatted_twice, "Formatting should be idempotent");
}

// ============================================================================
// Typst (.typ) file tests
// ============================================================================

#[test]
fn test_blocker_format_typst_headings() {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.typ\n{DEFAULT_BLOCKER_TYP}"));

	ctx.run_format("test.typ").expect("Format command should succeed");

	// Typst files get converted to .md
	let formatted = ctx.read_blocker("test.md");
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

#[test]
fn test_blocker_format_converts_typst_to_md() {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.typ\n{DEFAULT_BLOCKER_TYP}"));

	ctx.run_format("test.typ").expect("Format command should succeed");

	// Original .typ file should be removed
	assert!(!ctx.blocker_exists("test.typ"), "Original .typ file should be removed");

	// New .md file should exist
	assert!(ctx.blocker_exists("test.md"), "Converted .md file should exist");

	let formatted = ctx.read_blocker("test.md");
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

#[test]
fn test_blocker_format_typst_lists() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/test.typ
		= Project
		- task 1
		- task 2
		+ numbered item 1
		+ numbered item 2
	"#,
	);

	ctx.run_format("test.typ").expect("Format command should succeed");

	let formatted = ctx.read_blocker("test.md");
	insta::assert_snapshot!(formatted, @"
	# Project
	- task 1
	- task 2
	- numbered item 1
	- numbered item 2
	");
}

#[test]
fn test_blocker_format_typst_mixed_content() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/test.typ
		= Main Project
		- first task

		== Subproject
		- subtask 1
		- subtask 2

		= Another Project
		- another task
	"#,
	);

	ctx.run_format("test.typ").expect("Format command should succeed");

	let formatted = ctx.read_blocker("test.md");
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

#[test]
fn test_blocker_format_comment_with_code_block() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/test.md
		- switch to `curswant` and `curspos` for setting the jump target, with preservation of `curswant`
			```rs
			 // getcurpos() returns [bufnum, lnum, col, off, curswant]
			  let curpos: Vec<i64> = api::call_function("getcurpos", ()).unwrap_or_default();
			  let curswant = curpos.get(4).copied().unwrap_or(0);

			  // cursor(line, col, off, curswant) - col=0 means use curswant
			  let _ = api::call_function::<_, ()>("cursor", (target_line, 0, 0, curswant));
			```
	"#,
	);

	// Test that code blocks within comments can contain blank lines
	ctx.run_format("test.md").expect("Format command should succeed - code blocks can have blank lines");
}
