#[allow(dead_code, unused_imports)]
use chrono::prelude::*;
use serde::{Deserialize, Serialize};

mod _static {
	pub static CATEGORIES: [&str; 4] = ["d:data-collection", "h:home-chore", "w:workout", "i:close-git-issue"];
	pub static HARD_STOP_COEFF: f32 = 1.5;
}

#[derive(Debug)]
struct Category(String);
impl From<String> for Category {
	fn from(c: String) -> Self {
		let categories = &_static::CATEGORIES;
		let s = c.as_str();

		for category in categories.iter() {
			if category == &s {
				return Category(s.to_owned());
			}
			let split: Vec<&str> = category.split(':').collect();
			if split[0] == s || split[1] == s {
				return Category(category.to_string());
			}
		}

		println!("Error: Invalid category '{}'", c);
		std::process::exit(1);
	}
}
#[derive(Debug)]
struct Ongoing {
	timestamp: i64,
	category: Category,
	estimated_minutes: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
	timestamp: i64,
	category: Category,
	estimated_minutes: u32,
	status: bool,
	realised_time: u32,
}

fn main() {
	let args: Vec<String> = std::env::args().collect();
	match args.get(1).map(String::as_str) {
		Some("start") => start(&args),
		Some("done") => done(),
		Some("failed") => failed(),
		Some(cmd) if cmd.replace("-", "").starts_with('h') => help(),
		_ => panic!("No such option: {}", args.get(1).unwrap_or(&String::new())),
	}
}

fn start(args: &[String]) {
	let mut estimated_minutes: u32 = 0;
	let mut arg_category = String::new();
	for arg in args.iter().skip(2) {
		if let Ok(num) = arg.parse::<u32>() {
			if num > 90 {
				println!("Error: Cut your task into smaller parts. Anything above 90m does not make sense.");
				std::process::exit(1);
			}
			estimated_minutes = num;
		} else {
			arg_category = arg.to_string();
		}
	}
	let category = Category::from(arg_category);
	let timestamp = Utc::now().timestamp_millis();
	let entry = Ongoing {
		timestamp,
		category,
		estimated_minutes,
	};

	write_and_time(entry);
}

fn done() {
	println!("done")
}

fn failed() {
	println!("failed")
}

fn help() {
	let categories = &_static::CATEGORIES;

	println!(
		r#"Start a task with timer, then store error (to track improvement of your estimations of time spent on different task categories)
	Usage:
		my_todo start [EXPECTED_TIME] [CATEGORY]
		. . . // start doing the task, then:
		my_todo done || my_todo failed
	Commands:
		start   start a timer for a task
		done    mark the task as done and save the time it took to finish  
		failed  abort the task
	Arguments:
		[EXPECTED_TIME]  Time in minutes that you expect the task to take.
		[CATEGORY]       Category of the task. Can be one of:
			{}"#,
		categories.join(",\n    ")
	);
}

// ----------------------------------------------------------------------------

fn write_and_time(task: Ongoing) {
	std::fs::create_dir_all("/home/v/data/my_todo");
	let estimated_ms = Ongoing::timestamp + Ongoing::estimated_minutes as i64 * 60 * 1000;
	let hard_stop_ms = Ongoing::timestamp + (&_static.HARD_STOP_COEFF * (Ongoing::estimated_minutes * 60 * 1000) as f32) as i64;

	dbg!(&hard_stop_ms);
}
