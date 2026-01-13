//! Shared test infrastructure for integration tests.
//!
//! Provides `TestContext` - a unified test context that handles:
//! - XDG directory setup with proper environment variables
//! - Running commands against the compiled binary
//! - Mock state management for GitHub API simulation
//! - Named pipe communication for editor simulation
//!
//! # Example
//!
//! ```ignore
//! let ctx = TestContext::new(r#"
//!     //- /data/blockers/test.md
//!     - task 1
//! "#);
//!
//! let (status, stdout, stderr) = ctx.run(&["blocker", "list"]);
//! assert!(status.success());
//! ```

pub mod git;

use std::{
	io::Write,
	path::{Path, PathBuf},
	process::{Command, ExitStatus},
	sync::OnceLock,
};

use v_fixtures::{Fixture, fs_standards::xdg::Xdg};

static BINARY_COMPILED: OnceLock<()> = OnceLock::new();

/// Compile the binary before running any tests
pub fn ensure_binary_compiled() {
	BINARY_COMPILED.get_or_init(|| {
		let status = Command::new("cargo")
			.args(["build", "--features", "is_integration_test"])
			.status()
			.expect("Failed to execute cargo build");

		if !status.success() {
			panic!("Failed to build binary");
		}
	});
}

fn get_binary_path() -> PathBuf {
	ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push(env!("CARGO_PKG_NAME"));
	path
}

/// Unified test context for integration tests.
///
/// Combines functionality from the old `TodoTestContext` and `SyncTestContext`.
/// Handles XDG directory setup, command execution, and optional mock state.
pub struct TestContext {
	/// The Xdg wrapper managing temp directories
	pub xdg: Xdg,
	/// Path to mock GitHub state file (for sync tests)
	pub mock_state_path: PathBuf,
	/// Path to named pipe for editor simulation (for sync tests)
	pub pipe_path: PathBuf,
}

impl TestContext {
	/// Create a new test context from a fixture string.
	///
	/// Files in the fixture should use XDG category prefixes:
	/// - `/data/blockers/test.md` → `XDG_DATA_HOME/todo/blockers/test.md`
	/// - `/cache/current.txt` → `XDG_CACHE_HOME/todo/current.txt`
	/// - `/state/db.json` → `XDG_STATE_HOME/todo/db.json`
	///
	/// # Example
	///
	/// ```ignore
	/// let ctx = TestContext::new(r#"
	///     //- /data/blockers/test.md
	///     # Project
	///     - task 1
	/// "#);
	/// ```
	pub fn new(fixture_str: &str) -> Self {
		let fixture = Fixture::parse(fixture_str);
		let xdg = Xdg::new(fixture.write_to_tempdir(), env!("CARGO_PKG_NAME"));

		let mock_state_path = xdg.inner.root.join("mock_state.json");
		let pipe_path = xdg.inner.create_pipe("editor_pipe");

		Self { xdg, mock_state_path, pipe_path }
	}

	/// Run a command with proper XDG environment.
	///
	/// Returns (exit_status, stdout, stderr) for easy assertions.
	pub fn run(&self, args: &[&str]) -> (ExitStatus, String, String) {
		let mut cmd = Command::new(get_binary_path());
		cmd.args(args);
		for (key, value) in self.xdg.env_vars() {
			cmd.env(key, value);
		}
		let output = cmd.output().unwrap();
		(
			output.status,
			String::from_utf8_lossy(&output.stdout).into_owned(),
			String::from_utf8_lossy(&output.stderr).into_owned(),
		)
	}

	/// Run `open` command with mock GitHub state and editor pipe.
	///
	/// This is used for sync tests where we need to:
	/// 1. Set up mock GitHub state
	/// 2. Spawn the command
	/// 3. Signal the editor pipe to close
	/// 4. Wait for completion
	///
	/// Returns (exit_status, stdout, stderr).
	pub fn run_open(&self, issue_path: &Path) -> (ExitStatus, String, String) {
		let mut cmd = Command::new(get_binary_path());
		cmd.args(["--mock", "open", issue_path.to_str().unwrap()]);
		for (key, value) in self.xdg.env_vars() {
			cmd.env(key, value);
		}
		cmd.env("TODO_MOCK_STATE", &self.mock_state_path);
		cmd.env("TODO_MOCK_PIPE", &self.pipe_path);
		cmd.stdout(std::process::Stdio::piped());
		cmd.stderr(std::process::Stdio::piped());

		let child = cmd.spawn().unwrap();

		// Give the process time to start and begin waiting on the pipe
		std::thread::sleep(std::time::Duration::from_millis(100));

		// Signal the editor to close
		let mut pipe = std::fs::OpenOptions::new().write(true).open(&self.pipe_path).unwrap();
		pipe.write_all(b"x").unwrap();
		drop(pipe);

		let output = child.wait_with_output().unwrap();
		(
			output.status,
			String::from_utf8_lossy(&output.stdout).into_owned(),
			String::from_utf8_lossy(&output.stderr).into_owned(),
		)
	}

	/// Read a file from the data directory.
	pub fn read(&self, relative_path: &str) -> String {
		self.xdg.read_data(relative_path.trim_start_matches('/'))
	}

	/// Write a file to the data directory.
	pub fn write(&self, relative_path: &str, content: &str) {
		self.xdg.write_data(relative_path.trim_start_matches('/'), content);
	}

	/// Check if a file exists in the data directory.
	pub fn data_exists(&self, relative_path: &str) -> bool {
		self.xdg.data_exists(relative_path.trim_start_matches('/'))
	}

	/// Read the current project from cache.
	pub fn read_current_project(&self) -> Option<String> {
		if self.xdg.cache_exists("current_project.txt") {
			Some(self.xdg.read_cache("current_project.txt"))
		} else {
			None
		}
	}

	/// Read a blocker file (path relative to blockers directory).
	pub fn read_blocker(&self, blocker_relative_path: &str) -> String {
		self.xdg.read_data(&format!("blockers/{blocker_relative_path}"))
	}

	/// Check if a blocker file exists.
	pub fn blocker_exists(&self, blocker_relative_path: &str) -> bool {
		self.xdg.data_exists(&format!("blockers/{blocker_relative_path}"))
	}

	/// Get the data directory path.
	pub fn data_dir(&self) -> PathBuf {
		self.xdg.data_dir()
	}

	/// Set up mock GitHub to return an issue.
	///
	/// The issue parameter should be a serde_json::Value representing the mock state.
	pub fn setup_mock_state(&self, state: &serde_json::Value) {
		std::fs::write(&self.mock_state_path, serde_json::to_string_pretty(state).unwrap()).unwrap();
	}
}
