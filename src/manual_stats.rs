use std::fs::File;
use std::io::{Read, Write};
use crate::config::Config;
use anyhow::{Context, Result};
use std::collections::VecDeque;
use chrono::prelude::*;
use chrono::Duration;

use clap::Args;
use std::path::PathBuf;

pub fn update_or_open(config: Config, args: ManualArgs) -> Result<()> {
	let data_storage_dir: PathBuf = config.manual_stats.path.0.clone();
	let _ = std::fs::create_dir_all(data_storage_dir);
	let data_file_path = data_storage_dir.join("manual_daily_stats.json");
	let mut file = File::create(data_file_path.clone()).unwrap();

	//TODO!: assert at least one of [ev, open] is present.
	//anyhow::anyhow!("provide `ev` and/or `open` arguments"))?

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

	let mut records: VecDeque<Day> = match File::open(data_file_path.clone()) {
		Ok(mut file) => {
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap_or_else(|_| VecDeque::new())
		}
		Err(_) => VecDeque::new(),
	};

	records.retain(|day| day.time != time);
	records.push_back(record);

	let formatted_json = serde_json::to_string_pretty(&records).unwrap();
	file.write_all(formatted_json.as_bytes()).unwrap();

	Ok(())
}

#[derive(Args)]
pub struct ManualArgs {
	pub ev: i32,
	#[arg(short, long)]
	pub open: bool,
	#[arg(short, long)]
	pub yesterday: bool,
}

