//! File path handling for issue storage.

use std::path::{Path, PathBuf};

use v_utils::prelude::*;

use super::util::Extension;

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
/// Format: {number}_-_{sanitized_title}.{ext} or {number}_-_{sanitized_title}.{ext}.bak for closed issues
pub fn format_issue_filename(issue_number: u64, title: &str, extension: &Extension, closed: bool) -> String {
	let sanitized = sanitize_title_for_filename(title);
	let base = if sanitized.is_empty() {
		format!("{issue_number}.{}", extension.as_str())
	} else {
		format!("{issue_number}_-_{sanitized}.{}", extension.as_str())
	};
	if closed { format!("{base}.bak") } else { base }
}

/// Get the path for an issue file in XDG_DATA.
/// Structure: issues/{owner}/{repo}/{number}_-_{title}.{ext}[.bak]
/// For sub-issues: issues/{owner}/{repo}/{parent_number}_-_{parent_title}/{number}_-_{title}.{ext}[.bak]
pub fn get_issue_file_path(owner: &str, repo: &str, issue_number: u64, title: &str, extension: &Extension, closed: bool, parent_issue: Option<(u64, &str)>) -> PathBuf {
	let base = issues_dir().join(owner).join(repo);
	let filename = format_issue_filename(issue_number, title, extension, closed);
	match parent_issue {
		None => base.join(filename),
		Some((parent_num, parent_title)) => {
			let parent_dir = format!("{}_-_{}", parent_num, sanitize_title_for_filename(parent_title));
			base.join(parent_dir).join(filename)
		}
	}
}

/// Get the project directory path (where meta.json lives).
/// Structure: issues/{owner}/{repo}/
pub fn get_project_dir(owner: &str, repo: &str) -> PathBuf {
	issues_dir().join(owner).join(repo)
}

/// Find the local file path for a sub-issue given its number.
/// Searches in the parent issue's directory for files matching the pattern {number}_-_*.{ext}[.bak]
/// Returns None if no matching file is found.
pub fn find_sub_issue_file(owner: &str, repo: &str, parent_number: u64, parent_title: &str, sub_issue_number: u64) -> Option<PathBuf> {
	let base = issues_dir().join(owner).join(repo);
	let parent_dir = format!("{}_-_{}", parent_number, sanitize_title_for_filename(parent_title));
	let sub_dir = base.join(&parent_dir);

	if !sub_dir.exists() {
		return None;
	}

	// Look for files matching the sub-issue number pattern
	// This matches both regular files and .bak files (closed issues)
	let prefix = format!("{sub_issue_number}_-_");
	if let Ok(entries) = std::fs::read_dir(&sub_dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_file()
				&& let Some(name) = path.file_name().and_then(|n| n.to_str())
			{
				if name.starts_with(&prefix) {
					return Some(path);
				}
			}
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

/// Choose an issue file using fzf.
pub fn choose_issue_with_fzf(matches: &[PathBuf], initial_query: &str) -> Result<Option<PathBuf>> {
	use std::{
		io::Write,
		process::{Command, Stdio},
	};

	let issues_base = issues_dir();

	// Prepare file list with relative paths for display
	let file_list: Vec<String> = matches
		.iter()
		.filter_map(|p| p.strip_prefix(&issues_base).ok().map(|rel| rel.to_string_lossy().to_string()))
		.collect();

	// Spawn fzf
	let mut child = Command::new("fzf")
		.arg("--query")
		.arg(initial_query)
		.arg("--preview")
		.arg("cat {}")
		.arg("--preview-window")
		.arg("right:50%:wrap")
		.current_dir(&issues_base)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.spawn()?;

	// Write file list to fzf stdin
	if let Some(stdin) = child.stdin.as_mut() {
		for file in &file_list {
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
		.map_err(|_| eyre!("Issue file is not in issues directory: {:?}", issue_file_path))?;

	// Extract first two components as owner/repo
	let mut components = rel_path.components();
	let owner = components
		.next()
		.and_then(|c| c.as_os_str().to_str())
		.ok_or_else(|| eyre!("Could not extract owner from path: {:?}", issue_file_path))?
		.to_string();
	let repo = components
		.next()
		.and_then(|c| c.as_os_str().to_str())
		.ok_or_else(|| eyre!("Could not extract repo from path: {:?}", issue_file_path))?
		.to_string();

	Ok((owner, repo))
}
