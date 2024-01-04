use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::process::Command;

mod _static {
	pub static CATEGORIES: [&str; 12] = [
		"",
		"d:data-collection",
		"h:home-chore",
		"w:workout",
		"ci:close-git-issue",
		"t:tooling",
		"l:work-on-library",
		"s:trading-systems",
		"cp:code-python",
		"cr:code-rust",
		"cg:code-go",
		"pd:personal-data-collection",
	];
	pub static HARD_STOP_COEFF: f32 = 1.5;
	pub static STATE_PATH: &str = "/home/v/tmp/my_todo/ongoing.json";
	pub static SAVE_PATH: &str = "/home/v/data/personal/todo.json";
}

#[derive(Debug, Serialize, Deserialize)]
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
			if split.get(0) == Some(&s) || split.get(1) == Some(&s) {
				return Category(category.to_string());
			}
		}

		println!("Error: Invalid category '{}'", c);
		std::process::exit(1);
	}
}
#[derive(Serialize, Deserialize, Debug)]
struct Ongoing {
	timestamp_s: u32,
	category: Category,
	estimated_minutes: u32,
	description: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Result {
	timestamp_s: u32,
	category: Category,
	estimated_minutes: u32,
	description: String,
	completed: bool,
	realised_minutes: u32,
}

fn main() {
	let args: Vec<String> = std::env::args().collect();
	if args.len() == 1 {
		// try to pick up a dropped task
		run();
	}
	match args.get(1).map(String::as_str) {
		Some("start") => start(&args),
		Some("done") => done(),
		Some("failed") => failed(),
		Some(cmd) if cmd.replace("-", "").starts_with('h') => help(),
		_ => {
			eprintln!("\x1b[31mNo such option: {}\x1b[0m\nConsult \x1b[34m--help\x1b[0m", args.get(1).unwrap_or(&String::new()));
			std::process::exit(1);
		}
	}
}

fn start(args: &[String]) {
	let args: &[String] = &args[2..]; // for simpler count

	let estimated_minutes: u32 = {
		if args.len() >= 1 {
			if let Ok(num) = args[0].parse::<u32>() {
				if num > 90 {
					eprintln!("\x1b[31mCut your task into smaller parts. Anything above 90m does not make sense.\x1b[0m");
					std::process::exit(1);
				}
				num
			} else {
				eprintln!("\x1b[31mFirst argument after action must be estimated time in minutes\x1b[0m");
				std::process::exit(1);
			}
		} else {
			90_u32
		}
	};
	let arg_category: String = {
		if args.len() >= 2 {
			args[1].to_string()
		} else {
			"".to_owned()
		}
	};
	let description: String = {
		if args.len() >= 3 {
			args[2].to_string()
		} else {
			"".to_owned()
		}
	};

	let category = Category::from(arg_category);
	let timestamp_s = Utc::now().timestamp() as u32;
	let task = Ongoing {
		timestamp_s,
		category,
		estimated_minutes,
		description,
	};

	let parent_dir = std::path::Path::new(&_static::STATE_PATH).parent().unwrap();
	let _ = std::fs::create_dir_all(parent_dir);
	let mut file = File::create(&_static::STATE_PATH).unwrap();
	let serialized = serde_json::to_string(&task).unwrap();
	file.write_all(serialized.as_bytes()).unwrap();

	run();
}

fn save_result(mut completed: bool) {
	let mut file = File::open(&_static::STATE_PATH).unwrap();
	let mut contents = String::new();
	file.read_to_string(&mut contents).unwrap();
	let ongoing: Ongoing = serde_json::from_str(&contents).unwrap();

	let realised_minutes = {
		let diff_m = ((Utc::now().timestamp() as u32 - ongoing.timestamp_s) as f32 / 60.0) as u32;
		let hard_stop_m = (&_static::HARD_STOP_COEFF * ongoing.estimated_minutes as f32 + 0.5) as u32;
		if hard_stop_m < diff_m {
			completed = false; // It was possible to do `my_todo done` while executable is inactive, passing completed==true here, while far pasts the hard stop
			hard_stop_m
		} else {
			diff_m
		}
	};
	let result = Result {
		timestamp_s: ongoing.timestamp_s,
		category: ongoing.category,
		estimated_minutes: ongoing.estimated_minutes,
		description: ongoing.description,
		completed,
		realised_minutes,
	};

	let parent_dir = std::path::Path::new(&_static::SAVE_PATH).parent().unwrap();
	let _ = std::fs::create_dir_all(parent_dir);
	let mut results: VecDeque<Result> = match File::open(&_static::SAVE_PATH) {
		Ok(mut file) => {
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap_or_else(|_| VecDeque::new())
		}
		Err(_) => VecDeque::new(),
	};

	results.push_back(result);

	let mut file = File::create(&_static::SAVE_PATH).unwrap();
	file.write_all(serde_json::to_string(&results).unwrap().as_bytes()).unwrap();
	let _ = std::fs::remove_file(&_static::STATE_PATH);

	std::thread::sleep(std::time::Duration::from_millis(300)); // wait for eww to process previous request if any.
	let eww_output = Command::new("sh").arg("-c").arg("eww get todo_timer".to_owned()).output().unwrap();
	let todo_timer = String::from_utf8_lossy(&eww_output.stdout).trim().to_string();
	if !todo_timer.starts_with("Out") {
		let _ = Command::new("sh").arg("-c").arg("eww update todo_timer=None".to_owned()).output().unwrap();
	}
}
fn done() {
	save_result(true);
}

fn failed() {
	save_result(false);
}

fn help() {
	let categories = &_static::CATEGORIES;

	println!(
		r#"Start a task with timer, then store error (to track improvement of your estimations of time spent on different task categories)
	Usage:
		my_todo [ACTION] [EXPECTED_TIME] [CATEGORY] [DESCRIPTION]
	Example Usage:
		my_todo start 20 tooling gtk-default-browser
		. . . // start doing the task, then:
		my_todo done
	Actions:
		start   start a timer for a task
		done    mark the task as done and save the time it took to finish  
		failed  abort the task
	Defaults:
		if not provided, arguments default to:
		[EXPECTED_TIME]: 90
		[CATEGORY]: None
		[DESCRIPTION]: ""
	Arguments:
		[EXPECTED_TIME]  Time in minutes that you expect the task to take.
		[CATEGORY]       Category of the task. Can be one of:
			{}
		[DESCRIPTION]    Short descrption of the task,that will show up next to the timer."#,
		categories.join(",\n\t\t\t")
	);
}

// ----------------------------------------------------------------------------

//TODO!!!!: return Result \
fn run() {
	let task: Ongoing = {
		if std::path::Path::new(&_static::STATE_PATH).exists() {
			let mut file = File::open(&_static::STATE_PATH).unwrap();
			let mut contents = String::new();
			file.read_to_string(&mut contents).unwrap();
			serde_json::from_str(&contents).unwrap()
		} else {
			eprintln!("No record of an ongoing task found, exiting.");
			std::process::exit(0);
		}
	};
	let estimated_s = task.timestamp_s + task.estimated_minutes * 60;
	let hard_stop_s = task.timestamp_s + (&_static::HARD_STOP_COEFF * (task.estimated_minutes * 60) as f32) as u32;

	loop {
		let path = std::path::Path::new(&_static::STATE_PATH);
		if !path.exists() {
			break;
		}

		let now_s = Utc::now().timestamp();
		let e_diff = estimated_s as i64 - now_s as i64;
		let h_diff = hard_stop_s as i64 - now_s as i64;
		let description = {
			if task.description.as_str() == "" {
				format!("")
			} else {
				format!("_{}", task.description)
			}
		};

		let value = if h_diff < 0 {
			format!("Out{}", description)
		} else if e_diff < 0 {
			format!("-{:02}:{:02}{}", h_diff / 60, h_diff % 60, description)
		} else {
			format!("{:02}:{:02}{}", e_diff / 60, e_diff % 60, description)
		};

		let _ = Command::new("sh")
			.arg("-c")
			.arg(format!("eww update todo_timer={}", value.replace(" ", "_").unwrap()))
			.output()
			.unwrap();
		if value.starts_with("Out") {
			save_result(false);
			std::process::exit(0);
		}
		std::thread::sleep(std::time::Duration::from_secs(1));
	}
}
