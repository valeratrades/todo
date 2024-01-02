use crate::config::Config;
use crate::day_section::DaySection;
use crate::utils;
use anyhow::{Context, Result};

use clap::Args;
use std::path::PathBuf;

pub fn compile_quickfix(config: Config) -> Result<()>{
	let day_section = DaySection::build().unwrap();
	let path: PathBuf = day_section_path(&config, &day_section);

	let mut tasks: Vec<Task> = Vec::new();

	for entry in std::fs::read_dir(path).unwrap() {
		let entry = entry.unwrap();
		let path = entry.path();

		if path.is_file() {
			let task = Task::try_from(path)?;
			tasks.push(task);
		}
	}

	//let len = tasks.len();
	//for _ in 0..len {
	//	for j in 0..=len {
	//		if tasks[j].priority > tasks[j+1].priority {
	//			tasks.swap(j, j+1);
	//		}
	//	}
	//}

	let len = tasks.len() as isize;
	quick_sort(&mut tasks, 0, len - 1);

	dbg!(&tasks);


	// concat with description of the section

	// compile String to md with pandoc or something and pipe into zathura

	Ok(())
}

pub fn open_or_add(config: Config, flags: TodosFlags, name: Option<String>) -> Result<()> {
	let day_section = flags.extract_day_section();

	let mut path = day_section_path(&config, &day_section);

	if let Some(name) = name {
		path.push([&name, ".md"].concat());
		let _ = std::fs::File::create(&path).unwrap();
	}

	if flags.open == true {
		utils::open(path);
	}

	Ok(())
}

//=============================================================================

#[derive(Args)]
pub struct OpenArgs {
	#[clap(flatten)]
	pub shared: TodosFlags,
}
#[derive(Args)]
pub struct AddArgs {
	pub name: String,
	#[clap(flatten)]
	pub shared: TodosFlags,
}
#[derive(Args)]
pub struct TodosFlags {
	#[arg(long, short)]
	pub morning: bool,
	#[arg(long, short)]
	pub work: bool,
	#[arg(long, short)]
	pub evening: bool,
	#[arg(long, short)]
	pub night: bool,
	#[arg(long, short)]
	pub open: bool,
}
#[derive(Args)]
pub struct QuickfixArgs {}

impl TodosFlags {
	fn extract_day_section(&self) -> DaySection {
		match self {
		Self { morning: true, .. } => DaySection::Morning,
		Self { work: true, .. } => DaySection::Work,
		Self { evening: true, .. } => DaySection::Evening,
		Self { night: true, .. } => DaySection::Night,
			_ => DaySection::Evening,
		}
	}
}

fn day_section_path<'a>(config: &'a Config, day_section: &'a DaySection) -> PathBuf {
	let todos_dir = config.todos.path.0.clone();

	let path_appendix: &str = match day_section.to_owned() {
		DaySection::Morning => ".morning/",
		DaySection::Work => ".work/",
		DaySection::Evening => ".evening/",
		DaySection::Night => ".night/",
	};

	todos_dir.join(path_appendix)
}

#[derive(Debug)]
struct Task {
	priority: f32,
	path: PathBuf,
}
impl TryFrom<PathBuf> for Task {
	type Error = anyhow::Error;

	fn try_from(path: PathBuf) -> Result<Self> {
		let filename = path.file_name()
			.ok_or_else(|| anyhow::anyhow!("Filename not found in path"))?
			.to_str()
			.ok_or_else(|| anyhow::anyhow!("Filename is not valid UTF-8"))?;
		let split: Vec<_> = filename.split('-').collect();

		let formatting_error: String = format!("Error: Incorrect Task Format\nWant: \"3-4-my-task.md\"\nGot: {}", filename);

		if split.len() < 3 || split[0].len() != 1 || split[1].len() != 1 {
			return Err(anyhow::anyhow!(formatting_error.clone()));
		}
	
		let importance: u8 = split[0].parse().with_context(|| formatting_error.clone())?;
		let difficulty: u8 = split[1].parse().with_context(|| formatting_error.clone())?;
		let _name: String = split[2..split.len()].concat().trim_end_matches(".md").to_string();	

		let priority = importance * (10-difficulty);

		Ok(Task{ priority: priority.into(), path })
	}
}

fn quick_sort(arr: &mut [Task], low: isize, high: isize) {
	if low < high {
		let p = partition(arr, low, high);
		quick_sort(arr, low, p - 1);
		quick_sort(arr, p + 1, high);
	}
}
fn partition(arr: &mut [Task], low: isize, high: isize) -> isize {
	let pivot = high as usize;
	let mut store_index = low - 1;
	let mut last_index = high;

	loop {
		store_index += 1;
		while arr[store_index as usize].priority < arr[pivot].priority {
			store_index += 1;
		}
		last_index -= 1;
		while last_index >= 0 && arr[last_index as usize].priority > arr[pivot].priority {
			last_index -= 1;
		}
		if store_index >= last_index {
			break;
		} else {
			arr.swap(store_index as usize, last_index as usize);
		}
	}
	arr.swap(store_index as usize, pivot as usize);
	store_index
}
