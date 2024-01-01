use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::Command;

const TODO_DIR: &'static str = "/home/v/Todo/";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// opens the target path
	Open(OpenArgs),
	Add(AddArgs),
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

fn main() {
	let cli = Cli::parse();

	match cli.command {
		Commands::Open(open_args) => {
			let mut todos_flags = open_args.shared;
			todos_flags.open = true;
			action_todos(todos_flags, None);
		}
		Commands::Add(add_args) => {
			action_todos(add_args.shared, Some(add_args.name));
		}
	}
}

fn action_todos(flags: TodosFlags, name: Option<String>) {
	let mut path = PathBuf::from(TODO_DIR);

	if flags.morning {
		path.push(".morning/");
	} else if flags.work {
		path.push(".work/");
	} else {
		path.push(".evening/");
	}

	if let Some(name) = name {
		path.push([&name, ".md"].concat());
		let _ = std::fs::File::create(&path).unwrap();
	}

	if flags.open == true {
		Command::new("sh")
			.arg("-c")
			.arg(format!("$EDITOR {}", path.display()))
			.status()
			.expect("$EDITOR env variable is not defined");
	}
}
