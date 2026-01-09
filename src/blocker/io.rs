//! File and project I/O for standalone blocker files.
//!
//! This module handles:
//! - File path resolution and project management
//! - Urgent file handling
//! - Background blocker check process

use std::{io::Write as IoWrite, path::Path};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};

use super::{
	clockify::{self, HaltArgs, ResumeArgs},
	operations::{BlockerSequence, DisplayFormat},
	standard::{format_blocker_content, is_semantically_empty, normalize_content_by_extension},
};
use crate::milestones::SPRINT_HEADER_REL_PATH;

fn blockers_dir() -> std::path::PathBuf {
	v_utils::xdg_data_dir!("blockers")
}

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";
static BLOCKER_CURRENT_CACHE_FILENAME: &str = "blocker_current_cache.txt";
static PRE_URGENT_PROJECT_FILENAME: &str = "pre_urgent_project.txt";

#[derive(Args, Clone, Debug)]
pub struct BlockerArgs {
	#[command(subcommand)]
	pub command: Command,
	/// The relative path of the blocker file. Will be appended to the state directory.
	/// If contains one slash, the folder name will be used as workspace filter.
	#[arg(short, long)]
	pub relative_path: Option<String>,
	/// Use issue files instead of standalone blocker files.
	/// Changes the data source for all commands.
	#[arg(short, long)]
	pub integrated: bool,
	/// Output format for list command
	#[arg(short, long, default_value = "nested")]
	pub format: DisplayFormat,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
	/// Append a blocker
	/// # NB
	/// adds one and only one blocker. The structure is **not** a tree for a reason:
	/// - it forces prioritization (high leverage)
	/// - solving top 1 thing can often unlock many smaller ones for free
	Add {
		name: String,
		/// Project path or pattern to override the default project
		#[arg(short = 'p', long)]
		project: Option<String>,
		/// Mark as urgent (equivalent to --project urgent, creates if doesn't exist)
		#[arg(short = 'u', long)]
		urgent: bool,
		/// Create the file if it doesn't exist (touch)
		#[arg(short = 't', long)]
		touch: bool,
	},
	/// Pop the last one
	Pop,
	/// Full list of blockers down from the main task
	List,
	/// Compactly show the last entry
	Current {
		/// Show fully-qualified path with project prepended
		#[arg(short = 'f', long)]
		fully_qualified: bool,
	},
	/// Just open the blocker file with $EDITOR. Text as interface.
	Open {
		/// Optional pattern to open a different file (standalone: relative path, integrated: issue pattern)
		pattern: Option<String>,
		/// Create the file if it doesn't exist (touch, standalone mode only)
		#[arg(short = 't', long)]
		touch: bool,
		/// Set the opened file as current project after exiting the editor
		#[arg(short = 's', long)]
		set_after: bool,
		/// Mark as urgent (standalone mode only: opens workspace-specific urgent.md)
		#[arg(short = 'u', long)]
		urgent: bool,
	},
	/// Set the current project/issue for blocker operations
	Set {
		/// Pattern to match (standalone: relative path, integrated: issue pattern)
		pattern: String,
		/// Create the file if it doesn't exist (touch, standalone mode only)
		#[arg(short = 't', long)]
		touch: bool,
	},
	/// Resume tracking time on the current blocker task via Clockify
	Resume(ResumeArgs),
	/// Pause tracking time via Clockify
	Halt(HaltArgs),
}

fn get_current_blocker_cache_path(relative_path: &str) -> std::path::PathBuf {
	let cache_key = relative_path.replace('/', "_");
	v_utils::xdg_cache_file!(format!("{}_{}", cache_key, BLOCKER_CURRENT_CACHE_FILENAME))
}

fn save_current_blocker_cache(relative_path: &str, current_blocker: Option<String>) -> Result<()> {
	let cache_path = get_current_blocker_cache_path(relative_path);
	match current_blocker {
		Some(blocker) => std::fs::write(&cache_path, blocker)?,
		None => {
			let _ = std::fs::remove_file(&cache_path);
		}
	}
	Ok(())
}

fn load_current_blocker_cache(relative_path: &str) -> Option<String> {
	let cache_path = get_current_blocker_cache_path(relative_path);
	std::fs::read_to_string(&cache_path).ok()
}

/// Load blocker sequence from a file
fn load_blocker_sequence(relative_path: &str) -> BlockerSequence {
	let blocker_path = blockers_dir().join(relative_path);
	let content = std::fs::read_to_string(&blocker_path).unwrap_or_default();
	BlockerSequence::parse(&content)
}

/// Save blocker sequence to a file
fn save_blocker_sequence(relative_path: &str, seq: &BlockerSequence) -> Result<()> {
	let blocker_path = blockers_dir().join(relative_path);
	std::fs::write(&blocker_path, seq.serialize())?;
	Ok(())
}

/// File-based blocker source for standalone blocker files.
pub struct FileSource {
	relative_path: String,
}

impl FileSource {
	pub fn new(relative_path: String) -> Self {
		Self { relative_path }
	}

	pub fn relative_path(&self) -> &str {
		&self.relative_path
	}
}

impl super::source::BlockerSource for FileSource {
	fn load(&self) -> Result<BlockerSequence> {
		let blocker_path = blockers_dir().join(&self.relative_path);
		let content = std::fs::read_to_string(&blocker_path).unwrap_or_default();
		Ok(BlockerSequence::parse(&content))
	}

	fn save(&self, blockers: &BlockerSequence) -> Result<()> {
		let blocker_path = blockers_dir().join(&self.relative_path);
		std::fs::write(&blocker_path, blockers.serialize())?;
		Ok(())
	}

	fn display_name(&self) -> String {
		self.relative_path.clone()
	}

	fn path_for_hierarchy(&self) -> Option<std::path::PathBuf> {
		Some(std::path::PathBuf::from(&self.relative_path))
	}
}

/// Build the ownership hierarchy for a blocker file.
/// If fully_qualified is true, includes the project name extracted from relative_path.
fn build_ownership_hierarchy(relative_path: &str, fully_qualified: bool) -> Vec<String> {
	if fully_qualified {
		if let Some(project_name) = std::path::Path::new(relative_path).file_stem().and_then(|s| s.to_str()) {
			return vec![project_name.to_string()];
		}
	}
	Vec::new()
}

/// Get the current blocker with parent headers prepended (joined by ": ")
/// If fully_qualified is true, prepend the project name from the relative_path
fn get_current_blocker_with_headers(relative_path: &str, fully_qualified: bool) -> Option<String> {
	let seq = load_blocker_sequence(relative_path);
	let hierarchy = build_ownership_hierarchy(relative_path, fully_qualified);
	seq.current_with_context(&hierarchy)
}

fn parse_workspace_from_path(relative_path: &str) -> Result<Option<String>> {
	let slash_count = relative_path.matches('/').count();

	if slash_count == 0 {
		Ok(None)
	} else if slash_count == 1 {
		let parts: Vec<&str> = relative_path.split('/').collect();
		Ok(Some(parts[0].to_string()))
	} else {
		bail!("Relative path can contain at most one slash, found {}: {}", slash_count, relative_path);
	}
}

/// Helper to restart tracking for the current blocker in a project
async fn restart_tracking_for_project(relative_path: &str, workspace: Option<&str>) -> Result<()> {
	clockify::restart_tracking_for_project(|fully_qualified| get_current_blocker_with_headers(relative_path, fully_qualified), workspace).await
}

fn spawn_blocker_comparison_process(relative_path: String) -> Result<()> {
	use std::process::Command;

	let current_exe = std::env::current_exe()?;

	Command::new(current_exe)
		.args(["blocker", "--relative-path", &relative_path, "current"])
		.env("_BLOCKER_BACKGROUND_CHECK", "1")
		.spawn()?;

	Ok(())
}

async fn set_current_project(resolved_path: &str) -> Result<()> {
	// Validate the resolved path before saving
	parse_workspace_from_path(resolved_path)?;

	// Get the old project path before updating
	let current_project_file = v_utils::xdg_cache_file!(CURRENT_PROJECT_CACHE_FILENAME);
	let old_project = std::fs::read_to_string(&current_project_file).ok();

	// Check if the project actually changed
	let project_changed = old_project.as_ref().is_none_or(|old| old != resolved_path);

	// If currently on urgent, don't allow switching away (unless switching to another urgent file or urgent file no longer exists)
	if let Some(old_path) = &old_project
		&& is_urgent_file(old_path)
		&& !is_urgent_file(resolved_path)
	{
		let urgent_file_path = blockers_dir().join(old_path);
		if urgent_file_path.exists() {
			eprintln!("Cannot switch away from urgent project '{old_path}'. Complete urgent tasks first.");
			return Ok(());
		}
	}

	// If switching to urgent, save the previous project
	if is_urgent_file(resolved_path)
		&& let Some(old_path) = &old_project
		&& !is_urgent_file(old_path)
	{
		// Save non-urgent project before switching to urgent
		let pre_urgent_path = v_utils::xdg_state_file!(PRE_URGENT_PROJECT_FILENAME);
		std::fs::write(&pre_urgent_path, old_path)?;
	}

	// Save the new project path
	std::fs::write(&current_project_file, resolved_path)?;

	println!("Set current project to: {resolved_path}");

	// If project changed and tracking is enabled, handle the transition
	if project_changed && clockify::is_tracking_enabled() {
		// Stop tracking on the old project
		if let Some(old_path) = &old_project {
			let old_workspace = parse_workspace_from_path(old_path).ok().flatten();
			let _ = clockify::stop_current_tracking(old_workspace.as_deref()).await;
		}

		// Start tracking on the new project if it has a current blocker
		let new_workspace = parse_workspace_from_path(resolved_path)?.as_ref().map(|s| s.to_string());
		restart_tracking_for_project(resolved_path, new_workspace.as_deref()).await?;
	}

	// Spawn background process to check for clockify updates after project change
	spawn_blocker_comparison_process(resolved_path.to_string())?;

	Ok(())
}

async fn handle_background_blocker_check(relative_path: &str) -> Result<()> {
	// Read and format the blocker file
	let blocker_path = blockers_dir().join(relative_path);
	if blocker_path.exists() {
		let content = std::fs::read_to_string(&blocker_path)?;
		// Normalize content based on file extension (e.g., convert .typ to markdown)
		let normalized = normalize_content_by_extension(&content, &blocker_path)?;
		let formatted = format_blocker_content(&normalized)?;

		// Check if this is a Typst file that needs to be converted to markdown
		let extension = blocker_path.extension().and_then(|e| e.to_str());
		let write_path = if extension == Some("typ") {
			// Convert .typ to .md
			blocker_path.with_extension("md")
		} else {
			blocker_path.clone()
		};

		// Only write back if content changed
		if content != formatted {
			std::fs::write(&write_path, formatted)?;
			// If we converted from .typ to .md, remove the old .typ file
			if extension == Some("typ") {
				std::fs::remove_file(&blocker_path)?;
			}
		}
	}

	// Get the default project for tracking (not the file that was just opened/formatted)
	let default_project_path = {
		let persisted_project_file = v_utils::xdg_cache_file!(CURRENT_PROJECT_CACHE_FILENAME);
		std::fs::read_to_string(&persisted_project_file).unwrap_or_else(|_| "blockers.txt".to_string())
	};

	let cached_current = load_current_blocker_cache(&default_project_path);
	let seq = load_blocker_sequence(&default_project_path);
	let actual_current = seq.current_raw();

	if cached_current != actual_current {
		if clockify::is_tracking_enabled() {
			let workspace_from_path = parse_workspace_from_path(&default_project_path)?;

			let _ = clockify::stop_current_tracking(workspace_from_path.as_deref()).await;

			restart_tracking_for_project(&default_project_path, workspace_from_path.as_deref()).await?;
		}

		save_current_blocker_cache(&default_project_path, actual_current)?;
	}

	// After formatting, cleanup urgent file if it's empty
	cleanup_urgent_file_if_empty(relative_path).await?;

	// After formatting, check for urgent files and auto-switch if found
	// But only if the current project is not already an urgent file
	let current_project_path = v_utils::xdg_cache_file!(CURRENT_PROJECT_CACHE_FILENAME);
	let current_project = std::fs::read_to_string(&current_project_path).unwrap_or_else(|_| "blockers.txt".to_string());

	if !is_urgent_file(&current_project)
		&& let Some(urgent_path) = check_for_urgent_file()
	{
		eprintln!("Detected urgent file, switching to: {urgent_path}");
		set_current_project(&urgent_path).await?;
	}

	Ok(())
}

/// Check if a command has the urgent flag set
fn command_has_urgent_flag(command: &Command) -> bool {
	match command {
		Command::Add { urgent, .. } => *urgent,
		Command::Open { urgent, .. } => *urgent,
		_ => false,
	}
}

pub async fn main(_settings: &crate::config::LiveSettings, args: BlockerArgs) -> Result<()> {
	// If integrated mode, delegate to the integration module
	// EXCEPT for urgent operations - urgent always uses file-based source (no issue equivalent)
	if args.integrated && !command_has_urgent_flag(&args.command) {
		return super::integration::main_integrated(args.command, args.format).await;
	}

	let relative_path = match args.relative_path {
		Some(f) => f,
		None => {
			let persisted_project_file = v_utils::xdg_cache_file!(CURRENT_PROJECT_CACHE_FILENAME);
			match std::fs::read_to_string(&persisted_project_file) {
				Ok(s) => s,
				Err(_) => "blockers.txt".to_string(),
			}
		}
	};

	// Handle background blocker check
	if std::env::var("_BLOCKER_BACKGROUND_CHECK").is_ok() {
		return handle_background_blocker_check(&relative_path).await;
	}

	// Parse workspace from path if it contains a slash
	let workspace_from_path = parse_workspace_from_path(&relative_path)?;

	let blocker_path = blockers_dir().join(&relative_path);

	match args.command {
		Command::Add { name, project, urgent, touch } => {
			// Resolve the actual relative_path to use
			let target_relative_path = if urgent {
				// --urgent flag takes precedence: use workspace-specific "urgent.md"
				// Requires a workspace context
				let urgent_path = if let Some(workspace) = workspace_from_path.as_ref() {
					format!("{workspace}/urgent.md")
				} else {
					return Err(eyre!(
						"Cannot use --urgent without a workspace. Set a workspace project first (e.g., 'blocker set-project work/blockers.md')"
					));
				};
				// Check if we can create this urgent file
				check_urgent_creation_allowed(&urgent_path)?;
				urgent_path
			} else if let Some(project_pattern) = project {
				// --project flag provided
				resolve_project_path(&project_pattern, touch)?
			} else {
				// Use default project
				relative_path.clone()
			};

			// Re-parse workspace from the target path
			let target_workspace_from_path = parse_workspace_from_path(&target_relative_path)?;
			let target_blocker_path = blockers_dir().join(&target_relative_path);

			// If tracking is enabled, stop current task before adding new one
			if clockify::is_tracking_enabled() {
				let _ = clockify::stop_current_tracking(target_workspace_from_path.as_deref()).await; // Ignore errors when stopping
			}

			// Create parent directories if they don't exist (for urgent or other paths)
			if let Some(parent) = target_blocker_path.parent() {
				std::fs::create_dir_all(parent)?;
			}

			// Create the file if it doesn't exist and touch flag is set
			if touch && !target_blocker_path.exists() {
				std::fs::write(&target_blocker_path, "")?;
			}

			// Read existing content, add new line, format and write
			let mut seq = load_blocker_sequence(&target_relative_path);
			seq.add(&name);
			save_blocker_sequence(&target_relative_path, &seq)?;

			// Save current blocker to cache
			save_current_blocker_cache(&target_relative_path, Some(name.clone()))?;

			// Cleanup urgent file if it's now empty
			cleanup_urgent_file_if_empty(&target_relative_path).await?;

			// If adding to a different project (e.g., urgent), switch the current project
			if target_relative_path != relative_path {
				set_current_project(&target_relative_path).await?;
			} else if clockify::is_tracking_enabled() {
				// Only restart tracking here if we didn't switch projects
				// (set_current_project already handles tracking restart)
				restart_tracking_for_project(&target_relative_path, target_workspace_from_path.as_deref()).await?;
			}
		}
		Command::Pop => {
			// If tracking is enabled, stop current task before popping
			if clockify::is_tracking_enabled() {
				let _ = clockify::stop_current_tracking(workspace_from_path.as_deref()).await; // Ignore errors when stopping
			}

			// Read existing content, pop last content line, format and write
			let mut seq = load_blocker_sequence(&relative_path);
			seq.pop();
			save_blocker_sequence(&relative_path, &seq)?;

			// Get the new current blocker after popping
			let new_current = seq.current_raw();
			save_current_blocker_cache(&relative_path, new_current)?;

			// Cleanup urgent file if it's now empty
			cleanup_urgent_file_if_empty(&relative_path).await?;

			// If tracking is enabled and there's still a task, start tracking it
			if clockify::is_tracking_enabled() {
				restart_tracking_for_project(&relative_path, workspace_from_path.as_deref()).await?;
			}
		}
		Command::List => {
			let sprint_header = std::fs::read_to_string(v_utils::xdg_data_file!(SPRINT_HEADER_REL_PATH)).ok();
			if let Some(s) = sprint_header {
				println!("{s}");
			}
			let seq = load_blocker_sequence(&relative_path);
			let output = seq.render(args.format);
			println!("{output}");
		}
		Command::Current { fully_qualified } =>
			if let Some(output) = get_current_blocker_with_headers(&relative_path, fully_qualified) {
				const MAX_LEN: usize = 70;
				match output.len() {
					0..=MAX_LEN => println!("{output}"),
					_ => println!("{}...", &output[..(MAX_LEN - 3)]),
				}
			},
		Command::Open { pattern, touch, set_after, urgent } => {
			// Save current blocker state to cache before opening
			let seq = load_blocker_sequence(&relative_path);
			save_current_blocker_cache(&relative_path, seq.current_raw())?;

			// Determine which file to open
			let resolved_path = if urgent {
				// --urgent flag takes precedence: use workspace-specific "urgent.md"
				// Requires a workspace context
				let urgent_path = if let Some(workspace) = workspace_from_path.as_ref() {
					format!("{workspace}/urgent.md")
				} else {
					return Err(eyre!(
						"Cannot use --urgent without a workspace. Set a workspace project first (e.g., 'blocker set work/blockers.md')"
					));
				};
				// Check if we can create this urgent file (only if touch is enabled)
				if touch {
					check_urgent_creation_allowed(&urgent_path)?;
				}
				urgent_path
			} else {
				match pattern {
					Some(custom_path) => resolve_project_path(&custom_path, touch)?,
					None => relative_path.clone(),
				}
			};

			let path_to_open = blockers_dir().join(&resolved_path);

			// Create the file if it doesn't exist and touch flag is set
			if touch && !path_to_open.exists() {
				// Create parent directories if they don't exist
				if let Some(parent) = path_to_open.parent() {
					std::fs::create_dir_all(parent)?;
				}
				// Create an empty file
				std::fs::write(&path_to_open, "")?;
			}

			// Open the file
			v_utils::io::file_open::open(&path_to_open).await?;

			// If set_after flag is set, update the current project
			if set_after {
				set_current_project(&resolved_path).await?;
			} else {
				// Spawn background process to check for changes after editor closes
				spawn_blocker_comparison_process(resolved_path.clone())?;
			}
		}
		Command::Set { pattern, touch } => {
			// Resolve the project path using pattern matching
			let resolved_path = resolve_project_path(&pattern, touch)?;

			// Create the file if it doesn't exist and touch flag is set
			if touch {
				let project_blocker_path = blockers_dir().join(&resolved_path);
				if !project_blocker_path.exists() {
					// Create parent directories if they don't exist
					if let Some(parent) = project_blocker_path.parent() {
						std::fs::create_dir_all(parent)?;
					}
					std::fs::write(&project_blocker_path, "")?;
				}
			}

			set_current_project(&resolved_path).await?;
		}
		Command::Resume(resume_args) => {
			// Check that there is a current blocker
			let seq = load_blocker_sequence(&relative_path);
			if seq.current().is_none() {
				return Err(eyre!("No current blocker task found. Add one with 'todo blocker add <task>'"));
			}

			// Enable tracking state
			clockify::set_tracking_enabled(true)?;

			// Start tracking with current blocker description
			let relative_path_clone = relative_path.clone();
			clockify::start_tracking_for_task(
				|fully_qualified| get_current_blocker_with_headers(&relative_path_clone, fully_qualified).unwrap_or_default(),
				&resume_args,
				workspace_from_path.as_deref(),
			)
			.await?;
		}
		Command::Halt(halt_args) => {
			// Disable tracking state
			clockify::set_tracking_enabled(false)?;

			let workspace = workspace_from_path.as_deref().or(halt_args.workspace.as_deref());
			clockify::stop_current_tracking(workspace).await?;
		}
	};
	Ok(())
}

/// Search for projects using a grep-like pattern
fn search_projects_by_pattern(pattern: &str) -> Result<Vec<String>> {
	let blockers_dir = blockers_dir();
	// Search for both .md and .typ files
	let all_files = crate::utils::fd(&["-t", "f", "-e", "md", "-e", "typ"], &blockers_dir)?;
	let mut matches = Vec::new();

	for line in all_files.lines() {
		let relative_path = line.trim();
		if relative_path.is_empty() {
			continue;
		}

		// Extract filename without extension for matching
		if let Some(filename) = Path::new(relative_path).file_stem()
			&& let Some(filename_str) = filename.to_str()
		{
			let pattern_lower = pattern.to_lowercase();
			let filename_lower = filename_str.to_lowercase();
			let path_lower = relative_path.to_lowercase();

			// Check if pattern matches filename OR appears anywhere in the path (case-insensitive)
			if filename_lower.contains(&pattern_lower) || path_lower.contains(&pattern_lower) {
				matches.push(relative_path.to_string());
			}
		}
	}

	Ok(matches)
}

/// Use fzf to let user choose from multiple project matches
fn choose_project_with_fzf(matches: &[String], initial_query: &str) -> Result<Option<String>> {
	use std::process::{Command, Stdio};

	// Prepare input for fzf
	let input = matches.join("\n");

	let mut fzf = Command::new("fzf").args(["--query", initial_query]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;

	if let Some(stdin) = fzf.stdin.take() {
		let mut stdin_handle = stdin;
		IoWrite::write_all(&mut stdin_handle, input.as_bytes())?;
	}

	let output = fzf.wait_with_output()?;

	if output.status.success() {
		let chosen = String::from_utf8(output.stdout)?.trim().to_string();
		Ok(Some(chosen))
	} else {
		Ok(None)
	}
}

/// Resolve project path using pattern matching - works for both project and open commands
/// If `touch` is true and no matches are found, returns a new path based on the pattern
fn resolve_project_path(pattern: &str, touch: bool) -> Result<String> {
	// If it contains a slash, treat as literal path (e.g., "workspace/project.md")
	if pattern.contains('/') {
		return Ok(pattern.to_string());
	}

	// Strip .md/.typ suffix for pattern matching, so "uni.md" matches like "uni"
	let search_pattern = pattern.strip_suffix(".md").or_else(|| pattern.strip_suffix(".typ")).unwrap_or(pattern);

	let matches = search_projects_by_pattern(search_pattern)?;

	// If pattern has an extension, check for exact match first
	// e.g., "uni.md" should match "uni.md" exactly, not open fzf even if "uni_headless.md" also exists
	if (pattern.ends_with(".md") || pattern.ends_with(".typ"))
		&& let Some(exact_match) = matches.iter().find(|m| {
			// Extract just the filename from the match path
			Path::new(m).file_name().and_then(|f| f.to_str()) == Some(pattern)
		}) {
		eprintln!("Found exact match: {exact_match}");
		return Ok(exact_match.clone());
	}

	match matches.len() {
		0 => {
			if touch {
				// No matches but touch is enabled - create a new path from the pattern
				let new_path = if pattern.ends_with(".md") || pattern.ends_with(".typ") {
					pattern.to_string()
				} else {
					format!("{pattern}.md")
				};
				eprintln!("Creating new project file: {new_path}");
				Ok(new_path)
			} else {
				Err(eyre!("No projects found matching pattern: {pattern}"))
			}
		}
		1 => {
			eprintln!("Found unique match: {}", matches[0]);
			Ok(matches[0].clone())
		}
		_ => {
			eprintln!("Found {} matches for '{pattern}'. Opening fzf to choose:", matches.len());
			match choose_project_with_fzf(&matches, pattern)? {
				Some(chosen) => Ok(chosen),
				None => Err(eyre!("No project selected")),
			}
		}
	}
}

/// Check if an urgent file (urgent.md or urgent.typ) exists in any workspace
/// Returns the relative path to the urgent file if found
/// Only checks workspace-specific urgent files (e.g., workspace/urgent.md)
fn check_for_urgent_file() -> Option<String> {
	let blockers_dir = blockers_dir();

	// Check for workspace-specific urgent files
	// Look for */urgent.md and */urgent.typ in blockers_dir
	if let Ok(entries) = std::fs::read_dir(&blockers_dir) {
		for entry in entries.flatten() {
			if let Ok(metadata) = entry.metadata()
				&& metadata.is_dir()
			{
				let workspace_name = entry.file_name();

				// Check workspace/urgent.md
				let ws_urgent_md = entry.path().join("urgent.md");
				if ws_urgent_md.exists() {
					return Some(format!("{}/urgent.md", workspace_name.to_string_lossy()));
				}

				// Check workspace/urgent.typ
				let ws_urgent_typ = entry.path().join("urgent.typ");
				if ws_urgent_typ.exists() {
					return Some(format!("{}/urgent.typ", workspace_name.to_string_lossy()));
				}
			}
		}
	}

	None
}

/// Check if a relative path refers to an urgent file (urgent.md or urgent.typ)
/// Only workspace-specific urgent files are valid (e.g., workspace/urgent.md)
fn is_urgent_file(relative_path: &str) -> bool {
	relative_path.ends_with("/urgent.md") || relative_path.ends_with("/urgent.typ")
}

/// Check if creating a new urgent file is allowed
/// Returns Err if another urgent file already exists (unless it's the same path)
fn check_urgent_creation_allowed(target_urgent_path: &str) -> Result<()> {
	if let Some(existing_urgent) = check_for_urgent_file()
		&& existing_urgent != target_urgent_path
	{
		return Err(eyre!(
			"Cannot create urgent file '{}': another urgent file '{}' already exists. Complete it first.",
			target_urgent_path,
			existing_urgent
		));
	}
	Ok(())
}

/// Delete urgent file if it's semantically empty, and switch away from it if it was the current project
async fn cleanup_urgent_file_if_empty(relative_path: &str) -> Result<()> {
	if !is_urgent_file(relative_path) {
		return Ok(());
	}

	let blocker_path = blockers_dir().join(relative_path);
	if !blocker_path.exists() {
		return Ok(());
	}

	let content = std::fs::read_to_string(&blocker_path)?;
	let normalized = normalize_content_by_extension(&content, &blocker_path)?;

	if is_semantically_empty(&normalized) {
		// Delete the urgent file
		std::fs::remove_file(&blocker_path)?;
		eprintln!("Removed empty urgent file: {relative_path}");

		// Check if this was the current project
		let current_project_path = v_utils::xdg_cache_file!(CURRENT_PROJECT_CACHE_FILENAME);
		if let Ok(current_project) = std::fs::read_to_string(&current_project_path)
			&& current_project == relative_path
		{
			// Try to restore the previous project before urgent
			let pre_urgent_path = v_utils::xdg_state_file!(PRE_URGENT_PROJECT_FILENAME);
			let restore_project = if let Ok(prev_project) = std::fs::read_to_string(&pre_urgent_path) {
				// Clean up the pre-urgent cache file
				let _ = std::fs::remove_file(&pre_urgent_path);
				prev_project
			} else {
				// Fallback to a sensible default if no previous project was saved
				// Try to find any non-urgent project file, or use empty string to trigger error handling
				String::new()
			};

			if !restore_project.is_empty() {
				set_current_project(&restore_project).await?;
			} else {
				eprintln!("No previous project found to restore after urgent completion");
			}
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_is_urgent_file() {
		// Workspace-specific urgent files (only valid form)
		assert!(is_urgent_file("workspace/urgent.md"));
		assert!(is_urgent_file("workspace/urgent.typ"));
		assert!(is_urgent_file("workspace1/urgent.md"));

		// Root-level urgent files are NOT valid (must be in workspace)
		assert!(!is_urgent_file("urgent.md"));
		assert!(!is_urgent_file("urgent.typ"));

		// Non-urgent files
		assert!(!is_urgent_file("blockers.md"));
		assert!(!is_urgent_file("urgent"));
		assert!(!is_urgent_file("workspace/blockers.md"));
	}
}
