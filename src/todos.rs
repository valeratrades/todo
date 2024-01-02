use crate::config::Config;
use crate::day_section::DaySection;
use crate::utils;

use clap::Args;
use std::path::PathBuf;

pub fn compile(config: Config) {
	let day_section = DaySection::build().unwrap();
	let path: PathBuf = day_section_path(&config, &day_section);

	// apply formula to get the priority task according to time of day.

	// concat with description of the section

	// compile String to md with pandoc or something and pipe into zathura
}

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

pub fn open_or_add(config: Config, flags: TodosFlags, name: Option<String>) {
	let day_section = flags.extract_day_section();

	let mut path = day_section_path(&config, &day_section);

	if let Some(name) = name {
		path.push([&name, ".md"].concat());
		let _ = std::fs::File::create(&path).unwrap();
	}

	if flags.open == true {
		utils::open(path);
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
	importance: u8,
	difficulty: u8,
	name: String,
}
