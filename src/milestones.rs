use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use color_eyre::eyre::Result;
use reqwest::blocking::Client;
use serde::Deserialize;
use v_utils::{
	io::ExpandedPath,
	trades::{Timeframe, TimeframeDesignator},
};

use crate::config::AppConfig;

lazy_static::lazy_static! {
	static ref HEALTHCHECK_PATH: ExpandedPath = ExpandedPath::from_str("~/.local/run/todo/milestones_healthcheck.status").unwrap();
}

#[derive(Args)]
pub struct MilestonesArgs {
	#[command(subcommand)]
	command: MilestonesCommands,
}

#[derive(Subcommand)]
pub enum MilestonesCommands {
	Get {
		tf: Timeframe,
	},
	/// Ensures all milestones up to date, writes "OK" to ~/.local/run/todo/milestones_healthcheck.status if so.
	Healthcheck,
}

#[derive(Deserialize, Debug)]
struct Milestone {
	title: String,
	#[allow(dead_code)]
	state: String,
	due_on: Option<DateTime<Utc>>,
	description: Option<String>,
}

pub fn milestones_command(config: AppConfig, args: MilestonesArgs) -> Result<()> {
	match args.command {
		MilestonesCommands::Get { tf } => {
			let retrieved_milestones = request_milestones(&config)?;
			let milestone = get_milestone(tf, &retrieved_milestones)?;
			println!("{milestone}");
			Ok(())
		}
		MilestonesCommands::Healthcheck => healthcheck(&config),
	}
}

fn request_milestones(config: &AppConfig) -> Result<Vec<Milestone>> {
	let todos_url_output = std::process::Command::new("git")
		.args(["config", "--get", "remote.origin.url"])
		.current_dir(&config.todos.path)
		.output()?
		.stdout;
	let todos_url = String::from_utf8(todos_url_output).unwrap().trim().to_string();
	let sections = todos_url.split("/").collect::<Vec<&str>>();
	let (owner, repo) = (sections[sections.len() - 2], sections[sections.len() - 1]);

	let url = format!("https://api.github.com/repos/{}/{}/milestones", owner, repo);

	let client = Client::new();
	let res = client
		.get(&url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", config.github_token))
		.send()?;

	let milestones = res.json::<Vec<Milestone>>()?;
	Ok(milestones)
}

#[derive(Debug, thiserror::Error)]
#[error("Error on `{requested_tf}` milestone: {source}")]
struct GetMilestoneError {
	requested_tf: Timeframe,
	#[source]
	source: MilestoneError,
}

#[derive(Debug, thiserror::Error)]
enum MilestoneError {
	#[error("Milestone is missing due_on date")]
	MissingDueOn,

	#[error("Milestone is outdated (due_on: {due_on}). Try moving it to a later date.")]
	MilestoneOutdated { due_on: DateTime<Utc> },

	#[error("Requested milestone on minute-designated timeframe (`m`). You likely meant to request Monthly (`M`).")]
	MinuteMilestone,

	#[error("Milestone not found. Here are all the existing milestones:\n{existing_milestones:?}")]
	MilestoneNotFound { existing_milestones: Vec<String> },

	#[error("Missing description")]
	MissingDescription,
}

fn get_milestone(tf: Timeframe, retrieved_milestones: &[Milestone]) -> Result<String, GetMilestoneError> {
	if tf.designator == TimeframeDesignator::Minutes {
		return Err(GetMilestoneError {
			requested_tf: tf,
			source: MilestoneError::MinuteMilestone,
		});
	}

	match retrieved_milestones.iter().find(|m| m.title == tf.to_string()) {
		Some(milestone) => {
			let due_on = milestone.due_on.as_ref().ok_or(GetMilestoneError {
				requested_tf: tf,
				source: MilestoneError::MissingDueOn,
			})?;

			let diff = due_on.signed_duration_since(Utc::now());
			if diff.num_hours() < 0 {
				return Err(GetMilestoneError {
					requested_tf: tf,
					source: MilestoneError::MilestoneOutdated { due_on: *due_on },
				});
			}

			match milestone.description.clone() {
				Some(description) => Ok(description),
				None => Err(GetMilestoneError {
					requested_tf: tf,
					source: MilestoneError::MissingDescription,
				}),
			}
		}
		None => {
			let milestone_titles = retrieved_milestones.iter().map(|m| m.title.clone()).collect::<Vec<String>>();
			Err(GetMilestoneError {
				requested_tf: tf,
				source: MilestoneError::MilestoneNotFound {
					existing_milestones: milestone_titles,
				},
			})
		}
	}
}

static KEY_MILESTONES: [Timeframe; 6] = [
	Timeframe {
		designator: TimeframeDesignator::Days,
		n: 1,
	},
	Timeframe {
		designator: TimeframeDesignator::Weeks,
		n: 1,
	},
	Timeframe {
		designator: TimeframeDesignator::Months,
		n: 1,
	},
	Timeframe {
		designator: TimeframeDesignator::Quarters,
		n: 1,
	},
	Timeframe {
		designator: TimeframeDesignator::Years,
		n: 1,
	},
	Timeframe {
		designator: TimeframeDesignator::Years,
		n: 5,
	},
];

fn healthcheck(config: &AppConfig) -> Result<()> {
	let retrieved_milestones = request_milestones(config)?;
	let results = KEY_MILESTONES
		.iter()
		.map(|tf| get_milestone(*tf, &retrieved_milestones))
		.collect::<Vec<Result<String, GetMilestoneError>>>();

	let mut health = String::new();
	for result in results {
		match result {
			Ok(_) => {}
			Err(e) => {
				if !health.is_empty() {
					health.push('\n');
				}
				health.push_str(&e.to_string());
			}
		}
	}

	if health.is_empty() {
		health = "OK".to_string();
	}
	println!("{health}");

	std::fs::create_dir_all(HEALTHCHECK_PATH.0.parent().unwrap()).unwrap();
	std::fs::write(&*HEALTHCHECK_PATH, health).unwrap();
	Ok(())
}
