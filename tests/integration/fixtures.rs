//! Shared test fixtures for integration tests.
//!
//! Uses v_fixtures for file parsing and provides helpers for running
//! the todo binary with proper XDG environment variables.

use std::{fs, path::PathBuf, process::Command};

use tempfile::TempDir;
use v_fixtures::Fixture;

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

/// Test context for blocker operations.
/// Sets up XDG directories and manages file operations.
pub struct TodoTestContext {
	// Kept alive to preserve temp directory
	_temp_dir: TempDir,
	/// Root of the data/todo directory where blocker files live
	pub data_root: PathBuf,
	pub xdg_state_home: PathBuf,
	pub xdg_data_home: PathBuf,
	pub xdg_cache_home: PathBuf,
}

impl TodoTestContext {
	/// Create a new test context from a fixture string.
	///
	/// Files in the fixture are placed relative to the XDG_DATA_HOME/todo directory.
	/// Use paths like `/blockers/test.md` in the fixture.
	///
	/// Example:
	/// ```ignore
	/// let ctx = TodoTestContext::new(r#"
	///     //- /blockers/test.md
	///     # Project
	///     - task 1
	/// "#);
	/// ```
	pub fn new(fixture_str: &str) -> Self {
		// Create base temp directory
		let temp_dir = tempfile::Builder::new().prefix("todo_test_").tempdir().unwrap();
		let root = temp_dir.path();

		// Create XDG directory structure
		let xdg_state_home = root.join("state");
		let xdg_data_home = root.join("data");
		let xdg_cache_home = root.join("cache");

		fs::create_dir_all(xdg_state_home.join("todo")).unwrap();
		fs::create_dir_all(xdg_data_home.join("todo")).unwrap();
		fs::create_dir_all(xdg_cache_home.join("todo")).unwrap();

		let data_root = xdg_data_home.join("todo");

		// Parse fixture and write files to data/todo/
		let fixture = Fixture::parse(fixture_str);
		for file in &fixture.files {
			let path = data_root.join(file.path.trim_start_matches('/'));
			if let Some(parent) = path.parent() {
				fs::create_dir_all(parent).unwrap();
			}
			fs::write(&path, &file.text).unwrap();
		}

		Self {
			_temp_dir: temp_dir,
			data_root,
			xdg_state_home,
			xdg_data_home,
			xdg_cache_home,
		}
	}

	/// Run a todo command with proper XDG environment.
	pub fn run(&self, args: &[&str]) -> std::process::Output {
		Command::new(get_binary_path())
			.args(args)
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.env("XDG_DATA_HOME", &self.xdg_data_home)
			.env("XDG_CACHE_HOME", &self.xdg_cache_home)
			.output()
			.unwrap()
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
		let path = self.data_root.join("blockers").join(blocker_relative_path);
		fs::read_to_string(&path).unwrap()
	}

	/// Check if a blocker file exists.
	pub fn blocker_exists(&self, blocker_relative_path: &str) -> bool {
		let path = self.data_root.join("blockers").join(blocker_relative_path);
		path.exists()
	}

	/// Read a file from the data/todo directory.
	pub fn read(&self, relative_path: &str) -> String {
		let path = self.data_root.join(relative_path.trim_start_matches('/'));
		fs::read_to_string(&path).unwrap()
	}

	/// Check if a file exists.
	pub fn exists(&self, relative_path: &str) -> bool {
		let path = self.data_root.join(relative_path.trim_start_matches('/'));
		path.exists()
	}

	/// Get full path to a file in data/todo.
	pub fn path(&self, relative_path: &str) -> PathBuf {
		self.data_root.join(relative_path.trim_start_matches('/'))
	}

	/// Read the current project from cache.
	pub fn read_current_project(&self) -> Option<String> {
		let cache_file = self.xdg_cache_home.join("todo").join("current_project.txt");
		fs::read_to_string(&cache_file).ok()
	}

	/// Write a file to data/todo (useful for adding files after initial setup).
	#[expect(dead_code)] // May be useful for future tests
	pub fn write(&self, relative_path: &str, content: &str) {
		let path = self.data_root.join(relative_path.trim_start_matches('/'));
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).unwrap();
		}
		fs::write(&path, content).unwrap();
	}
}

/// Default content for markdown blocker tests.
pub const DEFAULT_BLOCKER_MD: &str = "\
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
pub const DEFAULT_BLOCKER_TYP: &str = "\
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
