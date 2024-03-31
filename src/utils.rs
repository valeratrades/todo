use crate::config::Config;
use anyhow::{anyhow, Result};
use chrono::Duration;
use std::{path::PathBuf, process::Command};

//TODO!: make it take a flag for whether to sync with git before and open opening, and add the whole to v_utils \
pub fn open(path: &PathBuf) -> Result<()> {
	if !path.exists() {
		return Err(anyhow!("File does not exist"));
	}
	Command::new("sh")
		.arg("-c")
		.arg(format!("$EDITOR {}", path.display()))
		.status()
		.map_err(|_| anyhow!("$EDITOR env variable is not defined or command failed"))?;

	Ok(())
}

pub fn format_date(days_back: usize, config: &Config) -> String {
	let date: String = (chrono::Utc::now() - Duration::days(days_back as i64))
		.format(&config.date_format.as_str())
		.to_string();
	date
}
