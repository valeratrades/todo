use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;
use v_utils::io::ExpandedPath;
use v_utils::macros::MyConfigPrimitives;

#[derive(Debug, MyConfigPrimitives)]
pub struct AppConfig {
	pub data_dir: PathBuf,
	pub date_format: String,
	pub todos: Todos,
	pub timer: Timer,
	pub activity_monitor: ActivityMonitor,
}
#[derive(Debug, MyConfigPrimitives)]
pub struct Todos {
	pub path: PathBuf,
	pub n_tasks_to_show: usize,
}
#[derive(Debug, MyConfigPrimitives)]
pub struct ActivityMonitor {
	pub delimitor: String,
	pub calendar_id: String,
	pub google_calendar_refresh_token: String,
	pub google_client_id: String,
	pub google_client_secret: String,
}
#[derive(Debug, Deserialize, Clone)]
pub struct Timer {
	pub hard_stop_coeff: f32,
}

impl AppConfig {
	pub fn new(path: ExpandedPath) -> Result<Self, config::ConfigError> {
		let builder = config::Config::builder().add_source(config::File::with_name(&path.to_string()));

		let settings: config::Config = builder.build()?;
		let settings: Self = settings.try_deserialize()?;

		let _ = std::fs::create_dir_all(&settings.data_dir);
		let _ = std::fs::create_dir_all(&settings.data_dir.join("tmp/"));

		Ok(settings)
	}
}
