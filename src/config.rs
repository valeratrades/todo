use crate::utils::ExpandedPath;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::convert::TryFrom;

#[derive(Deserialize)]
pub struct Config {
	pub todos: Todos,
	pub manual_stats: ManualStats,
	pub timer: Timer,
	pub activity_monitor: ActivityMonitor,
}

#[derive(Deserialize)]
pub struct Todos {
	pub path: ExpandedPath,
	pub n_tasks_to_show: usize,
}

#[derive(Deserialize)]
pub struct ManualStats {
	pub path: ExpandedPath,
}

#[derive(Deserialize)]
pub struct Timer {
	pub state_path: ExpandedPath,
	pub save_path: ExpandedPath,
	pub hard_stop_coeff: f32,
}

#[derive(Deserialize)]
pub struct ActivityMonitor {
	pub save_dir: ExpandedPath,
	pub delimitor: String,
}

impl TryFrom<ExpandedPath> for Config {
	type Error = anyhow::Error;

	fn try_from(path: ExpandedPath) -> Result<Self> {
		let config_str = std::fs::read_to_string(&path).with_context(|| format!("Failed to read config file at {:?}", path))?;

		let config: Config = toml::from_str(&config_str)
			.with_context(|| "The config file is not correctly formatted TOML\nand/or\n is missing some of the required fields")?;

		Ok(config)
	}
}
