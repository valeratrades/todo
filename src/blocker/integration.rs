//! Integration with issue files and the `open` module.
//!
//! This module provides helpers for working with blockers embedded in issue files.
//! The `--integrated` flag on `set` and `open` commands enables use of issue files
//! as the blocker source, using the `open/` module's file management and sync mechanics.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail, eyre};
use todo::Extension;

use super::{operations::BlockerSequence, standard::strip_blocker_prefix};
use crate::{
	marker::Marker,
	open::{
		files::{choose_issue_with_fzf, issues_dir, search_issue_files},
		util::normalize_issue_indentation,
	},
};

/// Cache file for current blocker selection
static CURRENT_BLOCKER_ISSUE_CACHE: &str = "current_blocker_issue.txt";

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

/// Extract blockers section from an issue file.
/// Looks for `<!--blockers-->` marker (md) or `// blockers` (typst) in the body.
/// Returns the content from that marker up to either:
/// - End of body (before sub-issues or comments)
/// - End of file
///
/// The returned content uses the same format as blocker.rs files.
fn extract_blockers_section(content: &str) -> Option<BlockerSequence> {
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

	Some(BlockerSequence::new(blockers_lines.join("\n")))
}

/// Issue-based blocker source for blockers embedded in issue files.
pub struct IssueSource {
	issue_path: PathBuf,
	/// Cached full file content (needed for update_blockers_in_issue)
	full_content: std::cell::RefCell<Option<String>>,
}

impl IssueSource {
	pub fn new(issue_path: PathBuf) -> Self {
		Self {
			issue_path,
			full_content: std::cell::RefCell::new(None),
		}
	}

	/// Get relative path for display
	pub fn display_relative(&self) -> String {
		let issues_dir = issues_dir();
		self.issue_path
			.strip_prefix(&issues_dir)
			.map(|p| p.to_string_lossy().to_string())
			.unwrap_or_else(|_| self.issue_path.to_string_lossy().to_string())
	}
}

impl super::source::BlockerSource for IssueSource {
	fn load(&self) -> Result<String> {
		let content = std::fs::read_to_string(&self.issue_path)?;
		// Cache the full content for later use in save()
		*self.full_content.borrow_mut() = Some(content.clone());

		// Extract just the blockers section
		if let Some(blockers) = extract_blockers_section(&content) {
			Ok(blockers.into_content())
		} else {
			// No blockers section found
			Ok(String::new())
		}
	}

	fn save(&self, content: &str) -> Result<()> {
		// We need the full file content to update the blockers section
		let full_content = self.full_content.borrow();
		let full = full_content.as_ref().ok_or_else(|| eyre!("Must call load() before save()"))?;

		let new_blockers = BlockerSequence::new(content.to_string());
		if let Some(updated) = update_blockers_in_issue(full, &new_blockers) {
			std::fs::write(&self.issue_path, updated)?;
			Ok(())
		} else {
			bail!("Failed to update blockers section in issue file")
		}
	}

	fn display_name(&self) -> String {
		self.display_relative()
	}

	fn path_for_hierarchy(&self) -> Option<PathBuf> {
		Some(self.issue_path.clone())
	}
}

/// Update the blockers section in an issue file.
/// Replaces the content between <!--blockers--> marker and the next section marker.
/// Returns the updated full file content.
fn update_blockers_in_issue(full_content: &str, new_blockers: &BlockerSequence) -> Option<String> {
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
	for line in new_blockers.content().lines() {
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

/// Resolve an issue file from a pattern, using fzf if multiple matches.
fn resolve_issue_file(pattern: &str) -> Result<PathBuf> {
	let matches = search_issue_files(pattern)?;

	match matches.len() {
		0 => {
			// No matches - open fzf with all files
			let all_files = search_issue_files("")?;
			if all_files.is_empty() {
				bail!("No issue files found. Use `todo open <url>` to fetch an issue first.");
			}
			match choose_issue_with_fzf(&all_files, pattern)? {
				Some(path) => Ok(path),
				None => bail!("No issue selected"),
			}
		}
		1 => Ok(matches[0].clone()),
		_ => {
			// Multiple matches - open fzf to choose
			match choose_issue_with_fzf(&matches, pattern)? {
				Some(path) => Ok(path),
				None => bail!("No issue selected"),
			}
		}
	}
}

/// Get the current issue source, or error if none set.
fn get_current_source() -> Result<IssueSource> {
	let issue_path = get_current_blocker_issue().ok_or_else(|| eyre!("No blocker file set. Use `todo blocker -i set <pattern>` first."))?;
	Ok(IssueSource::new(issue_path))
}

/// Main entry point for integrated blocker commands (works with issue files).
/// This is called when `--integrated` flag is set on the blocker command.
pub async fn main_integrated(command: super::io::Command) -> Result<()> {
	use super::{io::Command, source::BlockerSource};

	match command {
		Command::Set { pattern, touch: _ } => {
			// touch is ignored in integrated mode - issue files are managed by `todo open`
			let issue_path = resolve_issue_file(&pattern)?;
			set_current_blocker_issue(&issue_path)?;

			let source = IssueSource::new(issue_path);
			println!("Set blockers to: {}", source.display_name());

			// Load and show current blocker
			let content = source.load()?;
			if content.is_empty() {
				let marker = Marker::BlockersSection(todo::Header::new(1, "Blockers"));
				println!("No `{marker}` marker found in issue body.");
			} else {
				let seq = BlockerSequence::new(content);
				if let Some(current) = seq.current_with_context(&[]) {
					println!("Current: {current}");
				} else {
					println!("Blockers section is empty.");
				}
			}
		}

		Command::Open {
			pattern,
			touch: _,
			set_after,
			urgent: _,
		} => {
			// touch and urgent are ignored in integrated mode
			let issue_path = if let Some(pat) = pattern {
				resolve_issue_file(&pat)?
			} else {
				get_current_blocker_issue().ok_or_else(|| eyre!("No issue set. Use `todo blocker -i set <pattern>` first."))?
			};

			// Open the issue file with $EDITOR
			v_utils::io::file_open::open(&issue_path).await?;

			// If set_after flag is set, update the current blocker issue
			if set_after {
				set_current_blocker_issue(&issue_path)?;
				let source = IssueSource::new(issue_path);
				println!("Set blockers to: {}", source.display_name());
			}
		}

		Command::List => {
			let source = get_current_source()?;
			let content = source.load()?;

			if content.is_empty() {
				let marker = Marker::BlockersSection(todo::Header::new(1, "Blockers"));
				println!("No `{marker}` marker found in issue body.");
			} else {
				let seq = BlockerSequence::new(content);
				let items = seq.list();

				if items.is_empty() {
					println!("Blockers section is empty.");
				} else {
					for (text, is_header) in &items {
						if *is_header {
							println!("# {text}");
						} else {
							println!("- {text}");
						}
					}
				}
			}
		}

		Command::Current { fully_qualified } => {
			let source = get_current_source()?;
			let content = source.load()?;

			if !content.is_empty() {
				let seq = BlockerSequence::new(content);
				let hierarchy = if fully_qualified {
					source
						.path_for_hierarchy()
						.and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
						.map(|s| vec![s])
						.unwrap_or_default()
				} else {
					vec![]
				};

				if let Some(current) = seq.current_with_context(&hierarchy) {
					const MAX_LEN: usize = 70;
					match current.len() {
						0..=MAX_LEN => println!("{current}"),
						_ => println!("{}...", &current[..(MAX_LEN - 3)]),
					}
				}
			}
			// No blockers section or no current blocker - silently exit (for status line integration)
		}

		Command::Pop => {
			let source = get_current_source()?;
			let content = source.load()?;

			if content.is_empty() {
				let marker = Marker::BlockersSection(todo::Header::new(1, "Blockers"));
				bail!("No `{}` marker found in issue body.", marker);
			}

			let mut seq = BlockerSequence::new(content.clone());
			let popped = seq.pop()?;

			// Only save if something was actually popped
			if popped.is_some() {
				source.save(seq.content())?;
			}

			// Output results
			if let Some(popped_line) = popped {
				let stripped = strip_blocker_prefix(&popped_line);
				println!("Popped: {stripped}");
			}

			if let Some(new_current) = seq.current_with_context(&[]) {
				println!("Current: {new_current}");
			} else {
				println!("Blockers section is now empty.");
			}
		}

		Command::Add {
			name: _,
			project: _,
			urgent: _,
			touch: _,
		} => {
			bail!("Add command not supported in integrated mode. Use `todo blocker -i open` to edit the issue file directly.");
		}

		Command::Resume(_) | Command::Halt(_) => {
			bail!("Resume/Halt not yet supported in integrated mode.");
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r#"
  # Phase 1
  - First task
  	comment on first task
  - Second task

  # Phase 2
  - Third task
  "#);
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r#"
  # Main
  - Do something
  "#);
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r"
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r"
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r"
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
		let blockers = BlockerSequence::new("# Phase 1\n- First task\n\tcomment\n- Second task\n# Phase 2\n- Third task".to_string());
		assert_snapshot!(blockers.current().unwrap(), @"- Third task");
	}

	#[test]
	fn test_get_current_blocker_with_context() {
		let blockers = BlockerSequence::new("# Phase 1\n- First task\n# Phase 2\n- Third task".to_string());
		assert_snapshot!(blockers.current_with_context(&[]).unwrap(), @"Phase 2: Third task");
	}

	#[test]
	fn test_get_current_blocker_with_context_fully_qualified() {
		let blockers = BlockerSequence::new("# Phase 1\n- First task\n# Phase 2\n- Third task".to_string());
		let hierarchy = vec!["my_project".to_string()];
		assert_snapshot!(blockers.current_with_context(&hierarchy).unwrap(), @"my_project: Phase 2: Third task");
	}

	#[test]
	fn test_list_blockers() {
		let blockers = BlockerSequence::new("# Phase 1\n- [x] Completed task\n- [ ] Pending task\n- Regular item".to_string());
		let items = blockers.list();
		assert_snapshot!(format!("{:?}", items), @r#"[("Phase 1", true), ("[x] Completed task", false), ("[ ] Pending task", false), ("Regular item", false)]"#);
	}

	#[test]
	fn test_normalize_indentation() {
		let content = "- [ ] Title <!--url-->\n    Body with 4 spaces\n    - List item";
		assert_snapshot!(normalize_issue_indentation(content), @r"
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r"
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @r"
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
		assert_snapshot!(extract_blockers_section(content).unwrap().content(), @"- Task one");
	}

	#[test]
	fn test_pop_last_blocker_simple() {
		let mut blockers = BlockerSequence::new("- First task\n- Second task\n- Third task".to_string());
		blockers.pop().unwrap();
		assert_snapshot!(blockers.content(), @r"
  - First task
  - Second task
  ");
	}

	#[test]
	fn test_pop_last_blocker_with_headers() {
		let mut blockers = BlockerSequence::new("# Phase 1\n- First task\n# Phase 2\n- Third task".to_string());
		// Should pop "- Third task", leaving the header
		blockers.pop().unwrap();
		assert_snapshot!(blockers.content(), @r"
  # Phase 1
  - First task

  # Phase 2
  ");
	}

	#[test]
	fn test_pop_last_blocker_with_comments() {
		let mut blockers = BlockerSequence::new("- First task\n\tcomment on first\n- Second task\n\tcomment on second".to_string());
		// Should pop "- Second task", removing its comment too (comments belong to content above)
		blockers.pop().unwrap();
		assert_snapshot!(blockers.content(), @r"
  - First task
  	comment on first
  ");
	}

	#[test]
	fn test_pop_last_blocker_empty() {
		let mut blockers = BlockerSequence::empty();
		blockers.pop().unwrap();
		assert_snapshot!(blockers.content(), @"");
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
		let new_blockers = BlockerSequence::new("- First task\n- Second task".to_string());
		assert_snapshot!(update_blockers_in_issue(content, &new_blockers).unwrap(), @r"
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
		let new_blockers = BlockerSequence::new("- First task".to_string());
		let result = update_blockers_in_issue(content, &new_blockers).unwrap();
		// Should preserve the sub-issue
		assert!(result.contains("- [ ] Sub-issue"));
		assert_snapshot!(result, @r"
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
		let blockers = BlockerSequence::new("- Task".to_string());
		assert!(update_blockers_in_issue(content, &blockers).is_none());
	}
}
