use chrono::prelude::*;
use std::path::PathBuf;

//TODO!!!: move consts to config file or env variables.
const TODO_DIR: &'static str = "/home/v/Todo/";
//TODO!!!!: const for section splits

pub fn compile() {
	let waketime_env_output = std::process::Command::new("sh").arg("-c").arg("echo $WAKETIME").output().unwrap();
	let waketime_env = String::from_utf8_lossy(&waketime_env_output.stdout).to_string();
	let waketime = Waketime::from(waketime_env);

	let day_section_borders_env_output = std::process::Command::new("sh")
		.arg("-c")
		.arg("echo $DAY_SECTION_BORDERS")
		.output()
		.unwrap();
	let day_section_borders_env = String::from_utf8_lossy(&day_section_borders_env_output.stdout).to_string();
	let day_section_borders = DaySectionBorders::from(day_section_borders_env);
	let day_section = day_section_borders.now_in(waketime);

	// apply formula to get the priority task according to time of day.

	// concat with description of the section

	// compile String to md with pandoc or something and pipe into zathura
}

#[derive(Debug)]
struct Waketime {
	hours: u32,
	minutes: u32,
}
impl From<String> for Waketime {
	fn from(s: String) -> Self {
		let split: Vec<_> = s.split(':').collect();
		assert!(split.len() == 2, "ERROR: waketime should be supplied in the format: \"%H:%M\"");
		let hours: u32 = split[0].parse().unwrap();
		let minutes: u32 = split[1].parse().unwrap();
		Waketime { hours, minutes }
	}
}

enum DaySection {
	Morning,
	Work,
	Evening,
	Night,
}
#[derive(Debug)]
struct DaySectionBorders {
	morning_end: i32,
	work_end: i32,
	evening_end: i32,
}
impl DaySectionBorders {
	pub fn now_in(&self, wt: Waketime) -> DaySection {
		let nm = Utc::now().hour() * 60 + Utc::now().minute();
		let wt_m = wt.hours * 60 + wt.minutes;

		// shift everything wt minutes back
		// in python would be `(nm - wt) % 24`, but rust doesn't want to exhibit desired behaviour with % on negative numbers
		let mut now_shifted = nm as i32 - wt_m as i32;
		if now_shifted < 0 {
			now_shifted += 24 * 60;
		}

		match now_shifted {
			t if (t > 20 * 60) || (t <= self.morning_end) => DaySection::Morning,
			t if t <= self.work_end => DaySection::Work,
			t if t <= self.evening_end => DaySection::Evening,
			_ => DaySection::Night,
		}
	}
}
impl From<String> for DaySectionBorders {
	fn from(s: String) -> Self {
		let split: Vec<_> = s.split(':').collect();
		assert!(
			split.len() == 3,
			"ERROR: day section splits should be time of every section border after waketime, like:\"2.5:10.5:16\""
		);

		fn parse(s: &str) -> i32 {
			let border_h: f32 = s.parse().unwrap();
			let border_m: i32 = (border_h * 60.0) as i32;
			border_m
		}
		let morning_end: i32 = parse(split[0]);
		let work_end: i32 = parse(split[1]);
		let evening_end: i32 = parse(split[2]);
		DaySectionBorders {
			morning_end,
			work_end,
			evening_end,
		}
	}
}
