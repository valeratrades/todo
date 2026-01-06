//! Integration tests for blocker file formatting.

use rstest::{fixture, rstest};

use crate::fixtures::TodoTestContext;

/// Markdown fixture content that covers all formatting edge cases:
/// - Tasks at root level (before any header)
/// - Comments under tasks (tab-indented)
/// - H1 and H2 headers with tasks
/// - Space-indented comments (preserved as-is)
/// - Empty headers (no tasks, need spacing)
#[fixture]
fn blocker_md() -> &'static str {
	"\
- a
	comment under a
- b
- c
	comment under c

# d
- e

# f
- g

## h
- i

# j
## k
- l
		space-indented comment
- m
	   another space-indented comment

# n: empty header
# o: another empty header
# p: test spacing between same-level headers"
}

/// Typst fixture content for testing typst-to-markdown conversion.
#[fixture]
fn blocker_typ() -> &'static str {
	"\
= a
- b

= c
- d

== e
- f

= g
== h
- i
- j

= k: empty header
= l: another empty header
= m: test typst support"
}

// ============================================================================
// Markdown (.md) file tests
// ============================================================================

#[rstest]
fn test_blocker_format_adds_spaces_md(blocker_md: &str) {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.md\n{blocker_md}"));

	ctx.run_format("test.md").expect("Format command should succeed");

	let formatted = ctx.read_blocker("test.md");
	insta::assert_snapshot!(formatted, @"
	- a
		comment under a
	- b
	- c
		comment under c

	# d
	- e

	# f
	- g

	## h
	- i

	# j
	## k
	- l
			space-indented comment
	- m
		   another space-indented comment

	# n: empty header

	# o: another empty header

	# p: test spacing between same-level headers
	");
}

#[rstest]
fn test_blocker_format_idempotent_md(blocker_md: &str) {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.md\n{blocker_md}"));

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

#[rstest]
fn test_blocker_format_typst_headings(blocker_typ: &str) {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.typ\n{blocker_typ}"));

	ctx.run_format("test.typ").expect("Format command should succeed");

	// Typst files get converted to .md
	let formatted = ctx.read_blocker("test.md");
	insta::assert_snapshot!(formatted, @"
	# a
	- b

	# c
	- d

	## e
	- f

	# g
	## h
	- i
	- j

	# k: empty header

	# l: another empty header

	# m: test typst support
	");
}

#[rstest]
fn test_blocker_format_converts_typst_to_md(blocker_typ: &str) {
	let ctx = TodoTestContext::new(&format!("//- /data/blockers/test.typ\n{blocker_typ}"));

	ctx.run_format("test.typ").expect("Format command should succeed");

	// Original .typ file should be removed
	assert!(!ctx.blocker_exists("test.typ"), "Original .typ file should be removed");

	// New .md file should exist
	assert!(ctx.blocker_exists("test.md"), "Converted .md file should exist");

	let formatted = ctx.read_blocker("test.md");
	insta::assert_snapshot!(formatted, @"
	# a
	- b

	# c
	- d

	## e
	- f

	# g
	## h
	- i
	- j

	# k: empty header

	# l: another empty header

	# m: test typst support
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
