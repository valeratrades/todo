#![allow(non_snake_case)]
use crate::config::Config;
use crate::utils;
use anyhow::Result;
use chrono::prelude::*;
use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;

use clap::Args;
use std::path::PathBuf;

pub fn update_or_open(config: Config, args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.manual_stats.path.0.clone();
	let _ = std::fs::create_dir_all(&data_storage_dir);
	let data_file_path = data_storage_dir.join("manual_daily_stats.json");
	let mut file = OpenOptions::new().read(true).write(true).create(true).open(&data_file_path).unwrap();

	if args.ev == None && args.open == false {
		return Err(anyhow::anyhow!("provide `ev` and/or `open` arguments"));
	}

	let date: String = match args.yesterday {
		true => (Utc::now() - Duration::days(1)).format("%Y/%m/%d").to_string(),
		false => Utc::now().format("%Y/%m/%d").to_string(),
	};

	let file_contents = std::fs::read_to_string(&data_file_path).unwrap();
	let mut values: VecDeque<serde_json::Value> = match serde_json::from_str(&file_contents) {
		Ok(v) => v,
		Err(_) => VecDeque::new(),
	};

	let mut store_today_if_appending_yesterday: Option<Value> = None;

	let mut day: Day = {
		let mut temp_day: Option<Day> = None;

		if let Some(last_value) = values.back() {
			if let Some(Value::String(recorded_date)) = last_value.get("date") {
				if recorded_date == &date {
					if let Ok(day) = serde_json::from_value::<Day>(values.pop_back()/*NB: mutation*/.unwrap()) {
						temp_day = Some(day);
					}
				}
			}
		}

		if temp_day.is_none() {
			if let Some(previous_to_last_value) = values.iter().rev().nth(1) {
				if let Some(Value::String(recorded_date)) = previous_to_last_value.get("date") {
					if recorded_date == &date {
						store_today_if_appending_yesterday = Some(values.pop_back()/*NB: mutation*/.unwrap());
						if let Ok(yd) = serde_json::from_value::<Day>(values.pop_back()/*NB: mutation*/.unwrap()) {
							temp_day = Some(yd);
						}
					}
				}
			}
		}

		if temp_day.is_none() {
			if let Some(last_value) = values.back() {
				if let Some(Value::String(recorded_date)) = last_value.get("date") {
					let date_today = Utc::now().format("%Y/%m/%d").to_string();
					if args.yesterday == true && recorded_date == &date_today {
						store_today_if_appending_yesterday = Some(values.pop_back()/*NB: mutation*/.unwrap());
					}
				}
			}
		}

		if temp_day.is_none() {
			if let Some(ev) = args.ev {
				temp_day = Some(Day {
					date: date.clone(),
					ev,
					stats: Stats::default(),
				});
			} else {
				panic!("The day object is not initialized, so `ev` argument is required");
			}
		}

		temp_day.unwrap()
	};

	if let Some(ev) = args.ev {
		day.ev = ev;
	}

	values.push_back(serde_json::to_value(day).unwrap());

	if let Some(tmp_today) = store_today_if_appending_yesterday {
		if let Ok(day) = serde_json::from_value::<Day>(tmp_today) {
			values.push_back(serde_json::to_value(day).unwrap());
		}
	}

	let formatted_json = serde_json::to_string_pretty(&values).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();

	if args.open == true {
		utils::open(&data_file_path);
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
	//? tasks? And then merge with my_todo, and record all tasks right here.
}
