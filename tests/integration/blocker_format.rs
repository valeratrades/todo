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
}

impl TestSetup {
	fn new(initial_content: &str) -> Self {
		let temp_dir = tempfile::tempdir().unwrap();
		// XDG_STATE_HOME/todo/ becomes the state directory
		let state_dir = temp_dir.path().join("todo");
		fs::create_dir_all(&state_dir).unwrap();
		let blocker_file = state_dir.join("test_blocker.md");

		fs::write(&blocker_file, initial_content).unwrap();

		Self {
			xdg_state_home: temp_dir.path().to_path_buf(),
			blocker_file,
			_temp_dir: temp_dir,
		}
	}

	fn run_format(&self) -> Result<(), String> {
		let output = Command::new(get_binary_path())
			.args(["blocker", "--relative-path", "test_blocker.md", "format"])
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
- move gaussian pivot over in there

# git lfs: docs, music, etc
# eww: don't restore if outdated
# todo: blocker: doesn't add spaces between same level headers";

#[test]
fn test_blocker_format_adds_spaces() {
	let setup = TestSetup::new(INITIAL_CONTENT);

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
    - move gaussian pivot over in there

    # git lfs: docs, music, etc

    # eww: don't restore if outdated

    # todo: blocker: doesn't add spaces between same level headers
    ");
}

#[test]
fn test_blocker_format_idempotent() {
	let setup = TestSetup::new(INITIAL_CONTENT);

	// Run format command first time
	setup.run_format().expect("First format command should succeed");
	let formatted_once = setup.read_blocker_file();

	// Run format command second time (simulating open and close)
	setup.run_format().expect("Second format command should succeed");
	let formatted_twice = setup.read_blocker_file();

	// Should be idempotent
	assert_eq!(formatted_once, formatted_twice, "Formatting should be idempotent");
}
