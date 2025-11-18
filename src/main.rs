#![allow(clippy::len_zero)]
mod blocker;
mod clockify;
pub mod config;
mod manual_stats;
mod milestones;
pub mod mocks;
mod perf_eval;
mod shell_init;
pub mod utils;
use clap::{Parser, Subcommand};
use config::AppConfig;
#[cfg(not(feature = "is_integration_test"))]
use v_utils::clientside;

const MANUAL_PATH_APPENDIX: &str = "manual_stats/";

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
	/// Record day's ev and other stats.
	///Following records ev of 420 for yesterday, then opens the file.
	///```rust
	///todo manual -d1 --ev 420 -o
	///```
	Manual(manual_stats::ManualArgs),
	/// Operations with milestones (1d, 1w, 1M, 1Q, 1y)
	Milestones(milestones::MilestonesArgs),
	/// Shell aliases and hooks. Usage: `todos init <shell> | source`
	Init(shell_init::ShellInitArgs),
	/// Blockers tree
	Blocker(blocker::BlockerArgs),
	/// Clockify time tracking
	Clockify(clockify::ClockifyArgs),
	/// Performance evaluation with screenshots
	PerfEval(perf_eval::PerfEvalArgs),
}

fn main() {
	#[cfg(not(feature = "is_integration_test"))]
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
		Commands::Manual(manual_args) => manual_stats::update_or_open(config, manual_args),
		Commands::Milestones(milestones_command) => milestones::milestones_command(config, milestones_command),
		Commands::Init(args) => {
			shell_init::output(config, args);
			Ok(())
		}
		Commands::Blocker(args) => blocker::main(config, args),
		Commands::Clockify(args) => clockify::main(config, args),
		Commands::PerfEval(args) => perf_eval::main(config, args),
	};

	match success {
		Ok(_) => std::process::exit(0),
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	}
}
