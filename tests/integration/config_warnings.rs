use std::{fs, path::PathBuf, process::Command};

use tempfile::TempDir;

fn get_binary_path() -> PathBuf {
	// Ensure binary is compiled with is_integration_test feature
	crate::ensure_binary_compiled();

	// Get the path to the compiled binary
	let mut path = std::env::current_exe().unwrap();
	path.pop(); // Remove test binary name
	path.pop(); // Remove 'deps'
	path.push("todo");
	path
}

struct TestSetup {
	_temp_dir: TempDir,
	config_file: PathBuf,
}

impl TestSetup {
	fn new(config_content: &str) -> Self {
		let temp_dir = tempfile::tempdir().unwrap();
		let todos_dir = temp_dir.path().join("todos");

		// Create the todos directory first
		fs::create_dir_all(&todos_dir).unwrap();

		// Replace placeholder in config with actual todos dir path
		let config_with_path = config_content.replace("{TODOS_DIR}", todos_dir.to_str().unwrap());
		let config_file = temp_dir.path().join("config.toml");
		fs::write(&config_file, config_with_path).unwrap();

		Self { _temp_dir: temp_dir, config_file }
	}

	fn run_init(&self) -> (String, String) {
		let output = Command::new(get_binary_path())
			.args(["--config", self.config_file.to_str().unwrap(), "init", "bash"])
			.output()
			.unwrap();

		let stdout = String::from_utf8_lossy(&output.stdout).to_string();
		let stderr = String::from_utf8_lossy(&output.stderr).to_string();

		(stdout, stderr)
	}
}

fn sort_lines(s: &str) -> String {
	let mut lines: Vec<&str> = s.lines().collect();
	lines.sort();
	lines.join("\n")
}

#[test]
fn test_warn_unknown_config_section() {
	let config = r#"
[todos]
path = "{TODOS_DIR}"
n_tasks_to_show = 3

[unknown_section]
some_field = "value"
"#;

	let setup = TestSetup::new(config);
	let (_stdout, stderr) = setup.run_init();

	insta::assert_snapshot!(sort_lines(&stderr), @"warning: unknown configuration section '[unknown_section]' will be ignored");
}

#[test]
fn test_warn_unknown_field_in_known_section() {
	let config = r#"
[todos]
path = "{TODOS_DIR}"
n_tasks_to_show = 3
typo_field = "oops"

[manual_stats]
date_format = "%Y-%m-%d"
unknown_field = "value"
"#;

	let setup = TestSetup::new(config);
	let (_stdout, stderr) = setup.run_init();

	insta::assert_snapshot!(sort_lines(&stderr), @r"
	warning: unknown configuration field '[manual_stats].unknown_field' will be ignored
	warning: unknown configuration field '[todos].typo_field' will be ignored
	");
}

#[test]
fn test_no_warnings_for_valid_config() {
	let config = r#"
[todos]
path = "{TODOS_DIR}"
n_tasks_to_show = 3

[timer]
hard_stop_coeff = 1.5

[manual_stats]
date_format = "%Y-%m-%d"
"#;

	let setup = TestSetup::new(config);
	let (_stdout, stderr) = setup.run_init();

	insta::assert_snapshot!(sort_lines(&stderr), @"");
}

#[test]
fn test_multiple_unknown_fields() {
	let config = r#"
[todos]
path = "{TODOS_DIR}"
n_tasks_to_show = 3
unknown1 = "value1"
unknown2 = "value2"

[timer]
hard_stop_coeff = 1.5
unknown3 = "value3"
"#;

	let setup = TestSetup::new(config);
	let (_stdout, stderr) = setup.run_init();

	insta::assert_snapshot!(sort_lines(&stderr), @r"
	warning: unknown configuration field '[timer].unknown3' will be ignored
	warning: unknown configuration field '[todos].unknown1' will be ignored
	warning: unknown configuration field '[todos].unknown2' will be ignored
	");
}
