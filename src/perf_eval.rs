use std::process::Command;

use chrono::Local;
use clap::Args;
use color_eyre::eyre::{Context, Result};

use crate::config::{AppConfig, CACHE_DIR};

#[derive(Args, Debug)]
pub struct PerfEvalArgs {}

pub fn main(_config: AppConfig, _args: PerfEvalArgs) -> Result<()> {
	let cache_dir = CACHE_DIR.get().ok_or_else(|| color_eyre::eyre::eyre!("CACHE_DIR not initialized"))?;

	let now = Local::now();
	let date_dir = cache_dir.join(now.format("%Y-%m-%d").to_string());

	// Create the date directory if it doesn't exist
	std::fs::create_dir_all(&date_dir).wrap_err(format!("Failed to create directory: {}", date_dir.display()))?;

	let filename = format!("s1-{}.png", now.format("%H-%M-%S"));
	let screenshot_path = date_dir.join(filename);

	// Take screenshot using grim
	let status = Command::new("grim").arg(&screenshot_path).status().wrap_err("Failed to execute grim command")?;

	if !status.success() {
		return Err(color_eyre::eyre::eyre!("grim command failed with status: {}", status));
	}

	println!("Screenshot saved to: {}", screenshot_path.display());

	Ok(())
}
