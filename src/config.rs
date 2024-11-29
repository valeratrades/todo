use std::path::PathBuf;

use color_eyre::eyre::Result;
use serde::Deserialize;
use v_utils::{io::ExpandedPath, macros::MyConfigPrimitives};

#[derive(Debug, Default, derive_new::new, Clone, MyConfigPrimitives)]
pub struct AppConfig {
	pub data_dir: PathBuf,
	pub github_token: String,
	pub date_format: String,
	pub todos: Todos,
	pub timer: Timer,
	pub activity_monitor: ActivityMonitor,
}
#[derive(Default, Clone, derive_new::new, Debug, MyConfigPrimitives)]
pub struct Todos {
	pub path: PathBuf,
	pub n_tasks_to_show: usize,
}
#[derive(Default, Clone, derive_new::new, Debug, MyConfigPrimitives)]
pub struct ActivityMonitor {
	pub delimitor: String,
	pub calendar_id: String,
	pub google_calendar_refresh_token: String,
	pub google_client_id: String,
	pub google_client_secret: String,
}
#[derive(Default, Clone, derive_new::new, Debug, Deserialize)]
pub struct Timer {
	pub hard_stop_coeff: f32,
}

impl AppConfig {
	pub fn read(path: ExpandedPath) -> Result<Self, config::ConfigError> {
		let builder = config::Config::builder().add_source(config::File::with_name(&path.to_string()));

		let settings: config::Config = builder.build()?;
		let settings: Self = settings.try_deserialize()?;

		if !settings.todos.path.exists() {
			return Err(config::ConfigError::Message(format!(
				"Configured 'todos' directory does not exist: {}",
				settings.todos.path.display()
			)));
		}

		if !std::env::var("XDG_STATE_HOME").is_ok() {
			eprintln!("warning: XDG_STATE_HOME is not set, pointing it to ~/.local/state");
			std::env::set_var("XDG_STATE_HOME", "~/.local/state");
			let _ = std::fs::create_dir_all("~/.local/state");
		}

		let _ = std::fs::create_dir_all(&settings.data_dir);
		let _ = std::fs::create_dir_all(settings.data_dir.join("tmp/"));

		Ok(settings)
	}
}
