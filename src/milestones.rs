use chrono::{DateTime, Utc};
use clap::Args;
use color_eyre::eyre::Result;
use reqwest::blocking::Client;
use serde::Deserialize;
use v_utils::trades::{Timeframe, TimeframeDesignator};

use crate::config::AppConfig;

static MIN_DISTANCE_HOURS: usize = 8;

#[derive(Args)]
pub struct MilestonesArgs {
	pub tf: Timeframe,
}

#[derive(Deserialize, Debug)]
struct Milestone {
	title: String,
	#[allow(dead_code)]
	state: String,
	due_on: Option<DateTime<Utc>>,
	description: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("Milestone is missing due_on date")]
struct MissingDueOn {}

#[derive(Debug, thiserror::Error)]
#[error(
	"Milestone is outdated (due_on: {due_on})\ntry moving it to a later date. Must be at least {} hours away from `Utc::now()`",
	MIN_DISTANCE_HOURS
)]
struct MilestoneOutdated {
	due_on: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
#[error("Requested milestone on minute-designated timeframe (`m`). You likely meant to request Monthly (`M`).")]
struct MinuteMilestone {
	requested_tf: Timeframe,
}

pub fn get_milestone(config: AppConfig, args: MilestonesArgs) -> Result<()> {
	if args.tf.designator == TimeframeDesignator::Minutes {
		return Err(MinuteMilestone { requested_tf: args.tf }.into());
	}

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
	match milestones.iter().find(|m| m.title == args.tf.to_string()) {
		Some(milestone) => {
			let due_on = milestone.due_on.as_ref().ok_or(MissingDueOn {})?;

			let diff = due_on.signed_duration_since(Utc::now());
			if diff.num_hours() < MIN_DISTANCE_HOURS as i64 {
				return Err(MilestoneOutdated { due_on: *due_on }.into());
			}

			if let Some(description) = &milestone.description {
				println!("{}", description);
			}
		}
		None => {
			let milestone_titles = milestones.iter().map(|m| m.title.clone()).collect::<Vec<String>>();
			println!("Milestone not found, here are all the existing milestones:\n{}", milestone_titles.join("\n"));
		}
	}

	Ok(())
}
