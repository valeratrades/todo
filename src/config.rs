use std::{path::PathBuf, sync::OnceLock};

use color_eyre::eyre::Result;
use serde::Deserialize;
use v_utils::{io::ExpandedPath, macros::MyConfigPrimitives};

pub static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static EXE_NAME: &str = "todo";

#[derive(Debug, Default, derive_new::new, Clone, MyConfigPrimitives)]
pub struct AppConfig {
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
	pub fn read(path: Option<ExpandedPath>) -> Result<Self, config::ConfigError> {
		let mut builder = config::Config::builder().add_source(config::Environment::default());
		let settings: Self = match path {
			Some(path) => {
				let builder = builder.add_source(config::File::with_name(&path.to_string()).required(true));
				builder.build()?.try_deserialize()?
			}
			None => {
				let app_name = env!("CARGO_PKG_NAME");
				let app_name = env!("CARGO_PKG_NAME");
				let xdg_dirs = xdg::BaseDirectories::with_prefix(app_name).unwrap();
				let xdg_conf_dir = xdg_dirs.get_config_home().parent().unwrap().display().to_string();

				let locations = [
					format!("{xdg_conf_dir}/{app_name}"),
					format!("{xdg_conf_dir}/{app_name}/config"), //
				];
				for location in locations.iter() {
					builder = builder.add_source(config::File::with_name(location).required(false));
				}
				let raw: config::Config = builder.build()?;

				match raw.try_deserialize() {
					Ok(settings) => settings,
					Err(e) => {
						eprintln!("Config file does not exist or is invalid:");
						return Err(e);
					}
				}
			}
		};

		if !settings.todos.path.exists() {
			return Err(config::ConfigError::Message(format!(
				"Configured 'todos' directory does not exist: {}",
				settings.todos.path.display()
			)));
		}

		if std::env::var("XDG_STATE_HOME").is_err() {
			eprintln!("warning: XDG_STATE_HOME is not set, pointing it to ~/.local/state");
			std::env::set_var("XDG_STATE_HOME", "~/.local/state");
		}
		let state_dir = STATE_DIR.get_or_init(|| std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap().join(format!("{EXE_NAME}/")));
		let _ = std::fs::create_dir_all(state_dir);

		if std::env::var("XDG_DATA_HOME").is_err() {
			eprintln!("warning: XDG_DATA_HOME is not set, pointing it to ~/.local/share");
			std::env::set_var("XDG_DATA_HOME", "~/.local/share");
		}
		let data_dir = DATA_DIR.get_or_init(|| std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap().join(format!("{EXE_NAME}/")));
		let _ = std::fs::create_dir_all(data_dir);

		Ok(settings)
	}
}
