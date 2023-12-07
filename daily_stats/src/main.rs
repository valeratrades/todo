#![allow(non_snake_case)]
use chrono::prelude::*;
use chrono::Duration;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};

mod _static {
	pub static SAVE_PATH: &str = "/home/v/data/personal/daily_stats.json";
}

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
	Eating {
		making_breakfast,
		eating_breakfast,
		making_lunch,
		eating_lunch,
		making_diner,
		eating_diner,
	}
}

create_stats_class! {
	Masturbation {
	times,
	visuals__full_1__no_2__work_3,
	}
}

#[derive(Default, Debug, Serialize, Deserialize)]
struct Stats {
	masturbation: Masturbation,
	time_on_eating: Eating,
}

#[derive(Debug, Serialize, Deserialize)]
struct Day {
	time: String,
	ev: i32,
	stats: Stats,
}

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let ev: i32 = args[1].parse().unwrap();
	let time: String = match args.get(2).map(String::as_str) {
		Some("-y") => (Utc::now() - Duration::days(1)).format("%Y/%m/%d").to_string(),
		_ => Utc::now().format("%Y/%m/%d").to_string(),
	};
	let record = Day {
		time: time.clone(),
		ev,
		stats: Stats::default(),
	};

	let parent_dir = std::path::Path::new(&_static::SAVE_PATH).parent().unwrap();
	let _ = std::fs::create_dir_all(parent_dir);
	let mut records: VecDeque<Day> = match File::open(&_static::SAVE_PATH) {
		Ok(mut file) => {
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap_or_else(|_| VecDeque::new())
		}
		Err(_) => VecDeque::new(),
	};

	records.retain(|day| day.time != time);
	records.push_back(record);

	let mut file = File::create(&_static::SAVE_PATH).unwrap();
	let formatted_json = serde_json::to_string_pretty(&records).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();
}
