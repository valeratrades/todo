//! Integration tests for open command's sub-issue handling.
//!
//! These tests verify that sub-issue content is preserved correctly through
//! the parse → edit → serialize → sync cycle.
//!
//! The tests use a pipe-based mock mechanism:
//! 1. Create a named pipe (FIFO)
//! 2. Spawn the binary with TODO_MOCK_PIPE env var pointing to the pipe
//! 3. The binary waits for a signal on the pipe instead of opening an editor
//! 4. Test modifies the file while binary is waiting
//! 5. Test writes to the pipe to signal "editor closed"
//! 6. Binary continues and syncs changes

use std::{
	fs,
	io::Write,
	path::PathBuf,
	process::{Child, Command, Stdio},
	thread,
	time::Duration,
};

use tempfile::TempDir;

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

struct OpenTestSetup {
	temp_dir: TempDir,
	xdg_state_home: PathBuf,
	xdg_data_home: PathBuf,
	issues_dir: PathBuf,
	pipe_path: PathBuf,
}

impl OpenTestSetup {
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

		Self {
			xdg_state_home: temp_dir.path().join("state"),
			xdg_data_home: temp_dir.path().join("data"),
			issues_dir,
			pipe_path,
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

	fn read_issue_file(&self, path: &PathBuf) -> String {
		fs::read_to_string(path).unwrap()
	}

	/// Spawn the todo binary with mock pipe mode.
	/// Returns the child process handle.
	fn spawn_open(&self, file_path: &PathBuf) -> Child {
		Command::new(get_binary_path())
			.args(["--dbg", "open", file_path.to_str().unwrap()])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("TODO_MOCK_PIPE", &self.pipe_path)
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("Failed to spawn todo binary")
	}

	/// Signal the mock editor to "close" by writing to the pipe.
	fn signal_editor_close(&self) {
		// Open the pipe for writing - this unblocks the reading side
		let mut pipe = fs::OpenOptions::new().write(true).open(&self.pipe_path).expect("Failed to open pipe for writing");
		pipe.write_all(b"x").expect("Failed to write to pipe");
	}

	/// Wait for the child process to complete and return (stdout, stderr, success).
	fn wait_for_child(&self, mut child: Child) -> (String, String, bool) {
		let output = child.wait_with_output().expect("Failed to wait for child");
		let stdout = String::from_utf8_lossy(&output.stdout).to_string();
		let stderr = String::from_utf8_lossy(&output.stderr).to_string();
		(stdout, stderr, output.status.success())
	}
}

/// Test that sub-issue body content is preserved through the open command cycle
/// when no changes are made during editing.
#[test]
fn test_sub_issue_content_preserved_through_open() {
	let setup = OpenTestSetup::new();

	// Initial file content with all sub-issues having URLs (no GitHub creation needed)
	let content = "- [ ] Parent Issue <!--https://github.com/owner/repo/issues/46-->
\tParent body content.

\t- [x] Closed sub-issue <!--sub https://github.com/owner/repo/issues/77-->
\t\t<!-- omitted (use --render-closed to unfold) -->
\t- [ ] Open sub-issue with content <!--sub https://github.com/owner/repo/issues/78-->
\t\tThis is the body of the sub-issue.
\t\tIt has multiple lines.
";

	let issue_file = setup.write_issue_file("owner", "repo", "46_-_Parent_Issue.md", content);

	// Metadata must match all sub-issues
	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "46": {
      "issue_number": 46,
      "title": "Parent Issue",
      "extension": "md",
      "original_issue_body": "Parent body content.",
      "original_comments": [],
      "original_sub_issues": [
        {"number": 77, "state": "closed"},
        {"number": 78, "state": "open"}
      ],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	// Spawn the binary (it will wait on the pipe)
	let child = setup.spawn_open(&issue_file);

	// Give it a moment to start and reach the pipe wait
	thread::sleep(Duration::from_millis(100));

	// Signal editor close (no file modifications)
	setup.signal_editor_close();

	// Wait for completion
	let (stdout, stderr, success) = setup.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	// Read back the file
	let final_content = setup.read_issue_file(&issue_file);

	// Verify all content is preserved
	assert!(final_content.contains("Parent Issue"), "Parent title missing");
	assert!(final_content.contains("Parent body content"), "Parent body missing");
	assert!(final_content.contains("Closed sub-issue"), "Closed sub-issue title missing");
	assert!(final_content.contains("Open sub-issue with content"), "Open sub-issue title missing");
	assert!(final_content.contains("This is the body of the sub-issue"), "Sub-issue body missing");
	assert!(final_content.contains("It has multiple lines"), "Sub-issue multi-line content missing");
}

/// Test that multiple sub-issues (open and closed) are all preserved.
#[test]
fn test_multiple_sub_issues_preserved() {
	let setup = OpenTestSetup::new();

	let content = "- [ ] Complex Parent <!--https://github.com/owner/repo/issues/100-->
\tThe parent body.

\t- [ ] Open sub-issue 1 <!--sub https://github.com/owner/repo/issues/101-->
\t\tContent of open sub 1
\t- [x] Closed sub-issue 1 <!--sub https://github.com/owner/repo/issues/102-->
\t\t<!-- omitted -->
\t- [ ] Open sub-issue 2 <!--sub https://github.com/owner/repo/issues/103-->
\t\tContent of open sub 2
\t- [x] Closed sub-issue 2 <!--sub https://github.com/owner/repo/issues/104-->
\t\t<!-- omitted -->
";

	let issue_file = setup.write_issue_file("owner", "repo", "100_-_Complex_Parent.md", content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "100": {
      "issue_number": 100,
      "title": "Complex Parent",
      "extension": "md",
      "original_issue_body": "The parent body.",
      "original_comments": [],
      "original_sub_issues": [
        {"number": 101, "state": "open"},
        {"number": 102, "state": "closed"},
        {"number": 103, "state": "open"},
        {"number": 104, "state": "closed"}
      ],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	let child = setup.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	setup.signal_editor_close();

	let (stdout, stderr, success) = setup.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = setup.read_issue_file(&issue_file);

	// All sub-issues should be present
	assert!(final_content.contains("Open sub-issue 1"), "Open sub 1 missing");
	assert!(final_content.contains("Content of open sub 1"), "Open sub 1 content missing");
	assert!(final_content.contains("Closed sub-issue 1"), "Closed sub 1 missing");
	assert!(final_content.contains("Open sub-issue 2"), "Open sub 2 missing");
	assert!(final_content.contains("Content of open sub 2"), "Open sub 2 content missing");
	assert!(final_content.contains("Closed sub-issue 2"), "Closed sub 2 missing");
}

/// Test that adding blockers during edit are preserved.
/// This reproduces a bug where blockers added during editing get lost.
#[test]
fn test_adding_blockers_during_edit_are_preserved() {
	let setup = OpenTestSetup::new();

	// Start with simple issue without blockers
	let initial_content = "- [ ] blocker rewrite <!--https://github.com/owner/repo/issues/49-->
\tget all the present functionality + legacy supported
";

	let issue_file = setup.write_issue_file("owner", "repo", "49_-_blocker_rewrite.md", initial_content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "49": {
      "issue_number": 49,
      "title": "blocker rewrite",
      "extension": "md",
      "original_issue_body": "get all the present functionality + legacy supported",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	let child = setup.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));

	// Add blockers during "editing"
	let modified_content = "- [ ] blocker rewrite <!--https://github.com/owner/repo/issues/49-->
\tget all the present functionality + legacy supported
\t<!--blockers-->
\t- support for virtual blockers
\t- move all primitives into new blocker.rs
";
	fs::write(&issue_file, modified_content).unwrap();

	setup.signal_editor_close();

	let (stdout, stderr, success) = setup.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = setup.read_issue_file(&issue_file);
	insta::assert_snapshot!(final_content, @r#"
- [ ] blocker rewrite <!-- https://github.com/owner/repo/issues/49 -->
	get all the present functionality + legacy supported

	<!--blockers-->
	- support for virtual blockers
	- move all primitives into new blocker.rs
"#);
}

/// Test that blockers section is preserved through the open command cycle.
#[test]
fn test_blockers_preserved_through_open() {
	let setup = OpenTestSetup::new();

	let content = "- [ ] Issue with blockers <!--https://github.com/owner/repo/issues/49-->
\tget all the present functionality + legacy supported
\t<!--blockers-->
\t- support for virtual blockers
\t- move all primitives into new blocker.rs
\t- get clockify integration
";

	let issue_file = setup.write_issue_file("owner", "repo", "49_-_Issue_with_blockers.md", content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "49": {
      "issue_number": 49,
      "title": "Issue with blockers",
      "extension": "md",
      "original_issue_body": "get all the present functionality + legacy supported",
      "original_comments": [],
      "original_sub_issues": [],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	let child = setup.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	setup.signal_editor_close();

	let (stdout, stderr, success) = setup.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = setup.read_issue_file(&issue_file);

	// Blockers section should be preserved
	assert!(final_content.contains("blockers"), "Blockers marker missing");
	assert!(final_content.contains("support for virtual blockers"), "First blocker missing");
	assert!(final_content.contains("move all primitives"), "Second blocker missing");
	assert!(final_content.contains("clockify integration"), "Third blocker missing");
}

/// Test that closed sub-issues have their content folded to <!-- omitted -->.
#[test]
fn test_closed_sub_issues_content_folded() {
	let setup = OpenTestSetup::new();

	// Start with expanded content for the closed sub-issue
	let content = "- [ ] v2_interface <!--https://github.com/owner/repo/issues/46-->
\tMain issue body here.

\t- [x] Completed task <!--sub https://github.com/owner/repo/issues/77-->
\t\tThis task was done.
\t\tHere are the details.
\t- [ ] In-progress task <!--sub https://github.com/owner/repo/issues/78-->
\t\tDescription of the current task
\t\tWith some implementation notes
";

	let issue_file = setup.write_issue_file("owner", "repo", "46_-_v2_interface.md", content);

	let meta_content = r#"{
  "owner": "owner",
  "repo": "repo",
  "issues": {
    "46": {
      "issue_number": 46,
      "title": "v2_interface",
      "extension": "md",
      "original_issue_body": "Main issue body here.",
      "original_comments": [],
      "original_sub_issues": [
        {"number": 77, "state": "closed"},
        {"number": 78, "state": "open"}
      ],
      "parent_issue": null,
      "original_closed": false
    }
  }
}"#;
	setup.write_meta("owner", "repo", meta_content);

	let child = setup.spawn_open(&issue_file);
	thread::sleep(Duration::from_millis(100));
	setup.signal_editor_close();

	let (stdout, stderr, success) = setup.wait_for_child(child);
	assert!(success, "Open command failed. stdout: {}\nstderr: {}", stdout, stderr);

	let final_content = setup.read_issue_file(&issue_file);

	// Closed sub-issue title is preserved but content is folded
	assert!(final_content.contains("Completed task"), "Closed sub-issue title missing");
	assert!(final_content.contains("<!-- omitted"), "Closed sub-issue should show omitted marker");
	// Original body content is replaced with omitted marker for closed sub-issues
	assert!(!final_content.contains("This task was done"), "Closed sub-issue body should be omitted");

	// Open sub-issue body content should be preserved
	assert!(final_content.contains("In-progress task"), "Open sub-issue title missing");
	assert!(final_content.contains("Description of the current task"), "Open sub-issue body missing");
	assert!(final_content.contains("With some implementation notes"), "Open sub-issue multi-line content missing");
}
