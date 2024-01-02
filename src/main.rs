pub mod quickfix;
pub mod utils;
pub mod config;
use config::Config;
use utils::ExpandedPath;

use clap::{Args, Parser, Subcommand};
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
	Open(OpenArgs),
	/// Add a new task
	Add(AddArgs),
	/// Compile list of first priority tasks based on time of day
	Quickfix(QuickfixArgs),
}

#[derive(Args)]
struct OpenArgs {
	#[clap(flatten)]
	shared: TodosFlags,
}
#[derive(Args)]
struct AddArgs {
	name: String,
	#[clap(flatten)]
	shared: TodosFlags,
}
#[derive(Args)]
struct TodosFlags {
	#[arg(long, short)]
	morning: bool,
	#[arg(long, short)]
	work: bool,
	#[arg(long, short)]
	evening: bool,
	#[arg(long, short)]
	open: bool,
}

#[derive(Args)]
struct QuickfixArgs {}

fn main() {
	let cli = Cli::parse();

	let config = match Config::try_from(cli.config) {
		Ok(cfg) => cfg,
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	};

	match cli.command {
		Commands::Open(open_args) => {
			let mut todos_flags = open_args.shared;
			todos_flags.open = true;
			action_todos(config, todos_flags, None);
		}
		Commands::Add(add_args) => {
			action_todos(config, add_args.shared, Some(add_args.name));
		}
		Commands::Quickfix(_) => {
			quickfix::compile(config);
		}
	}
}

fn action_todos(config: Config, flags: TodosFlags, name: Option<String>) {
	let mut path = config.todos.path.0.clone();

	if flags.morning {
		path.push(".morning/");
	} else if flags.work {
		path.push(".work/");
	} else {
		path.push(".evening/");
	}

	if let Some(name) = name {
		path.push([&name, ".md"].concat());
		dbg!(&path);
		let _ = std::fs::File::create(&path).unwrap();
	}

	if flags.open == true {
		utils::open(path);
	}
}
