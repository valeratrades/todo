//! Integration tests for blocker project resolution and switching.
//!
//! These tests verify project resolution by checking the resulting state
//! (current_project.txt and file contents) rather than stdout/stderr messages.

use crate::common::TestContext;

#[test]
fn test_unique_pattern_without_extension_matches_directly() {
	let ctx = TestContext::new(
		r#"
		//- /data/blockers/a.md
		- task
		//- /data/blockers/b.md
		- task
	"#,
	);

	// "a" should match uniquely to a.md
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", "a"]);
	assert!(status.success());

	assert_eq!(ctx.read_current_project(), Some("a.md".to_string()));
}

#[test]
fn test_exact_match_in_workspace() {
	let target_fname = "a.md";
	let ctx = TestContext::new(&format!(
		"
		//- /data/blockers/A/{target_fname}
		- task a on project A
		//- /data/blockers/A/b.md
		- task b on project A
	"
	));

	// should match the exact filename even in workspace
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", target_fname]);
	assert!(status.success());

	assert_eq!(ctx.read_current_project(), Some(format!("A/{target_fname}")));
}

#[test]
fn test_set_project_cannot_switch_away_from_urgent() {
	let ctx = TestContext::new(
		r#"
		//- /data/blockers/A/urgent.md
		- urgent task
		//- /data/blockers/A/a.md
		- normal task
	"#,
	);

	// First set project to urgent
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", "A/urgent.md"]);
	assert!(status.success());
	assert_eq!(ctx.read_current_project(), Some("A/urgent.md".to_string()));

	// Now try to switch away from urgent - should be blocked
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", "A/a.md"]);
	assert!(status.success());

	// Project should still be urgent - switch was blocked
	assert_eq!(ctx.read_current_project(), Some("A/urgent.md".to_string()));
}

#[test]
fn test_set_project_can_switch_between_urgent_files() {
	let ctx = TestContext::new(
		r#"
		//- /data/blockers/A/urgent.md
		- task
		//- /data/blockers/B/urgent.md
		- task
	"#,
	);

	// First set project to A urgent
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", "A/urgent.md"]);
	assert!(status.success());
	assert_eq!(ctx.read_current_project(), Some("A/urgent.md".to_string()));

	// Should be able to switch to B urgent
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", "B/urgent.md"]);
	assert!(status.success());
	assert_eq!(ctx.read_current_project(), Some("B/urgent.md".to_string()));
}

#[test]
fn test_can_add_to_same_urgent_file() {
	let ctx = TestContext::new(
		r#"
		//- /data/blockers/A/urgent.md
		- existing urgent task
		//- /data/blockers/A/a.md
		- normal task
	"#,
	);

	// Set the current project to something in the same workspace
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "set", "A/a.md"]);
	assert!(status.success());

	// Adding to urgent should work because A/urgent.md already exists
	let (status, _, _) = ctx.run(&["blocker", "--individual-files", "add", "--urgent", "another urgent task"]);
	assert!(status.success());

	// Verify the task was added to the urgent file
	let urgent_content = ctx.read("/blockers/A/urgent.md");
	assert!(urgent_content.contains("another urgent task"));
}
