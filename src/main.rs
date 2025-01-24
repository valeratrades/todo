#![allow(clippy::len_zero)]
mod activity_monitor;
mod blocker;
pub mod config;
pub mod day_section;
mod manual_stats;
mod milestones;
pub mod mocks;
mod shell_init;
mod todos;
pub mod utils;
use clap::{Args, Parser, Subcommand};
use config::AppConfig;
use v_utils::clientside;

const MANUAL_PATH_APPENDIX: &str = "manual_stats/";
const MONITOR_PATH_APPENDIX: &str = "activities_monitor/";
const TOTALS_PATH_APPENDIX: &str = "activities_totals/";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[arg(long)]
	config: Option<v_utils::io::ExpandedPath>,
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
	/// Operations with milestones (1d, 1w, 1M, 1Q, 1y)
	Milestones(milestones::MilestonesArgs),
	/// Start monitoring user activities
	Monitor,
	/// Shell aliases and hooks. Usage: `todos init <shell> | source`
	Init(shell_init::ShellInitArgs),
	/// Blockers tree
	Blocker(blocker::BlockerArgs),
}
#[derive(Args)]
struct NoArgs {}

fn main() {
	clientside!();
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
		Commands::Monitor => activity_monitor::start(config),
		Commands::Milestones(milestones_command) => milestones::milestones_command(config, milestones_command),
		Commands::Init(args) => {
			shell_init::output(config, args);
			Ok(())
		}
		Commands::Blocker(args) => blocker::main(config, args),
	};

	match success {
		Ok(_) => std::process::exit(0),
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	}
}
