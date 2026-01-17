//! Touch mode: create or open issues from paths.

use std::path::PathBuf;

use v_utils::prelude::*;

use super::{
	fetch::fetch_and_store_issue,
	files::{get_issue_file_path, issues_dir, sanitize_title_for_filename, search_issue_files},
	meta::{allocate_virtual_issue_number, ensure_virtual_project},
};
use crate::github::BoxedGithubClient;

/// Parsed touch path components
/// Format: workspace/project/issue[.md] or workspace/project/parent/child[.md] (for sub-issues)
#[derive(Debug)]
pub struct TouchPath {
	pub owner: String,
	pub repo: String,
	/// Chain of issue titles (parent issues first, the target issue last)
	/// For a simple issue: ["issue_title"]
	/// For a sub-issue: ["parent_title", "child_title"]
	/// For nested: ["grandparent", "parent", "child"]
	pub issue_chain: Vec<String>,
}

/// Parse a path for --touch mode
/// Format: workspace/project/issue[.md] or workspace/project/parent_issue/child_issue[.md]
pub fn parse_touch_path(path: &str) -> Result<TouchPath> {
	let path_buf = PathBuf::from(path);

	// Check if path has .md extension
	let has_md_ext = path_buf.extension().and_then(|e| e.to_str()) == Some("md");

	// Collect all path components
	let components: Vec<&str> = path_buf.iter().filter_map(|c| c.to_str()).collect();

	// Need at least: workspace/project/issue
	if components.len() < 3 {
		bail!("Path must be in format: workspace/project/issue (got {} components)", components.len());
	}

	let owner = components[0].to_string();
	let repo = components[1].to_string();

	// Everything after workspace/project is the issue chain
	let mut issue_chain = Vec::new();

	// All components from index 2 onwards
	for component in &components[2..] {
		issue_chain.push(component.to_string());
	}

	// If we have an extension, strip it from the last component
	if has_md_ext && let Some(last) = issue_chain.last_mut() {
		// Strip the extension suffix (e.g., ".md")
		if let Some(stem) = last.rsplit_once('.') {
			*last = stem.0.to_string();
		}
	}

	Ok(TouchPath { owner, repo, issue_chain })
}

/// Create an issue on Github immediately, then fetch and store it locally.
/// For sub-issues: requires the immediate parent to already exist on Github.
pub async fn create_and_fetch_issue(gh: &BoxedGithubClient, touch_path: &TouchPath) -> Result<PathBuf> {
	let owner = &touch_path.owner;
	let repo = &touch_path.repo;

	// Get the issue title (last in chain)
	let issue_title = touch_path.issue_chain.last().unwrap();

	// Determine if this is a sub-issue (has parent issues in chain)
	let parent_chain = &touch_path.issue_chain[..touch_path.issue_chain.len() - 1];

	if parent_chain.is_empty() {
		// Top-level issue - create directly
		println!("Creating issue on Github: {issue_title}");
		let created = gh.create_issue(owner, repo, issue_title, "").await?;
		println!("Created issue #{} on Github", created.number);

		// Fetch and store the newly created issue
		fetch_and_store_issue(gh, owner, repo, created.number, None).await
	} else {
		// Sub-issue - parent must exist
		// Only support single parent (immediate parent must exist)
		if parent_chain.len() > 1 {
			bail!(
				"Cannot create nested sub-issue via --touch.\n\
				 Only immediate sub-issues are supported.\n\
				 Path: {owner}/{repo}/{}\n\
				 \n\
				 Create parent issues first, then create the sub-issue.",
				touch_path.issue_chain.join("/")
			);
		}

		let parent_title = &parent_chain[0];
		let parent_num = gh.find_issue_by_title(owner, repo, parent_title).await?.ok_or_else(|| {
			eyre!(
				"Parent issue '{parent_title}' not found on Github.\n\
				 Create the parent issue first:\n\
				   todo open --touch {owner}/{repo}/{parent_title}"
			)
		})?;

		// Create the sub-issue
		println!("Creating sub-issue on Github: {issue_title}");
		let created = gh.create_issue(owner, repo, issue_title, "").await?;
		gh.add_sub_issue(owner, repo, parent_num, created.id).await?;
		println!("Created sub-issue #{} under parent #{parent_num}", created.number);

		// Fetch and store the parent tree (includes the new sub-issue)
		let root_path = fetch_and_store_issue(gh, owner, repo, parent_num, None).await?;

		// Find and return the path to the newly created sub-issue
		find_subissue_path(&root_path, issue_title).ok_or_else(|| eyre!("Failed to find newly created sub-issue file. This is a bug."))
	}
}

/// Find a sub-issue file path within a parent issue's directory structure.
fn find_subissue_path(parent_path: &std::path::Path, title: &str) -> Option<PathBuf> {
	// The parent_path is the __main__.md file, get its directory
	let parent_dir = parent_path.parent()?;

	// Search for a file matching the title in the directory
	let sanitized_title = sanitize_title_for_filename(title);

	// Check both flat format and directory format
	for entry in std::fs::read_dir(parent_dir).ok()? {
		let entry = entry.ok()?;
		let path = entry.path();

		if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
			// Check flat file: {number}_-_{title}.md
			if file_name.ends_with(".md") && file_name.contains(&sanitized_title) {
				return Some(path);
			}
			// Check directory: {number}_-_{title}/__main__.md
			if path.is_dir() && file_name.contains(&sanitized_title) {
				let main_file = path.join("__main__.md");
				if main_file.exists() {
					return Some(main_file);
				}
			}
		}
	}

	None
}

/// Create a new virtual issue locally (no Github).
/// Virtual issues have locally-generated issue numbers and are stored in the same format.
pub fn create_virtual_issue(touch_path: &TouchPath) -> Result<PathBuf> {
	let owner = &touch_path.owner;
	let repo = &touch_path.repo;

	// Ensure virtual project exists (creates if needed)
	ensure_virtual_project(owner, repo)?;

	// For now, only support single-level issues (no sub-issues for virtual projects)
	if touch_path.issue_chain.len() > 1 {
		// TODO: Support sub-issues for virtual projects
		bail!("Sub-issues are not yet supported for virtual projects. Use a flat issue structure.");
	}

	// Get the issue title (last in chain)
	let issue_title = touch_path.issue_chain.last().unwrap();

	// Allocate a virtual issue number (for metadata tracking, not filename)
	let issue_number = allocate_virtual_issue_number(owner, repo)?;

	// Determine file path (no number prefix for virtual issues)
	let issue_file_path = get_issue_file_path(owner, repo, None, issue_title, false, &[]);

	// Create parent directories
	if let Some(parent) = issue_file_path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	// Create the issue file with basic structure
	// Virtual issues don't have a Github URL, so we use a special marker
	let content = format!("- [ ] {issue_title} <!--virtual:{owner}/{repo}#{issue_number}-->\n");

	std::fs::write(&issue_file_path, &content)?;

	// No longer saving metadata - it's derived from file paths

	println!("Created virtual issue #{issue_number}: {issue_title}");
	println!("Stored at: {issue_file_path:?}");

	Ok(issue_file_path)
}

/// Try to find an existing local issue file matching the touch path
/// Returns the path if found, None otherwise
pub fn find_local_issue_for_touch(touch_path: &TouchPath) -> Option<PathBuf> {
	let issues_base = issues_dir();

	// Path structure: issues/{owner}/{repo}/{number}_-_{title}.md
	let project_dir = issues_base.join(&touch_path.owner).join(&touch_path.repo);
	if !project_dir.exists() {
		return None;
	}

	// Search for files matching the issue title (last in chain)
	let issue_title = touch_path.issue_chain.last()?;
	// Sanitize and lowercase for comparison
	let sanitized_title_lower = sanitize_title_for_filename(issue_title).to_lowercase();

	// Search using the sanitized title
	if let Ok(matches) = search_issue_files(&sanitized_title_lower) {
		// Filter matches to only those in the correct project directory
		for path in matches {
			// Check if it's in the right project directory
			if !path.starts_with(&project_dir) {
				continue;
			}

			// Check extension matches
			if path.extension().and_then(|e| e.to_str()) != Some("md") {
				continue;
			}

			// Check the filename contains the sanitized title
			// Filename format: {number}_-_{sanitized_title}.md
			if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
				let stem_lower = stem.to_lowercase();
				// Extract the title part after "_-_" if present
				let title_part = stem_lower.split("_-_").nth(1).unwrap_or(&stem_lower);
				if title_part == sanitized_title_lower {
					return Some(path);
				}
			}
		}
	}

	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_touch_path_simple_with_extension() {
		// Simple issue with extension: workspace/project/issue.md
		let result = parse_touch_path("owner/repo/my-issue.md").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["my-issue"]);
	}

	#[test]
	fn test_parse_touch_path_simple_without_extension() {
		// Simple issue without extension: workspace/project/issue
		let result = parse_touch_path("owner/repo/my-issue").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["my-issue"]);
	}

	#[test]
	fn test_parse_touch_path_sub_issue() {
		// Sub-issue: workspace/project/parent/child.md
		let result = parse_touch_path("owner/repo/parent-issue/child-issue.md").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["parent-issue", "child-issue"]);
	}

	#[test]
	fn test_parse_touch_path_nested_sub_issue() {
		// Nested sub-issue: workspace/project/grandparent/parent/child.md
		let result = parse_touch_path("owner/repo/grandparent/parent/child.md").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["grandparent", "parent", "child"]);
	}

	#[test]
	fn test_parse_touch_path_unknown_extension_treated_as_no_extension() {
		// Unknown extension is treated as part of the filename (no extension detected)
		let result = parse_touch_path("owner/repo/issue.txt").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		// "issue.txt" is treated as the issue title since .txt is not a valid extension
		assert_eq!(result.issue_chain, vec!["issue.txt"]);
	}

	#[test]
	fn test_parse_touch_path_errors() {
		// Too few components
		assert!(parse_touch_path("owner/issue.md").is_err());
		assert!(parse_touch_path("issue.md").is_err());
	}
}
