//! Integration tests for sync conflict resolution.
//!
//! Tests the consensus-based sync logic where:
//! - Commit state on main = last synced truth
//! - Only conflict if BOTH local and remote changed since last sync
//! - Single-side changes auto-resolve
//!
//! Tests work with `Issue` directly - our canonical representation.
//! The mock GitHub layer translates to API format at the boundary.

use std::{
	io::Write,
	path::{Path, PathBuf},
	process::Command,
};

use todo::{Issue, ParseContext};
use v_fixtures::{Fixture, fs_standards::xdg::Xdg};

/// Default test repository coordinates (only needed for file paths and mock setup)
const DEFAULT_OWNER: &str = "testowner";
const DEFAULT_REPO: &str = "testrepo";
const DEFAULT_NUMBER: u64 = 1;

/// Parse an Issue from markdown content.
fn parse_issue(content: &str) -> Issue {
	let ctx = ParseContext::new(content.to_string(), "test.md".to_string());
	Issue::parse(content, &ctx).expect("failed to parse test issue")
}

/// Create a simple test issue with given title and body.
fn issue(title: &str, body: &str) -> Issue {
	let content = format!("- [ ] {title} <!-- https://github.com/{DEFAULT_OWNER}/{DEFAULT_REPO}/issues/{DEFAULT_NUMBER} -->\n\t{body}\n");
	parse_issue(&content)
}

/// Test context for sync operations.
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

	/// Set up a local issue file and metadata.
	/// `local` is the current local state, `original` is the last synced state.
	fn setup_local(&self, local: &Issue, original: &Issue) -> PathBuf {
		let issues_dir = format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}");
		let sanitized_title = local.meta.title.replace(' ', "_");
		let issue_filename = format!("{DEFAULT_NUMBER}_-_{sanitized_title}.md");

		// Write the local issue file
		self.xdg.write_data(&format!("{issues_dir}/{issue_filename}"), &local.serialize());

		// Write metadata with original as the consensus state
		self.write_meta(original);

		self.xdg.data_dir().join(&issues_dir).join(&issue_filename)
	}

	/// Set up mock GitHub to return an issue.
	/// This is the API boundary - translates Issue to mock format.
	fn setup_remote(&self, issue: &Issue) {
		let body = issue.body();
		let state = serde_json::json!({
			"issues": [{
				"owner": DEFAULT_OWNER,
				"repo": DEFAULT_REPO,
				"number": DEFAULT_NUMBER,
				"title": issue.meta.title,
				"body": body,
				"state": "open",
				"owner_login": "mock_user"
			}]
		});
		std::fs::write(&self.mock_state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
	}

	/// Write metadata file with the given issue as the "original" (last synced) state.
	fn write_meta(&self, original: &Issue) {
		let body = original.body();
		let meta_content = serde_json::json!({
			"owner": DEFAULT_OWNER,
			"repo": DEFAULT_REPO,
			"issues": {
				DEFAULT_NUMBER.to_string(): {
					"issue_number": DEFAULT_NUMBER,
					"title": original.meta.title,
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
		self.xdg.write_data(
			&format!("issues/{DEFAULT_OWNER}/{DEFAULT_REPO}/.meta.json"),
			&serde_json::to_string_pretty(&meta_content).unwrap(),
		);
	}

	fn run_open(&self, issue_path: &Path) -> std::process::Output {
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

	let original = issue("Test Issue", "original body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	let issue_path = ctx.setup_local(&local, &original);
	ctx.setup_remote(&remote);

	let output = ctx.run_open(&issue_path);

	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");

	assert!(!output.status.success(), "Should fail when both diverged");
	insta::assert_snapshot!(stderr, @r"");
}

#[test]
fn test_both_diverged_with_git_initiates_merge() {
	let ctx = SyncTestContext::new();

	let original = issue("Test Issue", "original body");
	let local = issue("Test Issue", "local body");
	let remote = issue("Test Issue", "remote changed body");

	let issue_path = ctx.setup_local(&local, &original);
	ctx.setup_remote(&remote);

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

	let original = issue("Test Issue", "original body");
	let remote = issue("Test Issue", "remote changed body");

	// Local matches original
	let issue_path = ctx.setup_local(&original, &original);
	ctx.setup_remote(&remote);

	let output = ctx.run_open(&issue_path);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	eprintln!("stdout: {stdout}");
	eprintln!("stderr: {stderr}");
	eprintln!("status: {:?}", output.status);

	assert!(output.status.success(), "Should succeed when only remote changed. stderr: {}", stderr);
}

#[test]
fn test_only_local_changed_pushes_local() {
	let ctx = SyncTestContext::new();

	let original = issue("Test Issue", "original body");
	let local = issue("Test Issue", "local changed body");

	let issue_path = ctx.setup_local(&local, &original);
	ctx.setup_remote(&original); // Remote still has original

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
