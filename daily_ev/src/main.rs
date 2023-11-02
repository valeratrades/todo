use chrono::prelude::*;
use chrono::Duration;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};

mod _static {
	pub static SAVE_PATH: &str = "/home/v/data/personal/daily_ev.json";
}

#[derive(Debug, Serialize, Deserialize)]
struct Day {
	time: String,
	ev: i32,
}

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let ev: i32 = args[1].parse().unwrap();
	// let time = Utc::now().format("%Y/%m/%d").to_string();
	let time: String = match args.get(2).map(String::as_str) {
		Some("-y") => (Utc::now() - Duration::days(1)).format("%Y/%m/%d").to_string(),
		_ => Utc::now().format("%Y/%m/%d").to_string(),
	};
	let record = Day { time: time.clone(), ev };

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
	file.write_all(serde_json::to_string(&records).unwrap().as_bytes()).unwrap();
}
