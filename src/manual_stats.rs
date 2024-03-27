#![allow(non_snake_case)]
use crate::config::Config;
use crate::utils;
use anyhow::Result;
use chrono::prelude::*;
use chrono::Duration;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;

use clap::Args;
use std::path::PathBuf;

use crate::MANUAL_PATH_APPENDIX;

pub fn update_or_open(config: Config, mut args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.data_dir.clone().join(MANUAL_PATH_APPENDIX);
	let _ = std::fs::create_dir(&data_storage_dir);

	let date: String = match args.yesterday {
		true => (Utc::now() - Duration::days(1)).format(&config.date_format.as_str()).to_string(),
		false => Utc::now().format(&config.date_format.as_str()).to_string(),
	};

	let target_file_path = data_storage_dir.join(&date);
	if args.ev == None && args.open == false {
		args.open = true;
	}

	let file_contents: String = match std::fs::read_to_string(&target_file_path) {
		Ok(s) => s,
		Err(_) => "".to_owned(),
	};
	let mut day: Day = match serde_json::from_str(&file_contents) {
		Ok(v) => v,
		Err(_) => {
			if let Some(ev) = args.ev {
				Day {
					date: date.clone(),
					ev,
					morning: Morning::default(),
					midday: Midday::default(),
					evening: Evening::default(),
					sleep: Sleep::default(),
				}
			} else {
				return Err(anyhow::anyhow!("The day object is not initialized, so `ev` argument is required"));
			}
		}
	};

	if let Some(ev) = args.ev {
		day.ev = ev;
	}

	let formatted_json = serde_json::to_string_pretty(&day).unwrap();
	let mut file = OpenOptions::new().read(true).write(true).create(true).open(&target_file_path).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();

	if args.open == true {
		utils::open(&target_file_path)?;
	}

	Ok(())
}

#[derive(Args)]
pub struct ManualArgs {
	#[arg(long)]
	pub ev: Option<i32>,
	#[arg(short, long)]
	pub open: bool,
	#[arg(short, long)]
	pub yesterday: bool, //TODO!!: change to -d, --days-ago <days> and make it accept a number of days to go back instead of just yesterday
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
}
