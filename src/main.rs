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
struct AddArgs {
	name: String,

	#[arg(long, short)]
	morning: bool,
	#[arg(long, short)]
	work: bool,
	#[arg(long, short)]
	evening: bool,
	#[arg(long, short)]
	open: bool,
}
trait Todos {
	fn dir(&self) -> PathBuf;
	fn add(&self, path: PathBuf) -> PathBuf;
	fn open(&self, path: PathBuf);
}
impl Todos for OpenArgs {
	fn dir(&self) -> PathBuf {
		let mut path = PathBuf::from(TODO_DIR);

		if self.morning {
			path.push(".morning/");
		} else if self.work {
			path.push(".work/");
		} else {
			path.push(".evening/");
		}
		path
	}

	fn add(&self, path: PathBuf) -> PathBuf {
		path
	}

	fn open(&self, path: PathBuf) {
		if self.open == true {
			Command::new("sh")
				.arg("-c")
				.arg(format!("$EDITOR {}", path.display()))
				.status()
				.expect("$EDITOR env variable is not defined");
		}
	}
}
impl Todos for AddArgs {
	fn dir(&self) -> PathBuf {
		let mut path = PathBuf::from(TODO_DIR);

		if self.morning {
			path.push(".morning/");
		} else if self.work {
			path.push(".work/");
		} else {
			path.push(".evening/");
		}
		path
	}

	fn add(&self, mut path: PathBuf) -> PathBuf {
		path.push([&self.name, ".md"].concat());
		let _ = std::fs::File::create(&path).unwrap();
		path
	}

	fn open(&self, path: PathBuf) {
		if self.open == true {
			Command::new("sh")
				.arg("-c")
				.arg(format!("$EDITOR {}", path.display()))
				.status()
				.expect("$EDITOR env variable is not defined");
		}
	}
}

fn main() {
	let cli = Cli::parse();

	match cli.command {
		Commands::Open(mut open_args) => {
			open_args.open = true;
			action_todos(open_args);
		}
		Commands::Add(add_args) => {
			action_todos(add_args);
		}
	}
}

fn action_todos<T: Todos>(args: T) {
	let mut path = args.dir();

	path = args.add(path);

	args.open(path);
}
