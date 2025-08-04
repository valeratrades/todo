use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, eyre};

use crate::{
	clockify,
	config::{AppConfig, CACHE_DIR, DATA_DIR, STATE_DIR},
	milestones::SPRINT_HEADER_REL_PATH,
};

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";
static BLOCKER_STATE_FILENAME: &str = "blocker_state.txt";

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

	/// Use legacy mode: hardcoded project ID with filename as description prefix
	#[arg(long)]
	pub legacy: bool,
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

async fn stop_current_tracking(workspace: Option<&str>) -> Result<()> {
	clockify::stop_time_entry_with_defaults(workspace).await
}

async fn start_tracking_for_task(
	description: String,
	relative_path: &str,
	resume_args: &ResumeArgs,
	workspace_override: Option<&str>,
) -> Result<()> {
	let workspace = workspace_override.or_else(|| resume_args.workspace.as_deref());
	
	clockify::start_time_entry_with_defaults(
		workspace,
		resume_args.project.as_deref(),
		description,
		resume_args.task.as_deref(),
		resume_args.tags.as_deref(),
		resume_args.billable,
		resume_args.legacy,
		Some(relative_path),
	)
	.await
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
			
			// If tracking is enabled, start tracking the new task
			if is_blocker_tracking_enabled() {
				let default_resume_args = ResumeArgs {
					workspace: None,
					project: None,
					task: None,
					tags: None,
					billable: false,
					legacy: false,
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
			
			// If tracking is enabled and there's still a task, start tracking it
			if is_blocker_tracking_enabled() {
				if let Some(current_task) = blockers.last() {
					let default_resume_args = ResumeArgs {
						workspace: None,
						project: None,
						task: None,
						tags: None,
						billable: false,
						legacy: false,
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
			v_utils::io::open(&blocker_path)?;
		}
		Command::Project { relative_path } => {
			// Validate the path before saving
			parse_workspace_from_path(&relative_path)?;
			let state_dir = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			std::fs::write(&state_dir, relative_path)?;
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
				
				clockify::start_time_entry_with_defaults(
					workspace,
					resume_args.project.as_deref(),
					description,
					resume_args.task.as_deref(),
					resume_args.tags.as_deref(),
					resume_args.billable,
					resume_args.legacy,
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
