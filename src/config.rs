use std::{path::PathBuf, sync::OnceLock};

use color_eyre::eyre::Result;
use serde::Deserialize;
use v_utils::{io::ExpandedPath, macros::MyConfigPrimitives};

pub static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static EXE_NAME: &str = "todo";

#[derive(Clone, Debug, Default, MyConfigPrimitives, derive_new::new)]
pub struct AppConfig {
	pub github_token: String,
	pub date_format: String,
	pub todos: Todos,
	pub timer: Timer,
}
#[derive(Clone, Debug, Default, MyConfigPrimitives)]
pub struct Todos {
	pub path: PathBuf,
	pub n_tasks_to_show: usize,
}
#[derive(Clone, Debug, Default, Deserialize)]
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
				let xdg_dirs = xdg::BaseDirectories::with_prefix(app_name);
				let xdg_conf_dir = xdg_dirs.get_config_home().unwrap().parent().unwrap().display().to_string();

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

		let cache_dir = CACHE_DIR.get_or_init(|| std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("~/.cache").join(EXE_NAME)));
		let _ = std::fs::create_dir_all(cache_dir);

		if std::env::var("XDG_DATA_HOME").is_err() {
			eprintln!("warning: XDG_DATA_HOME is not set, pointing it to ~/.local/share");
			std::env::set_var("XDG_DATA_HOME", "~/.local/share");
		}
		let data_dir = DATA_DIR.get_or_init(|| std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap().join(format!("{EXE_NAME}/")));
		let _ = std::fs::create_dir_all(data_dir);

		Ok(settings)
	}
}
