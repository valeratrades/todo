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
	jo_times: JoTimes,
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
struct JoTimes {
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
	alarm_to_run: Option<usize>,
	run_to_shower: Option<usize>,
	midday_hours_of_work: Option<usize>,
	ev: Option<usize>,
	//streaks: Streaks,
}

#[derive(Debug, Serialize, Deserialize)]
struct Streaks {
	no_jo_full_visuals: Streak,
	no_jo_no_visuals: Streak,
	no_jo_work_for_visuals: Streak,
	stable_sleep: Streak,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Streak {
	pb: usize,
	current: usize,
}

impl Day {
	fn update_pbs<T: AsRef<Path>>(&self, data_storage_dir: T) {
		fn announce_new_pb(new_value: usize, old_value: Option<usize>, name: &str) {
			let old_value = match old_value {
				Some(v) => v.to_string(),
				None => "None".to_owned(),
			};
			let announcement = format!("New pb on {}! ({} -> {})", name, old_value, new_value);
			println!("{}", announcement);
			std::process::Command::new("notify-send").arg(announcement).spawn().unwrap();
		}

		let pbs_path = data_storage_dir.as_ref().join("pbs.json");
		let mut pbs_as_value = match std::fs::read_to_string(&pbs_path) {
			Ok(s) => serde_json::from_str::<serde_json::Value>(&s).unwrap(), // so if we change the struct, we don't rewrite everything
			Err(_) => serde_json::Value::Null,
		};

		let mut conditional_update = |metric: &str, new_value: usize, condition: fn(usize, usize) -> bool| {
			let old_value = pbs_as_value
				.get(metric)
				.and_then(|v| v.as_u64())
				.map(|v| Some(v as usize))
				.unwrap_or(None);

			match old_value {
				Some(v) => {
					if condition(new_value, v) {
						announce_new_pb(new_value, Some(v), metric);
						pbs_as_value[metric] = serde_json::Value::from(new_value);
					} else {
						pbs_as_value[metric] = serde_json::Value::from(v);
					}
				}
				None => {
					announce_new_pb(new_value, None, metric);
					pbs_as_value[metric] = serde_json::Value::from(new_value);
				}
			}
		};

		if self.ev >= 0 {
			conditional_update("ev", self.ev as usize, |new, old| new > old);
		}
		if let Some(new_alarm) = self.morning.alarm_to_run {
			conditional_update("alarm_to_run", new_alarm, |new, old| new < old);
		}
		if let Some(new_run) = self.morning.run_to_shower {
			conditional_update("run_to_shower", new_run, |new, old| new < old);
		}
		if let Some(new_hours_of_work) = self.midday.hours_of_work {
			conditional_update("midday_hours_of_work", new_hours_of_work, |new, old| new > old);
		}

		//let mut jo_full_visuals = false;
		//{
		//	let old = match pbs_as_value.get("streak_no_jo_full_visuals") {
		//		Some(v) => v.as_u64().unwrap() as usize,
		//		None => 0,
		//	};
		//	let new = {
		//		if self.morning.transcendential.jo_times.full_visuals == 0
		//			&& self.midday.transcendential.jo_times.full_visuals == 0
		//			&& self.evening.transcendential.jo_times.full_visuals == 0
		//		{
		//			announce_new_pb(old + 1, old, "streak_no_jo_full_visuals");
		//			old + 1
		//		} else {
		//			jo_full_visuals = true;
		//			0
		//		}
		//	};
		//	pbs_as_value["streak_no_jo_full_visuals"] = serde_json::Value::from(new);
		//}
		//let mut jo_no_visuals = false;
		//{
		//	let old = match pbs_as_value.get("streak_no_jo_no_visuals") {
		//		Some(v) => v.as_u64().unwrap() as usize,
		//		None => 0,
		//	};
		//	let new = {
		//		if self.morning.transcendential.jo_times.no_visuals == 0
		//			&& self.midday.transcendential.jo_times.no_visuals == 0
		//			&& self.evening.transcendential.jo_times.no_visuals == 0
		//			&& !jo_full_visuals
		//		{
		//			announce_new_pb(old + 1, old, "streak_no_jo_no_visuals");
		//			old + 1
		//		} else {
		//			jo_no_visuals = true;
		//			0
		//		}
		//	};
		//	pbs_as_value["streak_no_jo_no_visuals"] = serde_json::Value::from(new);
		//}
		//{
		//	let old = match pbs_as_value.get("streak_no_jo_work_for_visuals") {
		//		Some(v) => v.as_u64().unwrap() as usize,
		//		None => 0,
		//	};
		//	let new = {
		//		if self.morning.transcendential.jo_times.work_for_visuals == 0
		//			&& self.midday.transcendential.jo_times.work_for_visuals == 0
		//			&& self.evening.transcendential.jo_times.work_for_visuals == 0
		//			&& !jo_no_visuals
		//		{
		//			announce_new_pb(old + 1, old, "streak_no_jo_work_for_visuals");
		//			old + 1
		//		} else {
		//			0
		//		}
		//	};
		//	pbs_as_value["streak_no_jo_work_for_visuals"] = serde_json::Value::from(new);
		//}
		//
		//{
		//	let old = match pbs_as_value.get_mut("streak_stable_sleep") {
		//		Some(v) => v.as_u64().unwrap() as usize,
		//		None => 0,
		//	};
		//	let mut invalidated = false;
		//	if let Some(v) = self.sleep.yd_to_bed_t_plus {
		//		if v > 0 {
		//			invalidated = true;
		//		}
		//	} else {
		//		invalidated = true;
		//	}
		//	if let Some(v) = self.sleep.from_bed_t_plus {
		//		if v > 0 {
		//			invalidated = true;
		//		}
		//	} else {
		//		invalidated = true;
		//	}
		//	if let Some(v) = self.sleep.from_bed_abs_diff_from_day_before {
		//		if v > 0 {
		//			invalidated = true;
		//		}
		//	} else {
		//		invalidated = true;
		//	}
		//
		//	let new = if invalidated {
		//		0
		//	} else {
		//		announce_new_pb(old + 1, old, "streak_stable_sleep");
		//		old + 1
		//	};
		//	pbs_as_value["streak_stable_sleep"] = serde_json::Value::from(new);
		//}
		//
		//{
		//	let old = match pbs_as_value.get_mut("meditation") {
		//		Some(v) => v.as_u64().unwrap() as usize,
		//		None => 0,
		//	};
		//	let mut invalidated = true;
		//	if !self.evening.focus_meditation.is_some() || self.evening.focus_meditation.unwrap() != 0 {
		//		invalidated = false;
		//	}
		//
		//	let new = if invalidated {
		//		0
		//	} else {
		//		announce_new_pb(old + 1, old, "meditation");
		//		old + 1
		//	};
		//	pbs_as_value["meditation"] = serde_json::Value::from(new);
		//}
		//
		let pb = serde_json::from_value::<Pbs>(pbs_as_value).unwrap();

		let formatted_json = serde_json::to_string_pretty(&pb).unwrap();
		let mut file = OpenOptions::new().read(true).write(true).create(true).open(&pbs_path).unwrap();
		file.write_all(formatted_json.as_bytes()).unwrap();
	}
}
