#![allow(clippy::len_zero)]
mod blocker;
mod blocker_rewrite;
mod clockify;
pub mod config;
mod github;
mod manual_stats;
mod milestones;
mod mock_github;
pub mod mocks;
mod open;
mod perf_eval;
mod shell_init;
pub mod utils;
mod watch_monitors;
use std::time::Duration;

use clap::{Parser, Subcommand};

const MANUAL_PATH_APPENDIX: &str = "manual_stats/";

#[derive(Parser)]
#[command(author, version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"), about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[clap(flatten)]
	settings_flags: config::SettingsFlags,
	/// Use mock GitHub client instead of real API (for testing)
	#[arg(long, global = true)]
	dbg: bool,
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
	/// Blocker management linked to issue files
	BlockerRewrite(blocker_rewrite::BlockerRewriteArgs),
	/// Clockify time tracking
	Clockify(clockify::ClockifyArgs),
	/// Performance evaluation with screenshots
	PerfEval(perf_eval::PerfEvalArgs),
	/// Watch monitors daemon - takes screenshots every 60s
	WatchMonitors(watch_monitors::WatchMonitorsArgs),
	/// Open a GitHub issue in $EDITOR
	Open(open::OpenArgs),
}

#[tokio::main]
async fn main() {
	#[cfg(not(feature = "is_integration_test"))]
	v_utils::clientside!();

	// Initialize tracing/logging (ignore if already initialized)
	let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).try_init();

	let cli = Cli::parse();

	let settings = match config::LiveSettings::new(cli.settings_flags.clone(), Duration::from_secs(3)) {
		Ok(s) => s,
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	};

	// Create the GitHub client based on --dbg flag
	let github_client: github::BoxedGitHubClient = if cli.dbg {
		std::sync::Arc::new(mock_github::MockGitHubClient::new("mock_user"))
	} else {
		match github::RealGitHubClient::new(&settings) {
			Ok(client) => std::sync::Arc::new(client),
			Err(e) => {
				// Only error if we're using a command that needs GitHub
				// For now, create a dummy that will error on use
				if matches!(cli.command, Commands::Open(_)) {
					eprintln!("Error: {}", e);
					std::process::exit(1);
				}
				// For other commands, create a mock (they won't use it)
				std::sync::Arc::new(mock_github::MockGitHubClient::new("unused"))
			}
		}
	};

	// All the functions here can rely on config being correct.
	let success = match cli.command {
		Commands::Manual(manual_args) => manual_stats::update_or_open(&settings, manual_args),
		Commands::Milestones(milestones_command) => milestones::milestones_command(&settings, milestones_command).await,
		Commands::Init(args) => {
			shell_init::output(&settings, args);
			Ok(())
		}
		Commands::Blocker(args) => blocker::main(&settings, args).await,
		Commands::BlockerRewrite(args) => blocker_rewrite::main(&settings, args).await,
		Commands::Clockify(args) => clockify::main(&settings, args).await,
		Commands::PerfEval(args) => perf_eval::main(&settings, args).await,
		Commands::WatchMonitors(args) => watch_monitors::main(&settings, args),
		Commands::Open(args) => open::open_command(&settings, github_client, args).await,
	};

	match success {
		Ok(_) => std::process::exit(0),
		Err(e) => {
			eprintln!("Error: {}", e);
			std::process::exit(1);
		}
	}
}
