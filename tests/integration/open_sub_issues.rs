//! Integration tests for open command's sub-issue handling.
//!
//! These tests verify that sub-issue content is preserved correctly through
//! the parse → edit → serialize → sync cycle.
//!
//! Note: These tests use existing sub-issues (all have URLs) so no GitHub API
//! calls are made. Tests for creating new sub-issues would require more setup.

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

struct OpenTestSetup {
	temp_dir: TempDir,
	xdg_state_home: PathBuf,
	xdg_data_home: PathBuf,
	issues_dir: PathBuf,
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

		Self {
			xdg_state_home: temp_dir.path().join("state"),
			xdg_data_home: temp_dir.path().join("data"),
			issues_dir,
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

	/// Run `todo --dbg open <file>` with EDITOR=true (exits immediately without modifying)
	fn run_open(&self, file_path: &PathBuf) -> Result<String, String> {
		let output = Command::new(get_binary_path())
			.args(["--dbg", "open", file_path.to_str().unwrap()])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("EDITOR", "true") // Unix true command - exits immediately
			.output()
			.unwrap();

		let stdout = String::from_utf8_lossy(&output.stdout).to_string();
		let stderr = String::from_utf8_lossy(&output.stderr).to_string();

		if !output.status.success() {
			return Err(format!("Exit code: {:?}\nstdout: {}\nstderr: {}", output.status.code(), stdout, stderr));
		}
		Ok(stdout)
	}
}

/// Test that sub-issue body content is preserved through the open command cycle.
/// All sub-issues have URLs so no GitHub creation is triggered.
#[test]
fn test_sub_issue_content_preserved_through_open() {
	let setup = OpenTestSetup::new();

	// All sub-issues have URLs - no new ones to create
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

	// Run the open command (EDITOR=true exits immediately, so file is unchanged)
	let result = setup.run_open(&issue_file);

	// Should succeed (mock GitHub client is used with --dbg)
	assert!(result.is_ok(), "Open command failed: {:?}", result.err());

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

/// Test that multiple sub-issues (open and closed) are all preserved
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

	let result = setup.run_open(&issue_file);
	assert!(result.is_ok(), "Open command failed: {:?}", result.err());

	let final_content = setup.read_issue_file(&issue_file);

	// All sub-issues should be present
	assert!(final_content.contains("Open sub-issue 1"), "Open sub 1 missing");
	assert!(final_content.contains("Content of open sub 1"), "Open sub 1 content missing");
	assert!(final_content.contains("Closed sub-issue 1"), "Closed sub 1 missing");
	assert!(final_content.contains("Open sub-issue 2"), "Open sub 2 missing");
	assert!(final_content.contains("Content of open sub 2"), "Open sub 2 content missing");
	assert!(final_content.contains("Closed sub-issue 2"), "Closed sub 2 missing");
}

/// Test the scenario with mixed open/closed sub-issues
/// Note: Closed sub-issues have their content folded (replaced with <!-- omitted -->)
/// This is intentional to reduce clutter - use --render-closed to see full content
#[test]
fn test_sub_issues_with_body_content_preserved() {
	let setup = OpenTestSetup::new();

	// Issue with sub-issues that have actual body content (not just omitted markers)
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

	let result = setup.run_open(&issue_file);
	assert!(result.is_ok(), "Open command failed: {:?}", result.err());

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
