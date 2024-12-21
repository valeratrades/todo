// Adaptation ideas:
//- leave quickfix, have it suggest tasks to be included into the next sprint from the thematic bucket. Or/and have it suggest tasks for the next day, given what's left in the ongoing sprint.
//- flags: allow any, if doesn't exist, create new bucket for it if `-c` is provided.
//- TaskSplit: rewrite to use {importance, difficulty per unit of time, estimated time}

use std::{
	fmt::{self, Display},
	path::PathBuf,
};

use clap::Args;
use color_eyre::eyre::{eyre, Context as _, Report, Result};
use tempfile::Builder;
use v_utils::io::OpenMode;

use crate::{
	config::AppConfig,
	day_section::DaySection,
	manual_stats::{Day, Repercussions},
	utils,
};

#[deprecated(note = "In the process of rewriting to work with 2w-splits instead")]
pub fn compile_quickfix(config: AppConfig) -> Result<()> {
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

	let len = tasks.len() as isize;
	quick_sort_tasks(&mut tasks, 0, len - 1);

	let n = config.todos.n_tasks_to_show;
	let _cut_index = if tasks.len() > n { tasks.len() - n } else { 0 };
	let to_show_tasks: Vec<_> = tasks[_cut_index..].iter().rev().cloned().collect();

	let mut quickfix_str = String::new();
	let len = to_show_tasks.len();
	for (i, task) in to_show_tasks.iter().enumerate().take(len) {
		quickfix_str.push_str(&format!("{}", task));
		if i < len - 1 {
			quickfix_str.push_str("# -----------------------------------------------------------------------------\n");
		}
	}

	let repercussions: Repercussions;
	{
		let date = utils::format_date(0, &config);
		let day = Day::load(&date, &config).ok();
		repercussions = Repercussions::from_day(day);
	}

	quickfix_str.push_str(&format!(
		r#"
# =============================================================================
{}

# -----------------------------------------------------------------------------

# General

Clear separation between tasks below 5m and above. Those below can be done whenever, those not - only in assigned time intervals.

# Repercussions

With current state of the day, the following repercussions will be applied:
{}
"#,
		day_section.description(),
		repercussions
	));

	let tmp_file = Builder::new().suffix(".pdf").tempfile()?;
	let tmp_path = tmp_file.path().to_path_buf();
	let mut p = pandoc::new();
	p.set_input(pandoc::InputKind::Pipe(quickfix_str));
	p.set_output(pandoc::OutputKind::File(tmp_path.clone()));
	p.execute()?;

	let _status = std::process::Command::new("sh").arg("-c").arg(format!("zathura {}", tmp_path.display())).status()?;

	Ok(())
}

pub fn open_or_add(config: AppConfig, flags: TodosFlags, name: Option<String>) -> Result<()> {
	let day_section = flags.extract_day_section();

	let mut path = day_section_path(&config, &day_section);

	if let Some(name) = name {
		path.push([&name, ".md"].concat());
		let _ = std::fs::File::create(&path).unwrap();
	}

	let mode = if flags.open { Some(OpenMode::Normal) } else { None };
	v_utils::io::sync_file_with_git(&path, mode)?;

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

fn day_section_path<'a>(config: &'a AppConfig, day_section: &'a DaySection) -> PathBuf {
	let todos_dir = config.todos.path.clone();

	let path_appendix: &str = match day_section.to_owned() {
		DaySection::Morning => ".morning/",
		DaySection::Work => ".work/",
		DaySection::Evening => ".evening/",
		DaySection::Night => ".night/",
	};

	todos_dir.join(path_appendix)
}

#[derive(Debug, Clone)]
struct TaskSplit {
	_importance: u8,
	_difficulty: u8,
	name: String,
}
#[derive(Debug, Clone)]
struct Task {
	priority: f32,
	path: PathBuf,
	split: TaskSplit,
}
impl TryFrom<PathBuf> for Task {
	type Error = Report;

	fn try_from(path: PathBuf) -> Result<Self> {
		let filename = path
			.file_name()
			.ok_or_else(|| eyre!("Filename not found in path"))?
			.to_str()
			.ok_or_else(|| eyre!("Filename is not valid UTF-8"))?;
		let split: Vec<_> = filename.split('-').collect();

		let formatting_error: String = format!("Error: Incorrect Task Format\nWant: \"3-4-my-task.md\"\nGot: {}", filename);

		if split.len() < 3 || split[0].len() != 1 || split[1].len() != 1 {
			return Err(eyre!(formatting_error.clone()));
		}

		let importance: u8 = split[0].parse().with_context(|| formatting_error.clone())?;
		let difficulty: u8 = split[1].parse().with_context(|| formatting_error.clone())?;
		let name: String = split[2..split.len()].join(" ").trim_end_matches(".md").to_string();

		let split = TaskSplit {
			_importance: importance,
			_difficulty: difficulty,
			name,
		};

		let priority = importance * (10 - difficulty);

		Ok(Task {
			priority: priority.into(),
			path,
			split,
		})
	}
}
fn quick_sort_tasks(arr: &mut [Task], low: isize, high: isize) {
	if low < high {
		let p = partition_tasks(arr, low, high);
		quick_sort_tasks(arr, low, p - 1);
		quick_sort_tasks(arr, p + 1, high);
	}
}
fn partition_tasks(arr: &mut [Task], low: isize, high: isize) -> isize {
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
	arr.swap(store_index as usize, pivot);
	store_index
}

impl Display for Task {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let inner_contents = std::fs::read_to_string(&self.path).unwrap().replace("\n#", "\n##"); // add extra header so when compiled contents are always subsections.

		write!(
			f,
			r#"# {}

{}

{}

"#,
			self.split.name,
			self.path.display(),
			inner_contents
		)
	}
}
