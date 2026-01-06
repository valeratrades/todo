use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};
use todo::Extension;

use crate::{
	blocker::{self, LineType},
	config::LiveSettings,
	marker::Marker,
};

/// Returns the base directory for issue storage: XDG_DATA_HOME/todo/issues/
fn issues_dir() -> PathBuf {
	v_utils::xdg_data_dir!("issues")
}

/// Cache file for current blocker selection
static CURRENT_BLOCKER_ISSUE_CACHE: &str = "current_blocker_issue.txt";

#[derive(Args, Clone, Debug)]
pub struct BlockerRewriteArgs {
	#[command(subcommand)]
	command: Command,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
	/// Set the current blocker file (uses same matching as `open`)
	Set {
		/// Pattern to match issue file (number, title, owner/repo pattern)
		pattern: String,
	},
	/// List all blockers from the linked issue file
	List,
	/// Show the current blocker (last item in the blockers list)
	Current {
		/// Show fully-qualified path with project prepended
		#[arg(short = 'f', long)]
		fully_qualified: bool,
	},
	/// Open the current blocker issue file with $EDITOR
	Open {
		/// Optional pattern to open a different issue file (uses same matching as `set`)
		pattern: Option<String>,
		/// Set the opened file as current blocker file after exiting the editor
		#[arg(short = 's', long)]
		set_after: bool,
	},
	/// Pop the last blocker from the blockers section
	Pop,
}

/// Get the path to the current blocker issue cache file
fn get_current_blocker_cache_path() -> PathBuf {
	v_utils::xdg_cache_file!(CURRENT_BLOCKER_ISSUE_CACHE)
}

/// Get the currently selected blocker issue file path
fn get_current_blocker_issue() -> Option<PathBuf> {
	let cache_path = get_current_blocker_cache_path();
	std::fs::read_to_string(&cache_path).ok().map(|s| PathBuf::from(s.trim())).filter(|p| p.exists())
}

/// Set the current blocker issue file path
fn set_current_blocker_issue(path: &Path) -> Result<()> {
	let cache_path = get_current_blocker_cache_path();
	std::fs::write(&cache_path, path.to_string_lossy().as_bytes())?;
	Ok(())
}

/// Sanitize a title for use in filenames.
/// Converts spaces to underscores and removes special characters.
fn sanitize_title_for_filename(title: &str) -> String {
	title
		.chars()
		.map(|c| {
			if c.is_alphanumeric() || c == '-' || c == '_' {
				c
			} else if c == ' ' {
				'_'
			} else {
				'\0'
			}
		})
		.filter(|&c| c != '\0')
		.collect::<String>()
		.trim_matches('_')
		.to_string()
}

/// Extract the issue title from the first line of an issue file.
fn extract_issue_title_from_file(path: &Path) -> Option<String> {
	let content = std::fs::read_to_string(path).ok()?;
	let first_line = content.lines().next()?;
	let line = first_line.trim();

	// Strip checkbox prefix
	let rest = line.strip_prefix("- [ ] ").or_else(|| line.strip_prefix("- [x] ")).or_else(|| line.strip_prefix("- [X] "))?;

	// Strip trailing marker (markdown: <!--...-->, typst: // ...)
	let title = if let Some(pos) = rest.find("<!--") {
		rest[..pos].trim()
	} else if let Some(pos) = rest.find(" // ") {
		rest[..pos].trim()
	} else {
		rest.trim()
	};

	if title.is_empty() { None } else { Some(title.to_string()) }
}

/// Search for issue files matching a pattern
/// Uses exact same logic as open.rs
fn search_issue_files(pattern: &str) -> Result<Vec<PathBuf>> {
	let issues_dir = issues_dir();
	if !issues_dir.exists() {
		return Ok(Vec::new());
	}

	// Search for .md, .typ, and their .bak variants (closed issues)
	let all_files = crate::utils::fd(&["-t", "f", "-e", "md", "-e", "typ", "-e", "bak", "--exclude", ".*"], &issues_dir)?;
	let mut matches = Vec::new();

	let pattern_lower = pattern.to_lowercase();
	let pattern_sanitized = sanitize_title_for_filename(pattern).to_lowercase();

	for line in all_files.lines() {
		let relative_path = line.trim();
		if relative_path.is_empty() {
			continue;
		}

		let path = issues_dir.join(relative_path);
		let relative_lower = relative_path.to_lowercase();

		if let Some(file_stem) = path.file_stem() {
			let file_stem_str = file_stem.to_string_lossy().to_lowercase();
			if file_stem_str.contains(&pattern_lower) || relative_lower.contains(&pattern_lower) || (!pattern_sanitized.is_empty() && file_stem_str.contains(&pattern_sanitized)) {
				matches.push(path);
				continue;
			}
		}

		if let Some(title) = extract_issue_title_from_file(&path)
			&& title.to_lowercase().contains(&pattern_lower)
		{
			matches.push(path);
		}
	}

	Ok(matches)
}

/// Use fzf to let user choose from multiple issue file matches
fn choose_issue_with_fzf(matches: &[PathBuf], initial_query: &str) -> Result<Option<PathBuf>> {
	use std::{
		io::Write as IoWrite,
		process::{Command, Stdio},
	};

	let issues_dir = issues_dir();

	let input: String = matches
		.iter()
		.filter_map(|p| p.strip_prefix(&issues_dir).ok().map(|r| r.to_string_lossy().to_string()))
		.collect::<Vec<_>>()
		.join("\n");

	let mut fzf = Command::new("fzf").args(["--query", initial_query]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;

	if let Some(stdin) = fzf.stdin.take() {
		let mut stdin_handle = stdin;
		stdin_handle.write_all(input.as_bytes())?;
	}

	let output = fzf.wait_with_output()?;

	if output.status.success() {
		let chosen = String::from_utf8(output.stdout)?.trim().to_string();
		Ok(Some(issues_dir.join(chosen)))
	} else {
		Ok(None)
	}
}

/// Extract blockers section from an issue file.
/// Looks for `<!--blockers-->` marker (md) or `// blockers` (typst) in the body.
/// Returns the content from that marker up to either:
/// - End of body (before sub-issues or comments)
/// - End of file
///
/// The returned content uses the same format as blocker.rs files.
fn extract_blockers_section(content: &str) -> Option<String> {
	let normalized = normalize_issue_indentation(content);
	let lines: Vec<&str> = normalized.lines().collect();

	// Find the blockers marker line
	let mut blockers_start_idx = None;
	let mut body_end_idx = lines.len();

	for (idx, line) in lines.iter().enumerate() {
		// Skip the issue title (first line)
		if idx == 0 {
			continue;
		}

		let stripped = line.strip_prefix('\t').unwrap_or(line);

		// Check for blockers marker (must be in body, so indented)
		if line.starts_with('\t') && blockers_start_idx.is_none() && matches!(Marker::decode(stripped, Extension::Md), Some(Marker::BlockersSection(_))) {
			blockers_start_idx = Some(idx + 1); // Start from line after marker
			continue;
		}

		// Check for sub-issue - any `- [ ]` or `- [x]` at same indent level (one tab) ends blockers
		// This catches both marked sub-issues and newly added ones without markers
		if line.starts_with('\t') && !line.starts_with("\t\t") && blockers_start_idx.is_some() {
			let trimmed = stripped.trim();
			if trimmed.starts_with("- [ ] ") || trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
				body_end_idx = idx;
				break;
			}
		}

		// Check for comment marker - this also ends the body
		if matches!(Marker::decode(stripped, Extension::Md), Some(Marker::Comment { .. } | Marker::NewComment)) {
			body_end_idx = idx;
			break;
		}
	}

	// If no blockers marker found, return None
	let start_idx = blockers_start_idx?;

	// Extract lines from blockers marker to body end
	// Remove the leading tab (body indent) from each line
	let blockers_lines: Vec<&str> = lines[start_idx..body_end_idx].iter().map(|l| l.strip_prefix('\t').unwrap_or(l)).collect();

	if blockers_lines.is_empty() {
		return None;
	}

	Some(blockers_lines.join("\n"))
}

/// Get the current blocker from the blockers section.
/// Uses the same logic as blocker.rs: last non-comment content line.
fn get_current_blocker_from_content(blockers_content: &str) -> Option<String> {
	blockers_content
		.lines()
		// Skip comment lines (tab-indented) - only consider content lines
		.rfind(|s: &&str| !s.is_empty() && !s.starts_with('\t'))
		.map(|s| s.to_owned())
}

/// Get the current blocker with parent headers prepended.
/// Uses blocker.rs logic for parsing headers.
/// If `fully_qualified` is true and `issue_path` is provided, prepends the issue title.
fn get_current_blocker_with_headers(blockers_content: &str, fully_qualified: bool, issue_path: Option<&Path>) -> Option<String> {
	let current = get_current_blocker_from_content(blockers_content)?;
	let stripped = blocker::strip_blocker_prefix(&current);

	let parent_headers = blocker::parse_parent_headers(blockers_content, &current);

	// Build final output with parent headers
	let mut parts = Vec::new();

	// Add project name if fully_qualified is true
	if fully_qualified && let Some(path) = issue_path {
		// Extract project name from issue file path (filename without extension)
		let project_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
		if !project_name.is_empty() {
			parts.push(project_name.to_string());
		}
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

/// Pop the last content line from the blockers section.
/// Returns the modified blockers content.
fn pop_last_blocker(blockers_content: &str) -> String {
	let lines: Vec<&str> = blockers_content.lines().collect();
	let mut content_line_indices: Vec<usize> = Vec::new();

	// Find indices of all content lines (headers and items, not comments)
	for (idx, line) in lines.iter().enumerate() {
		if let Some(line_type) = blocker::classify_line(line)
			&& line_type.is_content()
		{
			content_line_indices.push(idx);
		}
	}

	// Remove the last content line
	if let Some(&last_content_idx) = content_line_indices.last() {
		// Keep lines before the last content line
		let new_lines: Vec<&str> = lines.iter().enumerate().filter(|(idx, _)| *idx < last_content_idx).map(|(_, line)| *line).collect();

		new_lines.join("\n")
	} else {
		// No content lines to remove
		blockers_content.to_string()
	}
}

/// Update the blockers section in an issue file.
/// Replaces the content between <!--blockers--> marker and the next section marker.
/// Returns the updated full file content.
fn update_blockers_in_issue(full_content: &str, new_blockers_content: &str) -> Option<String> {
	let normalized = normalize_issue_indentation(full_content);
	let lines: Vec<&str> = normalized.lines().collect();

	// Find the blockers marker line
	let mut blockers_start_idx = None;
	let mut blockers_end_idx = lines.len();

	for (idx, line) in lines.iter().enumerate() {
		// Skip the issue title (first line)
		if idx == 0 {
			continue;
		}

		let stripped = line.strip_prefix('\t').unwrap_or(line);

		// Check for blockers marker (must be in body, so indented)
		if line.starts_with('\t') && blockers_start_idx.is_none() && matches!(Marker::decode(stripped, Extension::Md), Some(Marker::BlockersSection(_))) {
			blockers_start_idx = Some(idx + 1); // Start from line after marker
			continue;
		}

		// Check for sub-issue - any `- [ ]` or `- [x]` at same indent level (one tab) ends blockers
		if line.starts_with('\t') && !line.starts_with("\t\t") && blockers_start_idx.is_some() {
			let trimmed = stripped.trim();
			if trimmed.starts_with("- [ ] ") || trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
				blockers_end_idx = idx;
				break;
			}
		}

		// Check for comment marker - this also ends the body
		if matches!(Marker::decode(stripped, Extension::Md), Some(Marker::Comment { .. } | Marker::NewComment)) {
			blockers_end_idx = idx;
			break;
		}
	}

	// If no blockers marker found, return None
	let start_idx = blockers_start_idx?;

	// Build the new content
	let mut result_lines: Vec<String> = Vec::new();

	// Lines before blockers section
	for line in &lines[..start_idx] {
		result_lines.push(line.to_string());
	}

	// New blockers content (add body indent)
	for line in new_blockers_content.lines() {
		if line.is_empty() {
			result_lines.push(String::new());
		} else {
			result_lines.push(format!("\t{line}"));
		}
	}

	// Lines after blockers section
	for line in &lines[blockers_end_idx..] {
		result_lines.push(line.to_string());
	}

	Some(result_lines.join("\n"))
}

/// List all blockers from the content.
/// Returns tuples of (text, is_header, is_completed).
fn list_blockers_from_content(blockers_content: &str) -> Vec<(String, bool, bool)> {
	let mut result = Vec::new();

	for line in blockers_content.lines() {
		// Skip empty lines and comments (tab-indented)
		if line.is_empty() || line.starts_with('\t') {
			continue;
		}

		let line_type = blocker::classify_line(line);
		match line_type {
			Some(LineType::Header { text, .. }) => {
				result.push((text, true, false));
			}
			Some(LineType::Item) => {
				// Check if it's a checkbox item
				let trimmed = line.trim();
				let (completed, text) = if let Some(rest) = trimmed.strip_prefix("- [x] ").or_else(|| trimmed.strip_prefix("- [X] ")) {
					(true, rest.to_string())
				} else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
					(false, rest.to_string())
				} else {
					// Regular item (- prefix)
					let text = blocker::strip_blocker_prefix(trimmed);
					(false, text.to_string())
				};
				result.push((text, false, completed));
			}
			_ => {}
		}
	}

	result
}

/// Normalize issue content by converting space-based indentation to tab-based.
/// Same logic as open.rs
fn normalize_issue_indentation(content: &str) -> String {
	let spaces_per_indent = content
		.lines()
		.find_map(|line| {
			if line.starts_with(' ') && !line.trim().is_empty() {
				let spaces = line.len() - line.trim_start_matches(' ').len();
				if spaces >= 2 { Some(spaces.min(8)) } else { None }
			} else {
				None
			}
		})
		.unwrap_or(4);

	content
		.lines()
		.map(|line| {
			if line.is_empty() {
				return String::new();
			}

			let mut chars = line.chars().peekable();
			let mut space_count = 0;
			let mut tab_count = 0;

			while let Some(&ch) = chars.peek() {
				match ch {
					'\t' => {
						tab_count += 1;
						chars.next();
					}
					' ' => {
						space_count += 1;
						chars.next();
					}
					_ => break,
				}
			}

			let extra_tabs = space_count / spaces_per_indent;
			let total_tabs = tab_count + extra_tabs;
			let remaining_spaces = space_count % spaces_per_indent;

			let rest: String = chars.collect();
			let mut result = "\t".repeat(total_tabs);
			result.push_str(&" ".repeat(remaining_spaces));
			result.push_str(&rest);
			result
		})
		.collect::<Vec<_>>()
		.join("\n")
}

pub async fn main(_settings: &LiveSettings, args: BlockerRewriteArgs) -> Result<()> {
	match args.command {
		Command::Set { pattern } => {
			// Use same matching logic as open command
			let matches = search_issue_files(&pattern)?;

			let issue_path = match matches.len() {
				0 => {
					// No matches - open fzf with all files
					let all_files = search_issue_files("")?;
					if all_files.is_empty() {
						bail!("No issue files found. Use `todo open <url>` to fetch an issue first.");
					}
					match choose_issue_with_fzf(&all_files, &pattern)? {
						Some(path) => path,
						None => bail!("No issue selected"),
					}
				}
				1 => matches[0].clone(),
				_ => {
					// Multiple matches - open fzf to choose
					match choose_issue_with_fzf(&matches, &pattern)? {
						Some(path) => path,
						None => bail!("No issue selected"),
					}
				}
			};

			// Set as current blocker issue
			set_current_blocker_issue(&issue_path)?;

			// Get relative path for display
			let issues_dir = issues_dir();
			let rel_path = issue_path
				.strip_prefix(&issues_dir)
				.map(|p| p.to_string_lossy().to_string())
				.unwrap_or_else(|_| issue_path.to_string_lossy().to_string());

			println!("Set blocker file: {rel_path}");

			// Show current blocker if any
			let content = std::fs::read_to_string(&issue_path)?;
			if let Some(blockers_section) = extract_blockers_section(&content) {
				if let Some(current) = get_current_blocker_with_headers(&blockers_section, false, Some(&issue_path)) {
					println!("Current blocker: {current}");
				} else {
					println!("Blockers section is empty.");
				}
			} else {
				println!("No <!--blockers--> marker found in issue body.");
			}
		}

		Command::List => {
			let issue_path = get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker-rewrite set <pattern>` first."))?;

			let content = std::fs::read_to_string(&issue_path)?;

			if let Some(blockers_section) = extract_blockers_section(&content) {
				let blockers = list_blockers_from_content(&blockers_section);

				if blockers.is_empty() {
					println!("Blockers section is empty.");
				} else {
					for (text, is_header, completed) in &blockers {
						if *is_header {
							println!("# {text}");
						} else {
							let marker = if *completed { "[x]" } else { "[ ]" };
							println!("{marker} {text}");
						}
					}
				}
			} else {
				println!("No <!--blockers--> marker found in issue body.");
			}
		}

		Command::Current { fully_qualified } => {
			let issue_path = get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker-rewrite set <pattern>` first."))?;

			let content = std::fs::read_to_string(&issue_path)?;

			if let Some(blockers_section) = extract_blockers_section(&content)
				&& let Some(current) = get_current_blocker_with_headers(&blockers_section, fully_qualified, Some(&issue_path))
			{
				const MAX_LEN: usize = 70;
				match current.len() {
					0..=MAX_LEN => println!("{current}"),
					_ => println!("{}...", &current[..(MAX_LEN - 3)]),
				}
				// No current blocker - silently exit (for status line integration)
			}
			// No blockers section - silently exit
		}

		Command::Open { pattern, set_after } => {
			// Determine which file to open
			let issue_path = if let Some(pat) = pattern {
				// Use pattern matching to find issue file
				let matches = search_issue_files(&pat)?;

				match matches.len() {
					0 => {
						// No matches - open fzf with all files
						let all_files = search_issue_files("")?;
						if all_files.is_empty() {
							bail!("No issue files found. Use `todo open <url>` to fetch an issue first.");
						}
						match choose_issue_with_fzf(&all_files, &pat)? {
							Some(path) => path,
							None => bail!("No issue selected"),
						}
					}
					1 => matches[0].clone(),
					_ => {
						// Multiple matches - open fzf to choose
						match choose_issue_with_fzf(&matches, &pat)? {
							Some(path) => path,
							None => bail!("No issue selected"),
						}
					}
				}
			} else {
				// Use current blocker issue
				get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker-rewrite set <pattern>` first."))?
			};

			// Open the file with $EDITOR
			v_utils::io::file_open::open(&issue_path).await?;

			// If set_after flag is set, update the current blocker issue
			if set_after {
				set_current_blocker_issue(&issue_path)?;

				// Get relative path for display
				let issues_dir = issues_dir();
				let rel_path = issue_path
					.strip_prefix(&issues_dir)
					.map(|p| p.to_string_lossy().to_string())
					.unwrap_or_else(|_| issue_path.to_string_lossy().to_string());

				println!("Set blocker file: {rel_path}");
			}
		}

		Command::Pop => {
			let issue_path = get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker-rewrite set <pattern>` first."))?;

			let content = std::fs::read_to_string(&issue_path)?;

			if let Some(blockers_section) = extract_blockers_section(&content) {
				// Get the current blocker before popping (for display)
				let current_before = get_current_blocker_from_content(&blockers_section);

				// Pop the last blocker
				let new_blockers = pop_last_blocker(&blockers_section);

				// Update the issue file with new blockers section
				if let Some(updated_content) = update_blockers_in_issue(&content, &new_blockers) {
					std::fs::write(&issue_path, updated_content)?;

					if let Some(popped) = current_before {
						let stripped = blocker::strip_blocker_prefix(&popped);
						println!("Popped: {stripped}");
					}

					// Show new current blocker
					if let Some(new_current) = get_current_blocker_with_headers(&new_blockers, false, Some(&issue_path)) {
						println!("Current: {new_current}");
					} else {
						println!("Blockers section is now empty.");
					}
				} else {
					bail!("Failed to update blockers section in issue file.");
				}
			} else {
				bail!("No <!--blockers--> marker found in issue body.");
			}
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use insta::assert_snapshot;

	use super::*;

	#[test]
	fn test_extract_blockers_section_md() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	**Labels:** bug
	This is the issue body.

	<!--blockers-->
	# Phase 1
	- First task
		comment on first task
	- Second task

	# Phase 2
	- Third task
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		# Phase 1
		- First task
			comment on first task
		- Second task

		# Phase 2
		- Third task
		");
	}

	#[test]
	fn test_extract_blockers_section_typst() {
		let content = r#"- [ ] Issue Title // https://github.com/owner/repo/issues/1
	*Labels:* bug
	Body text.

	// blockers
	# Main
	- Do something
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		# Main
		- Do something
		");
	}

	#[test]
	fn test_extract_blockers_stops_at_subissue_with_marker() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body.

	<!--blockers-->
	- Blocker one
	- Blocker two
	- [ ] Sub-issue <!--sub https://github.com/owner/repo/issues/2-->
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		- Blocker one
		- Blocker two
		");
	}

	#[test]
	fn test_extract_blockers_stops_at_subissue_without_marker() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body.

	<!--blockers-->
	- last
	- middle
	- first
	- [ ] new sub-issue I just added
		without any markers around it
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		- last
		- middle
		- first
		");
	}

	#[test]
	fn test_extract_blockers_stops_at_comment() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body.

	<!--blockers-->
	- Blocker one
	- Blocker two

	<!--https://github.com/owner/repo/issues/1#issuecomment-123-->
	Comment body here.
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		- Blocker one
		- Blocker two
		");
	}

	#[test]
	fn test_no_blockers_marker() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Just some regular body text without blockers marker.
	- This is NOT a blocker, just body content.
"#;
		assert!(extract_blockers_section(content).is_none());
	}

	#[test]
	fn test_get_current_blocker() {
		let blockers_content = "# Phase 1\n- First task\n\tcomment\n- Second task\n# Phase 2\n- Third task";
		assert_snapshot!(get_current_blocker_from_content(blockers_content).unwrap(), @"- Third task");
	}

	#[test]
	fn test_get_current_blocker_with_headers() {
		let blockers_content = "# Phase 1\n- First task\n# Phase 2\n- Third task";
		assert_snapshot!(get_current_blocker_with_headers(blockers_content, false, None).unwrap(), @"Phase 2: Third task");
	}

	#[test]
	fn test_get_current_blocker_with_headers_fully_qualified() {
		let blockers_content = "# Phase 1\n- First task\n# Phase 2\n- Third task";
		let issue_path = Path::new("/home/user/issues/my_project.md");
		assert_snapshot!(get_current_blocker_with_headers(blockers_content, true, Some(issue_path)).unwrap(), @"my_project: Phase 2: Third task");
	}

	#[test]
	fn test_list_blockers() {
		let blockers_content = "# Phase 1\n- [x] Completed task\n- [ ] Pending task\n- Regular item";
		let items = list_blockers_from_content(blockers_content);
		assert_snapshot!(format!("{:?}", items), @r#"[("Phase 1", true, false), ("Completed task", false, true), ("Pending task", false, false), ("Regular item", false, false)]"#);
	}

	#[test]
	fn test_normalize_indentation() {
		let content = "- [ ] Title <!--url-->\n    Body with 4 spaces\n    - List item";
		assert_snapshot!(normalize_issue_indentation(content), @"
		- [ ] Title <!--url-->
			Body with 4 spaces
			- List item
		");
	}

	#[test]
	fn test_blockers_marker_flexible_whitespace() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body text.

	<!-- blockers -->
	- Task one
	- Task two
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		- Task one
		- Task two
		");
	}

	#[test]
	fn test_blocker_marker_singular() {
		// Support <!--blocker--> without the 's'
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body text.

	<!--blocker-->
	- Task one
	- Task two
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"
		- Task one
		- Task two
		");
	}

	#[test]
	fn test_blocker_marker_singular_typst() {
		// Support // blocker without the 's'
		let content = r#"- [ ] Issue Title // https://github.com/owner/repo/issues/1
	Body text.

	// blocker
	- Task one
"#;
		assert_snapshot!(extract_blockers_section(content).unwrap(), @"- Task one");
	}

	#[test]
	fn test_pop_last_blocker_simple() {
		let blockers_content = "- First task\n- Second task\n- Third task";
		assert_snapshot!(pop_last_blocker(blockers_content), @"
		- First task
		- Second task
		");
	}

	#[test]
	fn test_pop_last_blocker_with_headers() {
		let blockers_content = "# Phase 1\n- First task\n# Phase 2\n- Third task";
		// Should pop "- Third task", leaving the header
		assert_snapshot!(pop_last_blocker(blockers_content), @"
		# Phase 1
		- First task
		# Phase 2
		");
	}

	#[test]
	fn test_pop_last_blocker_with_comments() {
		let blockers_content = "- First task\n\tcomment on first\n- Second task\n\tcomment on second";
		// Should pop "- Second task", removing its comment too (comments belong to content above)
		assert_snapshot!(pop_last_blocker(blockers_content), @"
		- First task
			comment on first
		");
	}

	#[test]
	fn test_pop_last_blocker_empty() {
		let blockers_content = "";
		assert_snapshot!(pop_last_blocker(blockers_content), @"");
	}

	#[test]
	fn test_pop_last_blocker_only_comments() {
		let blockers_content = "\tcomment only";
		// No content to pop
		assert_snapshot!(pop_last_blocker(blockers_content), @"	comment only");
	}

	#[test]
	fn test_update_blockers_in_issue() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body text.

	<!--blockers-->
	- First task
	- Second task
	- Third task
"#;
		let new_blockers = "- First task\n- Second task";
		assert_snapshot!(update_blockers_in_issue(content, new_blockers).unwrap(), @"
		- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
			Body text.

			<!--blockers-->
			- First task
			- Second task
		");
	}

	#[test]
	fn test_update_blockers_in_issue_with_subissue() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body.

	<!--blockers-->
	- First task
	- Second task
	- [ ] Sub-issue <!--sub https://github.com/owner/repo/issues/2-->
"#;
		let new_blockers = "- First task";
		let result = update_blockers_in_issue(content, new_blockers).unwrap();
		// Should preserve the sub-issue
		assert!(result.contains("- [ ] Sub-issue"));
		assert_snapshot!(result, @"
		- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
			Body.

			<!--blockers-->
			- First task
			- [ ] Sub-issue <!--sub https://github.com/owner/repo/issues/2-->
		");
	}

	#[test]
	fn test_update_blockers_no_marker() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body without blockers marker.
"#;
		assert!(update_blockers_in_issue(content, "- Task").is_none());
	}
}
