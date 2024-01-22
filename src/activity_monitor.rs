use crate::config::Config;
use anyhow::Result;
use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::{
	ffi::OsStr,
	process::{Command, Output},
};

use crate::MONITOR_PATH_APPENDIX;
use crate::TOTALS_PATH_APPENDIX;

pub fn start(config: Config) -> Result<()> {
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

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Activity {
	name: String,
	start_s: i64,
	end_s: i64,
}

fn get_activity(config: &Config) -> String {
	let _ = std::fs::create_dir(&config.data_dir.join(MONITOR_PATH_APPENDIX));
	let _ = std::fs::create_dir(&config.data_dir.join(TOTALS_PATH_APPENDIX));

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
		Ok(1) => "Tmux".to_owned(),
		Ok(3) => "Editing todos/notes".to_owned(),
		Ok(5) => "Reading a book".to_owned(),
		Ok(0) => "Catching runtime exceptions".to_owned(),
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
			} else if class == "Zulip" {
				activity = "Zulip".to_owned();
			}
		}
	}
	//

	"PC".to_owned() + config.activity_monitor.delimitor.as_str() + &activity
}

/// Incredibly inefficient way of recording a new entry, because we load all the existing ones first.
fn record_activity(config: &Config, name: String, start_s: i64, end_s: i64) {
	let save_dir = &config.data_dir.join(MONITOR_PATH_APPENDIX);

	let record = Activity { name, start_s, end_s };

	let date = Utc::now().format(&config.date_format.as_str()).to_string();
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

//TODO!!!: change so it takes the target date instead. Once done, add a command to recompile all of the recorded days. \
fn compile_yd_totals(config: &Config) {
	let date_yd = (Utc::now() - chrono::Duration::days(1)).format(config.date_format.as_str()).to_string();
	let yd_totals_file = (&config.data_dir.join(TOTALS_PATH_APPENDIX)).join(&date_yd);
	if yd_totals_file.exists() {
		return;
	};

	let yd_activities_file = (&config.data_dir.join(MONITOR_PATH_APPENDIX)).join(&date_yd);
	let file_contents = match std::fs::read_to_string(&yd_activities_file) {
		Ok(c) => c,
		Err(_) => "[]".to_owned(),
	};
	let yd_activities: Vec<Activity> = serde_json::from_str(&file_contents).unwrap();

	fn write_grand_total(yd_activities: Vec<Activity>, config: &Config) {
		let grand_total = Total::from_activities(yd_activities, &config.activity_monitor.delimitor);

		let formatted_json = serde_json::to_string_pretty(&grand_total).unwrap();
		let mut file = std::fs::File::create(&config.data_dir.join(TOTALS_PATH_APPENDIX).join("Grand Total")).unwrap(); //NB: replaces the existing if any
		file.write_all(formatted_json.as_bytes()).unwrap();
	}
	write_grand_total(yd_activities.clone(), &config);

	fn compile_calendar(yd_activities: Vec<Activity>, config: &Config) {
		let mut calendar: Vec<(String, i64)> = Vec::new();

		// iter over activities: from first found compile grand_total over next 15m. If it is above 7.5 -> store as (Total, start_timestamp)
		let mut period_end: i64 = 0;
		let mut over_period: Vec<Activity> = Vec::new();
		for a in yd_activities.iter() {
			if a.start_s > period_end {
				let period_total = Total::from_activities(over_period.clone(), &config.activity_monitor.delimitor);
				if period_total.time_s > (7.5 * 60.0 + 0.5) as i64 {
					calendar.push((
						period_total.find_largest("".to_owned(), &config.activity_monitor.delimitor),
						period_end - 15 * 60,
					));
				}

				period_end = a.start_s + 15 * 60;
				over_period.clear();
			}
			over_period.push(a.clone());
		}

		let client_id = config.activity_monitor.google_client_id.clone();
		let client_secret = config.activity_monitor.google_client_secret.clone();
		let refresh_token = config.activity_monitor.google_calendar_refresh_token.clone();
		let redirect_uri = "http://localhost";
		let calendar_client = google_calendar::Client::new(client_id, client_secret, redirect_uri, "", refresh_token);
		let r = tokio::runtime::Runtime::new().unwrap();
		r.block_on(async {
			let access_token = calendar_client.refresh_access_token().await.unwrap().access_token;

			let client = reqwest::Client::new();

			// reference: https://developers.google.com/calendar/api/v3/reference/events#resource-representations
			let mut handles = Vec::new();
			for (activity_name, start_s) in calendar.iter() {
				let start_time = DateTime::from_timestamp(*start_s, 0).unwrap().to_rfc3339();
				let end_time = DateTime::from_timestamp(*start_s + 15 * 60, 0).unwrap().to_rfc3339();
				let event = &json!({
					"summary": activity_name,
					"start": {
						"dateTime": start_time,
						"timeZone": "UTC"
					},
					"end": {
						"dateTime": end_time,
						"timeZone": "UTC"
					}
				});
				let url = format!(
					"https://www.googleapis.com/calendar/v3/calendars/{}/events",
					config.activity_monitor.calendar_id.clone()
				);

				let handle = client
					.post(url)
					.header("Content-Type", "application/json")
					.header("Authorization", format!("Bearer {}", &access_token))
					.json(event)
					.send();

				handles.push(handle);
			}
			for handle in handles {
				match handle.await {
					Ok(_) => {}
					Err(e) => {
						std::process::Command::new("echo")
							.arg(format!("{} >> /home/v/logs/activity_monitor.log", e))
							.output()
							.unwrap();
					}
				};
			}
		});
	}
	compile_calendar(yd_activities.clone(), &config);
}

#[derive(Debug, Serialize, Deserialize)]
struct GoogleServiceAccount {
	r#type: String,
	project_id: String,
	private_key_id: String,
	private_key: String,
	client_email: String,
	client_id: String,
	auth_uri: String,
	token_uri: String,
	auth_provider_x509_cert_url: String,
	client_x509_cert_url: String,
	universe_domain: String,
}

//you'll want to use `position()` which returns an `Option<usize>`
//if it's none, `add` it
//if it's there, deref the usize

#[derive(Debug, Serialize, Deserialize)]
struct Total {
	name: String,
	time_s: i64,
	children: Vec<Total>,
}
//TODO!: when Phone is added, make it so that if any point of time is claimed by both, Phone takes precedence.
// to implement, will need to just apply a mask once immediately after reading the file.
impl Total {
	fn new(name: String) -> Self {
		Total {
			name,
			time_s: 0,
			children: Vec::<Total>::new(),
		}
	}

	fn from_activities(activities: Vec<Activity>, activities_delimiter: &String) -> Self {
		let mut grand_total = Total::new("Total".to_owned());
		for a in activities {
			let time = a.end_s - a.start_s;

			let split: Vec<&str> = a.name.split(activities_delimiter).collect();
			assert!(split.len() <= 3); // in the perfect world expand to infinite, but seems like the current logic will be sufficient.

			// there has to be a better way to do this, but I'm not smart enough for rust way yet.
			{
				grand_total.time_s += time;
			}
			let l0_name = split[0].to_owned();
			let l0_index = match grand_total.children.iter().position(|t| t.name == l0_name) {
				Some(index) => index,
				None => {
					grand_total.children.push(Total::new(l0_name));
					grand_total.children.len() - 1
				}
			};
			{
				grand_total.children[l0_index].time_s += time;
			}
			let l1_name = split[1].to_owned();
			let l1_index = match grand_total.children[l0_index].children.iter().position(|t| t.name == l1_name) {
				Some(index) => index,
				None => {
					grand_total.children[l0_index].children.push(Total::new(l1_name));
					grand_total.children.len() - 1
				}
			};
			{
				grand_total.children[l0_index].children[l1_index].time_s += time;
			}
			if split.len() > 2 {
				let l2_name = split[2].to_owned();
				let l2_index = match grand_total.children[l0_index].children[l1_index]
					.children
					.iter()
					.position(|t| t.name == l2_name)
				{
					Some(index) => index,
					None => {
						grand_total.children[l0_index].children[l1_index].children.push(Total::new(l2_name));
						grand_total.children.len() - 1
					}
				};
				{
					grand_total.children[l0_index].children[l1_index].children[l2_index].time_s += time;
				}
			}
		}
		grand_total
	}

	/// returns the full path of the most prominent activity.
	/// to find, goes down the tree, and at each level takes the largest child.
	fn find_largest(&self, mut collect_str: String, activities_delimiter: &String) -> String {
		if self.children.len() == 0 {
			return collect_str;
		} else {
			let mut largest = &self.children[0];
			for c in self.children.iter() {
				if c.time_s > largest.time_s {
					largest = c;
				}
			}

			if collect_str.len() > 0 {
				collect_str = collect_str + " - ";
			}
			collect_str = collect_str + &largest.name;
			return largest.find_largest(collect_str, activities_delimiter);
		}
	}
}
