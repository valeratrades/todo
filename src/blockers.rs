use clap::{Subcommand, Args};
use color_eyre::eyre::Result;

use crate::config::AppConfig;

#[derive(Debug, Clone, Args)]
pub struct BlockersArgs{
	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
	//TODO: Add(Vec<String>),
	Add,
	Pop,
	List,
}
//- commands: add (should handle multiples), pop, list
impl Command {
	pub fn main(&self) -> Result<()> {
		match self {
			Command::Add => Self::add(),
			Command::Pop => Self::pop(),
			Command::List => Self::list(),
		}
	}

	fn add() -> Result<()> {
		Ok(())
	}

	fn pop() -> Result<()> {
		Ok(())
	}

	fn list() -> Result<()> {
		dbg!(&"list");
		Ok(())
	}
}


pub fn main(_settings: AppConfig, args: BlockersArgs) -> Result<()> {
	args.command.main()
}
