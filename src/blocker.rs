use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, eyre};

use crate::{
	clockify,
	config::{AppConfig, CACHE_DIR, DATA_DIR, STATE_DIR},
	milestones::SPRINT_HEADER_REL_PATH,
};

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";

#[derive(Debug, Clone, Args)]
pub struct BlockerArgs {
	#[command(subcommand)]
	command: Command,
	#[arg(short, long)]
	/// The filename of the blocker file. Will be appended to the state directory. Can have any text-based format
	filename: Option<String>,
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
	/// Set the default `--filename`, for the project you're working on currently.
	Project { filename: String },
	/// Start tracking time on the current blocker task via Clockify
	Start(StartArgs),
	/// Stop tracking time via Clockify
	Stop(StopArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct StartArgs {
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
pub struct StopArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,
}

pub fn main(_settings: AppConfig, args: BlockerArgs) -> Result<()> {
	let filename = match args.filename {
		Some(f) => f,
		None => {
			let persisted_project_file = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			match std::fs::read_to_string(&persisted_project_file) {
				Ok(s) => s,
				Err(_) => "blockers.txt".to_string(),
			}
		}
	};

	let blocker_path = STATE_DIR.get().unwrap().join(&filename);
	let mut blockers: Vec<String> = std::fs::read_to_string(&blocker_path)
		.unwrap_or_else(|_| String::new())
		.split('\n')
		.filter(|s| !s.is_empty())
		.map(|s| s.to_owned())
		.collect();

	match args.command {
		Command::Add { name } => {
			blockers.push(name);
			std::fs::write(&blocker_path, blockers.join("\n"))?;
		}
		Command::Pop => {
			blockers.pop();
			std::fs::write(&blocker_path, blockers.join("\n"))?;
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
		Command::Project { filename } => {
			let state_dir = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			std::fs::write(&state_dir, filename)?;
		}
		Command::Start(start_args) => {
			// Get current blocker task description
			let description = match blockers.last() {
				Some(task) => task.clone(),
				None => return Err(eyre!("No current blocker task found. Add one with 'todo blocker add <task>'")),
			};

			tokio::runtime::Runtime::new()?.block_on(async {
				clockify::start_time_entry_with_defaults(
					start_args.workspace.as_deref(),
					start_args.project.as_deref(),
					description,
					start_args.task.as_deref(),
					start_args.tags.as_deref(),
					start_args.billable,
					start_args.legacy,
					Some(&filename), // Pass the filename for legacy mode
				)
				.await
			})?;
		}
		Command::Stop(stop_args) => {
			tokio::runtime::Runtime::new()?.block_on(async { clockify::stop_time_entry_with_defaults(stop_args.workspace.as_deref()).await })?;
		}
	};
	Ok(())
}
