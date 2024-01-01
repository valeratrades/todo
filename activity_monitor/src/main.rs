use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::{
	ffi::OsStr,
	process::{Command, Output},
};

//TODO!!: move to config file \\
const DELIMITOR: &'static str = " - ";
const SAVE_DIR: &'static str = "/home/v/data/personal/activity_monitor/";

fn main() {
	let mut prev_activity = String::new();
	let mut start_s = Utc::now().timestamp();
	loop {
		let activity = get_activity();
		if prev_activity != activity {
			let now = Utc::now().timestamp();
			record_activity(activity.clone(), start_s, now);

			prev_activity = activity;
			start_s = now;
		}
		std::thread::sleep(std::time::Duration::from_secs(1));
	}
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
	name: String,
	start_s: i64,
	end_s: i64,
}

fn get_activity() -> String {
	fn cmd<S>(command: S) -> Output
	where
		S: AsRef<OsStr>,
	{
		let output = Command::new("sh").arg("-c").arg(command).output().unwrap();
		output
	}

	let output = cmd("swaymsg -t get_workspaces | jq -r '.[] | select(.focused==true).name'");
	let output_str = String::from_utf8_lossy(&output.stdout);
	let focused = output_str.trim_end_matches('\n');

	let mut activity: String = match focused.parse() {
		Ok(1) => "Neovim".to_owned(),
		Ok(3) => "Editing todos/notes".to_owned(),
		Ok(5) => "Reading a book".to_owned(),
		Ok(0) => "Reading runtime exceptions".to_owned(),
		Ok(num) => format!("Workspace {}", num),
		Err(_) => unreachable!(),
	};

	// // if get_tree focused returns name of the app to be [discord, chrome, telegram] - overwrite.
	let output = cmd("swaymsg -t get_tree | jq -r '.. | (.nodes? // empty)[] | select(.focused==true)'");
	let output_str = String::from_utf8_lossy(&output.stdout);
	let json: Value = serde_json::from_str(&output_str).unwrap();

	if let Some(app_id) = json.get("app_id").and_then(|v| v.as_str()) {
		if app_id == "org.telegram.desktop" {
			activity = "Telegram".to_owned();
		}
	}
	if let Some(window_properties) = json.get("window_properties") {
		if let Some(class) = window_properties.get("class").and_then(|v| v.as_str()) {
			if class == "Google-chrome" {
				activity = "Google".to_owned();

				let title = window_properties.get("title").unwrap().to_string();
				let title_trimmed = title.replace("\"", "");
				let title_split: Vec<&str> = title_trimmed.split(" - ").collect();
				if title_split.len() >= 2 {
					let relevant = title_split[title_split.len() - 2];
					activity = activity + " - " + relevant;
				}
			} else if class == "discord" {
				activity = "Discord".to_owned();
			}
		}
	}
	//

	"PC".to_owned() + DELIMITOR + &activity
}

/// Incredibly inefficient way of recording a new entry, because we load all the existing ones first.
fn record_activity(name: String, start_s: i64, end_s: i64) {
	let save_dir = std::path::Path::new(SAVE_DIR);
	let _ = std::fs::create_dir_all(save_dir);

	let record = Record { name, start_s, end_s };

	let date = Utc::now().format("%Y-%m-%d").to_string();
	let target_path = [SAVE_DIR, &date].concat();
	let mut records: VecDeque<Record> = match File::open(&target_path) {
		Ok(mut file) => {
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap_or_else(|_| VecDeque::new())
		}
		Err(_) => VecDeque::new(),
	};
	records.push_back(record);

	let mut file = File::create(&target_path).unwrap();
	let json = serde_json::to_string(&records).unwrap();
	file.write_all(json.as_bytes()).unwrap();
}
