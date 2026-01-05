//! Shared test fixtures using rstest for integration tests.
//!
//! This module provides reusable test contexts that set up temporary directories
//! and environment variables needed for testing the todo binary.

use std::{fs, path::PathBuf, process::Command};

use rstest::fixture;
use tempfile::TempDir;

fn get_binary_path() -> PathBuf {
	crate::ensure_binary_compiled();

	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
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

	/// Read a blocker file's content.
	pub fn read_blocker_file(&self, filename: &str) -> String {
		let file_path = self.blockers_dir.join(filename);
		fs::read_to_string(&file_path).unwrap()
	}

	/// Read the current project from the cache file.
	pub fn read_current_project(&self) -> Option<String> {
		let cache_file = self.xdg_cache_home.join("todo").join("current_project.txt");
		fs::read_to_string(&cache_file).ok()
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
