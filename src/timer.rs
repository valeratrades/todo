use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use crate::utils;
use crate::config::Config;
use anyhow::{anyhow, Result};
use clap::Args;
use clap::Subcommand;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::process::Command;

pub fn timing_the_task(config: Config, args: TimerArgs) -> Result<()> {
	let state_path = &config.timer.state_path.0;
	let save_path = &config.timer.save_path.0;

	let success = match args.command {
		TimerCommands::Start(start_args) => {
			if start_args.time > 90 {
				return Err(anyhow!("Provided time is too large. Cut your task into smaller parts. Anything above 90m does not make sense."));
			}

			let timestamp_s = Utc::now().timestamp() as u32;
			let task = Ongoing {
				timestamp_s,
				category: start_args.category.extract_category_name(),
				estimated_minutes: start_args.time,
				description: start_args.description,
			};

			let _ = std::fs::create_dir_all(state_path.parent().unwrap());
			let mut file = File::create(state_path)?;
			let serialized = serde_json::to_string(&task).unwrap();
			file.write_all(serialized.as_bytes()).unwrap();

			run(&config)
		},
		TimerCommands::Open(_) => {
			utils::open(save_path)
		},
		TimerCommands::Done(_) => {
			save_result(&config, true)
		},
		TimerCommands::Failed(_) => {
			save_result(&config, false)
		},
		TimerCommands::ContinueOngoing(_) => {
			run(&config)
		},
	};

	success
}
#[derive(Args)]
pub struct TimerArgs {
	#[command(subcommand)]
	command: TimerCommands,
}

#[derive(Subcommand)]
enum TimerCommands {
	/// Start a timer for a task
	Start(TimerStartArgs),
	Done(TimerDoneArgs),
	Failed(TimerFailedArgs),
	Open(TimerOpenArgs),
	ContinueOngoing(TimerContinueArgs),
}

#[derive(Args)]
struct TimerStartArgs {
	#[arg(short, long, default_value = "90")]
	time: u32,
	#[arg(short, long, default_value = "")]
	description: String,
	#[clap(flatten)]
	category: CategoryFlags,
}

//NB: `t` and `d` are taken
#[derive(Args)]
struct CategoryFlags {
	#[arg(long)]
	home: bool,
	#[arg(long)]
	rust: bool,
	#[arg(long)]
	go: bool,
	#[arg(long)]
	python: bool,
	#[arg(long)]
	workout: bool,
	#[arg(long)]
	library: bool,
	#[arg(long)]
	git_issue: bool,
}

#[derive(Args)]
struct TimerDoneArgs {}
#[derive(Args)]
struct TimerFailedArgs {}
#[derive(Args)]
struct TimerOpenArgs {}
#[derive(Args)]
struct TimerContinueArgs {}

//-----------------------------------------------------------------------------


impl CategoryFlags {
	fn extract_category_name(&self) -> String {
		match self {
			Self { rust: true, .. } => "rust".to_owned(),
			Self { go: true, .. } => "go".to_owned(),
			Self { python: true, .. } => "python".to_owned(),
			Self { home: true, .. } => "home chore".to_owned(),
			Self { workout: true, .. } => "workout".to_owned(),
			Self { library: true, .. } => "library".to_owned(),
			Self { git_issue: true, .. } => "issue git".to_owned(),
			_ => "".to_owned(),
		}
	}
}

//TODO!: impl macro rules to define the struct of Catergoryflags from array of their names, and also impl the extract after.
//macro_rules! create_extract_category {
//	(struct $name:ident { $($field:ident : bool,)* }) => {
//		impl $name {
//		fn extract_category(&self) -> String {
//		match self {
//		$(
//		Self { $field: true, .. } => stringify!($field).replace("_", " ").to_owned(),
//		)*
//		_ => "".to_owned(),
//		}
//		}
//		}
//	};
//}


#[derive(Serialize, Deserialize, Debug)]
struct Ongoing {
	timestamp_s: u32,
	category: String,
	estimated_minutes: u32,
	description: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Record {
	timestamp_s: u32,
	category: String,
	estimated_minutes: u32,
	description: String,
	completed: bool,
	realised_minutes: u32,
}

fn save_result(config: &Config, mut completed: bool) -> Result <()> {
	let state_path = &config.timer.state_path.0;
	let save_path = &config.timer.save_path.0;
	let hard_stop_coeff = config.timer.hard_stop_coeff.clone();


	let mut file = File::open(state_path).unwrap();
	let mut contents = String::new();
	file.read_to_string(&mut contents).unwrap();
	let ongoing: Ongoing = serde_json::from_str(&contents).unwrap();

	let realised_minutes = {
		let diff_m = ((Utc::now().timestamp() as u32 - ongoing.timestamp_s) as f32 / 60.0) as u32;
		let hard_stop_m = (hard_stop_coeff * ongoing.estimated_minutes as f32 + 0.5) as u32;
		if hard_stop_m < diff_m {
			completed = false; // It was possible to do `my_todo done` while executable is inactive, passing completed==true here, while far past the hard stop
			hard_stop_m
		} else {
			diff_m
		}
	};
	let result = Record {
		timestamp_s: ongoing.timestamp_s,
		category: ongoing.category,
		estimated_minutes: ongoing.estimated_minutes,
		description: ongoing.description,
		completed,
		realised_minutes,
	};

	let _ = std::fs::create_dir_all(save_path.parent().unwrap());
	let mut results: VecDeque<Record> = match File::open(save_path) {
		Ok(mut file) => {
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap_or_else(|_| VecDeque::new())
		}
		Err(_) => VecDeque::new(),
	};

	results.push_back(result);

	let mut file = File::create(save_path).unwrap(); //NB: overrides the existing file if any.
	file.write_all(serde_json::to_string(&results).unwrap().as_bytes()).unwrap();
	let _ = std::fs::remove_file(state_path);

	std::thread::sleep(std::time::Duration::from_millis(300)); // wait for eww to process previous request if any.
	if let Ok(eww_output) = Command::new("sh").arg("-c").arg("eww get todo_timer".to_owned()).output() {
		let todo_timer = String::from_utf8_lossy(&eww_output.stdout).trim().to_string();
		if !todo_timer.starts_with("Out") {
			let _ = Command::new("sh").arg("-c").arg("eww update todo_timer=None".to_owned()).output().unwrap();
		}
	}

	Ok(())
}

fn run(config: &Config) -> Result<()> {
	let state_path = &config.timer.state_path.0;
	let hard_stop_coeff = config.timer.hard_stop_coeff.clone();

	let task: Ongoing = {
		if state_path.exists() {
			let mut file = File::open(state_path).unwrap();
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap()
		} else {
			eprintln!("No record of an ongoing task found, exiting.");
			std::process::exit(0);
		}
	};
	let estimated_s = task.timestamp_s + task.estimated_minutes * 60;
	let hard_stop_s = task.timestamp_s + (hard_stop_coeff * (task.estimated_minutes * 60) as f32) as u32;

	loop {
		if !state_path.exists() {
			break;
		}

		let now_s = Utc::now().timestamp();
		let e_diff = estimated_s as i64 - now_s as i64;
		let h_diff = hard_stop_s as i64 - now_s as i64;
		let description = {
			if task.description.as_str() == "" {
				format!("")
			} else {
				format!("_{}", task.description)
			}
		};

		let value = if h_diff < 0 {
			format!("Out{}", description)
		} else if e_diff < 0 {
			format!("-{:02}:{:02}{}", h_diff / 60, h_diff % 60, description)
		} else {
			format!("{:02}:{:02}{}", e_diff / 60, e_diff % 60, description)
		};

		let _ = Command::new("sh")
			.arg("-c")
			//.arg(format!("eww update todo_timer={}", value.replace(" ", "_"))) // eww panics if any spaces are passed here.
			.arg(format!("eww update todo_timer=\"{}\"", value))
			.output()
			.unwrap();
		if value.starts_with("Out") {
			save_result(config, false)?;
			std::process::exit(0);
		}
		std::thread::sleep(std::time::Duration::from_secs(1));
	}

	Ok(())
}


