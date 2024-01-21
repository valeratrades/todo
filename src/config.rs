use crate::utils::ExpandedPath;
use anyhow::{Context, Result};
use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;
use std::fmt;
use std::{convert::TryFrom, path::PathBuf};

impl TryFrom<ExpandedPath> for Config {
	type Error = anyhow::Error;

	fn try_from(path: ExpandedPath) -> Result<Self> {
		let config_str = std::fs::read_to_string(&path).with_context(|| format!("Failed to read config file at {:?}", path))?;

		let raw_config: RawConfig = toml::from_str(&config_str)
			.with_context(|| "The config file is not correctly formatted TOML\nand/or\n is missing some of the required fields")?;

		let config = raw_config.process()?;
		let _ = std::fs::create_dir_all(&config.data_dir);
		let _ = std::fs::create_dir_all(&config.data_dir.join("tmp/"));

		Ok(config)
	}
}

//-----------------------------------------------------------------------------
// Processed Config
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
pub struct ActivityMonitor {
	pub delimitor: String,
	pub calendar_id: String,
	pub calendar_token: String,
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
	pub raw_activity_monitor: RawActivityMonitor,
}
impl RawConfig {
	fn process(&self) -> Result<Config> {
		Ok(Config {
			data_dir: self.data_dir.0.clone(),
			date_format: self.date_format.clone(),
			todos: self.todos.process(),
			timer: self.timer.clone(),
			activity_monitor: self.raw_activity_monitor.process()?,
		})
	}
}

#[derive(Deserialize)]
pub struct RawTodos {
	pub path: ExpandedPath,
	pub n_tasks_to_show: usize,
}
impl RawTodos {
	fn process(&self) -> Todos {
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
pub struct RawActivityMonitor {
	pub delimitor: String,
	pub calendar_id: String,
	pub calendar_token: PrivateValue,
}
impl RawActivityMonitor {
	fn process(&self) -> Result<ActivityMonitor> {
		Ok(ActivityMonitor {
			delimitor: self.delimitor.clone(),
			calendar_id: self.calendar_id.clone(),
			calendar_token: self.calendar_token.process()?,
		})
	}
}

#[derive(Clone, Debug)]
pub enum PrivateValue {
	String(String),
	Env { env: String },
}
impl PrivateValue {
	pub fn process(&self) -> Result<String> {
		match self {
			PrivateValue::String(s) => Ok(s.clone()),
			PrivateValue::Env { env } => std::env::var(env).with_context(|| format!("Environment variable '{}' not found", env)),
		}
	}
}
impl<'de> Deserialize<'de> for PrivateValue {
	fn deserialize<D>(deserializer: D) -> Result<PrivateValue, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct PrivateValueVisitor;

		impl<'de> Visitor<'de> for PrivateValueVisitor {
			type Value = PrivateValue;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				formatter.write_str("a string or a map with a single key 'env'")
			}

			fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				Ok(PrivateValue::String(value.to_owned()))
			}

			fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
			where
				M: de::MapAccess<'de>,
			{
				let key: String = access.next_key()?.ok_or_else(|| de::Error::custom("expected a key"))?;
				if key == "env" {
					let value: String = access.next_value()?;
					Ok(PrivateValue::Env { env: value })
				} else {
					Err(de::Error::custom("expected key to be 'env'"))
				}
			}
		}

		deserializer.deserialize_any(PrivateValueVisitor)
	}
}
