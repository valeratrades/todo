use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, bail, eyre};

use crate::config::LiveSettings;

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
	Current,
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
	use std::process::Command;

	let issues_dir = issues_dir();
	if !issues_dir.exists() {
		return Ok(Vec::new());
	}

	let output = Command::new("find")
		.args([issues_dir.to_str().unwrap(), "(", "-name", "*.md", "-o", "-name", "*.typ", ")", "-type", "f", "!", "-name", ".*"])
		.output()?;

	if !output.status.success() {
		return Err(eyre!("Failed to search for issue files"));
	}

	let all_files = String::from_utf8(output.stdout)?;
	let mut matches = Vec::new();

	let pattern_lower = pattern.to_lowercase();
	let pattern_sanitized = sanitize_title_for_filename(pattern).to_lowercase();

	for line in all_files.lines() {
		let file_path = line.trim();
		if file_path.is_empty() {
			continue;
		}

		let path = PathBuf::from(file_path);

		let relative = if let Ok(rel) = path.strip_prefix(&issues_dir) {
			rel.to_string_lossy().to_string()
		} else {
			continue;
		};

		let relative_lower = relative.to_lowercase();

		if let Some(file_stem) = path.file_stem() {
			let file_stem_str = file_stem.to_string_lossy().to_lowercase();
			if file_stem_str.contains(&pattern_lower) || relative_lower.contains(&pattern_lower) || (!pattern_sanitized.is_empty() && file_stem_str.contains(&pattern_sanitized)) {
				matches.push(path);
				continue;
			}
		}

		if let Some(title) = extract_issue_title_from_file(&path) {
			if title.to_lowercase().contains(&pattern_lower) {
				matches.push(path);
			}
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

/// A blocker entry parsed from the issue file
#[derive(Clone, Debug, PartialEq)]
pub struct BlockerEntry {
	/// The blocker text
	pub text: String,
	/// Whether this blocker is completed (checked)
	pub completed: bool,
}

/// Parse blockers from an issue file.
/// Blockers are any semantic list (ordered or unordered) at the END of the issue body.
/// Returns the list of blocker entries in order.
pub fn parse_blockers_from_issue(content: &str) -> Vec<BlockerEntry> {
	// Normalize indentation (same as open.rs)
	let normalized = normalize_issue_indentation(content);
	let lines: Vec<&str> = normalized.lines().collect();

	// Find where the body content ends (before sub-issues and comments)
	// Body content is indented with one tab, ends when we see a sub-issue or comment marker
	let mut body_end_idx = lines.len();
	let mut in_body = false;

	for (idx, line) in lines.iter().enumerate() {
		// First line is the issue title
		if idx == 0 {
			continue;
		}

		// Skip labels line
		let stripped = line.strip_prefix('\t').unwrap_or(line);
		if stripped.starts_with("**Labels:**") || stripped.starts_with("*Labels:*") {
			continue;
		}

		// Check for sub-issue marker (md: <!--sub, typ: // sub)
		if (stripped.contains("<!--sub ") && stripped.contains("-->")) || stripped.contains(" // sub ") {
			body_end_idx = idx;
			break;
		}

		// Check for comment marker (md: <!--url#issuecomment or <!--new comment-->)
		// typ: // url#issuecomment or // new comment
		if (stripped.contains("<!--") && stripped.contains("#issuecomment"))
			|| stripped.contains("<!--new comment-->")
			|| stripped.contains("// new comment")
			|| (stripped.starts_with("// ") && stripped.contains("#issuecomment"))
		{
			body_end_idx = idx;
			break;
		}

		// Track when we're in the body (indented content after title)
		if line.starts_with('\t') && !in_body {
			in_body = true;
		}
	}

	// Now parse from the end of the body backwards to find the semantic list
	// A semantic list is consecutive lines that are list items (- item, * item, 1. item, etc.)
	let body_lines: Vec<&str> = lines[1..body_end_idx]
		.iter()
		.filter(|l| l.starts_with('\t')) // Only body content (indented)
		.map(|l| l.strip_prefix('\t').unwrap_or(l))
		.collect();

	// Find the last contiguous block of list items
	let mut blockers: Vec<BlockerEntry> = Vec::new();
	let mut in_list = false;
	let mut list_start_idx = 0;

	for (idx, line) in body_lines.iter().enumerate() {
		let trimmed = line.trim();

		// Check if this is a list item
		let is_list_item = is_list_item(trimmed);

		if is_list_item {
			if !in_list {
				in_list = true;
				list_start_idx = idx;
			}
		} else if !trimmed.is_empty() {
			// Non-empty non-list line breaks the list
			in_list = false;
		}
		// Empty lines within a list are allowed
	}

	// If we ended in a list, parse those items as blockers
	if in_list {
		for line in &body_lines[list_start_idx..] {
			let trimmed = line.trim();
			if let Some(entry) = parse_list_item(trimmed) {
				blockers.push(entry);
			}
		}
	}

	blockers
}

/// Check if a line is a list item (ordered or unordered)
fn is_list_item(line: &str) -> bool {
	let trimmed = line.trim();

	// Unordered: - item, * item, + item
	if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
		return true;
	}

	// Checkbox: - [ ] item, - [x] item
	if trimmed.starts_with("- [ ] ") || trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
		return true;
	}

	// Ordered: 1. item, 2. item, etc.
	if let Some(dot_pos) = trimmed.find(". ") {
		let num_part = &trimmed[..dot_pos];
		if num_part.chars().all(|c| c.is_ascii_digit()) {
			return true;
		}
	}

	false
}

/// Parse a list item into a BlockerEntry
fn parse_list_item(line: &str) -> Option<BlockerEntry> {
	let trimmed = line.trim();

	// Checkbox items
	if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
		return Some(BlockerEntry {
			text: rest.trim().to_string(),
			completed: false,
		});
	}
	if let Some(rest) = trimmed.strip_prefix("- [x] ").or_else(|| trimmed.strip_prefix("- [X] ")) {
		return Some(BlockerEntry {
			text: rest.trim().to_string(),
			completed: true,
		});
	}

	// Unordered items
	if let Some(rest) = trimmed.strip_prefix("- ") {
		return Some(BlockerEntry {
			text: rest.trim().to_string(),
			completed: false,
		});
	}
	if let Some(rest) = trimmed.strip_prefix("* ") {
		return Some(BlockerEntry {
			text: rest.trim().to_string(),
			completed: false,
		});
	}
	if let Some(rest) = trimmed.strip_prefix("+ ") {
		return Some(BlockerEntry {
			text: rest.trim().to_string(),
			completed: false,
		});
	}

	// Ordered items
	if let Some(dot_pos) = trimmed.find(". ") {
		let num_part = &trimmed[..dot_pos];
		if num_part.chars().all(|c| c.is_ascii_digit()) {
			let rest = &trimmed[dot_pos + 2..];
			return Some(BlockerEntry {
				text: rest.trim().to_string(),
				completed: false,
			});
		}
	}

	None
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

			println!("Set blocker file: {}", rel_path);

			// Show current blocker if any
			let content = std::fs::read_to_string(&issue_path)?;
			let blockers = parse_blockers_from_issue(&content);
			if let Some(current) = blockers.iter().filter(|b| !b.completed).last() {
				println!("Current blocker: {}", current.text);
			} else if blockers.is_empty() {
				println!("No blockers found in issue body.");
			} else {
				println!("All blockers completed!");
			}
		}

		Command::List => {
			let issue_path = get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker-rewrite set <pattern>` first."))?;

			let content = std::fs::read_to_string(&issue_path)?;
			let blockers = parse_blockers_from_issue(&content);

			if blockers.is_empty() {
				println!("No blockers found in issue body.");
			} else {
				for blocker in &blockers {
					let marker = if blocker.completed { "[x]" } else { "[ ]" };
					println!("{} {}", marker, blocker.text);
				}
			}
		}

		Command::Current => {
			let issue_path = get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker-rewrite set <pattern>` first."))?;

			let content = std::fs::read_to_string(&issue_path)?;
			let blockers = parse_blockers_from_issue(&content);

			// Current blocker is the last non-completed one
			if let Some(current) = blockers.iter().filter(|b| !b.completed).last() {
				const MAX_LEN: usize = 70;
				match current.text.len() {
					0..=MAX_LEN => println!("{}", current.text),
					_ => println!("{}...", &current.text[..(MAX_LEN - 3)]),
				}
			} else if blockers.is_empty() {
				// No blockers - silently exit (for status line integration)
			} else {
				// All completed - silently exit
			}
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_blockers_simple_list() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	**Labels:** bug
	This is the issue body with some description.

	- First blocker
	- Second blocker
	- Third blocker
"#;
		let blockers = parse_blockers_from_issue(content);
		assert_eq!(blockers.len(), 3);
		assert_eq!(blockers[0].text, "First blocker");
		assert_eq!(blockers[1].text, "Second blocker");
		assert_eq!(blockers[2].text, "Third blocker");
		assert!(!blockers[0].completed);
	}

	#[test]
	fn test_parse_blockers_checkbox_list() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body text here.

	- [x] Completed task
	- [ ] Pending task
	- [x] Another completed
"#;
		let blockers = parse_blockers_from_issue(content);
		assert_eq!(blockers.len(), 3);
		assert!(blockers[0].completed);
		assert!(!blockers[1].completed);
		assert!(blockers[2].completed);
	}

	#[test]
	fn test_parse_blockers_ordered_list() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Some body content.

	1. First step
	2. Second step
	3. Third step
"#;
		let blockers = parse_blockers_from_issue(content);
		assert_eq!(blockers.len(), 3);
		assert_eq!(blockers[0].text, "First step");
		assert_eq!(blockers[1].text, "Second step");
		assert_eq!(blockers[2].text, "Third step");
	}

	#[test]
	fn test_parse_blockers_stops_at_subissue() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body.

	- Blocker one
	- Blocker two
	- [ ] Sub-issue <!--sub https://github.com/owner/repo/issues/2-->
"#;
		let blockers = parse_blockers_from_issue(content);
		assert_eq!(blockers.len(), 2);
		assert_eq!(blockers[0].text, "Blocker one");
		assert_eq!(blockers[1].text, "Blocker two");
	}

	#[test]
	fn test_parse_blockers_stops_at_comment() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Body.

	- Blocker one
	- Blocker two

	<!--https://github.com/owner/repo/issues/1#issuecomment-123-->
	Comment body here.
"#;
		let blockers = parse_blockers_from_issue(content);
		assert_eq!(blockers.len(), 2);
	}

	#[test]
	fn test_parse_blockers_no_list() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Just some regular body text without any list.
	More text here.
"#;
		let blockers = parse_blockers_from_issue(content);
		assert!(blockers.is_empty());
	}

	#[test]
	fn test_parse_blockers_list_in_middle_not_end() {
		let content = r#"- [ ] Issue Title <!--https://github.com/owner/repo/issues/1-->
	Some intro text.

	- List item one
	- List item two

	And then more regular text after the list.
	This should mean the list is NOT blockers.
"#;
		let blockers = parse_blockers_from_issue(content);
		// The list is not at the end, so no blockers
		assert!(blockers.is_empty());
	}

	#[test]
	fn test_normalize_indentation() {
		let content = "- [ ] Title <!--url-->\n    Body with 4 spaces\n    - List item";
		let normalized = normalize_issue_indentation(content);
		assert!(normalized.contains("\tBody with 4 spaces"));
		assert!(normalized.contains("\t- List item"));
	}
}
