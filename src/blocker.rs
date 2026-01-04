use std::{collections::HashMap, io::Write, path::Path};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::{clockify, milestones::SPRINT_HEADER_REL_PATH};

fn blockers_dir() -> std::path::PathBuf {
	v_utils::xdg_data_dir!("blockers")
}

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";
static BLOCKER_STATE_FILENAME: &str = "blocker_state.txt";
static WORKSPACE_SETTINGS_FILENAME: &str = "workspace_settings.json";
static BLOCKER_CURRENT_CACHE_FILENAME: &str = "blocker_current_cache.txt";
static PRE_URGENT_PROJECT_FILENAME: &str = "pre_urgent_project.txt";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct WorkspaceSettings {
	fully_qualified: bool,
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
	/// Just open the \`blockers\` file with $EDITOR. Text as interface.
	Open {
		/// Optional file path relative to state directory to open instead of the default blocker file
		file_path: Option<String>,
		/// Create the file if it doesn't exist (touch)
		#[arg(short = 't', long)]
		touch: bool,
		/// Set the opened file as chosen project after exiting the editor
		#[arg(short = 's', long)]
		set_project_after: bool,
		/// Mark as urgent (equivalent to --file-path urgent, opens urgent.md)
		#[arg(short = 'u', long)]
		urgent: bool,
	},
	/// Set the default `--relative_path`, for the project you're working on currently.
	SetProject {
		relative_path: String,
		/// Create the file if it doesn't exist (touch)
		#[arg(short = 't', long)]
		touch: bool,
	},
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
	v_utils::xdg_state_file!(BLOCKER_STATE_FILENAME)
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HeaderLevel {
	One,
	Two,
	Three,
	Four,
	Five,
}

impl HeaderLevel {
	/// Get the numeric level (1-5)
	fn to_usize(self) -> usize {
		match self {
			HeaderLevel::One => 1,
			HeaderLevel::Two => 2,
			HeaderLevel::Three => 3,
			HeaderLevel::Four => 4,
			HeaderLevel::Five => 5,
		}
	}

	/// Create from numeric level (1-5)
	fn from_usize(level: usize) -> Option<Self> {
		match level {
			1 => Some(HeaderLevel::One),
			2 => Some(HeaderLevel::Two),
			3 => Some(HeaderLevel::Three),
			4 => Some(HeaderLevel::Four),
			5 => Some(HeaderLevel::Five),
			_ => None,
		}
	}
}

/// Line classification for blocker files
#[derive(Clone, Debug, PartialEq)]
pub enum LineType {
	/// Header with level and text (without # prefix)
	Header { level: HeaderLevel, text: String },
	/// List item or other content line (contributes to blocker list)
	Item,
	/// Comment line - tab-indented explanatory text (does not contribute to blocker list)
	Comment,
}

impl LineType {
	/// Check if this line type is a header
	#[allow(dead_code)]
	fn is_header(&self) -> bool {
		matches!(self, LineType::Header { .. })
	}

	/// Get the header level, or None if not a header
	#[allow(dead_code)]
	fn header_level(&self) -> Option<HeaderLevel> {
		match self {
			LineType::Header { level, .. } => Some(*level),
			_ => None,
		}
	}

	/// Get the header text, or None if not a header
	#[allow(dead_code)]
	fn header_text(&self) -> Option<&str> {
		match self {
			LineType::Header { text, .. } => Some(text),
			_ => None,
		}
	}

	/// Check if this line type contributes to the blocker list (headers and items)
	pub fn is_content(&self) -> bool {
		!matches!(self, LineType::Comment)
	}
}

/// Normalize content based on file extension
/// Converts file-specific syntax to a canonical markdown-like format:
/// - .md: pass through as-is
/// - .typ: convert Typst syntax to markdown (= to #, etc.)
/// - other: pass through as-is
fn normalize_content_by_extension(content: &str, file_path: &Path) -> Result<String> {
	let extension = file_path.extension().and_then(|e| e.to_str());

	match extension {
		Some("md") => Ok(content.to_string()),
		Some("typ") => typst_to_markdown(content),
		_ => Ok(content.to_string()),
	}
}

/// Convert Typst syntax to markdown format
/// Typst uses = for headings (more = means deeper), we convert to # (more # means deeper)
/// Typst list syntax is similar to markdown (- for bullets)
fn typst_to_markdown(content: &str) -> Result<String> {
	use typst::syntax::{SyntaxKind, ast::AstNode, parse};

	// Parse the Typst source into a syntax tree
	let syntax_node = parse(content);

	// Walk the syntax tree and convert to markdown
	let mut markdown_lines: Vec<String> = Vec::new();

	// Traverse the syntax tree
	for child in syntax_node.children() {
		// Skip pure whitespace nodes (space, parbreak)
		if matches!(child.kind(), SyntaxKind::Space | SyntaxKind::Parbreak) {
			// Check if this is a significant parbreak (actual empty line in source)
			let text = child.text();
			if text.matches('\n').count() > 1 {
				// Multiple newlines = intentional empty line
				markdown_lines.push(String::new());
			}
			continue;
		}

		// Get the text content of this node
		let node_text = child.clone().into_text();

		// Try to interpret as Heading
		if let Some(heading) = typst::syntax::ast::Heading::from_untyped(child) {
			let level_num = heading.depth().get();
			// Extract just the body text (without the = prefix)
			let body_text = heading.body().to_untyped().clone().into_text();
			let trimmed_body = body_text.trim();
			// Convert Typst heading (= foo) to markdown heading (# foo)
			markdown_lines.push(format!("{} {trimmed_body}", "#".repeat(level_num)));
			continue;
		}

		// Try to interpret as ListItem (bullet list)
		// Typst uses "- item" which is identical to markdown, so just keep it
		if let Some(_list_item) = typst::syntax::ast::ListItem::from_untyped(child) {
			let trimmed = node_text.trim();
			if !trimmed.is_empty() {
				markdown_lines.push(trimmed.to_string());
			}
			continue;
		}

		// Try to interpret as EnumItem (numbered list)
		// Convert numbered lists to markdown-style items with "- " prefix
		if let Some(_enum_item) = typst::syntax::ast::EnumItem::from_untyped(child) {
			let trimmed = node_text.trim();
			if !trimmed.is_empty() {
				// For numbered items, just treat as regular items
				// Strip the number/+ prefix and convert to -
				let item_text = if let Some(stripped) = trimmed.strip_prefix('+') {
					stripped.trim()
				} else {
					// Handle numbered format like "1. item"
					if let Some(pos) = trimmed.find('.') { trimmed[pos + 1..].trim() } else { trimmed }
				};
				markdown_lines.push(format!("- {item_text}"));
			}
			continue;
		}

		// For other content (paragraphs, text), keep as-is if non-empty
		let trimmed = node_text.trim();
		if !trimmed.is_empty() {
			markdown_lines.push(trimmed.to_string());
		}
	}

	Ok(markdown_lines.join("\n"))
}

/// Classify a line based on markdown syntax
/// - Lines starting with tab are Comments
/// - Lines starting with 2+ spaces (likely editor-converted tabs) are Comments
/// - Lines starting with # are Headers (levels 1-5)
/// - All other non-empty lines are Items
/// - Returns None for empty lines
fn classify_line_markdown(line: &str) -> Option<LineType> {
	if line.is_empty() {
		return None;
	}

	if line.starts_with('\t') {
		return Some(LineType::Comment);
	}

	// Treat lines starting with 2+ spaces as comments (likely from editor tab-to-space conversion)
	// We check for at least 2 spaces to avoid misclassifying accidentally indented content
	if line.starts_with("  ") && !line.trim_start().starts_with('-') {
		return Some(LineType::Comment);
	}

	let trimmed = line.trim();

	// Check for headers (# with space after)
	if trimmed.starts_with('#') {
		let mut count = 0;
		for ch in trimmed.chars() {
			if ch == '#' {
				count += 1;
			} else {
				break;
			}
		}

		// Valid header must have space after the # characters
		if count > 0 && trimmed.len() > count {
			let next_char = trimmed.chars().nth(count);
			if next_char == Some(' ') {
				let text = trimmed[count + 1..].to_string();

				// Warn if header is nested too deeply (level > 5)
				if count > 5 {
					eprintln!("Warning: Header level {count} is too deep (max 5 supported). Treating as regular item: {trimmed}");
					return Some(LineType::Item);
				}

				if let Some(level) = HeaderLevel::from_usize(count) {
					return Some(LineType::Header { level, text });
				}
			}
		}
	}

	Some(LineType::Item)
}

/// Classify a line based on its content (backwards compatibility wrapper)
/// Uses markdown classification by default
pub fn classify_line(line: &str) -> Option<LineType> {
	classify_line_markdown(line)
}

/// Check if the content is semantically empty (only comments or whitespace, no actual content)
fn is_semantically_empty(content: &str) -> bool {
	content.lines().filter_map(classify_line).all(|line_type| !line_type.is_content())
}

/// Format blocker list content according to standardization rules:
/// 1. Lines not starting with `^#* ` get prefixed with `- ` (markdown list format)
/// 2. Always have 1 empty line above `^#* ` lines (unless the line above also starts with `#`)
/// 3. Remove all other empty lines for standardization
/// 4. Comment lines (tab-indented) are preserved and must follow Content or Comment lines
/// 5. Code blocks (``` ... ```) within comments can contain blank lines
fn format_blocker_content(content: &str) -> Result<String> {
	let lines: Vec<&str> = content.lines().collect();

	// First pass: validate that comments don't follow empty lines (outside of code blocks)
	let mut in_code_block = false;
	for (idx, line) in lines.iter().enumerate() {
		// Track code block state - code blocks in comments are tab-indented with ```
		let trimmed = line.trim_start_matches('\t').trim_start();
		if trimmed.starts_with("```") {
			in_code_block = !in_code_block;
		}

		// Skip validation inside code blocks - blank lines are allowed there
		if in_code_block {
			continue;
		}

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
	let mut in_code_block = false;

	for line in lines.iter() {
		// Track code block state for formatting
		let trimmed_for_code = line.trim_start_matches('\t').trim_start();
		if trimmed_for_code.starts_with("```") {
			in_code_block = !in_code_block;
		}

		let line_type = classify_line(line);

		match line_type {
			None => {
				// Preserve empty lines inside code blocks, skip others
				if in_code_block {
					formatted_lines.push(String::new());
				}
				continue;
			}
			Some(LineType::Comment) => {
				// Normalize comment lines to tab indentation (convert leading spaces to tab)
				if line.starts_with('\t') {
					// Already tab-indented, preserve as-is
					formatted_lines.push(line.to_string());
				} else {
					// Space-indented, convert to tab
					let trimmed = line.trim_start();
					formatted_lines.push(format!("\t{trimmed}"));
				}
			}
			Some(LineType::Header { level, text }) => {
				// Check if we need an empty line before this header
				if !formatted_lines.is_empty() {
					let last_line = formatted_lines.last().unwrap();
					let prev_line_type = classify_line(last_line);

					// Add empty line based on header level relationship:
					// - No space if previous is larger rank (smaller level value) than current
					// - Space if previous is same or lower rank (same/larger level value) than current
					// - Space if previous line is not a header
					let needs_space = match prev_line_type {
						Some(LineType::Header { level: prev_level, .. }) => {
							// Using derived Ord: One < Two < Three < Four < Five
							prev_level >= level // same or lower rank (e.g., ## after # or ##)
						}
						_ => true, // previous line is not a header
					};

					if needs_space {
						formatted_lines.push(String::new());
					}
				}

				// Reconstruct the header line
				let header_prefix = "#".repeat(level.to_usize());
				formatted_lines.push(format!("{header_prefix} {text}"));
			}
			Some(LineType::Item) => {
				let trimmed = line.trim();
				// Ensure it starts with "- "
				if trimmed.starts_with("- ") {
					formatted_lines.push(trimmed.to_string());
				} else {
					formatted_lines.push(format!("- {trimmed}"));
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

	// Find indices of all content lines (headers and items, not comments)
	for (idx, line) in lines.iter().enumerate() {
		if let Some(line_type) = classify_line(line)
			&& line_type.is_content()
		{
			content_lines_indices.push(idx);
		}
	}

	// Remove the last content line and its associated comments
	if let Some(&last_content_idx) = content_lines_indices.last() {
		// Keep lines before the last content block, exclude the last content line and its comments
		let new_lines: Vec<&str> = lines
			.iter()
			.enumerate()
			.filter(|(idx, _)| {
				// Find where the last content block starts (the last content line)
				// And where it ends (next content line or EOF)

				*idx < last_content_idx
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
	let blocker_path = blockers_dir().join(relative_path);
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

/// Get the current blocker with parent headers prepended (joined by ": ")
/// If fully_qualified is true, prepend the project name from the relative_path
fn get_current_blocker_with_headers(relative_path: &str, fully_qualified: bool) -> Option<String> {
	let current = get_current_blocker(relative_path)?;
	let stripped = strip_blocker_prefix(&current);

	// Read blocker file to parse parent headers
	let blocker_path = blockers_dir().join(relative_path);
	let parent_headers = if blocker_path.exists() {
		let content = std::fs::read_to_string(&blocker_path).ok()?;
		parse_parent_headers(&content, &current)
	} else {
		Vec::new()
	};

	// Build final output with parent headers
	let mut parts = Vec::new();

	// Add project name if fully_qualified is true
	if fully_qualified {
		// Extract project name from relative_path (filename without extension)
		let project_name = std::path::Path::new(relative_path).file_stem().and_then(|s| s.to_str()).unwrap_or(relative_path);
		parts.push(project_name.to_string());
	}

	// Add parent headers
	parts.extend(parent_headers);

	// Add the stripped task
	if parts.is_empty() {
		Some(stripped.to_string())
	} else {
		Some(format!("{}: {stripped}", parts.join(": ")))
	}
}

/// Strip leading "# " or "- " prefix from a blocker line
pub fn strip_blocker_prefix(line: &str) -> &str {
	line.strip_prefix("# ").or_else(|| line.strip_prefix("- ")).unwrap_or(line)
}

/// Parse the tree of parent headers above a task
/// Returns a vector of header texts in order from top-level to immediate parent
pub fn parse_parent_headers(content: &str, task_line: &str) -> Vec<String> {
	let lines: Vec<&str> = content.lines().collect();

	// Find the index of the task line
	let task_index = match lines.iter().position(|&line| {
		// Match the task line exactly (after stripping prefix)
		let stripped = strip_blocker_prefix(line);
		stripped == task_line.strip_prefix("- ").unwrap_or(task_line)
	}) {
		Some(idx) => idx,
		None => return Vec::new(),
	};

	let mut headers = Vec::new();
	let mut current_level: Option<HeaderLevel> = None;

	// Walk backwards from the task to find parent headers
	for i in (0..task_index).rev() {
		let line = lines[i];

		// Classify the line
		if let Some(LineType::Header { level, text }) = classify_line(line) {
			// Only add headers that are parent levels (smaller level = higher in hierarchy)
			// Using derived Ord: One < Two < Three < Four < Five
			if current_level.is_none() || level < current_level.unwrap() {
				headers.push(text);
				current_level = Some(level);
			}
		}
	}

	// Reverse to get top-level first
	headers.reverse();
	headers
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
	v_utils::xdg_cache_file!(WORKSPACE_SETTINGS_FILENAME)
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

fn get_workspace_fully_qualified_setting(workspace: &str) -> Result<bool> {
	let cache = load_workspace_cache();

	if let Some(settings) = cache.workspaces.get(workspace) {
		Ok(settings.fully_qualified)
	} else {
		// Ask user for preference
		println!("Workspace '{workspace}' fully-qualified mode setting not found.");
		print!("Use fully-qualified mode (legacy) for this workspace? [y/N]: ");
		Write::flush(&mut std::io::stdout())?;

		let mut input = String::new();
		std::io::stdin().read_line(&mut input)?;
		let use_fully_qualified = input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes";

		// Save the preference
		let mut cache = load_workspace_cache();
		cache.workspaces.insert(
			workspace.to_string(),
			WorkspaceSettings {
				fully_qualified: use_fully_qualified,
			},
		);
		save_workspace_cache(&cache)?;

		println!("Saved fully-qualified mode preference for workspace '{workspace}': {use_fully_qualified}");
		Ok(use_fully_qualified)
	}
}

async fn stop_current_tracking(workspace: Option<&str>) -> Result<()> {
	clockify::stop_time_entry_with_defaults(workspace).await
}

async fn start_tracking_for_task(description: String, relative_path: &str, resume_args: &ResumeArgs, workspace_override: Option<&str>) -> Result<()> {
	let workspace = workspace_override.or(resume_args.workspace.as_deref());

	// Determine fully_qualified mode from workspace settings (legacy mode for clockify)
	let fully_qualified = if let Some(ws) = workspace {
		get_workspace_fully_qualified_setting(ws)?
	} else {
		// If no workspace specified, use default (false)
		false
	};

	// Get current blocker with parent headers prepended (use fully_qualified for clockify legacy mode)
	let final_description = get_current_blocker_with_headers(relative_path, fully_qualified).unwrap_or(description);

	clockify::start_time_entry_with_defaults(
		workspace,
		resume_args.project.as_deref(),
		final_description,
		resume_args.task.as_deref(),
		resume_args.tags.as_deref(),
		resume_args.billable,
	)
	.await
}

/// Helper to create default resume args (used in multiple places)
fn create_default_resume_args() -> ResumeArgs {
	ResumeArgs {
		workspace: None,
		project: None,
		task: None,
		tags: None,
		billable: false,
	}
}

/// Helper to restart tracking for the current blocker in a project
/// This is used when switching tasks or projects while tracking is enabled
async fn restart_tracking_for_project(relative_path: &str, workspace: Option<&str>) -> Result<()> {
	if let Some(current_blocker) = get_current_blocker(relative_path) {
		let default_resume_args = create_default_resume_args();
		let stripped_task = strip_blocker_prefix(&current_blocker).to_string();

		if let Err(e) = start_tracking_for_task(stripped_task, relative_path, &default_resume_args, workspace).await {
			eprintln!("Warning: Failed to start tracking for task: {e}");
		}
	}
	Ok(())
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
	if project_changed && is_blocker_tracking_enabled() {
		// Stop tracking on the old project
		if let Some(old_path) = &old_project {
			let old_workspace = parse_workspace_from_path(old_path).ok().flatten();
			let _ = stop_current_tracking(old_workspace.as_deref()).await;
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
	let actual_current = get_current_blocker(&default_project_path);

	if cached_current != actual_current {
		if is_blocker_tracking_enabled() {
			let workspace_from_path = parse_workspace_from_path(&default_project_path)?;

			let _ = stop_current_tracking(workspace_from_path.as_deref()).await;

			restart_tracking_for_project(&default_project_path, workspace_from_path.as_deref()).await?;
		}

		save_current_blocker_cache(&default_project_path, actual_current)?;
	}

	// After formatting, cleanup urgent file if it's empty
	cleanup_urgent_file_if_empty(relative_path).await?;

	// After formatting, check for urgent files and auto-switch if found
	if let Some(urgent_path) = check_for_urgent_file() {
		let current_project_path = v_utils::xdg_cache_file!(CURRENT_PROJECT_CACHE_FILENAME);
		let current_project = std::fs::read_to_string(&current_project_path).unwrap_or_else(|_| "blockers.txt".to_string());

		// Only switch if we're not already on the urgent project
		if current_project != urgent_path {
			eprintln!("Detected urgent file, switching to: {urgent_path}");
			set_current_project(&urgent_path).await?;
		}
	}

	Ok(())
}

pub async fn main(_settings: &crate::config::LiveSettings, args: BlockerArgs) -> Result<()> {
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
			if is_blocker_tracking_enabled() {
				let _ = stop_current_tracking(target_workspace_from_path.as_deref()).await; // Ignore errors when stopping
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
			let existing_content = std::fs::read_to_string(&target_blocker_path).unwrap_or_else(|_| String::new());
			let formatted = add_content_line(&existing_content, &name)?;
			std::fs::write(&target_blocker_path, formatted)?;

			// Save current blocker to cache
			save_current_blocker_cache(&target_relative_path, Some(name.clone()))?;

			// Cleanup urgent file if it's now empty
			cleanup_urgent_file_if_empty(&target_relative_path).await?;

			// If adding to a different project (e.g., urgent), switch the current project
			if target_relative_path != relative_path {
				set_current_project(&target_relative_path).await?;
			} else if is_blocker_tracking_enabled() {
				// Only restart tracking here if we didn't switch projects
				// (set_current_project already handles tracking restart)
				restart_tracking_for_project(&target_relative_path, target_workspace_from_path.as_deref()).await?;
			}
		}
		Command::Pop => {
			// If tracking is enabled, stop current task before popping
			if is_blocker_tracking_enabled() {
				let _ = stop_current_tracking(workspace_from_path.as_deref()).await; // Ignore errors when stopping
			}

			// Read existing content, pop last content line, format and write
			let existing_content = std::fs::read_to_string(&blocker_path).unwrap_or_else(|_| String::new());
			let formatted = pop_content_line(&existing_content)?;
			std::fs::write(&blocker_path, formatted)?;

			// Get the new current blocker after popping
			let new_current = get_current_blocker(&relative_path);
			save_current_blocker_cache(&relative_path, new_current.clone())?;

			// Cleanup urgent file if it's now empty
			cleanup_urgent_file_if_empty(&relative_path).await?;

			// If tracking is enabled and there's still a task, start tracking it
			if is_blocker_tracking_enabled() {
				restart_tracking_for_project(&relative_path, workspace_from_path.as_deref()).await?;
			}
		}
		Command::List => {
			let sprint_header = std::fs::read_to_string(v_utils::xdg_data_file!(SPRINT_HEADER_REL_PATH)).ok();
			if let Some(s) = sprint_header {
				println!("{s}");
			}
			let content = std::fs::read_to_string(&blocker_path).unwrap_or_else(|_| String::new());
			println!("{content}");
		}
		Command::Current { fully_qualified } =>
			if let Some(output) = get_current_blocker_with_headers(&relative_path, fully_qualified) {
				const MAX_LEN: usize = 70;
				match output.len() {
					0..=MAX_LEN => println!("{output}"),
					_ => println!("{}...", &output[..(MAX_LEN - 3)]),
				}
			},
		Command::Open {
			file_path,
			touch,
			set_project_after,
			urgent,
		} => {
			// Save current blocker state to cache before opening
			let current = get_current_blocker(&relative_path);
			save_current_blocker_cache(&relative_path, current)?;

			// Determine which file to open
			let resolved_path = if urgent {
				// --urgent flag takes precedence: use workspace-specific "urgent.md"
				// Requires a workspace context
				let urgent_path = if let Some(workspace) = workspace_from_path.as_ref() {
					format!("{workspace}/urgent.md")
				} else {
					return Err(eyre!(
						"Cannot use --urgent without a workspace. Set a workspace project first (e.g., 'blocker set-project work/blockers.md')"
					));
				};
				// Check if we can create this urgent file (only if touch is enabled)
				if touch {
					check_urgent_creation_allowed(&urgent_path)?;
				}
				urgent_path
			} else {
				match file_path {
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

			// If set_project_after flag is set, update the current project
			if set_project_after {
				set_current_project(&resolved_path).await?;
			} else {
				// Spawn background process to check for changes after editor closes
				spawn_blocker_comparison_process(resolved_path.clone())?;
			}
		}
		Command::SetProject { relative_path, touch } => {
			// Resolve the project path using pattern matching
			let resolved_path = resolve_project_path(&relative_path, touch)?;

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
			if get_current_blocker(&relative_path).is_none() {
				return Err(eyre!("No current blocker task found. Add one with 'todo blocker add <task>'"));
			}

			// Enable tracking state
			set_blocker_tracking_state(true)?;

			// Use the shared start_tracking_for_task function which handles parent headers
			// Pass empty description since start_tracking_for_task will get it from get_current_blocker_with_headers
			if let Err(e) = start_tracking_for_task(String::new(), &relative_path, &resume_args, workspace_from_path.as_deref()).await {
				eprintln!("Failed to start tracking: {e}");
				return Err(e);
			}
		}
		Command::Halt(pause_args) => {
			// Disable tracking state
			set_blocker_tracking_state(false)?;

			let workspace = workspace_from_path.as_deref().or(pause_args.workspace.as_deref());
			clockify::stop_time_entry_with_defaults(workspace).await?;
		}
		Command::Format => {
			// Read, format, and write back the blocker file
			if blocker_path.exists() {
				let content = std::fs::read_to_string(&blocker_path)?;
				// Normalize content based on file extension (e.g., convert .typ to markdown)
				let normalized = normalize_content_by_extension(&content, &blocker_path)?;
				let formatted = format_blocker_content(&normalized)?;

				// Check if this is a Typst file that needs to be converted to markdown
				let extension = blocker_path.extension().and_then(|e| e.to_str());
				let (write_path, new_relative_path) = if extension == Some("typ") {
					// Convert .typ to .md
					let new_path = blocker_path.with_extension("md");
					let new_rel = relative_path.strip_suffix(".typ").unwrap_or(&relative_path).to_string() + ".md";
					(new_path, new_rel)
				} else {
					(blocker_path.clone(), relative_path.clone())
				};

				if content != formatted {
					std::fs::write(&write_path, formatted)?;
					// If we converted from .typ to .md, remove the old .typ file
					if extension == Some("typ") {
						std::fs::remove_file(&blocker_path)?;
						println!("Converted and formatted blocker file: {relative_path} -> {new_relative_path}");
					} else {
						println!("Formatted blocker file: {relative_path}");
					}
				} else {
					println!("Blocker file already formatted: {relative_path}");
				}

				// Cleanup urgent file if it's now empty
				cleanup_urgent_file_if_empty(&relative_path).await?;
			} else {
				return Err(eyre!("Blocker file does not exist: {}", relative_path));
			}
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
	fn test_classify_line() {
		assert_eq!(classify_line(""), None);
		assert_eq!(classify_line("\tComment"), Some(LineType::Comment));
		assert_eq!(classify_line("Content"), Some(LineType::Item));
		// Lines with 2+ leading spaces are now treated as comments (likely tab-to-space conversion)
		assert_eq!(classify_line("  Spaces not tab"), Some(LineType::Comment));
		// But space-indented list items (with -) are still items
		assert_eq!(classify_line("  - Indented list item"), Some(LineType::Item));
		assert_eq!(
			classify_line("# Header 1"),
			Some(LineType::Header {
				level: HeaderLevel::One,
				text: "Header 1".to_string()
			})
		);
		assert_eq!(
			classify_line("## Header 2"),
			Some(LineType::Header {
				level: HeaderLevel::Two,
				text: "Header 2".to_string()
			})
		);
		assert_eq!(
			classify_line("### Header 3"),
			Some(LineType::Header {
				level: HeaderLevel::Three,
				text: "Header 3".to_string()
			})
		);
		assert_eq!(
			classify_line("#### Header 4"),
			Some(LineType::Header {
				level: HeaderLevel::Four,
				text: "Header 4".to_string()
			})
		);
		assert_eq!(
			classify_line("##### Header 5"),
			Some(LineType::Header {
				level: HeaderLevel::Five,
				text: "Header 5".to_string()
			})
		);
		assert_eq!(classify_line("#NoSpace"), Some(LineType::Item)); // Invalid header
		assert_eq!(classify_line("###### Header 6"), Some(LineType::Item)); // Level 6 not supported, treated as item
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
		// No empty line when going from larger rank (smaller #) to lower rank (more #)
		assert_eq!(format_blocker_content("# H1\n## H2").unwrap(), "# H1\n## H2");
		assert_eq!(format_blocker_content("# H1\n### H3").unwrap(), "# H1\n### H3");
		assert_eq!(format_blocker_content("## H2\n### H3").unwrap(), "## H2\n### H3");

		// Empty line when going from same rank to same rank
		assert_eq!(format_blocker_content("# H1\n# H2").unwrap(), "# H1\n\n# H2");
		assert_eq!(format_blocker_content("## H2a\n## H2b").unwrap(), "## H2a\n\n## H2b");

		// Empty line when going from lower rank (more #) to higher rank (fewer #)
		assert_eq!(format_blocker_content("## H2\n# H1").unwrap(), "## H2\n\n# H1");
		assert_eq!(format_blocker_content("### H3\n# H1").unwrap(), "### H3\n\n# H1");
		assert_eq!(format_blocker_content("### H3\n## H2").unwrap(), "### H3\n\n## H2");

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

	#[test]
	fn test_space_indented_comments_converted_to_tabs() {
		// Comments with leading spaces (e.g., from editor tab-to-space conversion) should be converted to tab-indented
		let input = "- Task 1\n    Comment with 4 spaces\n- Task 2";
		let expected = "- Task 1\n\tComment with 4 spaces\n- Task 2";
		assert_eq!(format_blocker_content(input).unwrap(), expected);

		// Multiple space-indented comments
		let input2 = "- Task 1\n    Comment 1\n    Comment 2\n- Task 2";
		let expected2 = "- Task 1\n\tComment 1\n\tComment 2\n- Task 2";
		assert_eq!(format_blocker_content(input2).unwrap(), expected2);

		// Mixed: some tabs, some spaces (should normalize to tabs)
		let input3 = "- Task 1\n\tTab comment\n    Space comment\n- Task 2";
		let expected3 = "- Task 1\n\tTab comment\n\tSpace comment\n- Task 2";
		assert_eq!(format_blocker_content(input3).unwrap(), expected3);

		// Comments with varying amounts of leading spaces (2+ spaces)
		let input4 = "- Task 1\n  Comment with 2 spaces\n   Comment with 3 spaces\n      Comment with 6 spaces";
		let expected4 = "- Task 1\n\tComment with 2 spaces\n\tComment with 3 spaces\n\tComment with 6 spaces";
		assert_eq!(format_blocker_content(input4).unwrap(), expected4);

		// Space-indented comments after headers
		let input5 = "# Section 1\n- Task 1\n    Comment about task 1";
		let expected5 = "# Section 1\n- Task 1\n\tComment about task 1";
		assert_eq!(format_blocker_content(input5).unwrap(), expected5);
	}

	#[test]
	fn test_space_indented_comments_edge_cases() {
		// Single space should NOT be treated as comment (too ambiguous)
		let input = "- Task 1\n Content with one space";
		let expected = "- Task 1\n- Content with one space";
		assert_eq!(format_blocker_content(input).unwrap(), expected);

		// Space-indented list items (with -) should remain as items, not become comments
		let input2 = "- Task 1\n  - Subtask with 2 spaces and dash";
		let expected2 = "- Task 1\n- Subtask with 2 spaces and dash";
		assert_eq!(format_blocker_content(input2).unwrap(), expected2);

		// Idempotency: formatting space-indented comments twice should yield same result
		let input3 = "- Task 1\n    Comment";
		let formatted_once = format_blocker_content(input3).unwrap();
		let formatted_twice = format_blocker_content(&formatted_once).unwrap();
		assert_eq!(formatted_once, formatted_twice);
		assert_eq!(formatted_once, "- Task 1\n\tComment");
	}

	#[test]
	fn test_line_type_methods() {
		let h1 = LineType::Header {
			level: HeaderLevel::One,
			text: "Test".to_string(),
		};
		let h2 = LineType::Header {
			level: HeaderLevel::Two,
			text: "Test".to_string(),
		};
		let item = LineType::Item;
		let comment = LineType::Comment;

		// Test is_header
		assert!(h1.is_header());
		assert!(h2.is_header());
		assert!(!item.is_header());
		assert!(!comment.is_header());

		// Test header_level
		assert_eq!(h1.header_level(), Some(HeaderLevel::One));
		assert_eq!(h2.header_level(), Some(HeaderLevel::Two));
		assert_eq!(item.header_level(), None);
		assert_eq!(comment.header_level(), None);

		// Test header_text
		assert_eq!(h1.header_text(), Some("Test"));
		assert_eq!(h2.header_text(), Some("Test"));
		assert_eq!(item.header_text(), None);
		assert_eq!(comment.header_text(), None);

		// Test is_content
		assert!(h1.is_content());
		assert!(h2.is_content());
		assert!(item.is_content());
		assert!(!comment.is_content());
	}

	#[test]
	fn test_header_level_ordering() {
		// Test that HeaderLevel has proper ordering (One < Two < Three < Four < Five)
		assert!(HeaderLevel::One < HeaderLevel::Two);
		assert!(HeaderLevel::Two < HeaderLevel::Three);
		assert!(HeaderLevel::Three < HeaderLevel::Four);
		assert!(HeaderLevel::Four < HeaderLevel::Five);

		// Test to_usize
		assert_eq!(HeaderLevel::One.to_usize(), 1);
		assert_eq!(HeaderLevel::Two.to_usize(), 2);
		assert_eq!(HeaderLevel::Three.to_usize(), 3);
		assert_eq!(HeaderLevel::Four.to_usize(), 4);
		assert_eq!(HeaderLevel::Five.to_usize(), 5);

		// Test from_usize
		assert_eq!(HeaderLevel::from_usize(1), Some(HeaderLevel::One));
		assert_eq!(HeaderLevel::from_usize(2), Some(HeaderLevel::Two));
		assert_eq!(HeaderLevel::from_usize(3), Some(HeaderLevel::Three));
		assert_eq!(HeaderLevel::from_usize(4), Some(HeaderLevel::Four));
		assert_eq!(HeaderLevel::from_usize(5), Some(HeaderLevel::Five));
		assert_eq!(HeaderLevel::from_usize(6), None);
		assert_eq!(HeaderLevel::from_usize(0), None);
	}

	#[test]
	fn test_parse_parent_headers_simple() {
		let content = "# Project A\n- task 1";
		let headers = parse_parent_headers(content, "- task 1");
		assert_eq!(headers, vec!["Project A"]);
	}

	#[test]
	fn test_parse_parent_headers_nested() {
		let content = "# Project A\n## Feature B\n### Component C\n- task 1";
		let headers = parse_parent_headers(content, "- task 1");
		assert_eq!(headers, vec!["Project A", "Feature B", "Component C"]);
	}

	#[test]
	fn test_parse_parent_headers_with_siblings() {
		let content = "# Project A\n## Feature B\n- task 1\n## Feature C\n- task 2";
		let headers = parse_parent_headers(content, "- task 2");
		assert_eq!(headers, vec!["Project A", "Feature C"]);
	}

	#[test]
	fn test_parse_parent_headers_skip_comments() {
		let content = "# Project A\n\tComment here\n## Feature B\n\tAnother comment\n- task 1";
		let headers = parse_parent_headers(content, "- task 1");
		assert_eq!(headers, vec!["Project A", "Feature B"]);
	}

	#[test]
	fn test_parse_parent_headers_no_headers() {
		let content = "- task 1\n- task 2\n- task 3";
		let headers = parse_parent_headers(content, "- task 3");
		assert_eq!(headers, Vec::<String>::new());
	}

	#[test]
	fn test_parse_parent_headers_multiple_levels_skipped() {
		// Should only get direct ancestors, skipping intermediate levels
		let content = "# Level 1\n### Level 3\n- task 1";
		let headers = parse_parent_headers(content, "- task 1");
		assert_eq!(headers, vec!["Level 1", "Level 3"]);
	}

	#[test]
	fn test_typst_to_markdown_headings() {
		// Test Typst heading conversion (= to #)
		let typst_input = "= Level 1\n== Level 2\n=== Level 3";
		let expected = "# Level 1\n## Level 2\n### Level 3";
		assert_eq!(typst_to_markdown(typst_input).unwrap(), expected);
	}

	#[test]
	fn test_typst_to_markdown_lists() {
		// Test Typst bullet list (same as markdown)
		let typst_input = "- First item\n- Second item";
		let expected = "- First item\n- Second item";
		assert_eq!(typst_to_markdown(typst_input).unwrap(), expected);
	}

	#[test]
	fn test_typst_to_markdown_enum_lists() {
		// Test Typst numbered list conversion
		let typst_input = "+ First\n+ Second";
		let markdown = typst_to_markdown(typst_input).unwrap();
		// Should convert to markdown list items
		assert!(markdown.contains("- First"));
		assert!(markdown.contains("- Second"));
	}

	#[test]
	fn test_typst_to_markdown_mixed() {
		// Test mixed content
		let typst_input = "= Project\n- task 1\n- task 2";
		let markdown = typst_to_markdown(typst_input).unwrap();
		assert!(markdown.contains("# Project"));
		assert!(markdown.contains("- task 1"));
		assert!(markdown.contains("- task 2"));
	}

	#[test]
	fn test_normalize_content_markdown() {
		use std::path::PathBuf;
		let content = "# Header\n- item";
		let path = PathBuf::from("test.md");
		// For .md files, content should pass through unchanged
		assert_eq!(normalize_content_by_extension(content, &path).unwrap(), content);
	}

	#[test]
	fn test_normalize_content_typst() {
		use std::path::PathBuf;
		let content = "= Header\n- item";
		let path = PathBuf::from("test.typ");
		// For .typ files, should convert to markdown
		let result = normalize_content_by_extension(content, &path).unwrap();
		assert!(result.contains("# Header"));
		assert!(result.contains("- item"));
	}

	#[test]
	fn test_normalize_content_plain() {
		use std::path::PathBuf;
		let content = "plain text\nmore text";
		let path = PathBuf::from("test.txt");
		// For other extensions, content should pass through unchanged
		assert_eq!(normalize_content_by_extension(content, &path).unwrap(), content);
	}

	#[test]
	fn test_is_semantically_empty() {
		// Empty string is semantically empty
		assert!(is_semantically_empty(""));

		// Only whitespace is semantically empty
		assert!(is_semantically_empty("   \n\n  \n"));

		// Only comments is semantically empty
		assert!(is_semantically_empty("\tComment 1\n\tComment 2"));

		// Comments and whitespace is semantically empty
		assert!(is_semantically_empty("\tComment\n\n\tAnother comment\n"));

		// Any content makes it not empty
		assert!(!is_semantically_empty("- Task 1"));
		assert!(!is_semantically_empty("# Header"));
		assert!(!is_semantically_empty("\tComment\n- Task"));
		assert!(!is_semantically_empty("# Header\n\tComment"));
	}

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

	#[test]
	fn test_format_idempotent_with_same_level_headers_at_end() {
		// Bug: when opening and closing a file, we fail to add spaces between
		// the headers of the same level at the end
		let input = "- move these todos over into a persisted directory\n\tcomment\n- move all typst projects\n- rewrite custom.sh\n\tcomment\n\n# marketmonkey\n- go in-depth on possibilities\n\n# SocialNetworks in rust\n- test twitter\n\n## yt\n- test\n\n# math tools\n## gauss\n- finish it\n- move gaussian pivot over in there\n\n# git lfs: docs, music, etc\n# eww: don't restore if outdated\n# todo: blocker: doesn't add spaces between same level headers";

		// First format
		let formatted_once = format_blocker_content(input).unwrap();

		// Simulate file write and read (write doesn't add trailing newline, read doesn't care)
		// This is what happens in handle_background_blocker_check
		let formatted_twice = format_blocker_content(&formatted_once).unwrap();

		// Check that there are spaces between same-level headers at the end
		assert!(
			formatted_once.contains("# git lfs: docs, music, etc\n\n# eww: don't restore if outdated"),
			"Missing space between first two headers"
		);
		assert!(
			formatted_once.contains("# eww: don't restore if outdated\n\n# todo: blocker: doesn't add spaces between same level headers"),
			"Missing space between last two headers"
		);

		// Should be idempotent
		assert_eq!(formatted_once, formatted_twice, "Formatting should be idempotent");
	}
}
