use anyhow::{anyhow, Result};
use std::{path::PathBuf, process::Command};

//TODO!: make it take a flag for whether to sync with git before and open opening, and add the whole to v_utils \
pub fn open(path: &PathBuf) -> Result<()> {
	Command::new("sh")
		.arg("-c")
		.arg(format!("$EDITOR {}", path.display()))
		.status()
		.map_err(|_| anyhow!("$EDITOR env variable is not defined or command failed"))?;

	Ok(())
}
