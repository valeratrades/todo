#![allow(clippy::get_first)]
#![allow(clippy::len_zero)]
#![feature(trait_alias)]

mod activity_monitor;
pub mod config;
pub mod day_section;
mod manual_stats;
pub mod mocks;
mod timer;
mod todos;
pub mod utils;
use clap::{Args, Parser, Subcommand};
use config::AppConfig;
use v_utils::io::ExpandedPath;

const MANUAL_PATH_APPENDIX: &str = "manual_stats/";
const MONITOR_PATH_APPENDIX: &str = "activities_monitor/";
const TOTALS_PATH_APPENDIX: &str = "activities_totals/";
const ONGOING_PATH_APPENDIX: &str = "tmp/timer_ongoing.json";
const TIMED_PATH_APPENDIX: &str = "timed_tasks/";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[arg(long, default_value = "~/.config/todo.toml")]
	config: ExpandedPath,
}

#[derive(Subcommand)]
enum Commands {
	/// Opens the target path
	///  Ex
	///```rust
	///todo open -w
	///```
	Open(todos::OpenArgs),
	/// Add a new task.
	/// Every entry has the following format:
	/// `{importance}-{difficulty}-{name}`,
	///where:
	///- importance: 0->9, the higher the more important
	///- difficulty: 0->9, the higher the more difficult
	///  Ex:
	///```rust
	///todo add 2-3-test -n
	///```
	Add(todos::AddArgs),
	/// Compile list of first priority tasks based on time of day
	///  Ex:
	///```rust
	///todo quickfix
	///```
	Quickfix(NoArgs),
	/// Record day's ev and other stats.
	///Following records ev of 420 for yesterday, then opens the file.
	///```rust
	///todo manual -d1 --ev 420 -o
	///```
	Manual(manual_stats::ManualArgs),
	/// Start a task with timer, then store error (to track improvement of your estimations of time spent on different task categories)
	///  Ex:
	///'''rust
	///todo do start -t=15 -w --description==do-da-work
	///. . . // start doing the task, then:
	///todo do done
	///'''
	Timer(timer::TimerArgs),
	/// Start monitoring user activities
	Monitor(NoArgs),
}
#[derive(Args)]
struct NoArgs {}

fn main() {
	let cli = Cli::parse();

	let config = match AppConfig::read(cli.config) {
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
		Commands::Add(add_args) => todos::open_or_add(config, add_args.shared, Some(add_args.name)),
		Commands::Quickfix(_) => todos::compile_quickfix(config),
		Commands::Manual(manual_args) => manual_stats::update_or_open(config, manual_args),
		Commands::Timer(timer_args) => timer::timing_the_task(config, timer_args),
		Commands::Monitor(_) => activity_monitor::start(config),
	};

	match success {
		Ok(_) => std::process::exit(0),
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	}
}
