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

use clap::Args;
use std::path::PathBuf;

use crate::MANUAL_PATH_APPENDIX;

pub fn update_or_open(config: Config, args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.data_dir.clone().join(MANUAL_PATH_APPENDIX);
	let _ = std::fs::create_dir(&data_storage_dir);

	let date: String = (Utc::now() - Duration::days(args.days_back as i64))
		.format(&config.date_format.as_str())
		.to_string();

	let target_file_path = data_storage_dir.join(&date);
	let ev_args = match args.command {
		ManualSubcommands::Open { .. } => {
			return utils::open(&target_file_path);
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
			}
		}
	};

	let formatted_json = serde_json::to_string_pretty(&day).unwrap();
	let mut file = OpenOptions::new().read(true).write(true).create(true).open(&target_file_path).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();

	if ev_args.open == true {
		utils::open(&target_file_path)?;
	}

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

// This macro will be used if I decide I want to do skiping of `None` values in the serialization again
//macro_rules! create_stats_class {
//	($name:ident { $($field:ident),* $(,)? }) => {
//		#[derive(Debug, Serialize, Deserialize, Default)]
//		struct $name {
//		$(
//			//#[serde(skip_serializing_if = "Option::is_none")] // this would just skip `None` values, instead of submitting them to serialization, to be `null`
//			$field: Option<i32>,
//		)*
//		}
//	};
//}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Sleep {
	yd_to_bed_t_plus: Option<i32>,
	from_bed_t_plus: Option<i32>,
	from_bed_diff_from_day_before: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Morning {
	alarm_to_run: Option<usize>,
	run: Option<usize>,
	run_to_shower: Option<usize>,
	making_breakfast: Option<usize>,
	eating_breakfast: Option<usize>,
	j_o_times: JOtimes,
}

#[derive(Debug, Serialize, Deserialize, Default)]
// could be called `_8h`
struct Midday {
	hours_of_work: Option<usize>,
	making_lunch: Option<usize>,
	eating_lunch: Option<usize>,
	j_o_times: JOtimes,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Evening {
	focus_meditation: Option<usize>, // fixed at 13m under current sota, but why not keep it flexible
	nsdr: Option<usize>,
	making_dinner: Option<usize>,
	eating_dinner: Option<usize>,
	j_o_times: JOtimes,
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
