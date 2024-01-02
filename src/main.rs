pub mod todos;
pub mod day_section;
pub mod utils;
pub mod config;
use config::Config;
use utils::ExpandedPath;

use clap::{Parser, Subcommand};
//use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[arg(long, default_value = "~/.config/todo.toml")]
	config: ExpandedPath,
}

#[derive(Subcommand)]
enum Commands {
	/// Opens the target path
	Open(todos::OpenArgs),
	/// Add a new task
	Add(todos::AddArgs),
	/// Compile list of first priority tasks based on time of day
	Quickfix(todos::QuickfixArgs),
}

fn main() {
	let cli = Cli::parse();

	let config = match Config::try_from(cli.config) {
		Ok(cfg) => cfg,
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	};

	// All the functions here can rely on config being correct.
	let success = match cli.command {
		Commands::Open(open_args) => {
			let mut todos_flags = open_args.shared;
			todos_flags.open = true;
			todos::open_or_add(config, todos_flags, None)
		}
		Commands::Add(add_args) => {
			todos::open_or_add(config, add_args.shared, Some(add_args.name))
		}
		Commands::Quickfix(_) => {
			todos::compile_quickfix(config)
		}
	};

	match success {
		Ok(_) => std::process::exit(0),
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	}
}
