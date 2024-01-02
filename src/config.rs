use crate::utils::ExpandedPath;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::convert::TryFrom;

#[derive(Deserialize)]
pub struct Config {
	pub todos: Todos,
}

#[derive(Deserialize)]
pub struct Todos {
	pub path: ExpandedPath,
	pub n_tasks_to_show: u8,
}

impl TryFrom<ExpandedPath> for Config {
	type Error = anyhow::Error;

	fn try_from(path: ExpandedPath) -> Result<Self> {
		let config_str = std::fs::read_to_string(&path).with_context(|| format!("Failed to read config file at {:?}", path))?;

		let config: Config = toml::from_str(&config_str).with_context(|| "The config file is not correctly formatted TOML")?;

		Ok(config)
	}
}
