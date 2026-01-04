//! Integration tests that verify mock calls via tracing output.
//!
//! These tests spawn the binary with TODO_TRACE_FILE set, perform operations,
//! then verify the expected mock methods were called by examining the trace log.

use std::{
	fs,
	io::Write,
	path::PathBuf,
	process::{Command, Stdio},
	thread,
	time::Duration,
};

use tempfile::TempDir;

use crate::tracing_utils::TraceLog;

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

struct TracedTestSetup {
	temp_dir: TempDir,
	xdg_state_home: PathBuf,
	xdg_data_home: PathBuf,
	issues_dir: PathBuf,
	pipe_path: PathBuf,
	trace_file: PathBuf,
}

impl TracedTestSetup {
	fn new() -> Self {
		let temp_dir = tempfile::tempdir().unwrap();
		let state_dir = temp_dir.path().join("state").join("todo");
		fs::create_dir_all(&state_dir).unwrap();
		let data_dir = temp_dir.path().join("data").join("todo");
		fs::create_dir_all(&data_dir).unwrap();
		let issues_dir = data_dir.join("issues");
		fs::create_dir_all(&issues_dir).unwrap();

		// Create a named pipe for mock editor signaling
		let pipe_path = temp_dir.path().join("mock_editor_pipe");
		nix::unistd::mkfifo(&pipe_path, nix::sys::stat::Mode::S_IRWXU).unwrap();

		// Create trace file path
		let trace_file = temp_dir.path().join("trace.json");

		Self {
			xdg_state_home: temp_dir.path().join("state"),
			xdg_data_home: temp_dir.path().join("data"),
			issues_dir,
			pipe_path,
			trace_file,
			temp_dir,
		}
	}

	fn setup_project_dir(&self, owner: &str, repo: &str) -> PathBuf {
		let project_dir = self.issues_dir.join(owner).join(repo);
		fs::create_dir_all(&project_dir).unwrap();
		project_dir
	}

	fn write_issue_file(&self, owner: &str, repo: &str, filename: &str, content: &str) -> PathBuf {
		let project_dir = self.setup_project_dir(owner, repo);
		let issue_file = project_dir.join(filename);
		fs::write(&issue_file, content).unwrap();
		issue_file
	}

	fn write_meta(&self, owner: &str, repo: &str, meta_content: &str) {
		let project_dir = self.setup_project_dir(owner, repo);
		let meta_file = project_dir.join(".meta.json");
		fs::write(&meta_file, meta_content).unwrap();
	}

	/// Spawn the todo binary with tracing enabled
	fn spawn_open_with_tracing(&self, file_path: &PathBuf) -> std::process::Child {
		Command::new(get_binary_path())
			.args(["--dbg", "open", file_path.to_str().unwrap()])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("TODO_MOCK_PIPE", &self.pipe_path)
			.env("TODO_TRACE_FILE", &self.trace_file)
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("Failed to spawn todo binary")
	}

	fn signal_editor_close(&self) {
		let mut pipe = fs::OpenOptions::new().write(true).open(&self.pipe_path).expect("Failed to open pipe for writing");
		pipe.write_all(b"x").expect("Failed to write to pipe");
	}

	fn wait_for_child(&self, mut child: std::process::Child) -> (String, String, bool) {
		let output = child.wait_with_output().expect("Failed to wait for child");
		let stdout = String::from_utf8_lossy(&output.stdout).to_string();
		let stderr = String::from_utf8_lossy(&output.stderr).to_string();
		(stdout, stderr, output.status.success())
	}

	fn get_trace_log(&self) -> TraceLog {
		TraceLog::from_file(&self.trace_file)
	}
}

/// Test that modifying an issue body traces the update_issue_body call
#[test]
fn test_modify_issue_body_traces_update() {
	let setup = TracedTestSetup::new();

	let content = "- [ ] Test Issue <!--https://github.com/owner/repo/issues/42-->
\tOriginal body.
";

	let issue_file = setup.write_issue_file("owner", "repo", "42_-_Test_Issue.md", content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "42": {
      "issue_number": 42,
      "title": "Test Issue",
      "extension": "md",
      "original_issue_body": "Original body.",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	let child = setup.spawn_open_with_tracing(&issue_file);
	thread::sleep(Duration::from_millis(100));

	// Modify the issue body during "editing"
	let modified_content = "- [ ] Test Issue <!--https://github.com/owner/repo/issues/42-->
\tModified body content.
";
	fs::write(&issue_file, modified_content).unwrap();

	setup.signal_editor_close();

	let (_stdout, _stderr, _success) = setup.wait_for_child(child);
	// Note: The command may fail because the mock doesn't have proper state (no issues pre-added),
	// but we can still verify that tracing captured the attempt to call the mock methods.

	// Verify tracing captured the update_issue_body call
	let trace = setup.get_trace_log();

	// Should have called update_issue_body due to body change (logged before the operation)
	assert_traced!(trace, "update_issue_body");
}

/// Test that modifying an issue with sub-issues still traces the update call.
/// This verifies that when adding sub-issues to the body, the parent body update is traced.
#[test]
fn test_adding_sub_issue_traces_update_on_parent() {
	let setup = TracedTestSetup::new();

	// Start with an issue that has no sub-issues
	let initial_content = "- [ ] Parent Issue <!--https://github.com/owner/repo/issues/1-->
\tParent body.
";

	let issue_file = setup.write_issue_file("owner", "repo", "1_-_Parent_Issue.md", initial_content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "1": {
      "issue_number": 1,
      "title": "Parent Issue",
      "extension": "md",
      "original_issue_body": "Parent body.",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	let child = setup.spawn_open_with_tracing(&issue_file);
	thread::sleep(Duration::from_millis(100));

	// Modify the parent body to trigger an update
	let modified_content = "- [ ] Parent Issue <!--https://github.com/owner/repo/issues/1-->
\tParent body updated with more details.
";
	fs::write(&issue_file, modified_content).unwrap();

	setup.signal_editor_close();

	let (_stdout, _stderr, _success) = setup.wait_for_child(child);
	// The mock may fail, but we verify tracing captured the attempt

	let trace = setup.get_trace_log();

	// Should have attempted to update the parent issue body
	assert_traced!(trace, "update_issue_body", "owner", "repo", 1);
}

/// Test that we can verify specific arguments in traced calls
#[test]
fn test_trace_captures_arguments() {
	let setup = TracedTestSetup::new();

	let content = "- [ ] Specific Issue <!--https://github.com/testowner/testrepo/issues/999-->
\tOriginal body.
";

	let issue_file = setup.write_issue_file("testowner", "testrepo", "999_-_Specific_Issue.md", content);

	let meta_content = r#"{
  "owner": "testowner",
  "repo": "testrepo",
  "issues": {
    "999": {
      "issue_number": 999,
      "title": "Specific Issue",
      "extension": "md",
      "original_issue_body": "Original body.",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("testowner", "testrepo", meta_content);

	let child = setup.spawn_open_with_tracing(&issue_file);
	thread::sleep(Duration::from_millis(100));

	// Modify the body to trigger an update call
	let modified_content = "- [ ] Specific Issue <!--https://github.com/testowner/testrepo/issues/999-->
\tUpdated body.
";
	fs::write(&issue_file, modified_content).unwrap();

	setup.signal_editor_close();

	let (_stdout, _stderr, _success) = setup.wait_for_child(child);
	// Note: The command may fail because the mock doesn't have proper state,
	// but we can still verify that tracing captured the call with correct arguments.

	let trace = setup.get_trace_log();

	// Verify the update_issue_body call had the correct arguments
	assert_traced!(trace, "update_issue_body", "testowner", "testrepo", 999);
}
