// Adaptation ideas:
//- leave quickfix, have it suggest tasks to be included into the next sprint from the thematic bucket. Or/and have it suggest tasks for the next day, given what's left in the ongoing sprint.
//- flags: allow any, if doesn't exist, create new bucket for it if `-c` is provided.
//- TaskSplit: rewrite to use {importance, difficulty per unit of time, estimated time}

use std::{
	fmt::{self, Display},
	path::PathBuf,
};

use clap::Args;
use color_eyre::eyre::{bail, eyre, Context as _, Report, Result};
use tempfile::Builder;
use v_utils::io::OpenMode;

use crate::{
	config::AppConfig,
	manual_stats::{Day, Repercussions},
	utils,
};

#[deprecated(note = "In the process of rewriting to work with 2w-splits instead")]
pub fn compile_quickfix(config: AppConfig) -> Result<()> {
	let path: PathBuf = section_path(&config, todo!())?;

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
		//day_section.description(),
		todo!(),
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
	let mut path = section_path(&config, &flags)?;

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
	/// Add tasks to named section of `sprints` (Mutually exclusive with `conditional`). Default is `other`.
	#[arg(long, short)]
	sprints: Option<String>,
	/// Add tasks to named section of `conditional` (Mutually exclusive with `sprints`). Default is `other`.
	//TODO!!!!: shouldn't require `<importance>-<difficulty>` prefix, on the contrary - here should error if it's provided.
	#[arg(long, short)]
	conditional: bool,
	/// Create the parenting section if it doesn't exist.
	#[arg(long, short)]
	parents: bool,
	/// Open the file after creating it.
	#[arg(long, short)]
	pub open: bool,
}

fn section_path<'a>(config: &AppConfig, flags: &TodosFlags) -> Result<PathBuf> {
	let todos_dir = config.todos.path.clone();

	let from_root = match (flags.sprints.as_ref(), flags.conditional) {
		(Some(name), false) => name,
		(None, true) => "conditional",
		(None, false) => "other",
		(Some(_), true) => bail!("Flags `sprints` and `conditional` are mutually exclusive."),
	};

	match std::fs::metadata(todos_dir.join(from_root)).is_ok() {
		true => Ok(todos_dir.join(from_root)),
		false => match flags.parents {
			true => {
				let path = todos_dir.join(from_root);
				std::fs::create_dir_all(&path)?;
				Ok(path)
			}
			false => bail!("Section '{from_root}' does not exist. Add `-p` flag to automatically create it."),
		},
	}
}

#[derive(Debug, Clone)]
enum TaskMeta {
	Simple {
		name: String,
	},
	Full {
		importance: u8, // wanted to go with 16 hexadecimal digits, but that would prevent having things sorted for fre.
		est_hours: f32,
		difficulty_density: u8,
		name: String, //? Do I need this?
	},
}
impl TaskMeta {
	pub fn try_parse_full(name: &str) -> Result<Self> {
		let split: Vec<_> = name.split('-').collect();

		let formatting_error: String = format!("Error: Incorrect Task Format\nWant: \"3-4-my-task.md\"\nGot: {}", name);

		// [1] can be longer than 1 char; total number of dashes can be more than 4, as we don't control actual contents, - don't check those.
		if split.len() < 4 || split[0].len() != 1 || split[2].len() != 1 {
			bail!(formatting_error.clone());
		}

		let importance: u8 = split[0].parse().with_context(|| formatting_error.clone())?;
		let est_hours: f32 = split[1].parse().with_context(|| formatting_error.clone())?;
		let difficulty_density: u8 = split[2].parse().with_context(|| formatting_error.clone())?;
		let name: String = split[2..split.len()].join(" ").trim_end_matches(".md").to_string();
		Ok(TaskMeta::Full {
			importance,
			est_hours,
			difficulty_density,
			name,
		})
	}

	//TODO!: start erroring on provision of `full` metadata here, to prevent habitual user logical errors. (? do I care though?)
	pub fn parse_simple(name: &str) -> Self {
		let name = name.trim_end_matches(".md").to_string();
		TaskMeta::Simple { name }
	}
}
impl TaskMeta {
	//HACK: old way of calcing this, hardly useful as of now.
	fn priority(&self) -> u8 {
		match self {
			TaskMeta::Simple { .. } => 0,
			TaskMeta::Full { importance, difficulty_density, .. } => importance * (10 - difficulty_density),
		}
	}

	fn name(&self) -> &str {
		match self {
			TaskMeta::Simple { name } => name,
			TaskMeta::Full { name, .. } => name,
		}
	}
}

#[derive(Debug, Clone)]
struct Task {
	meta: TaskMeta,
	path: PathBuf,
}
impl Task {
	fn priority(&self) -> u8 {
		self.meta.priority()
	}
}
impl TryFrom<PathBuf> for Task {
	type Error = Report;

	fn try_from(path: PathBuf) -> Result<Self> {
		let filename = path
			.file_name()
			.ok_or_else(|| eyre!("Filename not found in path"))?
			.to_str()
			.ok_or_else(|| eyre!("Filename is not valid UTF-8"))?;

		let meta: TaskMeta = {
			let p = path.parent().unwrap();
			match p.to_str() {
				Some("other") => TaskMeta::parse_simple(filename),
				_ => match p.parent().unwrap().to_str() {
					Some("sprints") => TaskMeta::try_parse_full(filename)?,
					Some("conditional") => TaskMeta::parse_simple(filename),
					_ => bail!("Error: Task in unknown section."),
				},
			}
		};
		Ok(Task { path, meta })
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
		while arr[store_index as usize].priority() < arr[pivot].priority() {
			store_index += 1;
		}
		last_index -= 1;
		while last_index >= 0 && arr[last_index as usize].priority() > arr[pivot].priority() {
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
			self.meta.name(),
			self.path.display(),
			inner_contents
		)
	}
}
