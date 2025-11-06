use std::{fs, path::PathBuf, process::Command};

use tempfile::TempDir;

fn get_binary_path() -> PathBuf {
	// Get the path to the compiled binary
	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

struct TestSetup {
	_temp_dir: TempDir,
	blocker_file: PathBuf,
	xdg_state_home: PathBuf,
	relative_path: String,
}

impl TestSetup {
	fn new(initial_content: &str, filename: &str) -> Self {
		let temp_dir = tempfile::tempdir().unwrap();
		// XDG_STATE_HOME/todo/ becomes the state directory
		let state_dir = temp_dir.path().join("todo");
		fs::create_dir_all(&state_dir).unwrap();
		let blocker_file = state_dir.join(filename);

		fs::write(&blocker_file, initial_content).unwrap();

		Self {
			xdg_state_home: temp_dir.path().to_path_buf(),
			blocker_file,
			_temp_dir: temp_dir,
			relative_path: filename.to_string(),
		}
	}

	fn new_md(initial_content: &str) -> Self {
		Self::new(initial_content, "test_blocker.md")
	}

	fn new_typst(initial_content: &str) -> Self {
		Self::new(initial_content, "test_blocker.typ")
	}

	fn run_format(&self) -> Result<(), String> {
		let output = Command::new(get_binary_path())
			.args(["blocker", "--relative-path", &self.relative_path, "format"])
			.env("XDG_STATE_HOME", &self.xdg_state_home)
			.output()
			.unwrap();

		if !output.status.success() {
			return Err(String::from_utf8_lossy(&output.stderr).to_string());
		}
		Ok(())
	}

	fn read_blocker_file(&self) -> String {
		fs::read_to_string(&self.blocker_file).unwrap()
	}

	fn read_formatted_file(&self) -> String {
		// For .typ files, the formatted file will be .md
		if self.blocker_file.extension().and_then(|e| e.to_str()) == Some("typ") {
			let md_file = self.blocker_file.with_extension("md");
			fs::read_to_string(&md_file).unwrap()
		} else {
			self.read_blocker_file()
		}
	}
}

const INITIAL_CONTENT: &str = "\
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

// ============================================================================
// Markdown (.md) file tests
// ============================================================================

#[test]
fn test_blocker_format_adds_spaces_md() {
	let setup = TestSetup::new_md(INITIAL_CONTENT);

	setup.run_format().expect("Format command should succeed");

	let formatted = setup.read_blocker_file();
	insta::assert_snapshot!(formatted, @r"
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

	# todo: blocker: doesn't add spaces between same level headers
	");
}

#[test]
fn test_blocker_format_idempotent_md() {
	let setup = TestSetup::new_md(INITIAL_CONTENT);

	// Run format command first time
	setup.run_format().expect("First format command should succeed");
	let formatted_once = setup.read_blocker_file();

	// Run format command second time (simulating open and close)
	setup.run_format().expect("Second format command should succeed");
	let formatted_twice = setup.read_blocker_file();

	// Should be idempotent
	assert_eq!(formatted_once, formatted_twice, "Formatting should be idempotent");
}

// ============================================================================
// Typst (.typ) file tests
// ============================================================================

const TYPST_INITIAL_CONTENT: &str = "\
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

#[test]
fn test_blocker_format_typst_headings() {
	let setup = TestSetup::new_typst(TYPST_INITIAL_CONTENT);

	setup.run_format().expect("Format command should succeed");

	let formatted = setup.read_formatted_file();

	// Verify that Typst headings (=) are converted to markdown (#)
	assert!(formatted.contains("# marketmonkey"), "Should convert = to #");
	assert!(formatted.contains("## yt"), "Should convert == to ##");
	assert!(formatted.contains("## gauss"), "Should convert == to ##");

	// Verify proper spacing between same-level headers
	assert!(formatted.contains("# git lfs: docs, music, etc\n\n# eww"), "Should have spaces between same-level headers");
}

#[test]
fn test_blocker_format_converts_typst_to_md() {
	let setup = TestSetup::new_typst(TYPST_INITIAL_CONTENT);

	// Run format command - should convert .typ to .md
	setup.run_format().expect("First format command should succeed");

	// The original .typ file should no longer exist
	assert!(!setup.blocker_file.exists(), "Original .typ file should be removed");

	// A new .md file should exist
	let md_file = setup.blocker_file.with_extension("md");
	assert!(md_file.exists(), "Converted .md file should exist");

	// Read the markdown file
	let formatted = fs::read_to_string(&md_file).unwrap();

	// Verify it contains markdown syntax
	assert!(formatted.contains("# marketmonkey"), "Should contain markdown headers");
	assert!(formatted.contains("- go in-depth"), "Should contain list items");
}

#[test]
fn test_blocker_format_typst_lists() {
	let typst_content = "\
= Project
- task 1
- task 2
+ numbered item 1
+ numbered item 2";

	let setup = TestSetup::new_typst(typst_content);
	setup.run_format().expect("Format command should succeed");

	let formatted = setup.read_formatted_file();

	// Verify bullet lists are preserved
	assert!(formatted.contains("- task 1"), "Should preserve bullet list items");
	assert!(formatted.contains("- task 2"), "Should preserve bullet list items");

	// Verify numbered lists are converted to bullet lists
	assert!(formatted.contains("- numbered item 1"), "Should convert + to -");
	assert!(formatted.contains("- numbered item 2"), "Should convert + to -");
}

#[test]
fn test_blocker_format_typst_mixed_content() {
	let typst_content = "\
= Main Project
- first task

== Subproject
- subtask 1
- subtask 2

= Another Project
- another task";

	let setup = TestSetup::new_typst(typst_content);
	setup.run_format().expect("Format command should succeed");

	let formatted = setup.read_formatted_file();
	insta::assert_snapshot!(formatted, @r"
	# Main Project
	- first task

	## Subproject
	- subtask 1
	- subtask 2

	# Another Project
	- another task
	");
}
