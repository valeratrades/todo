use std::{fs, path::PathBuf, process::Command};

use tempfile::TempDir;

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

struct TestSetup {
	_temp_dir: TempDir,
	xdg_state_home: PathBuf,
	xdg_data_home: PathBuf,
	xdg_cache_home: PathBuf,
	blockers_dir: PathBuf,
}

impl TestSetup {
	fn new() -> Self {
		let temp_dir = tempfile::tempdir().unwrap();
		let state_dir = temp_dir.path().join("state").join("todo");
		fs::create_dir_all(&state_dir).unwrap();
		let cache_dir = temp_dir.path().join("cache").join("todo");
		fs::create_dir_all(&cache_dir).unwrap();
		let blockers_dir = temp_dir.path().join("data").join("todo").join("blockers");
		fs::create_dir_all(&blockers_dir).unwrap();

		Self {
			xdg_state_home: temp_dir.path().join("state"),
			xdg_data_home: temp_dir.path().join("data"),
			xdg_cache_home: temp_dir.path().join("cache"),
			blockers_dir,
			_temp_dir: temp_dir,
		}
	}

	fn create_blocker_file(&self, filename: &str, content: &str) {
		let file_path = self.blockers_dir.join(filename);
		if let Some(parent) = file_path.parent() {
			fs::create_dir_all(parent).unwrap();
		}
		fs::write(&file_path, content).unwrap();
	}

	fn run_set_project(&self, pattern: &str) -> std::process::Output {
		Command::new(get_binary_path())
			.args(["blocker", "set-project", pattern])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("XDG_CACHE_HOME", &self.xdg_cache_home)
			.output()
			.unwrap()
	}
}

#[test]
fn test_exact_match_with_extension_skips_fzf() {
	let setup = TestSetup::new();

	// Create two files where one is a prefix of the other
	setup.create_blocker_file("uni.md", "- task for uni");
	setup.create_blocker_file("uni_headless.md", "- task for uni_headless");

	// "uni.md" should match exactly, not open fzf
	let output = setup.run_set_project("uni.md");

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(output.status.success(), "Command should succeed. stderr: {}, stdout: {}", stderr, stdout);
	assert!(stderr.contains("Found exact match: uni.md"), "Should find exact match, got: {}", stderr);
}

#[test]
fn test_pattern_without_extension_opens_fzf_on_multiple_matches() {
	let setup = TestSetup::new();

	// Create two files where one is a prefix of the other
	setup.create_blocker_file("uni.md", "- task for uni");
	setup.create_blocker_file("uni_headless.md", "- task for uni_headless");

	// "uni" (without extension) should try to open fzf since there are multiple matches
	// fzf will fail in test environment (no tty), but we can check the stderr message
	let output = setup.run_set_project("uni");

	// The command will fail because fzf can't run without tty
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(stderr.contains("Found 2 matches"), "Should find multiple matches and attempt fzf, got: {}", stderr);
}

#[test]
fn test_unique_pattern_without_extension_matches_directly() {
	let setup = TestSetup::new();

	// Create files with distinct names
	setup.create_blocker_file("project_alpha.md", "- task for alpha");
	setup.create_blocker_file("project_beta.md", "- task for beta");

	// "alpha" should match uniquely
	let output = setup.run_set_project("alpha");

	assert!(output.status.success(), "Command should succeed");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(stderr.contains("Found unique match: project_alpha.md"), "Should find unique match, got: {}", stderr);
}

#[test]
fn test_exact_match_in_workspace() {
	let setup = TestSetup::new();

	// Create files in a workspace subdirectory
	setup.create_blocker_file("work/uni.md", "- task for work uni");
	setup.create_blocker_file("work/uni_headless.md", "- task for work uni_headless");

	// "uni.md" should match the exact filename even in workspace
	let output = setup.run_set_project("uni.md");

	assert!(output.status.success(), "Command should succeed");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(stderr.contains("Found exact match: work/uni.md"), "Should find exact match in workspace, got: {}", stderr);
}
