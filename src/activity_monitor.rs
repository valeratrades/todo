use crate::config::Config;
use anyhow::Result;
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

const DATE_FORMAT: &'static str = "%Y-%m-%d";

pub fn start(config: Config) -> Result<()> {
	let _ = std::fs::create_dir_all(&config.activity_monitor.activities_dir.0);
	let _ = std::fs::create_dir_all(&config.activity_monitor.totals_dir.0);

	let mut prev_activity_name = String::new();
	let mut start_s = Utc::now().timestamp();
	loop {
		let activity_name = get_activity(&config);
		if prev_activity_name != activity_name {
			let now = Utc::now().timestamp();
			record_activity(&config, activity_name.clone(), start_s, now);
			compile_yd_totals(&config);

			prev_activity_name = activity_name;
			start_s = now;
		}
		std::thread::sleep(std::time::Duration::from_secs(1));
	}
}

#[derive(Debug, Serialize, Deserialize)]
struct Activity {
	name: String,
	start_s: i64,
	end_s: i64,
}

fn get_activity(config: &Config) -> String {
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

	"PC".to_owned() + config.activity_monitor.delimitor.as_str() + &activity
}

/// Incredibly inefficient way of recording a new entry, because we load all the existing ones first.
fn record_activity(config: &Config, name: String, start_s: i64, end_s: i64) {
	let save_dir = &config.activity_monitor.activities_dir.0;
	let _ = std::fs::create_dir_all(&save_dir);

	let record = Activity { name, start_s, end_s };

	let date = Utc::now().format(DATE_FORMAT).to_string();
	let target_path = save_dir.join(&date);
	let mut records: VecDeque<Activity> = match File::open(&target_path) {
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

//-----------------------------------------------------------------------------

fn compile_yd_totals(config: &Config) {
	let date_yd = (Utc::now() - chrono::Duration::days(1)).format(DATE_FORMAT).to_string();
	let yd_totals_file = (&config.activity_monitor.totals_dir.0).join(&date_yd);
	if yd_totals_file.exists() {
		return;
	};

	let yd_activities_file = (&config.activity_monitor.activities_dir.0).join(&date_yd);
	let file_contents = match std::fs::read_to_string(&yd_activities_file) {
		Ok(c) => c,
		Err(_) => "[]".to_owned(),
	};
	let yd_activities: Vec<Activity> = serde_json::from_str(&file_contents).unwrap();

	let mut totals: Vec<Total> = Vec::new();
	for a in yd_activities {
		let time = a.end_s - a.start_s;

		//NB: rely on always having `2 <= levels <= 3` in the name
		let split: Vec<&str> = a.name.split(&config.activity_monitor.delimitor).collect();

		// there has to be a better way to do this, but I'm not smart enough for rust way yet.
		let l0_name = split[0].to_owned();
		let l0_index = match totals.iter().position(|t| t.name == l0_name) {
			Some(index) => index,
			None => {
				totals.push(Total::new(l0_name));
				totals.len() - 1
			}
		};
		{
			totals[l0_index].time_s += time;
		}
		let l1_name = split[1].to_owned();
		let l1_index = match totals[l0_index].children.iter().position(|t| t.name == l1_name) {
			Some(index) => index,
			None => {
				totals[l0_index].children.push(Total::new(l1_name));
				totals.len() - 1
			}
		};
		{
			totals[l0_index].children[l1_index].time_s += time;
		}
		if split.len() > 2 {
			let l2_name = split[2].to_owned();
			let l2_index = match totals[l0_index].children[l1_index].children.iter().position(|t| t.name == l2_name) {
				Some(index) => index,
				None => {
					totals[l0_index].children[l1_index].children.push(Total::new(l2_name));
					totals.len() - 1
				}
			};
			{
				totals[l0_index].children[l1_index].children[l2_index].time_s += time;
			}
		}
	}

	let formatted_json = serde_json::to_string_pretty(&totals).unwrap();
	let mut file = std::fs::File::create(&yd_totals_file).unwrap(); //NB: replaces the existing if any
	file.write_all(formatted_json.as_bytes()).unwrap();
}
#[derive(Debug, Serialize, Deserialize)]
struct Total {
	name: String,
	time_s: i64,
	children: Vec<Total>,
}
impl Total {
	fn new(name: String) -> Self {
		Total {
			name,
			time_s: 0,
			children: Vec::<Total>::new(),
		}
	}
}
