//! Shared test fixtures using rstest for integration tests.
//!
//! This module provides reusable test contexts that set up temporary directories,
//! named pipes, and environment variables needed for testing the todo binary.

use std::{
	fs,
	io::Write,
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
};

use rstest::fixture;
use tempfile::TempDir;

/// Base test context with temporary directories and mock pipe.
/// Provides the foundation for all integration tests that spawn the binary.
pub struct TestContext {
	pub temp_dir: TempDir,
	pub xdg_state_home: PathBuf,
	pub xdg_data_home: PathBuf,
	pub issues_dir: PathBuf,
	pub pipe_path: PathBuf,
}

impl TestContext {
	/// Set up a project directory for the given owner/repo.
	pub fn setup_project_dir(&self, owner: &str, repo: &str) -> PathBuf {
		let project_dir = self.issues_dir.join(owner).join(repo);
		fs::create_dir_all(&project_dir).unwrap();
		project_dir
	}

	/// Write an issue file to the test environment.
	pub fn write_issue_file(&self, owner: &str, repo: &str, filename: &str, content: &str) -> PathBuf {
		let project_dir = self.setup_project_dir(owner, repo);
		let issue_file = project_dir.join(filename);
		fs::write(&issue_file, content).unwrap();
		issue_file
	}

	/// Write metadata file for a project.
	pub fn write_meta(&self, owner: &str, repo: &str, meta_content: &str) {
		let project_dir = self.setup_project_dir(owner, repo);
		let meta_file = project_dir.join(".meta.json");
		fs::write(&meta_file, meta_content).unwrap();
	}

	/// Read an issue file back.
	pub fn read_issue_file(&self, path: &Path) -> String {
		fs::read_to_string(path).unwrap()
	}

	/// Spawn the todo binary with mock pipe mode.
	pub fn spawn_open(&self, file_path: &Path) -> Child {
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
	pub fn signal_editor_close(&self) {
		let mut pipe = fs::OpenOptions::new().write(true).open(&self.pipe_path).expect("Failed to open pipe for writing");
		pipe.write_all(b"x").expect("Failed to write to pipe");
	}

	/// Wait for the child process to complete and return (stdout, stderr, success).
	pub fn wait_for_child(&self, child: Child) -> (String, String, bool) {
		let output = child.wait_with_output().expect("Failed to wait for child");
		let stdout = String::from_utf8_lossy(&output.stdout).to_string();
		let stderr = String::from_utf8_lossy(&output.stderr).to_string();
		(stdout, stderr, output.status.success())
	}
}

/// Test context with additional tracing support.
/// Extends TestContext with a trace file for verifying mock calls.
pub struct TracedContext {
	pub base: TestContext,
	pub trace_file: PathBuf,
}

impl TracedContext {
	/// Spawn the todo binary with tracing enabled.
	pub fn spawn_open_with_tracing(&self, file_path: &Path) -> Child {
		Command::new(get_binary_path())
			.args(["--dbg", "open", file_path.to_str().unwrap()])
			.env("XDG_STATE_HOME", &self.base.xdg_state_home)
			.env("XDG_DATA_HOME", &self.base.xdg_data_home)
			.env("TODO_MOCK_PIPE", &self.base.pipe_path)
			.env("TODO_TRACE_FILE", &self.trace_file)
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("Failed to spawn todo binary")
	}
}

impl std::ops::Deref for TracedContext {
	type Target = TestContext;

	fn deref(&self) -> &Self::Target {
		&self.base
	}
}

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

/// Fixture that creates a basic test context with temp directories and mock pipe.
#[fixture]
pub fn ctx() -> TestContext {
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

	TestContext {
		xdg_state_home: temp_dir.path().join("state"),
		xdg_data_home: temp_dir.path().join("data"),
		issues_dir,
		pipe_path,
		temp_dir,
	}
}

/// Fixture that creates a test context with tracing support.
#[fixture]
pub fn traced_ctx() -> TracedContext {
	let base = ctx();
	let trace_file = base.temp_dir.path().join("trace.json");

	TracedContext { base, trace_file }
}

/// Test context for blocker file format tests.
/// Sets up temp directories and provides helpers for blocker file operations.
pub struct BlockerFormatContext {
	#[expect(dead_code)] // Kept alive to preserve temp directory
	pub temp_dir: TempDir,
	pub xdg_state_home: PathBuf,
	pub xdg_data_home: PathBuf,
	#[expect(dead_code)] // Available for test assertions if needed
	pub blockers_dir: PathBuf,
	pub blocker_file: PathBuf,
	pub relative_path: String,
}

impl BlockerFormatContext {
	/// Read the blocker file content.
	pub fn read_blocker_file(&self) -> String {
		fs::read_to_string(&self.blocker_file).unwrap()
	}

	/// Read the formatted file (handles .typ → .md conversion).
	pub fn read_formatted_file(&self) -> String {
		if self.blocker_file.extension().and_then(|e| e.to_str()) == Some("typ") {
			let md_file = self.blocker_file.with_extension("md");
			fs::read_to_string(&md_file).unwrap()
		} else {
			self.read_blocker_file()
		}
	}

	/// Run the blocker format command.
	pub fn run_format(&self) -> Result<(), String> {
		let output = Command::new(get_binary_path())
			.args(["blocker", "--relative-path", &self.relative_path, "format"])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.output()
			.unwrap();

		if !output.status.success() {
			return Err(String::from_utf8_lossy(&output.stderr).to_string());
		}
		Ok(())
	}
}

/// Create a blocker format test context with a specific file.
/// This is the internal implementation used by fixtures.
fn blocker_format_ctx_with_file(content: &str, filename: &str) -> BlockerFormatContext {
	let temp_dir = tempfile::tempdir().unwrap();
	let state_dir = temp_dir.path().join("state").join("todo");
	fs::create_dir_all(&state_dir).unwrap();
	let blockers_dir = temp_dir.path().join("data").join("todo").join("blockers");
	fs::create_dir_all(&blockers_dir).unwrap();
	let blocker_file = blockers_dir.join(filename);
	fs::write(&blocker_file, content).unwrap();

	BlockerFormatContext {
		xdg_state_home: temp_dir.path().join("state"),
		xdg_data_home: temp_dir.path().join("data"),
		blockers_dir,
		blocker_file,
		relative_path: filename.to_string(),
		temp_dir,
	}
}

/// Fixture for markdown blocker file with default content.
#[fixture]
pub fn blocker_md_ctx() -> BlockerFormatContext {
	blocker_format_ctx_with_file(DEFAULT_BLOCKER_CONTENT, "test_blocker.md")
}

/// Fixture for typst blocker file with default content.
#[fixture]
pub fn blocker_typ_ctx() -> BlockerFormatContext {
	blocker_format_ctx_with_file(DEFAULT_TYPST_CONTENT, "test_blocker.typ")
}

/// Create a blocker format test context with custom content.
/// Use this when tests need specific content rather than the default.
pub fn blocker_format_ctx_custom(content: &str, filename: &str) -> BlockerFormatContext {
	blocker_format_ctx_with_file(content, filename)
}

/// Default content for markdown blocker tests.
pub const DEFAULT_BLOCKER_CONTENT: &str = "\
- move these todos over into a persisted directory
	comment
- move all typst projects
- rewrite custom.sh
	comment

# marketmonkey
- go in-depth on possibilities

# SocialNetworks in rust
- test twitter

## yt
- test

# math tools
## gauss
- finish it
		a space-indented comment comment
- move gaussian pivot over in there
	   another space-indented comment

# git lfs: docs, music, etc
# eww: don't restore if outdated
# todo: blocker: doesn't add spaces between same level headers";

/// Default content for typst blocker tests.
pub const DEFAULT_TYPST_CONTENT: &str = "\
= marketmonkey
- go in-depth on possibilities

= SocialNetworks in rust
- test twitter

== yt
- test

= math tools
== gauss
- finish it
- move gaussian pivot over in there

= git lfs: docs, music, etc
= eww: don't restore if outdated
= todo: blocker: test typst support";

/// Test context for blocker project resolution tests.
pub struct BlockerProjectContext {
	#[expect(dead_code)] // Kept alive to preserve temp directory
	pub temp_dir: TempDir,
	pub xdg_state_home: PathBuf,
	pub xdg_data_home: PathBuf,
	pub xdg_cache_home: PathBuf,
	pub blockers_dir: PathBuf,
}

impl BlockerProjectContext {
	/// Create a blocker file with given content.
	pub fn create_blocker_file(&self, filename: &str, content: &str) {
		let file_path = self.blockers_dir.join(filename);
		if let Some(parent) = file_path.parent() {
			fs::create_dir_all(parent).unwrap();
		}
		fs::write(&file_path, content).unwrap();
	}

	/// Run the set-project command.
	pub fn run_set_project(&self, pattern: &str) -> std::process::Output {
		Command::new(get_binary_path())
			.args(["blocker", "set-project", pattern])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("XDG_CACHE_HOME", &self.xdg_cache_home)
			.output()
			.unwrap()
	}

	/// Run the add --urgent command.
	pub fn run_add_urgent(&self, task_name: &str) -> std::process::Output {
		Command::new(get_binary_path())
			.args(["blocker", "add", "--urgent", task_name])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("XDG_CACHE_HOME", &self.xdg_cache_home)
			.output()
			.unwrap()
	}
}

/// Fixture for blocker project resolution tests.
#[fixture]
pub fn blocker_project_ctx() -> BlockerProjectContext {
	let temp_dir = tempfile::tempdir().unwrap();
	let state_dir = temp_dir.path().join("state").join("todo");
	fs::create_dir_all(&state_dir).unwrap();
	let cache_dir = temp_dir.path().join("cache").join("todo");
	fs::create_dir_all(&cache_dir).unwrap();
	let blockers_dir = temp_dir.path().join("data").join("todo").join("blockers");
	fs::create_dir_all(&blockers_dir).unwrap();

	BlockerProjectContext {
		xdg_state_home: temp_dir.path().join("state"),
		xdg_data_home: temp_dir.path().join("data"),
		xdg_cache_home: temp_dir.path().join("cache"),
		blockers_dir,
		temp_dir,
	}
}
