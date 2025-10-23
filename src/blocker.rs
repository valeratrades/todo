use std::{collections::HashMap, io::Write, path::Path};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::{
	clockify,
	config::{AppConfig, CACHE_DIR, DATA_DIR, STATE_DIR},
	milestones::SPRINT_HEADER_REL_PATH,
};

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";
static BLOCKER_STATE_FILENAME: &str = "blocker_state.txt";
static WORKSPACE_SETTINGS_FILENAME: &str = "workspace_settings.json";
static BLOCKER_CURRENT_CACHE_FILENAME: &str = "blocker_current_cache.txt";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct WorkspaceSettings {
	legacy: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct WorkspaceCache {
	workspaces: HashMap<String, WorkspaceSettings>,
}

#[derive(Args, Clone, Debug)]
pub struct BlockerArgs {
	#[command(subcommand)]
	command: Command,
	#[arg(short, long)]
	/// The relative path of the blocker file. Will be appended to the state directory. If contains one slash, the folder name will be used as workspace filter. Can have any text-based format
	relative_path: Option<String>,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
	/// Append a blocker
	/// # NB
	/// adds one and only one blocker. The structure is **not** a tree for a reason:
	/// - it forces prioritization (high leverage)
	/// - solving top 1 thing can often unlock many smaller ones for free
	Add { name: String },
	/// Pop the last one
	Pop,
	/// Full list of blockers down from the main task
	List,
	/// Compactly show the last entry
	Current,
	/// Just open the \`blockers\` file with $EDITOR. Text as interface.
	Open {
		/// Optional file path relative to state directory to open instead of the default blocker file
		file_path: Option<String>,
		/// Create the file if it doesn't exist (touch)
		#[arg(short = 't', long)]
		touch: bool,
	},
	/// Set the default `--relative_path`, for the project you're working on currently.
	SetProject { relative_path: String },
	/// Resume tracking time on the current blocker task via Clockify
	Resume(ResumeArgs),
	/// Pause tracking time via Clockify
	Halt(HaltArgs),
	/// Apply formatting to the blocker file.
	/// Here mostly for completeness, as formatting is automatically applied on all the provided methods for natively modifying the file.
	Format,
}

#[derive(Clone, Debug, Parser)]
pub struct ResumeArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,

	/// Project ID or name (if omitted, uses cached project default)
	#[arg(short = 'p', long)]
	pub project: Option<String>,

	/// Task ID or name (optional)
	#[arg(short = 't', long)]
	pub task: Option<String>,

	/// Comma-separated tag IDs or names (optional)
	#[arg(short = 'g', long)]
	pub tags: Option<String>,

	/// Mark entry as billable
	#[arg(short = 'b', long, default_value_t = false)]
	pub billable: bool,
}

#[derive(Clone, Debug, Parser)]
pub struct HaltArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,
}

fn get_blocker_state_path() -> std::path::PathBuf {
	STATE_DIR.get().unwrap().join(BLOCKER_STATE_FILENAME)
}

fn is_blocker_tracking_enabled() -> bool {
	let state_path = get_blocker_state_path();
	match std::fs::read_to_string(&state_path) {
		Ok(content) => content.trim() == "true",
		Err(_) => {
			// File doesn't exist, create it with "false" and return false
			let _ = std::fs::write(&state_path, "false");
			false
		}
	}
}

fn set_blocker_tracking_state(enabled: bool) -> Result<()> {
	let state_path = get_blocker_state_path();
	std::fs::write(&state_path, if enabled { "true" } else { "false" })?;
	Ok(())
}

fn get_current_blocker_cache_path(relative_path: &str) -> std::path::PathBuf {
	let cache_key = relative_path.replace('/', "_");
	CACHE_DIR.get().unwrap().join(format!("{}_{}", cache_key, BLOCKER_CURRENT_CACHE_FILENAME))
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

/// Line classification for blocker files
#[derive(Clone, Debug, PartialEq)]
enum LineType {
	/// Content line - headers or list items (contributes to blocker list)
	Content,
	/// Comment line - tab-indented explanatory text (does not contribute to blocker list)
	Comment,
}

/// Classify a line based on its content
/// - Lines starting with tab are Comments
/// - All other non-empty lines are Content
/// - Returns None for empty lines
fn classify_line(line: &str) -> Option<LineType> {
	if line.is_empty() {
		None
	} else if line.starts_with('\t') {
		Some(LineType::Comment)
	} else {
		Some(LineType::Content)
	}
}

/// Format blocker list content according to standardization rules:
/// 1. Lines not starting with `^#* ` get prefixed with `- ` (markdown list format)
/// 2. Always have 1 empty line above `^#* ` lines (unless the line above also starts with `#`)
/// 3. Remove all other empty lines for standardization
/// 4. Comment lines (tab-indented) are preserved and must follow Content or Comment lines
fn format_blocker_content(content: &str) -> Result<String> {
	let lines: Vec<&str> = content.lines().collect();

	// First pass: validate that comments don't follow empty lines
	for (idx, line) in lines.iter().enumerate() {
		if let Some(LineType::Comment) = classify_line(line) {
			// Check if previous line was empty
			if idx > 0 && lines[idx - 1].is_empty() {
				return Err(eyre!(
					"Comment line at position {} cannot follow an empty line. Comments must follow content or other comments.",
					idx + 1
				));
			}
			// Check if it's the first line
			if idx == 0 {
				return Err(eyre!(
					"Comment line at position {} cannot be first line. Comments must follow content or other comments.",
					idx + 1
				));
			}
		}
	}

	let mut formatted_lines: Vec<String> = Vec::new();

	for line in lines.iter() {
		let line_type = classify_line(line);

		match line_type {
			None => {
				// Skip empty lines - we'll add them back strategically
				continue;
			}
			Some(LineType::Comment) => {
				// Preserve comment line with tab indentation
				formatted_lines.push(line.to_string());
			}
			Some(LineType::Content) => {
				let trimmed = line.trim();

				// Check if current line is a header (starts with # followed by space or another #)
				let is_header = trimmed.starts_with('#') && (trimmed.len() > 1 && (trimmed.chars().nth(1) == Some(' ') || trimmed.chars().nth(1) == Some('#')));

				// If this is a header and we have previous lines, check if we need an empty line before it
				if is_header && !formatted_lines.is_empty() {
					// Check if the previous non-empty line was also a header
					let last_line = formatted_lines.last().unwrap();
					let prev_is_header = last_line.trim().starts_with('#');

					// Add empty line before header only if previous line wasn't a header
					if !prev_is_header {
						formatted_lines.push(String::new());
					}

					formatted_lines.push(trimmed.to_string());
				} else if is_header {
					// First line is a header, no empty line needed
					formatted_lines.push(trimmed.to_string());
				} else {
					// Not a header - ensure it starts with "- "
					if trimmed.starts_with("- ") {
						formatted_lines.push(trimmed.to_string());
					} else {
						formatted_lines.push(format!("- {}", trimmed));
					}
				}
			}
		}
	}

	Ok(formatted_lines.join("\n"))
}

/// Add a content line to the blocker file, preserving comments and formatting
fn add_content_line(content: &str, new_line: &str) -> Result<String> {
	// Parse content into lines, add the new content line, then format
	let mut lines: Vec<&str> = content.lines().collect();
	lines.push(new_line);
	format_blocker_content(&lines.join("\n"))
}

/// Remove the last content line from the blocker file, preserving comments (except comments belonging to the removed line)
fn pop_content_line(content: &str) -> Result<String> {
	let lines: Vec<&str> = content.lines().collect();
	let mut content_lines_indices: Vec<usize> = Vec::new();

	// Find indices of all content lines
	for (idx, line) in lines.iter().enumerate() {
		if classify_line(line) == Some(LineType::Content) {
			content_lines_indices.push(idx);
		}
	}

	// Remove the last content line and its associated comments
	if let Some(&last_content_idx) = content_lines_indices.last() {
		// Find the next content line index (or end of file)
		let next_content_idx = content_lines_indices.iter()
			.rev()
			.nth(1) // Get second-to-last content line
			.map(|&idx| idx + 1) // Start keeping from the line after it
			.unwrap_or(0); // Or keep nothing if this was the only content line

		// Keep lines before the last content block, exclude the last content line and its comments
		let new_lines: Vec<&str> = lines
			.iter()
			.enumerate()
			.filter(|(idx, _)| {
				// Find where the last content block starts (the last content line)
				// And where it ends (next content line or EOF)
				let is_before_last_block = *idx < last_content_idx;
				is_before_last_block
			})
			.map(|(_, line)| *line)
			.collect();

		format_blocker_content(&new_lines.join("\n"))
	} else {
		// No content lines to remove
		format_blocker_content(content)
	}
}

fn get_current_blocker(relative_path: &str) -> Option<String> {
	let blocker_path = STATE_DIR.get().unwrap().join(relative_path);
	let blockers: Vec<String> = std::fs::read_to_string(&blocker_path)
		.unwrap_or_else(|_| String::new())
		.split('\n')
		.filter(|s| !s.is_empty())
		// Skip comment lines (tab-indented) - only consider content lines
		.filter(|s| !s.starts_with('\t'))
		.map(|s| s.to_owned())
		.collect();
	blockers.last().cloned()
}

/// Strip leading "# " or "- " prefix from a blocker line
fn strip_blocker_prefix(line: &str) -> &str {
	line.strip_prefix("# ").or_else(|| line.strip_prefix("- ")).unwrap_or(line)
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

fn get_workspace_settings_path() -> std::path::PathBuf {
	CACHE_DIR.get().unwrap().join(WORKSPACE_SETTINGS_FILENAME)
}

fn load_workspace_cache() -> WorkspaceCache {
	let cache_path = get_workspace_settings_path();
	match std::fs::read_to_string(&cache_path) {
		Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
		Err(_) => WorkspaceCache::default(),
	}
}

fn save_workspace_cache(cache: &WorkspaceCache) -> Result<()> {
	let cache_path = get_workspace_settings_path();
	let content = serde_json::to_string_pretty(cache)?;
	std::fs::write(&cache_path, content)?;
	Ok(())
}

fn get_workspace_legacy_setting(workspace: &str) -> Result<bool> {
	let cache = load_workspace_cache();

	if let Some(settings) = cache.workspaces.get(workspace) {
		Ok(settings.legacy)
	} else {
		// Ask user for preference
		println!("Workspace '{}' legacy mode setting not found.", workspace);
		print!("Use legacy mode for this workspace? [y/N]: ");
		Write::flush(&mut std::io::stdout())?;

		let mut input = String::new();
		std::io::stdin().read_line(&mut input)?;
		let use_legacy = input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes";

		// Save the preference
		let mut cache = load_workspace_cache();
		cache.workspaces.insert(workspace.to_string(), WorkspaceSettings { legacy: use_legacy });
		save_workspace_cache(&cache)?;

		println!("Saved legacy mode preference for workspace '{}': {}", workspace, use_legacy);
		Ok(use_legacy)
	}
}

async fn stop_current_tracking(workspace: Option<&str>) -> Result<()> {
	clockify::stop_time_entry_with_defaults(workspace).await
}

async fn start_tracking_for_task(description: String, relative_path: &str, resume_args: &ResumeArgs, workspace_override: Option<&str>) -> Result<()> {
	let workspace = workspace_override.or(resume_args.workspace.as_deref());

	// Determine legacy mode from workspace settings
	let legacy = if let Some(ws) = workspace {
		get_workspace_legacy_setting(ws)?
	} else {
		// If no workspace specified, use default (false)
		false
	};

	clockify::start_time_entry_with_defaults(
		workspace,
		resume_args.project.as_deref(),
		description,
		resume_args.task.as_deref(),
		resume_args.tags.as_deref(),
		resume_args.billable,
		legacy,
		Some(relative_path),
	)
	.await
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

fn handle_background_blocker_check(relative_path: &str) -> Result<()> {
	// Read and format the blocker file
	let blocker_path = STATE_DIR.get().unwrap().join(relative_path);
	if blocker_path.exists() {
		let content = std::fs::read_to_string(&blocker_path)?;
		let formatted = format_blocker_content(&content)?;

		// Only write back if content changed
		if content != formatted {
			std::fs::write(&blocker_path, formatted)?;
		}
	}

	let cached_current = load_current_blocker_cache(relative_path);
	let actual_current = get_current_blocker(relative_path);

	if cached_current != actual_current {
		if is_blocker_tracking_enabled() {
			let workspace_from_path = parse_workspace_from_path(relative_path)?;

			tokio::runtime::Runtime::new()?.block_on(async {
				let _ = stop_current_tracking(workspace_from_path.as_deref()).await;

				if let Some(new_task) = &actual_current {
					let default_resume_args = ResumeArgs {
						workspace: None,
						project: None,
						task: None,
						tags: None,
						billable: false,
					};

					let stripped_task = strip_blocker_prefix(new_task).to_string();
					if let Err(e) = start_tracking_for_task(stripped_task, relative_path, &default_resume_args, workspace_from_path.as_deref()).await {
						eprintln!("Warning: Failed to start tracking for updated task: {}", e);
					}
				}
			});
		}

		save_current_blocker_cache(relative_path, actual_current)?;
	}

	Ok(())
}

pub fn main(_settings: AppConfig, args: BlockerArgs) -> Result<()> {
	let relative_path = match args.relative_path {
		Some(f) => f,
		None => {
			let persisted_project_file = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			match std::fs::read_to_string(&persisted_project_file) {
				Ok(s) => s,
				Err(_) => "blockers.txt".to_string(),
			}
		}
	};

	// Handle background blocker check
	if std::env::var("_BLOCKER_BACKGROUND_CHECK").is_ok() {
		return handle_background_blocker_check(&relative_path);
	}

	// Parse workspace from path if it contains a slash
	let workspace_from_path = parse_workspace_from_path(&relative_path)?;

	let blocker_path = STATE_DIR.get().unwrap().join(&relative_path);

	match args.command {
		Command::Add { name } => {
			// If tracking is enabled, stop current task before adding new one
			if is_blocker_tracking_enabled() {
				tokio::runtime::Runtime::new()?.block_on(async {
					let _ = stop_current_tracking(workspace_from_path.as_deref()).await; // Ignore errors when stopping
				});
			}

			// Read existing content, add new line, format and write
			let existing_content = std::fs::read_to_string(&blocker_path).unwrap_or_else(|_| String::new());
			let formatted = add_content_line(&existing_content, &name)?;
			std::fs::write(&blocker_path, formatted)?;

			// Save current blocker to cache
			save_current_blocker_cache(&relative_path, Some(name.clone()))?;

			// If tracking is enabled, start tracking the new task
			if is_blocker_tracking_enabled() {
				let default_resume_args = ResumeArgs {
					workspace: None,
					project: None,
					task: None,
					tags: None,
					billable: false,
				};

				tokio::runtime::Runtime::new()?.block_on(async {
					if let Err(e) = start_tracking_for_task(name, &relative_path, &default_resume_args, workspace_from_path.as_deref()).await {
						eprintln!("Warning: Failed to start tracking for new task: {}", e);
					}
				});
			}
		}
		Command::Pop => {
			// If tracking is enabled, stop current task before popping
			if is_blocker_tracking_enabled() {
				tokio::runtime::Runtime::new()?.block_on(async {
					let _ = stop_current_tracking(workspace_from_path.as_deref()).await; // Ignore errors when stopping
				});
			}

			// Read existing content, pop last content line, format and write
			let existing_content = std::fs::read_to_string(&blocker_path).unwrap_or_else(|_| String::new());
			let formatted = pop_content_line(&existing_content)?;
			std::fs::write(&blocker_path, formatted)?;

			// Get the new current blocker after popping
			let new_current = get_current_blocker(&relative_path);
			save_current_blocker_cache(&relative_path, new_current.clone())?;

			// If tracking is enabled and there's still a task, start tracking it
			if is_blocker_tracking_enabled() {
				if let Some(current_task) = new_current {
					let default_resume_args = ResumeArgs {
						workspace: None,
						project: None,
						task: None,
						tags: None,
						billable: false,
					};

					let stripped_task = strip_blocker_prefix(&current_task).to_string();
					tokio::runtime::Runtime::new()?.block_on(async {
						if let Err(e) = start_tracking_for_task(stripped_task, &relative_path, &default_resume_args, workspace_from_path.as_deref()).await {
							eprintln!("Warning: Failed to start tracking for previous task: {}", e);
						}
					});
				}
			}
		}
		Command::List => {
			let sprint_header = std::fs::read_to_string(DATA_DIR.get().unwrap().join(SPRINT_HEADER_REL_PATH)).ok();
			if let Some(s) = sprint_header {
				println!("{s}");
			}
			let content = std::fs::read_to_string(&blocker_path).unwrap_or_else(|_| String::new());
			println!("{}", content);
		}
		Command::Current =>
			if let Some(last) = get_current_blocker(&relative_path) {
				let stripped = strip_blocker_prefix(&last);

				const MAX_LEN: usize = 70;
				match stripped.len() {
					0..=MAX_LEN => println!("{}", stripped),
					_ => println!("{}...", &stripped[..(MAX_LEN - 3)]),
				}
			},
		Command::Open { file_path, touch } => {
			// Save current blocker state to cache before opening
			let current = get_current_blocker(&relative_path);
			save_current_blocker_cache(&relative_path, current)?;

			// Determine which file to open
			let resolved_path = match file_path {
				Some(custom_path) => resolve_project_path(&custom_path)?,
				None => relative_path.clone(),
			};

			let path_to_open = STATE_DIR.get().unwrap().join(&resolved_path);

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
			v_utils::io::open(&path_to_open)?;

			// Spawn background process to check for changes after editor closes
			spawn_blocker_comparison_process(relative_path.clone())?;
		}
		Command::SetProject { relative_path } => {
			// Resolve the project path using pattern matching
			let resolved_path = resolve_project_path(&relative_path)?;

			// Validate the resolved path before saving
			parse_workspace_from_path(&resolved_path)?;
			let state_dir = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			std::fs::write(&state_dir, &resolved_path)?;

			println!("Set current project to: {}", resolved_path);

			// Spawn background process to check for clockify updates after project change
			spawn_blocker_comparison_process(resolved_path)?;
		}
		Command::Resume(resume_args) => {
			// Get current blocker task description
			let description = match get_current_blocker(&relative_path) {
				Some(task) => strip_blocker_prefix(&task).to_string(),
				None => return Err(eyre!("No current blocker task found. Add one with 'todo blocker add <task>'")),
			};

			// Enable tracking state
			set_blocker_tracking_state(true)?;

			tokio::runtime::Runtime::new()?.block_on(async {
				let workspace = workspace_from_path.as_deref().or(resume_args.workspace.as_deref());

				// Determine legacy mode from workspace settings
				let legacy = if let Some(ws) = workspace { get_workspace_legacy_setting(ws)? } else { false };

				clockify::start_time_entry_with_defaults(
					workspace,
					resume_args.project.as_deref(),
					description,
					resume_args.task.as_deref(),
					resume_args.tags.as_deref(),
					resume_args.billable,
					legacy,
					Some(&relative_path), // Pass the relative_path for legacy mode
				)
				.await
			})?;
		}
		Command::Halt(pause_args) => {
			// Disable tracking state
			set_blocker_tracking_state(false)?;

			let workspace = workspace_from_path.as_deref().or(pause_args.workspace.as_deref());
			tokio::runtime::Runtime::new()?.block_on(async { clockify::stop_time_entry_with_defaults(workspace).await })?;
		}
		Command::Format => {
			// Read, format, and write back the blocker file
			if blocker_path.exists() {
				let content = std::fs::read_to_string(&blocker_path)?;
				let formatted = format_blocker_content(&content)?;

				if content != formatted {
					std::fs::write(&blocker_path, formatted)?;
					println!("Formatted blocker file: {}", relative_path);
				} else {
					println!("Blocker file already formatted: {}", relative_path);
				}
			} else {
				return Err(eyre!("Blocker file does not exist: {}", relative_path));
			}
		}
	};
	Ok(())
}

/// Search for projects using a grep-like pattern
fn search_projects_by_pattern(pattern: &str) -> Result<Vec<String>> {
	use std::process::Command;

	let state_dir = STATE_DIR.get().unwrap();
	let output = Command::new("find").args([state_dir.to_str().unwrap(), "-name", "*.md", "-type", "f"]).output()?;

	if !output.status.success() {
		return Err(eyre!("Failed to search for files"));
	}

	let all_files = String::from_utf8(output.stdout)?;
	let mut matches = Vec::new();

	for line in all_files.lines() {
		let file_path = line.trim();
		if file_path.is_empty() {
			continue;
		}

		// Convert absolute path to relative path from STATE_DIR
		let relative_path = if let Ok(rel_path) = Path::new(file_path).strip_prefix(state_dir) {
			rel_path.to_string_lossy().to_string()
		} else {
			continue; // Skip files not in STATE_DIR
		};

		// Extract filename without extension for matching
		if let Some(filename) = Path::new(&relative_path).file_stem() {
			if let Some(filename_str) = filename.to_str() {
				let pattern_lower = pattern.to_lowercase();
				let filename_lower = filename_str.to_lowercase();
				let path_lower = relative_path.to_lowercase();

				// Check if pattern matches filename OR appears anywhere in the path (case-insensitive)
				if filename_lower.contains(&pattern_lower) || path_lower.contains(&pattern_lower) {
					matches.push(relative_path.to_string());
				}
			}
		}
	}

	Ok(matches)
}

/// Use fzf to let user choose from multiple project matches
fn choose_project_with_fzf(matches: &[String], initial_query: &str) -> Result<Option<String>> {
	use std::{
		io::Write as IoWrite,
		process::{Command, Stdio},
	};

	// Prepare input for fzf
	let input = matches.join("\n");

	let mut fzf = Command::new("fzf").args(["--query", initial_query]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;

	if let Some(stdin) = fzf.stdin.take() {
		let mut stdin_handle = stdin;
		stdin_handle.write_all(input.as_bytes())?;
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
fn resolve_project_path(pattern: &str) -> Result<String> {
	// First, check if it's already a valid path
	if pattern.contains('/') || pattern.ends_with(".md") {
		return Ok(pattern.to_string());
	}

	let matches = search_projects_by_pattern(pattern)?;

	match matches.len() {
		0 => Err(eyre!("No projects found matching pattern: {pattern}")),
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_classify_line() {
		assert_eq!(classify_line(""), None);
		assert_eq!(classify_line("\tComment"), Some(LineType::Comment));
		assert_eq!(classify_line("Content"), Some(LineType::Content));
		assert_eq!(classify_line("  Spaces not tab"), Some(LineType::Content));
	}

	#[test]
	fn test_comment_validation_errors() {
		// Comment as first line
		assert!(format_blocker_content("\tComment").is_err());
		// Comment after empty line
		assert!(format_blocker_content("- Task\n\n\tComment").is_err());
	}

	#[test]
	fn test_comment_preservation() {
		// Single and multiple comments
		let input = "- Task 1\n\tComment 1\n- Task 2\n\tComment A\n\tComment B";
		let expected = "- Task 1\n\tComment 1\n- Task 2\n\tComment A\n\tComment B";
		assert_eq!(format_blocker_content(input).unwrap(), expected);
	}

	#[test]
	fn test_header_empty_line_rules() {
		// No empty line between headers
		assert_eq!(format_blocker_content("# H1\n## H2").unwrap(), "# H1\n## H2");
		// Empty line before header after item
		assert_eq!(format_blocker_content("item\n\n# Header").unwrap(), "- item\n\n# Header");
		// Valid header needs space: # vs #NoSpace
		assert_eq!(format_blocker_content("#NoSpace").unwrap(), "- #NoSpace");
	}

	#[test]
	fn test_add_pop_preserve_comments() {
		let input = "- Task 1\n\tComment 1";
		// Add preserves comments
		assert_eq!(add_content_line(input, "Task 2").unwrap(), "- Task 1\n\tComment 1\n- Task 2");
		// Pop preserves comments
		let input2 = "- Task 1\n\tComment 1\n- Task 2\n\tComment 2";
		assert_eq!(pop_content_line(input2).unwrap(), "- Task 1\n\tComment 1");
	}

	#[test]
	fn test_empty_lines_removed() {
		// Multiple empty lines collapsed
		let input = "item 1\n\n\nitem 2\n\n\n\nitem 3";
		assert_eq!(format_blocker_content(input).unwrap(), "- item 1\n- item 2\n- item 3");
	}
}
