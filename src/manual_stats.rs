#![allow(non_snake_case)]
use crate::config::Config;
use crate::utils;
use anyhow::{anyhow, ensure, Result};
use chrono::prelude::*;
use chrono::Duration;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use clap::Args;
use std::path::PathBuf;

use crate::MANUAL_PATH_APPENDIX;
pub fn update_or_open(config: Config, args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.data_dir.clone().join(MANUAL_PATH_APPENDIX);
	let _ = std::fs::create_dir(&data_storage_dir);

	let date: String = (Utc::now() - Duration::days(args.days_back as i64))
		.format(&config.date_format.as_str())
		.to_string();
	let filename = format!("{}.json", date);

	let target_file_path = data_storage_dir.join(&filename);
	let ev_args = match args.command {
		ManualSubcommands::Open { .. } => {
			if !target_file_path.exists() {
				return Err(anyhow!("Tried to open ev file of a day that was not initialized"));
			}
			utils::open(&target_file_path)?;
			return process_manual_updates(&target_file_path);
		}
		ManualSubcommands::Ev(ev) => ev.to_validated()?,
	};

	let file_contents: String = match std::fs::read_to_string(&target_file_path) {
		Ok(s) => s,
		Err(_) => "".to_owned(),
	};
	let day = match serde_json::from_str::<Day>(&file_contents) {
		Ok(d) => {
			let ev = match (ev_args.add, ev_args.subtract) {
				(true, false) => d.ev + ev_args.ev,
				(false, true) => d.ev - ev_args.ev,
				(false, false) => ev_args.ev,
				(true, true) => unreachable!(),
			};
			let mut d: Day = d;
			d.ev = ev;
			d
		}
		Err(_) => {
			ensure!(
				ev_args.replace,
				"The day object is not initialized, so `ev` argument must be provided with `-r --replace` flag"
			);
			Day {
				date: date.clone(),
				ev: ev_args.ev,
				morning: Morning::default(),
				midday: Midday::default(),
				evening: Evening::default(),
				sleep: Sleep::default(),
				non_negotiables_done: None,
			}
		}
	};
	day.update_pbs(&data_storage_dir);

	let formatted_json = serde_json::to_string_pretty(&day).unwrap();
	let mut file = OpenOptions::new().read(true).write(true).create(true).open(&target_file_path).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();

	if ev_args.open == true {
		utils::open(&target_file_path)?;
		process_manual_updates(&target_file_path)?;
	}

	Ok(())
}

fn process_manual_updates<T: AsRef<Path>>(path: T) -> Result<()> {
	if !path.as_ref().exists() {
		return Err(anyhow!("File does not exist, the fuck you just did"));
	}
	let day: Day = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
	day.update_pbs(path.as_ref().parent().unwrap());
	Ok(())
}

#[derive(Args)]
pub struct ManualArgs {
	#[arg(short, long, default_value = "0")]
	pub days_back: usize,
	#[command(subcommand)]
	pub command: ManualSubcommands,
}
#[derive(Subcommand)]
pub enum ManualSubcommands {
	Ev(ManualEv),
	Open {},
}
#[derive(Args)]
pub struct ManualEv {
	pub ev: i32,
	#[arg(short, long)]
	pub open: bool,
	#[arg(short, long)]
	pub add: bool,
	#[arg(short, long)]
	pub subtract: bool,
	#[arg(short, long, default_value = "true")]
	pub replace: bool,
}
impl ManualEv {
	//? This seems ugly. There has to be a way to do this natively with clap, specifically with the `conflicts_with` attribute
	fn to_validated(&self) -> Result<Self> {
		let replace = match self.add || self.subtract {
			true => false,
			false => self.replace,
		};
		if self.add && self.subtract {
			return Err(anyhow!("Exactly one of 'add', 'subtract', or 'replace' must be specified."));
		}
		if !self.add && !self.subtract && !self.replace {
			return Err(anyhow!("Exactly one of 'add', 'subtract', or 'replace' must be specified."));
		}
		Ok(Self {
			ev: self.ev,
			open: self.open,
			add: self.add,
			subtract: self.subtract,
			replace,
		})
	}
}

//=============================================================================

// So I'm assuming the PbTracker is actually a mirror of the Day struct, with fields set to their best ever values. Although: 1) What about the changes to the structs 2) Streaks, where the members could be multiple?

// Basically only serialization to pb format is needed. Let's also flatten it, and require manual specification of the recorded name.

#[derive(Debug, Serialize, Deserialize, Default)]
struct Transcendential {
	making_food: Option<usize>,
	eating_food: Option<usize>,
	j_o_times: JOtimes,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Sleep {
	yd_to_bed_t_plus: Option<i32>,
	from_bed_t_plus: Option<i32>,
	from_bed_abs_diff_from_day_before: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Morning {
	alarm_to_run: Option<usize>,
	run: Option<usize>,
	run_to_shower: Option<usize>,
	#[serde(flatten)]
	transcendential: Transcendential,
}

#[derive(Debug, Serialize, Deserialize, Default)]
// could be called `_8h`
struct Midday {
	hours_of_work: Option<usize>,
	#[serde(flatten)]
	transcendential: Transcendential,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Evening {
	focus_meditation: Option<usize>, // fixed at 13m under current sota, but why not keep it flexible
	nsdr: Option<usize>,
	#[serde(flatten)]
	transcendential: Transcendential,
}

#[derive(Debug, Serialize, Deserialize, Default)]
// removed the Option for ease of input, let's see how capable I am of always filling these in. Otherwise I'll have to add them back.
struct JOtimes {
	full_visuals: usize,
	no_visuals: usize,
	work_for_visuals: usize,
}

#[derive(Debug, Serialize, Deserialize)]
/// Unless specified otherwise, all times are in minutes
struct Day {
	date: String,
	ev: i32,
	morning: Morning,
	midday: Midday,
	evening: Evening,
	sleep: Sleep,
	non_negotiables_done: Option<usize>, // currently having 2 non-negotiables set for each day; but don't want to fix the value to that range, in case it changes.
}

#[derive(Debug, Serialize, Deserialize)]
struct Pbs {
	alarm_to_run: usize,
	run_to_shower: usize,
	midday_hours_of_work: usize,
	ev: usize,
}

impl Day {
	fn update_pbs<T: AsRef<Path>>(&self, data_storage_dir: T) {
		fn announce_new_pb(new_value: usize, old_value: usize, name: &str) {
			let announcement = format!("New pb on {}! ({} -> {})", name, old_value, new_value);
			println!("{}", announcement);
			std::process::Command::new("notify-send").arg(announcement).spawn().unwrap();
		}

		let pb_path = data_storage_dir.as_ref().join("pbs.json");
		let mut pb_as_value = match std::fs::read_to_string(&pb_path) {
			Ok(s) => serde_json::from_str::<serde_json::Value>(&s).unwrap(), // so if we change the struct, we don't rewrite everything
			Err(_) => serde_json::Value::Null,
		};

		if self.ev >= 0 {
			let new = self.ev;
			let old = match pb_as_value.get_mut("ev") {
				Some(v) => v.as_u64().unwrap() as usize,
				None => std::u64::MAX as usize,
			};
			if new > old as i32 {
				announce_new_pb(new as usize, old, "ev");
				pb_as_value["ev"] = serde_json::Value::from(new);
			}
		}
		if let Some(new) = self.morning.alarm_to_run {
			let old = match pb_as_value.get_mut("alarm_to_run") {
				Some(v) => v.as_u64().unwrap() as usize,
				None => std::u64::MAX as usize,
			};
			if new < old {
				announce_new_pb(new, old, "alarm_to_run");
				pb_as_value["alarm_to_run"] = serde_json::Value::from(new);
			}
		}
		if let Some(new) = self.morning.run_to_shower {
			let old = match pb_as_value.get_mut("run_to_shower") {
				Some(v) => v.as_u64().unwrap() as usize,
				None => std::u64::MAX as usize,
			};
			if new < old {
				announce_new_pb(new, old, "run_to_shower");
				pb_as_value["run_to_shower"] = serde_json::Value::from(new);
			}
		}
		if let Some(new) = self.midday.hours_of_work {
			let old = match pb_as_value.get_mut("midday_hours_of_work") {
				Some(v) => v.as_u64().unwrap() as usize,
				None => std::u64::MAX as usize,
			};
			if new > old {
				announce_new_pb(new, old, "midday_hours_of_work");
				pb_as_value["midday_hours_of_work"] = serde_json::Value::from(new);
			}
		}
		let pb = serde_json::from_value::<Pbs>(pb_as_value).unwrap();
		//TODO!: streaks

		assert!(pb_path.exists());
		let formatted_json = serde_json::to_string_pretty(&pb).unwrap();
		let mut file = OpenOptions::new().read(true).write(true).create(true).open(&pb_path).unwrap();
		file.write_all(formatted_json.as_bytes()).unwrap();
	}
}
