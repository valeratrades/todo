//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Commit state on main = last synced truth
//! - Only conflict if BOTH local and remote changed since last sync
//! - Single-side changes auto-resolve

use std::{io::Write, path::PathBuf, process::Command};

use v_fixtures::{Fixture, fs_standards::xdg::Xdg};

/// Default test repository coordinates
const DEFAULT_OWNER: &str = "testowner";
const DEFAULT_REPO: &str = "testrepo";
const DEFAULT_NUMBER: u64 = 1;

/// Minimal issue representation for testing.
/// Contains only what matters for sync: title and body.
#[derive(Clone)]
struct MockIssue {
	title: String,
	body: String,
}

impl MockIssue {
	fn new(title: &str, body: &str) -> Self {
		Self {
			title: title.to_string(),
			body: body.to_string(),
		}
	}
}

/// Test context for sync operations.
/// Uses v_fixtures for XDG layout and provides helpers for mock GitHub and editor control.
struct SyncTestContext {
	/// XDG-aware fixture
	xdg: Xdg,
	/// Path to mock state JSON file
	mock_state_path: PathBuf,
	/// Path to named pipe for editor control
	pipe_path: PathBuf,
}

impl SyncTestContext {
	fn new() -> Self {
		let fixture = Fixture::parse("");
		let xdg = Xdg::new(fixture.write_to_tempdir(), "todo");

		// Create mock state file and named pipe in the temp root
		let mock_state_path = xdg.inner.root.join("mock_state.json");
		let pipe_path = xdg.inner.create_pipe("editor_pipe");

		Self { xdg, mock_state_path, pipe_path }
	}

	/// Set up a local issue file with metadata.
	/// Uses default owner/repo/number.
	/// Returns the path to the issue file.
	fn setup_local(&self, issue: &MockIssue) -> PathBuf {
		self.setup_local_full(DEFAULT_OWNER, DEFAULT_REPO, DEFAULT_NUMBER, issue)
	}

	/// Set up a local issue with explicit coordinates.
	fn setup_local_full(&self, owner: &str, repo: &str, number: u64, issue: &MockIssue) -> PathBuf {
		let issues_dir = format!("issues/{owner}/{repo}");
		let sanitized_title = issue.title.replace(' ', "_");
		let issue_filename = format!("{number}_-_{sanitized_title}.md");

		// Create issue file
		let issue_content = format!("- [ ] {} <!-- https://github.com/{owner}/{repo}/issues/{number} -->\n\t{}\n", issue.title, issue.body);
		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), &issue_content);

		// Create metadata file with same body as local (they match at "original" state)
		let meta_content = serde_json::json!({
			"owner": owner,
			"repo": repo,
			"issues": {
				number.to_string(): {
					"issue_number": number,
					"title": issue.title,
					"extension": "md",
					"original_issue_body": issue.body,
					"original_comments": [],
					"original_sub_issues": [],
					"parent_issue": null,
					"original_close_state": "Open"
				}
			},
			"virtual_project": false,
			"next_virtual_issue_number": 0
		});
		self.xdg.write_data(&format!("{issues_dir}/.meta.json"), &serde_json::to_string_pretty(&meta_content).unwrap());

		self.xdg.data_dir().join(&issues_dir).join(&issue_filename)
	}

	/// Set up mock GitHub to return a remote issue.
	/// Uses default owner/repo/number.
	fn setup_remote(&self, issue: &MockIssue) {
		self.setup_remote_full(DEFAULT_OWNER, DEFAULT_REPO, DEFAULT_NUMBER, issue);
	}

	/// Set up mock GitHub with explicit coordinates.
	fn setup_remote_full(&self, owner: &str, repo: &str, number: u64, issue: &MockIssue) {
		let state = serde_json::json!({
			"issues": [{
				"owner": owner,
				"repo": repo,
				"number": number,
				"title": issue.title,
				"body": issue.body,
				"state": "open",
				"owner_login": "mock_user"
			}]
		});
		std::fs::write(&self.mock_state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
	}

	/// Set the "original" (last synced) state in metadata.
	/// This is what both local and remote are compared against.
	fn set_original(&self, issue: &MockIssue) {
		self.set_original_full(DEFAULT_OWNER, DEFAULT_REPO, DEFAULT_NUMBER, issue);
	}

	/// Set original state with explicit coordinates.
	fn set_original_full(&self, owner: &str, repo: &str, number: u64, issue: &MockIssue) {
		let meta_content = serde_json::json!({
			"owner": owner,
			"repo": repo,
			"issues": {
				number.to_string(): {
					"issue_number": number,
					"title": issue.title,
					"extension": "md",
					"original_issue_body": issue.body,
					"original_comments": [],
					"original_sub_issues": [],
					"parent_issue": null,
					"original_close_state": "Open"
				}
			},
			"virtual_project": false,
			"next_virtual_issue_number": 0
		});
		self.xdg
			.write_data(&format!("issues/{owner}/{repo}/.meta.json"), &serde_json::to_string_pretty(&meta_content).unwrap());
	}

	/// Run todo open command.
	/// Spawns command in background, signals editor to close, waits for result.
	fn run_open(&self, issue_path: &PathBuf) -> std::process::Output {
		crate::ensure_binary_compiled();

		let mut binary_path = std::env::current_exe().unwrap();
		binary_path.pop(); // Remove test binary name
		binary_path.pop(); // Remove 'deps'
		binary_path.push("todo");

		// Build env vars from XDG
		let mut cmd = Command::new(&binary_path);
		cmd.args(["--dbg", "open", issue_path.to_str().unwrap()]);
		for (key, value) in self.xdg.env_vars() {
			cmd.env(key, value);
		}
		cmd.env("TODO_MOCK_STATE", &self.mock_state_path);
		cmd.env("TODO_MOCK_PIPE", &self.pipe_path);
		cmd.stdout(std::process::Stdio::piped());
		cmd.stderr(std::process::Stdio::piped());

		let child = cmd.spawn().unwrap();

		// Give the command time to start and reach the pipe wait
		std::thread::sleep(std::time::Duration::from_millis(100));

		// Signal editor to "close" by writing to the pipe
		let mut pipe = std::fs::OpenOptions::new().write(true).open(&self.pipe_path).unwrap();
		pipe.write_all(b"x").unwrap();
		drop(pipe);

		// Wait for command to complete
		child.wait_with_output().unwrap()
	}

	/// Initialize git in the issues directory
	fn init_git(&self) {
		let issues_dir = self.xdg.data_dir().join("issues");
		std::fs::create_dir_all(&issues_dir).unwrap();

		let output = Command::new("git").args(["init"]).current_dir(&issues_dir).output().unwrap();
		assert!(output.status.success(), "Failed to init git");

		Command::new("git").args(["config", "user.email", "test@test.local"]).current_dir(&issues_dir).status().unwrap();
		Command::new("git").args(["config", "user.name", "Test User"]).current_dir(&issues_dir).status().unwrap();
	}

	/// Make a git commit in the issues directory
	fn git_commit(&self, message: &str) {
		let issues_dir = self.xdg.data_dir().join("issues");
		Command::new("git").args(["add", "-A"]).current_dir(&issues_dir).status().unwrap();
		Command::new("git").args(["commit", "-m", message]).current_dir(&issues_dir).status().unwrap();
	}
}

#[test]
fn test_both_diverged_triggers_conflict() {
	let ctx = SyncTestContext::new();

	let original = MockIssue::new("Test Issue", "original body");
	let local = MockIssue::new("Test Issue", "local body");
	let remote = MockIssue::new("Test Issue", "remote changed body");

	// Setup: local has changed, remote has changed, original is different from both
	let issue_path = ctx.setup_local(&local);
	ctx.set_original(&original);
	ctx.setup_remote(&remote);

	let output = ctx.run_open(&issue_path);

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	// Should fail because both diverged and git is not initialized
	assert!(!output.status.success(), "Should fail when both diverged");

	// Should detect conflict and suggest initializing git
	assert!(
		stderr.contains("Conflict") || stderr.contains("conflict") || stderr.contains("both local and remote"),
		"Expected conflict detection message. stderr: {}",
		stderr
	);

	// Should suggest git init (since git is not initialized in temp dir)
	assert!(stderr.contains("git init"), "Expected suggestion to initialize git. stderr: {}", stderr);
}

#[test]
fn test_both_diverged_with_git_initiates_merge() {
	let ctx = SyncTestContext::new();

	let original = MockIssue::new("Test Issue", "original body");
	let local = MockIssue::new("Test Issue", "local body");
	let remote = MockIssue::new("Test Issue", "remote changed body");

	// Setup: local has changed, remote has changed, original is different from both
	let issue_path = ctx.setup_local(&local);
	ctx.set_original(&original);
	ctx.setup_remote(&remote);

	// Initialize git and make initial commit
	ctx.init_git();
	ctx.git_commit("Initial commit");

	let output = ctx.run_open(&issue_path);

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	// With git initialized, the merge should be attempted
	assert!(
		stdout.contains("Merging") || stdout.contains("merged") || stderr.contains("CONFLICT") || stderr.contains("Conflict"),
		"Expected merge activity or conflict. stdout: {}, stderr: {}",
		stdout,
		stderr
	);
}

#[test]
fn test_only_remote_changed_takes_remote() {
	let ctx = SyncTestContext::new();

	// Both local and original are the same; only remote changed
	let original = MockIssue::new("Test Issue", "original body");
	let remote = MockIssue::new("Test Issue", "remote changed body");

	let issue_path = ctx.setup_local(&original);
	// original is already set to match local by setup_local
	ctx.setup_remote(&remote);

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	// Should succeed - only remote changed, accept it
	assert!(output.status.success(), "Should succeed when only remote changed. stderr: {}", stderr);
	assert!(stdout.contains("Remote changed"), "Expected remote update message. stdout: {}", stdout);
}

#[test]
fn test_only_local_changed_pushes_local() {
	let ctx = SyncTestContext::new();

	// Local has changed, remote still matches original
	let original = MockIssue::new("Test Issue", "original body");
	let local = MockIssue::new("Test Issue", "local changed body");

	let issue_path = ctx.setup_local(&local);
	ctx.set_original(&original);
	ctx.setup_remote(&original); // Remote still has original

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	// Expected behavior: only local changed, so we should push local to remote
	assert!(output.status.success(), "Should succeed when only local changed. stderr: {}", stderr);
	assert!(
		stdout.contains("Updating") || stdout.contains("Synced") || stdout.contains("No changes"),
		"Expected sync activity. stdout: {}, stderr: {}",
		stdout,
		stderr
	);
}
