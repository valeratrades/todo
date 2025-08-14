use std::{collections::HashMap, io::Write};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

use crate::{
	clockify,
	config::{AppConfig, CACHE_DIR, DATA_DIR, STATE_DIR},
	milestones::SPRINT_HEADER_REL_PATH,
};

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";
static BLOCKER_STATE_FILENAME: &str = "blocker_state.txt";
static WORKSPACE_SETTINGS_FILENAME: &str = "workspace_settings.json";
static BLOCKER_CURRENT_CACHE_FILENAME: &str = "blocker_current_cache.txt";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceSettings {
	legacy: bool,
}

impl Default for WorkspaceSettings {
	fn default() -> Self {
		Self { legacy: false }
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WorkspaceCache {
	workspaces: HashMap<String, WorkspaceSettings>,
}

#[derive(Debug, Clone, Args)]
pub struct BlockerArgs {
	#[command(subcommand)]
	command: Command,
	#[arg(short, long)]
	/// The relative path of the blocker file. Will be appended to the state directory. If contains one slash, the folder name will be used as workspace filter. Can have any text-based format
	relative_path: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
	/// Append a blocker
	/// # NB
	/// adds one and only one blocker. The structure is **not** a tree for a reason:
	/// - it forces prioritization (high leverage)
	/// - solving top 1 thing can often unlock many smaller ones for free
	Add { name: String },
	/// Pop the last one
	Pop,
	/// Full list of blockers down from the main task
	List,
	/// Compactly show the last entry
	Current,
	/// Just open the \`blockers\` file with $EDITOR. Text as interface.
	Open,
	/// Set the default `--relative_path`, for the project you're working on currently.
	Project { relative_path: String },
	/// Resume tracking time on the current blocker task via Clockify
	Resume(ResumeArgs),
	/// Pause tracking time via Clockify
	Pause(PauseArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct ResumeArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,

	/// Project ID or name (if omitted, uses cached project default)
	#[arg(short = 'p', long)]
	pub project: Option<String>,

	/// Task ID or name (optional)
	#[arg(short = 't', long)]
	pub task: Option<String>,

	/// Comma-separated tag IDs or names (optional)
	#[arg(short = 'g', long)]
	pub tags: Option<String>,

	/// Mark entry as billable
	#[arg(short = 'b', long, default_value_t = false)]
	pub billable: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct PauseArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,
}

fn get_blocker_state_path() -> std::path::PathBuf {
	STATE_DIR.get().unwrap().join(BLOCKER_STATE_FILENAME)
}

fn is_blocker_tracking_enabled() -> bool {
	let state_path = get_blocker_state_path();
	match std::fs::read_to_string(&state_path) {
		Ok(content) => content.trim() == "true",
		Err(_) => {
			// File doesn't exist, create it with "false" and return false
			let _ = std::fs::write(&state_path, "false");
			false
		}
	}
}

fn set_blocker_tracking_state(enabled: bool) -> Result<()> {
	let state_path = get_blocker_state_path();
	std::fs::write(&state_path, if enabled { "true" } else { "false" })?;
	Ok(())
}

fn get_current_blocker_cache_path(relative_path: &str) -> std::path::PathBuf {
	let cache_key = relative_path.replace('/', "_");
	CACHE_DIR.get().unwrap().join(format!("{}_{}", cache_key, BLOCKER_CURRENT_CACHE_FILENAME))
}

fn save_current_blocker_cache(relative_path: &str, current_blocker: Option<String>) -> Result<()> {
	let cache_path = get_current_blocker_cache_path(relative_path);
	match current_blocker {
		Some(blocker) => std::fs::write(&cache_path, blocker)?,
		None => {
			let _ = std::fs::remove_file(&cache_path);
		}
	}
	Ok(())
}

fn load_current_blocker_cache(relative_path: &str) -> Option<String> {
	let cache_path = get_current_blocker_cache_path(relative_path);
	std::fs::read_to_string(&cache_path).ok()
}

fn get_current_blocker(relative_path: &str) -> Option<String> {
	let blocker_path = STATE_DIR.get().unwrap().join(relative_path);
	let blockers: Vec<String> = std::fs::read_to_string(&blocker_path)
		.unwrap_or_else(|_| String::new())
		.split('\n')
		.filter(|s| !s.is_empty())
		.map(|s| s.to_owned())
		.collect();
	blockers.last().cloned()
}

fn parse_workspace_from_path(relative_path: &str) -> Result<Option<String>> {
	let slash_count = relative_path.matches('/').count();

	if slash_count == 0 {
		Ok(None)
	} else if slash_count == 1 {
		let parts: Vec<&str> = relative_path.split('/').collect();
		Ok(Some(parts[0].to_string()))
	} else {
		return Err(eyre!("Relative path can contain at most one slash, found {}: {}", slash_count, relative_path));
	}
}

fn get_workspace_settings_path() -> std::path::PathBuf {
	CACHE_DIR.get().unwrap().join(WORKSPACE_SETTINGS_FILENAME)
}

fn load_workspace_cache() -> WorkspaceCache {
	let cache_path = get_workspace_settings_path();
	match std::fs::read_to_string(&cache_path) {
		Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
		Err(_) => WorkspaceCache::default(),
	}
}

fn save_workspace_cache(cache: &WorkspaceCache) -> Result<()> {
	let cache_path = get_workspace_settings_path();
	let content = serde_json::to_string_pretty(cache)?;
	std::fs::write(&cache_path, content)?;
	Ok(())
}

fn get_workspace_legacy_setting(workspace: &str) -> Result<bool> {
	let cache = load_workspace_cache();

	if let Some(settings) = cache.workspaces.get(workspace) {
		Ok(settings.legacy)
	} else {
		// Ask user for preference
		println!("Workspace '{}' legacy mode setting not found.", workspace);
		print!("Use legacy mode for this workspace? [y/N]: ");
		Write::flush(&mut std::io::stdout())?;

		let mut input = String::new();
		std::io::stdin().read_line(&mut input)?;
		let use_legacy = input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes";

		// Save the preference
		let mut cache = load_workspace_cache();
		cache.workspaces.insert(workspace.to_string(), WorkspaceSettings { legacy: use_legacy });
		save_workspace_cache(&cache)?;

		println!("Saved legacy mode preference for workspace '{}': {}", workspace, use_legacy);
		Ok(use_legacy)
	}
}

async fn stop_current_tracking(workspace: Option<&str>) -> Result<()> {
	clockify::stop_time_entry_with_defaults(workspace).await
}

async fn start_tracking_for_task(description: String, relative_path: &str, resume_args: &ResumeArgs, workspace_override: Option<&str>) -> Result<()> {
	let workspace = workspace_override.or_else(|| resume_args.workspace.as_deref());

	// Determine legacy mode from workspace settings
	let legacy = if let Some(ws) = workspace {
		get_workspace_legacy_setting(ws)?
	} else {
		// If no workspace specified, use default (false)
		false
	};

	clockify::start_time_entry_with_defaults(
		workspace,
		resume_args.project.as_deref(),
		description,
		resume_args.task.as_deref(),
		resume_args.tags.as_deref(),
		resume_args.billable,
		legacy,
		Some(relative_path),
	)
	.await
}

fn spawn_blocker_comparison_process(relative_path: String) -> Result<()> {
	use std::process::Command;

	let current_exe = std::env::current_exe()?;

	Command::new(current_exe)
		.args(&["blocker", "--relative-path", &relative_path, "current"])
		.env("_BLOCKER_BACKGROUND_CHECK", "1")
		.spawn()?;

	Ok(())
}

fn handle_background_blocker_check(relative_path: &str) -> Result<()> {
	let cached_current = load_current_blocker_cache(relative_path);
	let actual_current = get_current_blocker(relative_path);

	if cached_current != actual_current {
		if is_blocker_tracking_enabled() {
			let workspace_from_path = parse_workspace_from_path(relative_path)?;

			tokio::runtime::Runtime::new()?.block_on(async {
				let _ = stop_current_tracking(workspace_from_path.as_deref()).await;

				if let Some(new_task) = &actual_current {
					let default_resume_args = ResumeArgs {
						workspace: None,
						project: None,
						task: None,
						tags: None,
						billable: false,
					};

					if let Err(e) = start_tracking_for_task(new_task.clone(), relative_path, &default_resume_args, workspace_from_path.as_deref()).await {
						eprintln!("Warning: Failed to start tracking for updated task: {}", e);
					}
				}
			});
		}

		save_current_blocker_cache(relative_path, actual_current)?;
	}

	Ok(())
}

pub fn main(_settings: AppConfig, args: BlockerArgs) -> Result<()> {
	let relative_path = match args.relative_path {
		Some(f) => f,
		None => {
			let persisted_project_file = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			match std::fs::read_to_string(&persisted_project_file) {
				Ok(s) => s,
				Err(_) => "blockers.txt".to_string(),
			}
		}
	};

	// Handle background blocker check
	if std::env::var("_BLOCKER_BACKGROUND_CHECK").is_ok() {
		return handle_background_blocker_check(&relative_path);
	}

	// Parse workspace from path if it contains a slash
	let workspace_from_path = parse_workspace_from_path(&relative_path)?;

	let blocker_path = STATE_DIR.get().unwrap().join(&relative_path);
	let mut blockers: Vec<String> = std::fs::read_to_string(&blocker_path)
		.unwrap_or_else(|_| String::new())
		.split('\n')
		.filter(|s| !s.is_empty())
		.map(|s| s.to_owned())
		.collect();

	match args.command {
		Command::Add { name } => {
			// If tracking is enabled, stop current task before adding new one
			if is_blocker_tracking_enabled() {
				tokio::runtime::Runtime::new()?.block_on(async {
					let _ = stop_current_tracking(workspace_from_path.as_deref()).await; // Ignore errors when stopping
				});
			}

			blockers.push(name.clone());
			std::fs::write(&blocker_path, blockers.join("\n"))?;

			// Save current blocker to cache
			save_current_blocker_cache(&relative_path, Some(name.clone()))?;

			// If tracking is enabled, start tracking the new task
			if is_blocker_tracking_enabled() {
				let default_resume_args = ResumeArgs {
					workspace: None,
					project: None,
					task: None,
					tags: None,
					billable: false,
				};

				tokio::runtime::Runtime::new()?.block_on(async {
					if let Err(e) = start_tracking_for_task(name, &relative_path, &default_resume_args, workspace_from_path.as_deref()).await {
						eprintln!("Warning: Failed to start tracking for new task: {}", e);
					}
				});
			}
		}
		Command::Pop => {
			// If tracking is enabled, stop current task before popping
			if is_blocker_tracking_enabled() {
				tokio::runtime::Runtime::new()?.block_on(async {
					let _ = stop_current_tracking(workspace_from_path.as_deref()).await; // Ignore errors when stopping
				});
			}

			blockers.pop();
			std::fs::write(&blocker_path, blockers.join("\n"))?;

			// Save current blocker to cache
			save_current_blocker_cache(&relative_path, blockers.last().cloned())?;

			// If tracking is enabled and there's still a task, start tracking it
			if is_blocker_tracking_enabled() {
				if let Some(current_task) = blockers.last() {
					let default_resume_args = ResumeArgs {
						workspace: None,
						project: None,
						task: None,
						tags: None,
						billable: false,
					};

					tokio::runtime::Runtime::new()?.block_on(async {
						if let Err(e) = start_tracking_for_task(current_task.clone(), &relative_path, &default_resume_args, workspace_from_path.as_deref()).await {
							eprintln!("Warning: Failed to start tracking for previous task: {}", e);
						}
					});
				}
			}
		}
		Command::List => {
			let sprint_header = match std::fs::read_to_string(DATA_DIR.get().unwrap().join(SPRINT_HEADER_REL_PATH)) {
				Ok(s) => Some(s),
				Err(_) => None,
			};
			if let Some(s) = sprint_header {
				println!("{s}");
			}
			println!("{}", blockers.join("\n"));
		}
		Command::Current =>
			if let Some(last) = blockers.last() {
				const MAX_LEN: usize = 70;
				match last.len() {
					0..=MAX_LEN => println!("{}", last),
					_ => println!("{}...", &last[..(MAX_LEN - 3)]),
				}
			},
		Command::Open => {
			// Save current blocker state to cache before opening
			save_current_blocker_cache(&relative_path, blockers.last().cloned())?;

			// Open the file
			v_utils::io::open(&blocker_path)?;

			// Spawn background process to check for changes after editor closes
			spawn_blocker_comparison_process(relative_path.clone())?;
		}
		Command::Project { relative_path } => {
			// Validate the path before saving
			parse_workspace_from_path(&relative_path)?;
			let state_dir = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			std::fs::write(&state_dir, &relative_path)?;

			// Spawn background process to check for clockify updates after project change
			spawn_blocker_comparison_process(relative_path)?;
		}
		Command::Resume(resume_args) => {
			// Get current blocker task description
			let description = match blockers.last() {
				Some(task) => task.clone(),
				None => return Err(eyre!("No current blocker task found. Add one with 'todo blocker add <task>'")),
			};

			// Enable tracking state
			set_blocker_tracking_state(true)?;

			tokio::runtime::Runtime::new()?.block_on(async {
				let workspace = workspace_from_path.as_deref().or_else(|| resume_args.workspace.as_deref());

				// Determine legacy mode from workspace settings
				let legacy = if let Some(ws) = workspace { get_workspace_legacy_setting(ws)? } else { false };

				clockify::start_time_entry_with_defaults(
					workspace,
					resume_args.project.as_deref(),
					description,
					resume_args.task.as_deref(),
					resume_args.tags.as_deref(),
					resume_args.billable,
					legacy,
					Some(&relative_path), // Pass the relative_path for legacy mode
				)
				.await
			})?;
		}
		Command::Pause(pause_args) => {
			// Disable tracking state
			set_blocker_tracking_state(false)?;

			let workspace = workspace_from_path.as_deref().or_else(|| pause_args.workspace.as_deref());
			tokio::runtime::Runtime::new()?.block_on(async { clockify::stop_time_entry_with_defaults(workspace).await })?;
		}
	};
	Ok(())
}
