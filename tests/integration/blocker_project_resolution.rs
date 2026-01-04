//! Integration tests for blocker project resolution and switching.

use rstest::rstest;

use crate::fixtures::{BlockerProjectContext, blocker_project_ctx};

#[rstest]
fn test_exact_match_with_extension_skips_fzf(blocker_project_ctx: BlockerProjectContext) {
	// Create two files where one is a prefix of the other
	blocker_project_ctx.create_blocker_file("uni.md", "- task for uni");
	blocker_project_ctx.create_blocker_file("uni_headless.md", "- task for uni_headless");

	// "uni.md" should match exactly, not open fzf
	let output = blocker_project_ctx.run_set_project("uni.md");

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(output.status.success(), "Command should succeed. stderr: {}, stdout: {}", stderr, stdout);
	assert!(stderr.contains("Found exact match: uni.md"), "Should find exact match, got: {}", stderr);
}

#[rstest]
fn test_unique_pattern_without_extension_matches_directly(blocker_project_ctx: BlockerProjectContext) {
	// Create files with distinct names
	blocker_project_ctx.create_blocker_file("project_alpha.md", "- task for alpha");
	blocker_project_ctx.create_blocker_file("project_beta.md", "- task for beta");

	// "alpha" should match uniquely
	let output = blocker_project_ctx.run_set_project("alpha");

	assert!(output.status.success(), "Command should succeed");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(stderr.contains("Found unique match: project_alpha.md"), "Should find unique match, got: {}", stderr);
}

#[rstest]
fn test_exact_match_in_workspace(blocker_project_ctx: BlockerProjectContext) {
	// Create files in a workspace subdirectory
	blocker_project_ctx.create_blocker_file("work/uni.md", "- task for work uni");
	blocker_project_ctx.create_blocker_file("work/uni_headless.md", "- task for work uni_headless");

	// "uni.md" should match the exact filename even in workspace
	let output = blocker_project_ctx.run_set_project("uni.md");

	assert!(output.status.success(), "Command should succeed");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(stderr.contains("Found exact match: work/uni.md"), "Should find exact match in workspace, got: {}", stderr);
}

#[rstest]
fn test_set_project_cannot_switch_away_from_urgent(blocker_project_ctx: BlockerProjectContext) {
	// Create workspace urgent and regular project files
	blocker_project_ctx.create_blocker_file("work/urgent.md", "- urgent task");
	blocker_project_ctx.create_blocker_file("work/normal.md", "- normal task");

	// First set project to urgent
	let output = blocker_project_ctx.run_set_project("work/urgent.md");
	assert!(output.status.success(), "Should set urgent project");
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(stdout.contains("Set current project to: work/urgent.md"), "Should set to work/urgent.md, got: {}", stdout);

	// Now try to switch away from urgent - should be blocked
	let output = blocker_project_ctx.run_set_project("work/normal.md");
	assert!(output.status.success(), "Command should succeed (but not switch)");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("Cannot switch away from urgent project"),
		"Should block switch from urgent, got stderr: {}",
		stderr
	);

	// Should NOT have switched - stdout should be empty (no "Set current project" message)
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(!stdout.contains("Set current project to: work/normal.md"), "Should NOT switch to work/normal.md, got: {}", stdout);
}

#[rstest]
fn test_set_project_can_switch_between_urgent_files(blocker_project_ctx: BlockerProjectContext) {
	// Create two workspace urgent files
	blocker_project_ctx.create_blocker_file("work/urgent.md", "- work urgent task");
	blocker_project_ctx.create_blocker_file("personal/urgent.md", "- personal urgent task");

	// First set project to work urgent
	let output = blocker_project_ctx.run_set_project("work/urgent.md");
	assert!(output.status.success(), "Should set work urgent project");
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(stdout.contains("Set current project to: work/urgent.md"), "Should set to work/urgent.md, got: {}", stdout);

	// Should be able to switch to personal urgent
	let output = blocker_project_ctx.run_set_project("personal/urgent.md");
	assert!(output.status.success(), "Should switch between urgent files");
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("Set current project to: personal/urgent.md"),
		"Should switch to personal/urgent.md, got: {}",
		stdout
	);
}

#[rstest]
fn test_can_add_to_same_urgent_file(blocker_project_ctx: BlockerProjectContext) {
	// Create one workspace urgent file already
	blocker_project_ctx.create_blocker_file("work/urgent.md", "- existing urgent task");

	// Set the current project to something in the same workspace
	blocker_project_ctx.create_blocker_file("work/normal.md", "- normal task");
	let output = blocker_project_ctx.run_set_project("work/normal.md");
	assert!(output.status.success(), "Should set normal project");

	// Adding to urgent should work because work/urgent.md already exists and is the target
	let output = blocker_project_ctx.run_add_urgent("another urgent task");
	assert!(
		output.status.success(),
		"Should be able to add to the existing urgent file, got: {}",
		String::from_utf8_lossy(&output.stderr)
	);
}
