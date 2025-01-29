use clap::{Args, Subcommand};
use reqwest::blocking::Client;
use v_utils::prelude::*;

use crate::config::{AppConfig, DATA_DIR};

pub static HEALTHCHECK_REL_PATH: &str = "healthcheck.status";
pub static SPRINT_HEADER_REL_PATH: &str = "sprint_header.md";

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
	/// Ensures all milestones up to date, if yes - writes "OK" to $XDG_DATA_HOME/todo/healthcheck.status
	/// Can get outdated easily, so printed output of the command is prepended with the filename
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
	info!(?res);

	let milestones = res.json::<Vec<Milestone>>()?;
	Ok(milestones)
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
#[error("Error on `{requested_tf}` milestone: {source}")]
struct GetMilestoneError {
	requested_tf: Timeframe,
	#[source]
	source: MilestoneError,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
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
	if tf.designator() == TimeframeDesignator::Minutes {
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
	Timeframe::from_naive(1, TimeframeDesignator::Days),
	Timeframe::from_naive(2, TimeframeDesignator::Weeks),
	Timeframe::from_naive(1, TimeframeDesignator::Quarters),
	Timeframe::from_naive(1, TimeframeDesignator::Years),
	Timeframe::from_naive(3, TimeframeDesignator::Years),
	Timeframe::from_naive(7, TimeframeDesignator::Years),
];

fn healthcheck(config: &AppConfig) -> Result<()> {
	use std::fs;

	let share_dir = share_dir!();

	let healthcheck_path = share_dir.join(HEALTHCHECK_REL_PATH);

	let retrieved_milestones = request_milestones(config)?;
	let results = KEY_MILESTONES
		.iter()
		.map(|tf| get_milestone(*tf, &retrieved_milestones))
		.collect::<Vec<Result<String, GetMilestoneError>>>();
{

	let mut health = String::new();
	for result in &results {
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
	println!("{}\n{health}", healthcheck_path.display());

	//TODO: switch to v_utils::share_dir
	fs::write(healthcheck_path, health).unwrap();
	}

	{
		let sprint_ms = &<std::result::Result<std::string::String, GetMilestoneError> as Clone>::clone(&results[1]).map_err(|e| eyre!("Couldn't parse 2w milestone which MUST be defined: {e}"))?;
		let sprint_header = sprint_ms.lines().next().ok_or_else(|| eyre!("2w milestone does not have a description. MUST have a description."))?;
		if !sprint_header.starts_with("# ") {
			eprintln!("2w milestone description does not start with a header. It SHOULD start with '# '.");
		}
		fs::write(share_dir.join(SPRINT_HEADER_REL_PATH), sprint_header).unwrap();
	}




	Ok(())
}
