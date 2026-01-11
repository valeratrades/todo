//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Commit state on main = last synced truth
//! - Only conflict if BOTH local and remote changed since last sync
//! - Single-side changes auto-resolve
//!
//! Tests work with serialized Issue content (markdown) - our canonical representation.
//! The mock GitHub layer translates to/from API format at the boundary.

use std::{io::Write, path::PathBuf, process::Command};

use v_fixtures::{Fixture, fs_standards::xdg::Xdg};

/// Default test repository coordinates (only needed for file paths and mock setup)
const DEFAULT_OWNER: &str = "testowner";
const DEFAULT_REPO: &str = "testrepo";
const DEFAULT_NUMBER: u64 = 1;

/// Create serialized Issue content for testing.
/// This is the canonical markdown format used by the application.
fn issue(title: &str, body: &str) -> String {
	format!("- [ ] {title} <!-- https://github.com/{DEFAULT_OWNER}/{DEFAULT_REPO}/issues/{DEFAULT_NUMBER} -->\n\t{body}\n")
}

/// Test context for sync operations.
/// Uses v_fixtures for XDG layout and provides helpers for mock GitHub and editor control.
struct SyncTestContext {
	xdg: Xdg,
	mock_state_path: PathBuf,
	pipe_path: PathBuf,
}

impl SyncTestContext {
	fn new() -> Self {
		let fixture = Fixture::parse("");
		let xdg = Xdg::new(fixture.write_to_tempdir(), "todo");

		let mock_state_path = xdg.inner.root.join("mock_state.json");
		let pipe_path = xdg.inner.create_pipe("editor_pipe");

		Self { xdg, mock_state_path, pipe_path }
	}

	/// Set up a local issue file from serialized content.
	/// Also initializes metadata with given body as "original" (last synced state).
	fn setup_local(&self, content: &str, title: &str, original_body: &str) -> PathBuf {
		let issues_dir = format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}");
		let sanitized_title = title.replace(' ', "_");
		let issue_filename = format!("{DEFAULT_NUMBER}_-_{sanitized_title}.md");

		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), content);
		self.write_meta(title, original_body);

		self.xdg.data_dir().join(&issues_dir).join(&issue_filename)
	}

	/// Set up mock GitHub to return an issue.
	/// This is the API boundary - translates our Issue concept to mock format.
	fn setup_remote(&self, title: &str, body: &str) {
		let state = serde_json::json!({
			"issues": [{
				"owner": DEFAULT_OWNER,
				"repo": DEFAULT_REPO,
				"number": DEFAULT_NUMBER,
				"title": title,
				"body": body,
				"state": "open",
				"owner_login": "mock_user"
			}]
		});
		std::fs::write(&self.mock_state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
	}

	/// Update metadata to set a different "original" (last synced) state.
	fn set_original(&self, title: &str, body: &str) {
		self.write_meta(title, body);
	}

	fn write_meta(&self, title: &str, original_body: &str) {
		let meta_content = serde_json::json!({
			"owner": DEFAULT_OWNER,
			"repo": DEFAULT_REPO,
			"issues": {
				DEFAULT_NUMBER.to_string(): {
					"issue_number": DEFAULT_NUMBER,
					"title": title,
					"extension": "md",
					"original_issue_body": original_body,
					"original_comments": [],
					"original_sub_issues": [],
					"parent_issue": null,
					"original_close_state": "Open"
				}
			},
			"virtual_project": false,
			"next_virtual_issue_number": 0
		});
		self.xdg.write_data(
			&format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}/.meta.json"),
			&serde_json::to_string_pretty(&meta_content).unwrap(),
		);
	}

	fn run_open(&self, issue_path: &PathBuf) -> std::process::Output {
		crate::ensure_binary_compiled();

		let mut binary_path = std::env::current_exe().unwrap();
		binary_path.pop();
		binary_path.pop();
		binary_path.push("todo");

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

		std::thread::sleep(std::time::Duration::from_millis(100));

		let mut pipe = std::fs::OpenOptions::new().write(true).open(&self.pipe_path).unwrap();
		pipe.write_all(b"x").unwrap();
		drop(pipe);

		child.wait_with_output().unwrap()
	}

	fn init_git(&self) {
		let issues_dir = self.xdg.data_dir().join("issues");
		std::fs::create_dir_all(&issues_dir).unwrap();

		let output = Command::new("git").args(["init"]).current_dir(&issues_dir).output().unwrap();
		assert!(output.status.success(), "Failed to init git");

		Command::new("git").args(["config", "user.email", "test@test.local"]).current_dir(&issues_dir).status().unwrap();
		Command::new("git").args(["config", "user.name", "Test User"]).current_dir(&issues_dir).status().unwrap();
	}

	fn git_commit(&self, message: &str) {
		let issues_dir = self.xdg.data_dir().join("issues");
		Command::new("git").args(["add", "-A"]).current_dir(&issues_dir).status().unwrap();
		Command::new("git").args(["commit", "-m", message]).current_dir(&issues_dir).status().unwrap();
	}
}

#[test]
fn test_both_diverged_triggers_conflict() {
	let ctx = SyncTestContext::new();

	// Three-way state: original, local (changed), remote (changed differently)
	let local_content = issue("Test Issue", "local body");
	let issue_path = ctx.setup_local(&local_content, "Test Issue", "original body");
	ctx.setup_remote("Test Issue", "remote changed body");

	let output = ctx.run_open(&issue_path);

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(!output.status.success(), "Should fail when both diverged");
	assert!(
		stderr.contains("Conflict") || stderr.contains("conflict") || stderr.contains("both local and remote"),
		"Expected conflict detection message. stderr: {}",
		stderr
	);
	assert!(stderr.contains("git init"), "Expected suggestion to initialize git. stderr: {}", stderr);
}

#[test]
fn test_both_diverged_with_git_initiates_merge() {
	let ctx = SyncTestContext::new();

	let local_content = issue("Test Issue", "local body");
	let issue_path = ctx.setup_local(&local_content, "Test Issue", "original body");
	ctx.setup_remote("Test Issue", "remote changed body");

	ctx.init_git();
	ctx.git_commit("Initial commit");

	let output = ctx.run_open(&issue_path);

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

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

	// Local matches original, only remote changed
	let content = issue("Test Issue", "original body");
	let issue_path = ctx.setup_local(&content, "Test Issue", "original body");
	ctx.setup_remote("Test Issue", "remote changed body");

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	assert!(output.status.success(), "Should succeed when only remote changed. stderr: {}", stderr);
	assert!(stdout.contains("Remote changed"), "Expected remote update message. stdout: {}", stdout);
}

#[test]
fn test_only_local_changed_pushes_local() {
	let ctx = SyncTestContext::new();

	// Local changed, remote still matches original
	let local_content = issue("Test Issue", "local changed body");
	let issue_path = ctx.setup_local(&local_content, "Test Issue", "original body");
	ctx.setup_remote("Test Issue", "original body");

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	assert!(output.status.success(), "Should succeed when only local changed. stderr: {}", stderr);
	assert!(
		stdout.contains("Updating") || stdout.contains("Synced") || stdout.contains("No changes"),
		"Expected sync activity. stdout: {}, stderr: {}",
		stdout,
		stderr
	);
}
