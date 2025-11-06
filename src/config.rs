use std::{collections::HashSet, path::PathBuf, sync::OnceLock};

use color_eyre::eyre::Result;
use serde::Deserialize;
use v_utils::{io::ExpandedPath, macros::MyConfigPrimitives};

pub static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static EXE_NAME: &str = "todo";

#[derive(Clone, Debug, Default, MyConfigPrimitives, derive_new::new)]
pub struct AppConfig {
	pub todos: Option<Todos>,
	pub timer: Option<Timer>,
	pub milestones: Option<Milestones>,
	pub manual_stats: Option<ManualStats>,
}

#[derive(Clone, Debug, Default, MyConfigPrimitives)]
pub struct Todos {
	pub path: PathBuf,
	pub n_tasks_to_show: usize,
}

#[derive(Clone, Debug, MyConfigPrimitives)]
pub struct Milestones {
	pub github_token: String,
}

#[derive(Clone, Debug, MyConfigPrimitives)]
pub struct ManualStats {
	pub date_format: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Timer {
	pub hard_stop_coeff: f32,
}

impl AppConfig {
	pub fn read(path: Option<ExpandedPath>) -> Result<Self, config::ConfigError> {
		let mut builder = config::Config::builder().add_source(config::Environment::default());
		let (settings, file_config): (Self, Option<config::Config>) = match path {
			Some(ref p) => {
				// Build file-only config for validation
				let file_only = config::Config::builder().add_source(config::File::with_name(&p.to_string()).required(true)).build()?;
				let builder = builder.add_source(config::File::with_name(&p.to_string()).required(true));
				let raw = builder.build()?;
				(raw.try_deserialize()?, Some(file_only))
			}
			None => {
				let app_name = env!("CARGO_PKG_NAME");
				let xdg_dirs = xdg::BaseDirectories::with_prefix(app_name);
				let xdg_conf_dir = xdg_dirs.get_config_home().unwrap().parent().unwrap().display().to_string();

				let locations = [
					format!("{xdg_conf_dir}/{app_name}"),
					format!("{xdg_conf_dir}/{app_name}/config"), //
				];
				// Build file-only config for validation
				let mut file_builder = config::Config::builder();
				for location in locations.iter() {
					builder = builder.add_source(config::File::with_name(location).required(false));
					file_builder = file_builder.add_source(config::File::with_name(location).required(false));
				}
				let raw: config::Config = builder.build()?;
				let file_only = file_builder.build().ok();

				match raw.clone().try_deserialize() {
					Ok(settings) => (settings, file_only),
					Err(e) => {
						eprintln!("Config file does not exist or is invalid:");
						return Err(e);
					}
				}
			}
		};

		// Check for unknown configuration fields (only in file config, not env vars)
		if let Some(file_cfg) = file_config {
			Self::warn_unknown_fields(&file_cfg);
		}

		// Only validate todos path if todos config is present
		if let Some(ref todos) = settings.todos {
			if !todos.path.exists() {
				return Err(config::ConfigError::Message(format!("Configured 'todos' directory does not exist: {}", todos.path.display())));
			}
		}

		#[cfg(not(feature = "is_integration_test"))]
		{
			if std::env::var("XDG_STATE_HOME").is_err() {
				eprintln!("warning: XDG_STATE_HOME is not set, pointing it to ~/.local/state");
				// SAFETY: Only called during initialization, before any threads are spawned
				unsafe {
					std::env::set_var("XDG_STATE_HOME", "~/.local/state");
				}
			}
			let state_dir = STATE_DIR.get_or_init(|| std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap().join(format!("{EXE_NAME}/")));
			let _ = std::fs::create_dir_all(state_dir);
		}

		#[cfg(feature = "is_integration_test")]
		{
			// In integration tests, STATE_DIR must be set from XDG_STATE_HOME environment variable
			let state_dir = STATE_DIR.get_or_init(|| {
				std::env::var("XDG_STATE_HOME")
					.map(PathBuf::from)
					.expect("XDG_STATE_HOME must be set for integration tests")
					.join(format!("{EXE_NAME}/"))
			});
			let _ = std::fs::create_dir_all(state_dir);
		}

		let cache_dir = CACHE_DIR.get_or_init(|| std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("~/.cache").join(EXE_NAME)));
		let _ = std::fs::create_dir_all(cache_dir);

		#[cfg(not(feature = "is_integration_test"))]
		{
			if std::env::var("XDG_DATA_HOME").is_err() {
				eprintln!("warning: XDG_DATA_HOME is not set, pointing it to ~/.local/share");
				// SAFETY: Only called during initialization, before any threads are spawned
				unsafe {
					std::env::set_var("XDG_DATA_HOME", "~/.local/share");
				}
			}
			let data_dir = DATA_DIR.get_or_init(|| std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap().join(format!("{EXE_NAME}/")));
			let _ = std::fs::create_dir_all(data_dir);
		}

		#[cfg(feature = "is_integration_test")]
		{
			// In integration tests, DATA_DIR is set from XDG_DATA_HOME if available
			let data_dir = DATA_DIR.get_or_init(|| {
				std::env::var("XDG_DATA_HOME")
					.map(PathBuf::from)
					.unwrap_or_else(|_| PathBuf::from("~/.local/share"))
					.join(format!("{EXE_NAME}/"))
			});
			let _ = std::fs::create_dir_all(data_dir);
		}

		Ok(settings)
	}

	fn warn_unknown_fields(file_config: &config::Config) {
		// Define all known top-level sections
		let known_sections: HashSet<&str> = ["todos", "timer", "milestones", "manual_stats"].iter().copied().collect();

		// Define known fields for each section
		let known_todos_fields: HashSet<&str> = ["path", "n_tasks_to_show"].iter().copied().collect();
		let known_timer_fields: HashSet<&str> = ["hard_stop_coeff"].iter().copied().collect();
		let known_milestones_fields: HashSet<&str> = ["github_token"].iter().copied().collect();
		let known_manual_stats_fields: HashSet<&str> = ["date_format"].iter().copied().collect();

		// Get all keys from the file config and check for unknown sections
		use std::collections::HashMap;
		if let Ok(table) = file_config.clone().try_deserialize::<HashMap<String, serde_json::Value>>() {
			for (section, value) in table.iter() {
				if !known_sections.contains(section.as_str()) {
					eprintln!("warning: unknown configuration section '[{section}]' will be ignored");
				} else {
					// Check for unknown fields within known sections
					if let serde_json::Value::Object(fields) = value {
						let known_fields = match section.as_str() {
							"todos" => &known_todos_fields,
							"timer" => &known_timer_fields,
							"milestones" => &known_milestones_fields,
							"manual_stats" => &known_manual_stats_fields,
							_ => continue,
						};

						for field_name in fields.keys() {
							if !known_fields.contains(field_name.as_str()) {
								eprintln!("warning: unknown configuration field '[{section}].{field_name}' will be ignored");
							}
						}
					}
				}
			}
		}
	}
}
