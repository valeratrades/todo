use crate::utils::ExpandedPath;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{convert::TryFrom, path::PathBuf};

impl TryFrom<ExpandedPath> for Config {
	type Error = anyhow::Error;

	fn try_from(path: ExpandedPath) -> Result<Self> {
		let config_str = std::fs::read_to_string(&path).with_context(|| format!("Failed to read config file at {:?}", path))?;

		let raw_config: RawConfig = toml::from_str(&config_str)
			.with_context(|| "The config file is not correctly formatted TOML\nand/or\n is missing some of the required fields")?;

		let config = raw_config.apprehend();
		let _ = std::fs::create_dir_all(&config.data_dir);

		Ok(config)
	}
}

//-----------------------------------------------------------------------------
// Apprehended Config
//-----------------------------------------------------------------------------

pub struct Config {
	pub data_dir: PathBuf,
	pub date_format: String,
	pub todos: Todos,
	pub timer: Timer,
	pub activity_monitor: ActivityMonitor,
}
pub struct Todos {
	pub path: PathBuf,
	pub n_tasks_to_show: usize,
}

//-----------------------------------------------------------------------------
// Raw Config
//-----------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RawConfig {
	pub data_dir: ExpandedPath,
	pub date_format: String,
	pub todos: RawTodos,
	pub timer: Timer,
	pub activity_monitor: ActivityMonitor,
}
impl RawConfig {
	fn apprehend(&self) -> Config {
		Config {
			data_dir: self.data_dir.0.clone(),
			date_format: self.date_format.clone(),
			todos: self.todos.apprehend(),
			timer: self.timer.clone(),
			activity_monitor: self.activity_monitor.clone(),
		}
	}
}

#[derive(Deserialize)]
pub struct RawTodos {
	pub path: ExpandedPath,
	pub n_tasks_to_show: usize,
}
impl RawTodos {
	fn apprehend(&self) -> Todos {
		Todos {
			path: self.path.0.clone(),
			n_tasks_to_show: self.n_tasks_to_show,
		}
	}
}

#[derive(Deserialize, Clone)]
pub struct Timer {
	pub hard_stop_coeff: f32,
}

#[derive(Deserialize, Clone)]
pub struct ActivityMonitor {
	pub delimitor: String,
}
