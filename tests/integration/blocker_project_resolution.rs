//! Integration tests for blocker project resolution and switching.
//!
//! These tests verify project resolution by checking the resulting state
//! (current_project.txt and file contents) rather than stdout/stderr messages.

use crate::fixtures::TodoTestContext;

#[test]
fn test_exact_match_with_extension_skips_fzf() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/uni.md
		- task for uni
		//- /data/blockers/uni_headless.md
		- task for uni_headless
	"#,
	);

	// "uni.md" should match exactly to uni.md, not uni_headless.md
	let output = ctx.run(&["blocker", "set-project", "uni.md"]);
	assert!(output.status.success());

	assert_eq!(ctx.read_current_project(), Some("uni.md".to_string()));
}

#[test]
fn test_unique_pattern_without_extension_matches_directly() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/project_alpha.md
		- task for alpha
		//- /data/blockers/project_beta.md
		- task for beta
	"#,
	);

	// "alpha" should match uniquely to project_alpha.md
	let output = ctx.run(&["blocker", "set-project", "alpha"]);
	assert!(output.status.success());

	assert_eq!(ctx.read_current_project(), Some("project_alpha.md".to_string()));
}

#[test]
fn test_exact_match_in_workspace() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/work/uni.md
		- task for work uni
		//- /data/blockers/work/uni_headless.md
		- task for work uni_headless
	"#,
	);

	// "uni.md" should match the exact filename even in workspace
	let output = ctx.run(&["blocker", "set-project", "uni.md"]);
	assert!(output.status.success());

	assert_eq!(ctx.read_current_project(), Some("work/uni.md".to_string()));
}

#[test]
fn test_set_project_cannot_switch_away_from_urgent() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/work/urgent.md
		- urgent task
		//- /data/blockers/work/normal.md
		- normal task
	"#,
	);

	// First set project to urgent
	let output = ctx.run(&["blocker", "set-project", "work/urgent.md"]);
	assert!(output.status.success());
	assert_eq!(ctx.read_current_project(), Some("work/urgent.md".to_string()));

	// Now try to switch away from urgent - should be blocked
	let output = ctx.run(&["blocker", "set-project", "work/normal.md"]);
	assert!(output.status.success());

	// Project should still be urgent - switch was blocked
	assert_eq!(ctx.read_current_project(), Some("work/urgent.md".to_string()));
}

#[test]
fn test_set_project_can_switch_between_urgent_files() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/work/urgent.md
		- work urgent task
		//- /data/blockers/personal/urgent.md
		- personal urgent task
	"#,
	);

	// First set project to work urgent
	let output = ctx.run(&["blocker", "set-project", "work/urgent.md"]);
	assert!(output.status.success());
	assert_eq!(ctx.read_current_project(), Some("work/urgent.md".to_string()));

	// Should be able to switch to personal urgent
	let output = ctx.run(&["blocker", "set-project", "personal/urgent.md"]);
	assert!(output.status.success());
	assert_eq!(ctx.read_current_project(), Some("personal/urgent.md".to_string()));
}

#[test]
fn test_can_add_to_same_urgent_file() {
	let ctx = TodoTestContext::new(
		r#"
		//- /data/blockers/work/urgent.md
		- existing urgent task
		//- /data/blockers/work/normal.md
		- normal task
	"#,
	);

	// Set the current project to something in the same workspace
	let output = ctx.run(&["blocker", "set-project", "work/normal.md"]);
	assert!(output.status.success());

	// Adding to urgent should work because work/urgent.md already exists
	let output = ctx.run(&["blocker", "add", "--urgent", "another urgent task"]);
	assert!(output.status.success());

	// Verify the task was added to the urgent file
	let urgent_content = ctx.read("/blockers/work/urgent.md");
	assert!(urgent_content.contains("another urgent task"));
}
