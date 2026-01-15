//! File path handling for issue storage.

use std::path::{Path, PathBuf};

use todo::FetchedIssue;
use v_utils::prelude::*;

use super::util::Extension;

/// The filename used for the main issue file when it has a directory for sub-issues.
/// When an issue has sub-issues, instead of `123_-_title.md` being a file, it becomes
/// `123_-_title/__main__.md` with sub-issues stored alongside.
pub const MAIN_ISSUE_FILENAME: &str = "__main__";

/// Returns the base directory for issue storage: XDG_DATA_HOME/todo/issues/
pub fn issues_dir() -> PathBuf {
	v_utils::xdg_data_dir!("issues")
}

/// Sanitize a title for use in filenames.
/// Converts spaces to underscores and removes special characters.
pub fn sanitize_title_for_filename(title: &str) -> String {
	title
		.chars()
		.map(|c| {
			if c.is_alphanumeric() || c == '-' || c == '_' {
				c
			} else if c == ' ' {
				'_'
			} else {
				// Skip special characters
				'\0'
			}
		})
		.filter(|&c| c != '\0')
		.collect::<String>()
		.trim_matches('_')
		.to_string()
}

/// Format an issue filename from number and title.
/// Format: {number}_-_{sanitized_title}.{ext} or just {sanitized_title}.{ext} if no number
/// Adds .bak suffix for closed issues.
pub fn format_issue_filename(issue_number: Option<u64>, title: &str, extension: &Extension, closed: bool) -> String {
	let sanitized = sanitize_title_for_filename(title);
	let base = match issue_number {
		Some(num) if sanitized.is_empty() => format!("{num}.{}", extension.as_str()),
		Some(num) => format!("{num}_-_{sanitized}.{}", extension.as_str()),
		None if sanitized.is_empty() => format!("untitled.{}", extension.as_str()),
		None => format!("{sanitized}.{}", extension.as_str()),
	};
	if closed { format!("{base}.bak") } else { base }
}

/// Format a FetchedIssue into a directory name: `{number}_-_{sanitized_title}`
fn format_issue_dir_name(issue: &FetchedIssue) -> String {
	format!("{}_-_{}", issue.number(), sanitize_title_for_filename(&issue.title))
}

/// Get the path for an issue file in XDG_DATA.
/// Structure: issues/{owner}/{repo}/{number}_-_{title}.{ext}[.bak] (or just {title}.{ext} for pending/virtual)
/// For nested sub-issues: issues/{owner}/{repo}/{ancestor1_dir}/{ancestor2_dir}/.../{number}_-_{title}.{ext}[.bak]
///
/// `ancestors` is the chain of parent issues from root to immediate parent (not including the issue itself).
pub fn get_issue_file_path(owner: &str, repo: &str, issue_number: Option<u64>, title: &str, extension: &Extension, closed: bool, ancestors: &[FetchedIssue]) -> PathBuf {
	let mut path = issues_dir().join(owner).join(repo);

	// Build nested directory structure for all ancestors
	for ancestor in ancestors {
		path = path.join(format_issue_dir_name(ancestor));
	}

	let filename = format_issue_filename(issue_number, title, extension, closed);
	path.join(filename)
}

/// Get the project directory path (where meta.json lives).
/// Structure: issues/{owner}/{repo}/
pub fn get_project_dir(owner: &str, repo: &str) -> PathBuf {
	issues_dir().join(owner).join(repo)
}

/// Find the local file path for a sub-issue given its number.
/// Searches in the ancestors' nested directory for files matching either:
/// - The flat pattern: {number}_-_*.{ext}[.bak]
/// - The directory pattern: {number}_-_*/__main__.{ext}[.bak]
/// `ancestors` is the chain from root to immediate parent of the sub-issue.
/// Returns None if no matching file is found.
pub fn find_sub_issue_file(owner: &str, repo: &str, ancestors: &[FetchedIssue], sub_issue_number: u64) -> Option<PathBuf> {
	let mut sub_dir = issues_dir().join(owner).join(repo);

	// Build nested directory path from ancestors
	for ancestor in ancestors {
		sub_dir = sub_dir.join(format_issue_dir_name(ancestor));
	}

	if !sub_dir.exists() {
		return None;
	}

	// Look for files/directories matching the sub-issue number pattern
	let prefix = format!("{sub_issue_number}_-_");
	if let Ok(entries) = std::fs::read_dir(&sub_dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
				continue;
			};

			if !name.starts_with(&prefix) {
				continue;
			}

			// Check for flat file pattern: {number}_-_{title}.{ext}[.bak]
			if path.is_file() {
				return Some(path);
			}

			// Check for directory pattern: {number}_-_{title}/__main__.{ext}[.bak]
			if path.is_dir()
				&& let Some(main_file) = find_main_file_in_dir(&path)
			{
				return Some(main_file);
			}
		}
	}

	None
}

/// Find a `__main__` file in a directory (any supported extension).
/// Returns the path to the __main__ file if found.
fn find_main_file_in_dir(dir: &Path) -> Option<PathBuf> {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return None;
	};

	for entry in entries.flatten() {
		let path = entry.path();
		if !path.is_file() {
			continue;
		}

		let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
			continue;
		};

		// Handle .bak suffix: __main__.md.bak -> stem is "md" after first file_stem
		// We need to check the stem or the stem of the stem
		if stem == MAIN_ISSUE_FILENAME {
			return Some(path);
		}

		// For .bak files: __main__.md.bak -> file_stem gives "__main__.md"
		// So we need to check if it starts with __main__
		if stem.starts_with(MAIN_ISSUE_FILENAME) && (stem == format!("{MAIN_ISSUE_FILENAME}.md") || stem == format!("{MAIN_ISSUE_FILENAME}.typ")) {
			return Some(path);
		}
	}

	None
}

/// Get the directory name for an issue (used when it has sub-issues).
/// Format: {number}_-_{sanitized_title}
pub fn get_issue_dir_name(issue_number: Option<u64>, title: &str) -> String {
	let sanitized = sanitize_title_for_filename(title);
	match issue_number {
		Some(num) if sanitized.is_empty() => format!("{num}"),
		Some(num) => format!("{num}_-_{sanitized}"),
		None if sanitized.is_empty() => "untitled".to_string(),
		None => sanitized,
	}
}

/// Get the path to the issue directory (where sub-issues would be stored).
/// This is the same as the file path but without the extension.
pub fn get_issue_dir_path(owner: &str, repo: &str, issue_number: Option<u64>, title: &str, ancestors: &[FetchedIssue]) -> PathBuf {
	let mut path = issues_dir().join(owner).join(repo);

	// Build nested directory structure for all ancestors
	for ancestor in ancestors {
		path = path.join(format_issue_dir_name(ancestor));
	}

	path.join(get_issue_dir_name(issue_number, title))
}

/// Get the path for the main issue file when stored inside a directory.
/// Format: {dir}/__main__.{ext}[.bak]
pub fn get_main_file_path(issue_dir: &Path, extension: &Extension, closed: bool) -> PathBuf {
	let filename = if closed {
		format!("{MAIN_ISSUE_FILENAME}.{}.bak", extension.as_str())
	} else {
		format!("{MAIN_ISSUE_FILENAME}.{}", extension.as_str())
	};
	issue_dir.join(filename)
}

/// Find the actual file path for an issue, checking both flat and directory formats.
/// This handles the case where we need to find an existing issue file regardless of format.
///
/// Checks in order:
/// 1. Flat format: {parent}/{number}_-_{title}.{ext}[.bak]
/// 2. Directory format: {parent}/{number}_-_{title}/__main__.{ext}[.bak]
///
/// Returns None if no file is found in either format.
pub fn find_issue_file(owner: &str, repo: &str, issue_number: Option<u64>, title: &str, extension: &Extension, ancestors: &[FetchedIssue]) -> Option<PathBuf> {
	// Try flat format first (both open and closed)
	let flat_path = get_issue_file_path(owner, repo, issue_number, title, extension, false, ancestors);
	if flat_path.exists() {
		return Some(flat_path);
	}

	let flat_closed_path = get_issue_file_path(owner, repo, issue_number, title, extension, true, ancestors);
	if flat_closed_path.exists() {
		return Some(flat_closed_path);
	}

	// Try directory format
	let issue_dir = get_issue_dir_path(owner, repo, issue_number, title, ancestors);
	if issue_dir.is_dir() {
		// Check for __main__ file (both open and closed)
		let main_path = get_main_file_path(&issue_dir, extension, false);
		if main_path.exists() {
			return Some(main_path);
		}

		let main_closed_path = get_main_file_path(&issue_dir, extension, true);
		if main_closed_path.exists() {
			return Some(main_closed_path);
		}
	}

	None
}

/// Read the body content from a sub-issue file.
/// Strips the title line and returns just the body content.
pub fn read_sub_issue_body_from_file(file_path: &Path) -> Option<String> {
	let content = std::fs::read_to_string(file_path).ok()?;
	let mut lines = content.lines();

	// Skip the title line
	lines.next()?;

	// Collect body lines, stripping one level of indentation (they should be at depth 1)
	let body_lines: Vec<&str> = lines.map(|line| line.strip_prefix('\t').unwrap_or(line)).collect();

	let body = body_lines.join("\n").trim().to_string();
	if body.is_empty() { None } else { Some(body) }
}

/// Search for issue files matching a pattern.
/// Returns paths relative to the issues directory.
pub fn search_issue_files(pattern: &str) -> Result<Vec<PathBuf>> {
	let issues_base = issues_dir();
	if !issues_base.exists() {
		return Ok(vec![]);
	}

	let mut matches = Vec::new();
	let pattern_lower = pattern.to_lowercase();

	// Walk the issues directory
	fn walk_dir(dir: &Path, pattern: &str, matches: &mut Vec<PathBuf>) -> std::io::Result<()> {
		for entry in std::fs::read_dir(dir)? {
			let entry = entry?;
			let path = entry.path();

			if path.is_dir() {
				walk_dir(&path, pattern, matches)?;
			} else if path.is_file() {
				// Check if file matches the pattern
				if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
					// Only consider .md and .typ files (including .bak versions)
					if name.ends_with(".md") || name.ends_with(".typ") || name.ends_with(".md.bak") || name.ends_with(".typ.bak") {
						// Match against filename or full path
						let name_lower = name.to_lowercase();
						let path_str = path.to_string_lossy().to_lowercase();
						if pattern.is_empty() || name_lower.contains(pattern) || path_str.contains(pattern) {
							matches.push(path);
						}
					}
				}
			}
		}
		Ok(())
	}

	walk_dir(&issues_base, &pattern_lower, &mut matches)?;

	// Sort by modification time (most recent first)
	matches.sort_by(|a, b| {
		let a_time = std::fs::metadata(a).and_then(|m| m.modified()).ok();
		let b_time = std::fs::metadata(b).and_then(|m| m.modified()).ok();
		b_time.cmp(&a_time)
	});

	Ok(matches)
}

/// Exact match level for fzf queries.
#[derive(Clone, Copy, Debug, Default)]
pub enum ExactMatchLevel {
	/// Default fuzzy matching
	#[default]
	Fuzzy,
	/// Exact substring match with space-separated terms (fzf --exact)
	ExactTerms,
	/// Regex pattern matching (substring)
	RegexSubstring,
	/// Regex pattern matching (full line)
	RegexLine,
}

impl TryFrom<u8> for ExactMatchLevel {
	type Error = &'static str;

	fn try_from(count: u8) -> Result<Self, Self::Error> {
		match count {
			0 => Ok(Self::Fuzzy),
			1 => Ok(Self::ExactTerms),
			2 => Ok(Self::RegexSubstring),
			3 => Ok(Self::RegexLine),
			_ => Err("--exact / -e can be specified at most 3 times"),
		}
	}
}

/// Choose an issue file using fzf.
///
/// Takes all files and lets fzf handle the filtering with the specified exact match level.
/// Uses --select-1 to auto-select when only one match.
pub fn choose_issue_with_fzf(files: &[PathBuf], initial_query: &str, exact: ExactMatchLevel) -> Result<Option<PathBuf>> {
	use std::{
		io::Write,
		process::{Command, Stdio},
	};

	use regex::Regex;

	let issues_base = issues_dir();

	// Prepare file list with relative paths for display
	let file_list: Vec<String> = files
		.iter()
		.filter_map(|p| p.strip_prefix(&issues_base).ok().map(|rel| rel.to_string_lossy().to_string()))
		.collect();

	// For regex modes, pre-filter the file list
	let (filtered_list, fzf_query): (Vec<&String>, String) = match exact {
		ExactMatchLevel::Fuzzy | ExactMatchLevel::ExactTerms => {
			// No pre-filtering, pass query to fzf
			(file_list.iter().collect(), initial_query.to_string())
		}
		ExactMatchLevel::RegexSubstring => {
			// Pre-filter with regex (substring match)
			if initial_query.is_empty() {
				(file_list.iter().collect(), String::new())
			} else {
				let re = Regex::new(initial_query).map_err(|e| eyre!("Invalid regex pattern: {e}"))?;
				let filtered: Vec<&String> = file_list.iter().filter(|f| re.is_match(f)).collect();
				(filtered, String::new()) // Clear query since we already filtered
			}
		}
		ExactMatchLevel::RegexLine => {
			// Pre-filter with regex (full line match, auto-anchor if needed)
			if initial_query.is_empty() {
				(file_list.iter().collect(), String::new())
			} else {
				let pattern = {
					let has_start = initial_query.starts_with('^');
					let has_end = initial_query.ends_with('$');
					match (has_start, has_end) {
						(true, true) => initial_query.to_string(),
						(true, false) => format!("{initial_query}$"),
						(false, true) => format!("^{initial_query}"),
						(false, false) => format!("^{initial_query}$"),
					}
				};
				let re = Regex::new(&pattern).map_err(|e| eyre!("Invalid regex pattern: {e}"))?;
				let filtered: Vec<&String> = file_list.iter().filter(|f| re.is_match(f)).collect();
				(filtered, String::new()) // Clear query since we already filtered
			}
		}
	};

	// Build fzf command
	let mut cmd = Command::new("fzf");

	cmd.arg("--query").arg(&fzf_query);

	// Add --exact flag for ExactTerms mode
	if matches!(exact, ExactMatchLevel::ExactTerms) {
		cmd.arg("--exact");
	}

	cmd.arg("--select-1")
		.arg("--preview")
		.arg("cat {}")
		.arg("--preview-window")
		.arg("right:50%:wrap")
		.current_dir(&issues_base)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped());

	let mut child = cmd.spawn()?;

	// Write file list to fzf stdin
	if let Some(stdin) = child.stdin.as_mut() {
		for file in &filtered_list {
			writeln!(stdin, "{file}")?;
		}
	}

	let output = child.wait_with_output()?;

	if output.status.success() {
		let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
		if !selected.is_empty() {
			return Ok(Some(issues_base.join(selected)));
		}
	}

	Ok(None)
}

/// Extract owner/repo from an issue file path.
/// Path format: .../{owner}/{repo}/{issue_file} or .../{owner}/{repo}/{parent_dir}/{sub_issue_file}
pub fn extract_owner_repo_from_path(issue_file_path: &Path) -> Result<(String, String)> {
	let issues_base = issues_dir();

	// Get relative path from issues base
	let rel_path = issue_file_path
		.strip_prefix(&issues_base)
		.map_err(|_| eyre!("Issue file is not in issues directory: {issue_file_path:?}"))?;

	// Extract first two components as owner/repo
	let mut components = rel_path.components();
	let owner = components
		.next()
		.and_then(|c| c.as_os_str().to_str())
		.ok_or_else(|| eyre!("Could not extract owner from path: {issue_file_path:?}"))?
		.to_string();
	let repo = components
		.next()
		.and_then(|c| c.as_os_str().to_str())
		.ok_or_else(|| eyre!("Could not extract repo from path: {issue_file_path:?}"))?
		.to_string();

	Ok((owner, repo))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_main_issue_filename_constant() {
		assert_eq!(MAIN_ISSUE_FILENAME, "__main__");
	}

	#[test]
	fn test_get_issue_dir_name() {
		// With number and title
		assert_eq!(get_issue_dir_name(Some(123), "My Issue"), "123_-_My_Issue");

		// With number only
		assert_eq!(get_issue_dir_name(Some(123), ""), "123");

		// Without number
		assert_eq!(get_issue_dir_name(None, "My Issue"), "My_Issue");

		// Without number and title
		assert_eq!(get_issue_dir_name(None, ""), "untitled");
	}

	#[test]
	fn test_get_main_file_path() {
		let dir = PathBuf::from("/tmp/issues/123_-_title");

		// Open issue
		assert_eq!(get_main_file_path(&dir, &Extension::Md, false), PathBuf::from("/tmp/issues/123_-_title/__main__.md"));

		// Closed issue
		assert_eq!(get_main_file_path(&dir, &Extension::Md, true), PathBuf::from("/tmp/issues/123_-_title/__main__.md.bak"));

		// Typst extension
		assert_eq!(get_main_file_path(&dir, &Extension::Typ, false), PathBuf::from("/tmp/issues/123_-_title/__main__.typ"));
	}

	#[test]
	fn test_find_main_file_in_dir() {
		use std::fs;
		let temp_dir = tempfile::tempdir().unwrap();
		let dir = temp_dir.path();

		// No __main__ file
		assert!(find_main_file_in_dir(dir).is_none());

		// Create __main__.md
		fs::write(dir.join("__main__.md"), "test").unwrap();
		assert_eq!(find_main_file_in_dir(dir), Some(dir.join("__main__.md")));

		// Clean up and test .bak version
		fs::remove_file(dir.join("__main__.md")).unwrap();
		fs::write(dir.join("__main__.md.bak"), "test").unwrap();
		assert_eq!(find_main_file_in_dir(dir), Some(dir.join("__main__.md.bak")));

		// Test typst
		fs::remove_file(dir.join("__main__.md.bak")).unwrap();
		fs::write(dir.join("__main__.typ"), "test").unwrap();
		assert_eq!(find_main_file_in_dir(dir), Some(dir.join("__main__.typ")));
	}

	#[test]
	fn test_exact_match_level_try_from() {
		assert!(matches!(ExactMatchLevel::try_from(0), Ok(ExactMatchLevel::Fuzzy)));
		assert!(matches!(ExactMatchLevel::try_from(1), Ok(ExactMatchLevel::ExactTerms)));
		assert!(matches!(ExactMatchLevel::try_from(2), Ok(ExactMatchLevel::RegexSubstring)));
		assert!(matches!(ExactMatchLevel::try_from(3), Ok(ExactMatchLevel::RegexLine)));
		assert!(ExactMatchLevel::try_from(4).is_err());
		assert!(ExactMatchLevel::try_from(255).is_err());
	}
}
