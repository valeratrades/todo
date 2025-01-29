use clap::{Args, Subcommand};
use color_eyre::eyre::Result;

use crate::config::{AppConfig, STATE_DIR};
use crate::milestones::SPRINT_HEADER_REL_PATH;
use crate::config::DATA_DIR;

static BLOCKER_REL_PATH: &str = "blocker.txt";

#[derive(Debug, Clone, Args)]
pub struct BlockerArgs {
	#[command(subcommand)]
	command: Command,
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
}

pub fn main(_settings: AppConfig, args: BlockerArgs) -> Result<()> {
	let blocker_path = STATE_DIR.get().unwrap().join(BLOCKER_REL_PATH);
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
	};
	Ok(())
}
