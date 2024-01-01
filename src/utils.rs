use std::{path::PathBuf, process::Command};

pub fn open(path: PathBuf) {
	Command::new("sh")
		.arg("-c")
		.arg(format!("$EDITOR {}", path.display()))
		.status()
		.expect("$EDITOR env variable is not defined");
}
