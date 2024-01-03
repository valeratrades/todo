use anyhow::Result;
use chrono::prelude::*;

#[derive(Debug)]
struct Waketime {
	hours: u32,
	minutes: u32,
}
impl From<String> for Waketime {
	fn from(s: String) -> Self {
		let split: Vec<_> = s.split(':').collect();
		assert!(split.len() == 2, "ERROR: waketime should be in the format: \"%H:%M\"");
		let hours: u32 = split[0].parse().unwrap();
		let minutes: u32 = split[1].parse().unwrap();
		Waketime { hours, minutes }
	}
}

pub enum DaySection {
	Morning,
	Work,
	Evening,
	Night,
}
impl DaySection {
	pub fn build() -> Result<Self> {
		let waketime_env = std::env::var("WAKETIME")?;
		let waketime = Waketime::from(waketime_env);

		let day_section_borders_env = std::env::var("DAY_SECTION_BORDERS")?;
		let day_section_borders = DaySectionBorders::from(day_section_borders_env);
		let day_section = day_section_borders.now_in(waketime);
		Ok(day_section)
	}

	pub fn description(&self) -> &'static str {
		match self {
			DaySection::Morning => r#"
# Morning
for physical things

## Talking
On constructive topics. No unprompted monologues, but can be interacted with to schedule plans or answer a question.
"#,
			DaySection::Work => r#"
# Work
for necessary things

## Talking
Absolutely none, except for strictly work-related reasons, in which case the conversation is immediately to the point, and to be cut down for time.
"#,
			DaySection::Evening => r#"
# Evening
fun and reflection

## Talking
Whatever you feel like, no limits whatsoever.
"#,
			DaySection::Night => r#"
# Night
## SLEEP
"#,
		}
	}
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
