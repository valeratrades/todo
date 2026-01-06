//! Shared test fixtures for integration tests.
//!
//! Uses v_fixtures for file parsing and the Xdg wrapper for proper
//! XDG directory structure.

use std::{path::PathBuf, process::Command};

use v_fixtures::{Fixture, fs_standards::xdg::Xdg};

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

/// Test context for blocker operations.
/// Sets up XDG directories using v_fixtures and manages file operations.
pub struct TodoTestContext {
	/// The Xdg wrapper managing temp directories
	xdg: Xdg,
}

impl TodoTestContext {
	/// Create a new test context from a fixture string.
	///
	/// Files in the fixture should use XDG category prefixes:
	/// - `/data/blockers/test.md` → `XDG_DATA_HOME/todo/blockers/test.md`
	/// - `/cache/current.txt` → `XDG_CACHE_HOME/todo/current.txt`
	/// - `/state/db.json` → `XDG_STATE_HOME/todo/db.json`
	///
	/// Example:
	/// ```ignore
	/// let ctx = TodoTestContext::new(r#"
	///     //- /data/blockers/test.md
	///     # Project
	///     - task 1
	/// "#);
	/// ```
	pub fn new(fixture_str: &str) -> Self {
		let fixture = Fixture::parse(fixture_str);
		let xdg = Xdg::new(fixture.write_to_tempdir(), "todo");
		Self { xdg }
	}

	/// Run a todo command with proper XDG environment.
	pub fn run(&self, args: &[&str]) -> std::process::Output {
		let mut cmd = Command::new(get_binary_path());
		cmd.args(args);
		for (key, value) in self.xdg.env_vars() {
			cmd.env(key, value);
		}
		cmd.output().unwrap()
	}

	/// Run blocker format command on a specific file.
	/// The path should be relative to the blockers directory (e.g., "test.md" or "work/test.md")
	pub fn run_format(&self, blocker_relative_path: &str) -> Result<(), String> {
		let output = self.run(&["blocker", "--relative-path", blocker_relative_path, "format"]);
		if !output.status.success() {
			return Err(String::from_utf8_lossy(&output.stderr).to_string());
		}
		Ok(())
	}

	/// Read a blocker file (path relative to blockers directory).
	pub fn read_blocker(&self, blocker_relative_path: &str) -> String {
		self.xdg.read_data(&format!("blockers/{blocker_relative_path}"))
	}

	/// Check if a blocker file exists.
	pub fn blocker_exists(&self, blocker_relative_path: &str) -> bool {
		self.xdg.data_exists(&format!("blockers/{blocker_relative_path}"))
	}

	/// Read a file from the data directory.
	pub fn read(&self, relative_path: &str) -> String {
		self.xdg.read_data(relative_path.trim_start_matches('/'))
	}

	/// Read the current project from cache.
	pub fn read_current_project(&self) -> Option<String> {
		if self.xdg.cache_exists("current_project.txt") {
			Some(self.xdg.read_cache("current_project.txt"))
		} else {
			None
		}
	}
}
