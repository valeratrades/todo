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
	config_file: PathBuf,
	_todos_dir: PathBuf,
}

impl TestSetup {
	fn new(config_content: &str) -> Self {
		let temp_dir = tempfile::tempdir().unwrap();
		let config_file = temp_dir.path().join("config.toml");
		let todos_dir = temp_dir.path().join("todos");

		fs::create_dir_all(&todos_dir).unwrap();
		fs::write(&config_file, config_content).unwrap();

		Self {
			_temp_dir: temp_dir,
			config_file,
			_todos_dir: todos_dir,
		}
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

#[test]
fn test_warn_unknown_config_section() {
	let config = format!(
		r#"
[todos]
path = "{}"
n_tasks_to_show = 3

[unknown_section]
some_field = "value"
"#,
		std::env::temp_dir().join("test_todos").display()
	);

	let setup = TestSetup::new(&config);
	let (_stdout, stderr) = setup.run_init();

	assert!(
		stderr.contains("warning: unknown configuration section '[unknown_section]' will be ignored"),
		"Expected warning about unknown section, got stderr:\n{}",
		stderr
	);
}

#[test]
fn test_warn_unknown_field_in_known_section() {
	let config = format!(
		r#"
[todos]
path = "{}"
n_tasks_to_show = 3
typo_field = "oops"

[manual_stats]
date_format = "%Y-%m-%d"
unknown_field = "value"
"#,
		std::env::temp_dir().join("test_todos").display()
	);

	let setup = TestSetup::new(&config);
	let (_stdout, stderr) = setup.run_init();

	assert!(
		stderr.contains("warning: unknown configuration field '[todos].typo_field' will be ignored"),
		"Expected warning about typo_field in [todos], got stderr:\n{}",
		stderr
	);

	assert!(
		stderr.contains("warning: unknown configuration field '[manual_stats].unknown_field' will be ignored"),
		"Expected warning about unknown_field in [manual_stats], got stderr:\n{}",
		stderr
	);
}

#[test]
fn test_no_warnings_for_valid_config() {
	let config = format!(
		r#"
[todos]
path = "{}"
n_tasks_to_show = 3

[timer]
hard_stop_coeff = 1.5

[manual_stats]
date_format = "%Y-%m-%d"
"#,
		std::env::temp_dir().join("test_todos").display()
	);

	let setup = TestSetup::new(&config);
	let (_stdout, stderr) = setup.run_init();

	// Filter out warnings about XDG env vars (those are expected and unrelated to config validation)
	let config_warnings: Vec<&str> = stderr.lines().filter(|line| line.contains("unknown configuration")).collect();

	assert!(
		config_warnings.is_empty(),
		"Expected no config warnings for valid config, but got:\n{}",
		config_warnings.join("\n")
	);
}

#[test]
fn test_multiple_unknown_fields() {
	let config = format!(
		r#"
[todos]
path = "{}"
n_tasks_to_show = 3
unknown1 = "value1"
unknown2 = "value2"

[timer]
hard_stop_coeff = 1.5
unknown3 = "value3"
"#,
		std::env::temp_dir().join("test_todos").display()
	);

	let setup = TestSetup::new(&config);
	let (_stdout, stderr) = setup.run_init();

	assert!(
		stderr.contains("warning: unknown configuration field '[todos].unknown1' will be ignored"),
		"Expected warning about unknown1"
	);
	assert!(
		stderr.contains("warning: unknown configuration field '[todos].unknown2' will be ignored"),
		"Expected warning about unknown2"
	);
	assert!(
		stderr.contains("warning: unknown configuration field '[timer].unknown3' will be ignored"),
		"Expected warning about unknown3"
	);
}
