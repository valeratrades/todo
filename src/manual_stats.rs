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

pub fn update_or_open(config: Config, args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.data_dir.clone().join(MANUAL_PATH_APPENDIX);
	let _ = std::fs::create_dir(&data_storage_dir);

	if args.ev == None && args.open == false {
		return Err(anyhow::anyhow!("provide `ev` and/or `open` arguments"));
	}

	let date: String = match args.yesterday {
		true => (Utc::now() - Duration::days(1)).format(&config.date_format.as_str()).to_string(),
		false => Utc::now().format(&config.date_format.as_str()).to_string(),
	};
	let target_file_path = data_storage_dir.join(&date);

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
					stats: Stats::default(),
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
	pub yesterday: bool,
}

//=============================================================================

macro_rules! create_stats_class {
	($name:ident { $($field:ident),* $(,)? }) => {
		#[derive(Debug, Serialize, Deserialize, Default)]
		struct $name {
		$(
		//#[serde(skip_serializing_if = "Option::is_none")] // this would just skip `None` values, instead of submitting them to serialization, to be `null`
		$field: Option<i32>,
		)*
		}
	};
}

create_stats_class! {
	Sleep {
		yd_to_bed_t_plus_min,
		from_bed_t_plus_min,
		// not sure this is the best word though. It's difference, but I want to accent that it is |distance|
		to_bed_distance_yd_from_day_before_min,
	}
}

create_stats_class! {
	Eating {
		making_breakfast,
		eating_breakfast,
		making_lunch,
		eating_lunch,
		making_diner,
		eating_diner,
	}
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Masturbation {
	times: i32,
	visuals__full_none_work: Option<String>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
struct Stats {
	time_on_eating: Eating,
	masturbation: Masturbation,
	sleep: Sleep,
}

#[derive(Debug, Serialize, Deserialize)]
struct Day {
	date: String,
	ev: i32,
	stats: Stats,
}
