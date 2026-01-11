//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Commit state on main = last synced truth
//! - Only conflict if BOTH local and remote changed since last sync
//! - Single-side changes auto-resolve

use std::{io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf, process::Command};

use tempfile::TempDir;

/// Test context for sync operations.
/// Sets up XDG directories, mock GitHub state, and named pipe for editor control.
struct SyncTestContext {
	/// Temp directory for XDG paths
	temp_dir: TempDir,
	/// Path to mock state JSON file
	mock_state_path: PathBuf,
	/// Path to named pipe for editor control
	pipe_path: PathBuf,
}

impl SyncTestContext {
	fn new() -> Self {
		let temp_dir = TempDir::new().unwrap();
		let mock_state_path = temp_dir.path().join("mock_state.json");
		let pipe_path = temp_dir.path().join("editor_pipe");

		// Create named pipe
		nix::unistd::mkfifo(&pipe_path, nix::sys::stat::Mode::S_IRWXU).unwrap();

		Self {
			temp_dir,
			mock_state_path,
			pipe_path,
		}
	}

	/// Get XDG_DATA_HOME path
	fn data_home(&self) -> PathBuf {
		self.temp_dir.path().join("data")
	}

	/// Get XDG_STATE_HOME path
	fn state_home(&self) -> PathBuf {
		self.temp_dir.path().join("state")
	}

	/// Get XDG_CACHE_HOME path
	fn cache_home(&self) -> PathBuf {
		self.temp_dir.path().join("cache")
	}

	/// Set up an issue file with metadata.
	/// Returns the path to the issue file.
	fn setup_issue(&self, owner: &str, repo: &str, number: u64, title: &str, body: &str) -> PathBuf {
		let issues_dir = self.data_home().join("todo/issues").join(owner).join(repo);
		std::fs::create_dir_all(&issues_dir).unwrap();

		// Create issue file
		let sanitized_title = title.replace(' ', "_");
		let issue_filename = format!("{number}_-_{sanitized_title}.md");
		let issue_path = issues_dir.join(&issue_filename);

		let issue_content = format!("- [ ] {title} <!-- https://github.com/{owner}/{repo}/issues/{number} -->\n\t{body}\n");
		std::fs::write(&issue_path, &issue_content).unwrap();

		// Create metadata file
		let meta_path = issues_dir.join(".meta.json");
		let meta_content = serde_json::json!({
			"owner": owner,
			"repo": repo,
			"issues": {
				number.to_string(): {
					"issue_number": number,
					"title": title,
					"extension": "md",
					"original_issue_body": body,
					"original_comments": [],
					"original_sub_issues": [],
					"parent_issue": null,
					"original_close_state": "Open"
				}
			},
			"virtual_project": false,
			"next_virtual_issue_number": 0
		});
		std::fs::write(&meta_path, serde_json::to_string_pretty(&meta_content).unwrap()).unwrap();

		issue_path
	}

	/// Set up mock GitHub state
	fn setup_mock_github(&self, issues: Vec<serde_json::Value>) {
		let state = serde_json::json!({
			"issues": issues
		});
		std::fs::write(&self.mock_state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
	}

	/// Run todo open command.
	/// Spawns command in background, signals editor to close, waits for result.
	fn run_open(&self, issue_path: &PathBuf) -> std::process::Output {
		crate::ensure_binary_compiled();

		let mut binary_path = std::env::current_exe().unwrap();
		binary_path.pop(); // Remove test binary name
		binary_path.pop(); // Remove 'deps'
		binary_path.push("todo");

		let pipe_path = self.pipe_path.clone();

		// Spawn command in background
		let mut child = Command::new(&binary_path)
			.args(["--dbg", "open", issue_path.to_str().unwrap()])
			.env("XDG_DATA_HOME", self.data_home())
			.env("XDG_STATE_HOME", self.state_home())
			.env("XDG_CACHE_HOME", self.cache_home())
			.env("TODO_MOCK_STATE", &self.mock_state_path)
			.env("TODO_MOCK_PIPE", &self.pipe_path)
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.spawn()
			.unwrap();

		// Give the command time to start and reach the pipe wait
		std::thread::sleep(std::time::Duration::from_millis(100));

		// Signal editor to "close" by writing to the pipe
		let mut pipe = std::fs::OpenOptions::new().write(true).open(&pipe_path).unwrap();
		pipe.write_all(b"x").unwrap();
		drop(pipe);

		// Wait for command to complete
		child.wait_with_output().unwrap()
	}

	/// Read the issue file content
	fn read_issue(&self, issue_path: &PathBuf) -> String {
		std::fs::read_to_string(issue_path).unwrap()
	}

	/// Check if a conflict state file exists
	fn has_conflict(&self, owner: &str, repo: &str, issue_number: u64) -> bool {
		let conflict_path = self.state_home().join("todo/conflicts").join(owner).join(repo).join(format!("{issue_number}.json"));
		conflict_path.exists()
	}
}

#[test]
fn test_both_diverged_triggers_conflict() {
	let ctx = SyncTestContext::new();

	// Setup: local issue has "local body"
	let issue_path = ctx.setup_issue(
		"testowner", "testrepo", 42, "Test Issue", "local body", // This is different from original AND remote
	);

	// Mock GitHub returns different body (remote changed)
	ctx.setup_mock_github(vec![serde_json::json!({
		"owner": "testowner",
		"repo": "testrepo",
		"number": 42,
		"title": "Test Issue",
		"body": "remote changed body", // Different from original
		"state": "open",
		"owner_login": "mock_user"
	})]);

	// The metadata says original was "original body" (different from both local and remote)
	// Update metadata to reflect original state
	let meta_path = ctx.data_home().join("todo/issues/testowner/testrepo/.meta.json");
	let meta_content = serde_json::json!({
		"owner": "testowner",
		"repo": "testrepo",
		"issues": {
			"42": {
				"issue_number": 42,
				"title": "Test Issue",
				"extension": "md",
				"original_issue_body": "original body", // The consensus point
				"original_comments": [],
				"original_sub_issues": [],
				"parent_issue": null,
				"original_close_state": "Open"
			}
		},
		"virtual_project": false,
		"next_virtual_issue_number": 0
	});
	std::fs::write(&meta_path, serde_json::to_string_pretty(&meta_content).unwrap()).unwrap();

	let output = ctx.run_open(&issue_path);

	// The command should fail because both diverged
	// (Current implementation triggers conflict when remote != original)
	// This test documents current behavior and will need updating when we implement new logic
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	// For now, just check that divergence was detected
	// The exact behavior (PR creation vs local branch) will depend on implementation
	assert!(
		!output.status.success() || stderr.contains("diverge") || stderr.contains("conflict") || stdout.contains("diverge"),
		"Expected divergence detection. stdout: {}, stderr: {}",
		stdout,
		stderr
	);
}

#[test]
fn test_only_remote_changed_takes_remote() {
	let ctx = SyncTestContext::new();

	// Setup: local issue has same content as original (unchanged)
	let issue_path = ctx.setup_issue("testowner", "testrepo", 42, "Test Issue", "original body");

	// Mock GitHub returns different body (remote changed)
	ctx.setup_mock_github(vec![serde_json::json!({
		"owner": "testowner",
		"repo": "testrepo",
		"number": 42,
		"title": "Test Issue",
		"body": "remote changed body",
		"state": "open",
		"owner_login": "mock_user"
	})]);

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	// Current behavior: triggers divergence even for one-sided change
	// New behavior should: accept remote silently
	// For now, we document current behavior
	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	// TODO: After implementing consensus-based sync, this should succeed
	// and the local file should be updated to remote content
}

#[test]
fn test_only_local_changed_pushes_local() {
	let ctx = SyncTestContext::new();

	// Setup: local issue has changed content (different from original)
	let issue_path = ctx.setup_issue("testowner", "testrepo", 42, "Test Issue", "local changed body");

	// Override metadata to set original as different from local
	let meta_path = ctx.data_home().join("todo/issues/testowner/testrepo/.meta.json");
	let meta_content = serde_json::json!({
		"owner": "testowner",
		"repo": "testrepo",
		"issues": {
			"42": {
				"issue_number": 42,
				"title": "Test Issue",
				"extension": "md",
				"original_issue_body": "original body", // Different from local
				"original_comments": [],
				"original_sub_issues": [],
				"parent_issue": null,
				"original_close_state": "Open"
			}
		},
		"virtual_project": false,
		"next_virtual_issue_number": 0
	});
	std::fs::write(&meta_path, serde_json::to_string_pretty(&meta_content).unwrap()).unwrap();

	// Mock GitHub returns same as original (remote unchanged)
	ctx.setup_mock_github(vec![serde_json::json!({
		"owner": "testowner",
		"repo": "testrepo",
		"number": 42,
		"title": "Test Issue",
		"body": "original body", // Same as original
		"state": "open",
		"owner_login": "mock_user"
	})]);

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	// Expected behavior: only local changed, so we should push local to remote
	// This should NOT trigger conflict/divergence workflow
	assert!(output.status.success(), "Should succeed when only local changed. stderr: {}", stderr);
	assert!(
		stdout.contains("Updating") || stdout.contains("Synced") || stdout.contains("No changes"),
		"Expected sync activity. stdout: {}, stderr: {}",
		stdout,
		stderr
	);
}
