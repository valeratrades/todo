use clap::{Args, Subcommand};
use color_eyre::eyre::Result;

use crate::config::{AppConfig, STATE_DIR};

static STATE_APPENDIX: &str = "blocker.txt";

#[derive(Debug, Clone, Args)]
pub struct BlockerArgs {
	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
	/// Append a blocker
	/// #NB
	/// adds one and only one blocker. The structure is **not** a tree for a reason:
	/// - it forces prioritization (high leverage)
	/// - solving top 1 thing can often unlock many smaller ones for free
	Add { name: String },
	/// Pop the last one
	Pop,
	/// Full list of blockers down from the main task
	List,
	/// Get last blocker only
	Top,
}

pub fn main(_settings: AppConfig, args: BlockerArgs) -> Result<()> {
	let blocker_path = STATE_DIR.get().unwrap().join(STATE_APPENDIX);
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
			println!("{}", blockers.join("\n"));
		}
		Command::Top => match blockers.last() {
			Some(last) => println!("{}", last),
			None => println!(),
		},
	};
	Ok(())
}
