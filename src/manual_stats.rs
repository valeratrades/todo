#![allow(non_snake_case)]
use crate::config::AppConfig;
use crate::utils;
use anyhow::{anyhow, ensure, Result};
use clap::Args;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use v_utils::io::OpenMode;

static PBS_FILENAME: &'static str = ".pbs.json";

use crate::MANUAL_PATH_APPENDIX;
pub fn update_or_open(config: AppConfig, args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.data_dir.clone().join(MANUAL_PATH_APPENDIX);
	let _ = std::fs::create_dir(&data_storage_dir);

	let date = utils::format_date(args.days_back, &config);
	let filename = format!("{}.json", date);

	let target_file_path = data_storage_dir.join(&filename);
	let ev_args = match args.command {
		ManualSubcommands::Open(open_args) => match open_args.pbs {
			false => {
				if !target_file_path.exists() {
					return Err(anyhow!("Tried to open ev file of a day that was not initialized"));
				}
				v_utils::io::open(&target_file_path)?;
				return process_manual_updates(&target_file_path, &config);
			}
			true => {
				let pbs_path = data_storage_dir.join(PBS_FILENAME);
				return v_utils::io::open_with_mode(&pbs_path, OpenMode::Readonly);
			}
		},
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
			let mut d = Day::default();
			d.ev = ev_args.ev;
			d.date = date.clone();
			d
		}
	};
	day.update_pbs(&data_storage_dir, &config);

	let formatted_json = serde_json::to_string_pretty(&day).unwrap();
	let mut file = OpenOptions::new().read(true).write(true).create(true).open(&target_file_path).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();

	if ev_args.open == true {
		v_utils::io::open(&target_file_path)?;
		process_manual_updates(&target_file_path, &config)?;
	}

	Ok(())
}

fn process_manual_updates<T: AsRef<Path>>(path: T, config: &AppConfig) -> Result<()> {
	if !path.as_ref().exists() {
		return Err(anyhow!("File does not exist, the fuck you just did"));
	}
	let day: Day = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
	day.update_pbs(path.as_ref().parent().unwrap(), config);
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
	Open(ManualOpen),
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

#[derive(Args)]
pub struct ManualOpen {
	#[arg(short, long)]
	pub pbs: bool,
}

//=============================================================================

// So I'm assuming the PbTracker is actually a mirror of the Day struct, with fields set to their best ever values. Although: 1) What about the changes to the structs 2) Streaks, where the members could be multiple?

// Basically only serialization to pb format is needed. Let's also flatten it, and require manual specification of the recorded name.

#[derive(Debug, Serialize, Deserialize, Default)]
struct Transcendential {
	making_food: Option<usize>,
	eating_food: Option<usize>,
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
	shower_to_breakfast_work_efficiency_percent_of_optimal: Option<usize>,
	#[serde(flatten)]
	transcendential: Transcendential,
	breakfast_to_work: Option<usize>,
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
	focus_meditation: usize, // fixed at 13m under current sota, but why not keep it flexible
	nsdr: usize,
	#[serde(flatten)]
	transcendential: Transcendential,
}

#[derive(Debug, Serialize, Deserialize, Default)]
// removed the Option for ease of input, let's see how capable I am of always filling these in. Otherwise I'll have to add them back.
struct JoMins {
	full_visuals: usize,
	no_visuals: usize,
	work_for_visuals: usize,
}

#[derive(Debug, Serialize, Deserialize, Default)]
/// Unless specified otherwise, all times are in minutes
struct Day {
	date: String,
	ev: i32,
	morning: Morning,
	midday: Midday,
	evening: Evening,
	sleep: Sleep,
	jo_mins: JoMins,
	non_negotiables_done: usize, // currently having 2 non-negotiables set for each day; but don't want to fix the value to that range, in case it changes.
	number_of_NOs: usize,
	caffeine_only_during_work: Option<bool>,
	checked_messages_only_during_eating: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Streak {
	pb: usize,
	current: usize,
}

impl Day {
	fn update_pbs<T: AsRef<Path>>(&self, data_storage_dir: T, config: &AppConfig) {
		//TODO!!: fix error with adding extra brackets to ~/.data/personal/manual_stats/.pbs.json
		fn announce_new_pb(new_value: usize, old_value: Option<usize>, name: &str) {
			let old_value = match old_value {
				Some(v) => v.to_string(),
				None => "None".to_owned(),
			};
			let announcement = format!("New pb on {}! ({} -> {})", name, old_value, new_value);
			println!("{}", announcement);
			std::process::Command::new("notify-send").arg(announcement).spawn().unwrap();
		}

		let pbs_path = data_storage_dir.as_ref().join(PBS_FILENAME);
		let yd_date = utils::format_date(1, config); // no matter what file is being checked, we only ever care about physical yesterday
		let mut pbs_as_value = match std::fs::read_to_string(&pbs_path) {
			Ok(s) => serde_json::from_str::<serde_json::Value>(&s).unwrap(), // Value so we don't need to rewrite everything on `Day` struct changes. Both in terms of extra code, and recorded pb values. Previously had a Pbs struct, but that has proven to be unnecessary.
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

		// Returns bool for convienience of recursing some of these
		let mut streak_update = |metric: &str, condition: &dyn Fn(&Day) -> bool| -> bool {
			let load_streaks_from = data_storage_dir.as_ref().join(&format!("{}.json", yd_date));
			let yd_streaks_source = match std::fs::read_to_string(&load_streaks_from) {
				Ok(s) => Some(serde_json::from_str::<Day>(&s).unwrap()),
				Err(_) => None,
			};

			let pb_streaks = pbs_as_value.get("streaks").unwrap_or(&serde_json::Value::Null);
			let read_streak: Streak = pb_streaks
				.get(metric)
				.map_or_else(Streak::default, |v| serde_json::from_value::<Streak>(v.clone()).unwrap_or_default());

			let is_validated: bool = yd_streaks_source.is_some() && condition(&yd_streaks_source.unwrap());
			let skip = match pb_streaks.get("__last_date_processed") {
				Some(v) => v.as_str().expect("The only way this panics is if user manually changes pbs file") == yd_date,
				None => false,
			};
			if !skip {
				let mut new_streak = if is_validated {
					Streak {
						pb: read_streak.pb,
						current: read_streak.current + 1,
					}
				} else {
					Streak {
						pb: read_streak.pb,
						current: 0,
					}
				};
				if new_streak.current > read_streak.pb {
					announce_new_pb(new_streak.current, Some(read_streak.current), metric);
					new_streak.pb = new_streak.current;
				}
				pbs_as_value["streaks"][metric] = serde_json::to_value(new_streak).unwrap();
			} else {
				pbs_as_value["streaks"][metric] = serde_json::to_value(read_streak).unwrap();
			}

			is_validated
		};

		let full_visuals_condition = |d: &Day| d.jo_mins.full_visuals == 0;
		let no_jo_full_visuals = streak_update("no_jo_full_visuals", &full_visuals_condition);

		let no_visuals_condition = |d: &Day| d.jo_mins.no_visuals == 0 && no_jo_full_visuals;
		let no_jo_no_visuals = streak_update("no_jo_no_visuals", &no_visuals_condition);

		let work_for_visuals_condition = |d: &Day| d.jo_mins.work_for_visuals == 0 && no_jo_no_visuals;
		let _ = streak_update("no_jo_work_for_visuals", &work_for_visuals_condition);

		let stable_sleep_condition = |d: &Day| {
			d.sleep.yd_to_bed_t_plus == Some(0) && d.sleep.from_bed_t_plus == Some(0) && d.sleep.from_bed_abs_diff_from_day_before == Some(0)
		};
		let _ = streak_update("stable_sleep", &stable_sleep_condition);

		let meditation_condition = |d: &Day| d.evening.focus_meditation > 0;
		let _ = streak_update("focus_meditation", &meditation_condition);

		let nsdr_condition = |d: &Day| d.evening.nsdr > 0;
		let _ = streak_update("nsdr", &nsdr_condition);

		let perfect_morning_condition = |d: &Day| {
			d.morning.alarm_to_run.is_some_and(|v| v < 10)
				&& d.morning.run_to_shower.is_some_and(|v| v <= 5)
				&& d.morning.shower_to_breakfast_work_efficiency_percent_of_optimal.is_some_and(|v| v > 90)
				&& d.morning.transcendential.eating_food.is_some_and(|v| v < 20)
				&& d.morning.breakfast_to_work.is_some_and(|v| v <= 5)
		};
		let _ = streak_update("perfect_morning", &perfect_morning_condition);

		let no_streak_condition = |d: &Day| d.number_of_NOs > 0;
		let _ = streak_update("NOs_streak", &no_streak_condition);

		let responsible_caffeine_condition = |d: &Day| d.caffeine_only_during_work == Some(true);
		let _ = streak_update("responsible_caffeine", &responsible_caffeine_condition);

		let responsible_messengers_condition = |d: &Day| d.checked_messages_only_during_eating == Some(true);
		let _ = streak_update("responsible_messengers", &responsible_messengers_condition);

		let running_streak_condition = |d: &Day| d.morning.run.is_some_and(|v| v > 0);
		let _ = streak_update("running_streak", &running_streak_condition);

		pbs_as_value["streaks"]["__last_date_processed"] = serde_json::Value::from(yd_date);

		let formatted_json = serde_json::to_string_pretty(&pbs_as_value).unwrap();
		let mut file = OpenOptions::new().read(true).write(true).create(true).open(&pbs_path).unwrap();
		file.write_all(formatted_json.as_bytes()).unwrap();
	}
}
