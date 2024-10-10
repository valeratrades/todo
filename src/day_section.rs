use chrono::prelude::*;
use color_eyre::eyre::Result;

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

	pub fn description(&self) -> &str {
		match self {
			DaySection::Morning =>
				r#"
# Morning
for physical things

## Do every time
- run
- shower
- 3-4 egg omelet
- 15m of typing practice right before the transition to `Work`

### check
- dishwasher
- vacuum
- any verifications

### suggestions
- typing practice

## Actions:
### Talking
On constructive topics. No unprompted monologues, but can be interacted with to schedule plans or answer a question.

### Clothes
Running outfit. Even after running I just change the underwear, and remain in it, until I transition to the `Work`.

### Payed work
Can be asked, but not requested.

### Remember
> There are no shortcuts for you, Valera
"#,
			DaySection::Work =>
				r#"
# Work
For Necessary things.

You work with what you have, and you get shit done. In these 8 hours it does not matter what project could be good for your future; what skills would be good to learn; how your environment could be improved. You do the things to maximize their immediate productive output. It does not matter if you had some project to watch your todos, or make the screen redder closer to night, - you maximize the day's dollar value, and *nothing* else.

## Actions:
### Coffee
The only day section permitting cafein intake.

### Talking
Absolutely none, except for strictly work-related reasons, in which case the conversation is immediately to the point, and to be cut down for time.

### Clothes
Full suit, or just shirt, costum trousers and tie if it's hot.

### Payed work
First priority

### Phone
Turned off fully, and placed inside a mailbox if any urges arise.

### Remember
> It's not going to be fun. You will feel tired, bored, at times physically suffering. You must say "no" to everything your whole life. Your existence is optimising for pressing keys in correct order, sitting at a desk 14 hours a day. This is the cost of greatness.
"#,
			DaySection::Evening =>
				r#"
# Evening
fun and reflection

## Do every time
- go through the telegram entries made during the work session.

## Actions
### Talking
Whatever you feel like, no limits whatsoever.

### Clothes
Any clothes I want, that are not taken up by the previous day sections.

### Payed work
Can be asked, but not requested.

### Remember
> Iteration Speed
"#,
			DaySection::Night =>
				r#"
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
		DaySectionBorders { morning_end, work_end, evening_end }
	}
}
