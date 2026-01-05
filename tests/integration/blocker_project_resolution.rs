//! Integration tests for blocker project resolution and switching.
//!
//! These tests verify project resolution by checking the resulting state
//! (current_project.txt and file contents) rather than stdout/stderr messages.

use rstest::rstest;

use crate::fixtures::{BlockerProjectContext, blocker_project_ctx};

#[rstest]
fn test_exact_match_with_extension_skips_fzf(blocker_project_ctx: BlockerProjectContext) {
	// Create two files where one is a prefix of the other
	blocker_project_ctx.create_blocker_file("uni.md", "- task for uni");
	blocker_project_ctx.create_blocker_file("uni_headless.md", "- task for uni_headless");

	// "uni.md" should match exactly to uni.md, not uni_headless.md
	let output = blocker_project_ctx.run_set_project("uni.md");
	assert!(output.status.success());

	assert_eq!(blocker_project_ctx.read_current_project(), Some("uni.md".to_string()));
}

#[rstest]
fn test_unique_pattern_without_extension_matches_directly(blocker_project_ctx: BlockerProjectContext) {
	// Create files with distinct names
	blocker_project_ctx.create_blocker_file("project_alpha.md", "- task for alpha");
	blocker_project_ctx.create_blocker_file("project_beta.md", "- task for beta");

	// "alpha" should match uniquely to project_alpha.md
	let output = blocker_project_ctx.run_set_project("alpha");
	assert!(output.status.success());

	assert_eq!(blocker_project_ctx.read_current_project(), Some("project_alpha.md".to_string()));
}

#[rstest]
fn test_exact_match_in_workspace(blocker_project_ctx: BlockerProjectContext) {
	// Create files in a workspace subdirectory
	blocker_project_ctx.create_blocker_file("work/uni.md", "- task for work uni");
	blocker_project_ctx.create_blocker_file("work/uni_headless.md", "- task for work uni_headless");

	// "uni.md" should match the exact filename even in workspace
	let output = blocker_project_ctx.run_set_project("uni.md");
	assert!(output.status.success());

	assert_eq!(blocker_project_ctx.read_current_project(), Some("work/uni.md".to_string()));
}

#[rstest]
fn test_set_project_cannot_switch_away_from_urgent(blocker_project_ctx: BlockerProjectContext) {
	// Create workspace urgent and regular project files
	blocker_project_ctx.create_blocker_file("work/urgent.md", "- urgent task");
	blocker_project_ctx.create_blocker_file("work/normal.md", "- normal task");

	// First set project to urgent
	let output = blocker_project_ctx.run_set_project("work/urgent.md");
	assert!(output.status.success());
	assert_eq!(blocker_project_ctx.read_current_project(), Some("work/urgent.md".to_string()));

	// Now try to switch away from urgent - should be blocked
	let output = blocker_project_ctx.run_set_project("work/normal.md");
	assert!(output.status.success());

	// Project should still be urgent - switch was blocked
	assert_eq!(blocker_project_ctx.read_current_project(), Some("work/urgent.md".to_string()));
}

#[rstest]
fn test_set_project_can_switch_between_urgent_files(blocker_project_ctx: BlockerProjectContext) {
	// Create two workspace urgent files
	blocker_project_ctx.create_blocker_file("work/urgent.md", "- work urgent task");
	blocker_project_ctx.create_blocker_file("personal/urgent.md", "- personal urgent task");

	// First set project to work urgent
	let output = blocker_project_ctx.run_set_project("work/urgent.md");
	assert!(output.status.success());
	assert_eq!(blocker_project_ctx.read_current_project(), Some("work/urgent.md".to_string()));

	// Should be able to switch to personal urgent
	let output = blocker_project_ctx.run_set_project("personal/urgent.md");
	assert!(output.status.success());
	assert_eq!(blocker_project_ctx.read_current_project(), Some("personal/urgent.md".to_string()));
}

#[rstest]
fn test_can_add_to_same_urgent_file(blocker_project_ctx: BlockerProjectContext) {
	// Create one workspace urgent file already
	blocker_project_ctx.create_blocker_file("work/urgent.md", "- existing urgent task\n");

	// Set the current project to something in the same workspace
	blocker_project_ctx.create_blocker_file("work/normal.md", "- normal task");
	let output = blocker_project_ctx.run_set_project("work/normal.md");
	assert!(output.status.success());

	// Adding to urgent should work because work/urgent.md already exists
	let output = blocker_project_ctx.run_add_urgent("another urgent task");
	assert!(output.status.success());

	// Verify the task was added to the urgent file
	let urgent_content = blocker_project_ctx.read_blocker_file("work/urgent.md");
	assert!(urgent_content.contains("another urgent task"));
}
