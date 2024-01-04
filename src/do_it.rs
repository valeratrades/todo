use crate::utils;
use anyhow::{Context, Result};
use clap::Args;
use clap::{Parser, Subcommand};

pub fn timing_the_task(config: Config, args: DoArgs) -> Result<()> {
	let state_path = config.do_it.state_path.0.clone();
	let save_path = config.do_it.state_path.0.clone();

	match args {
		DoCommands::Open => {
			utils::open(&path);
		}
	}

	Ok(())
}

#[derive(Args)]
pub struct DoArgs {
	#[command(subcommand)]
	command: DoCommands,
}

#[derive(Subcommand)]
enum DoCommands {
	/// Start a timer for a task
	Start(DoStartArgs),
	Done(),
	Failed(),
	Open(),
}

#[derive(Args)]
struct DoStartArgs {
	#[arg(short, long, default_value = 90)]
	time: u32,
	#[arg(short, long)]
	description: Option<String>,
	#[clap(flatten)]
	category: Option<CategoryFlags>,
}

#[derive(Args)]
struct CategoryFlags {
	//	"",
	//"d:data-collection",
	//"h:home-chore",
	//"w:workout",
	//"ci:close-git-issue",
	//"t:tooling",
	//"l:work-on-library",
	//"s:trading-systems",
	//"cp:code-python",
	//"cr:code-rust",
	//"cg:code-go",
	//"pd:personal-data-collection",
}
