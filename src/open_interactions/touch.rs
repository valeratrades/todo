//! Touch mode: create or open issues from paths.

use std::path::PathBuf;

use todo::{CloseState, Extension};
use v_utils::prelude::*;

use super::{
	files::{get_issue_file_path, issues_dir, sanitize_title_for_filename, search_issue_files},
	meta::{IssueMetaEntry, allocate_virtual_issue_number, ensure_virtual_project, save_issue_meta},
};

/// Parsed touch path components
/// Format: workspace/project/issue[.md|.typ] or workspace/project/parent/child[.md|.typ] (for sub-issues)
#[derive(Debug)]
pub struct TouchPath {
	pub owner: String,
	pub repo: String,
	/// Chain of issue titles (parent issues first, the target issue last)
	/// For a simple issue: ["issue_title"]
	/// For a sub-issue: ["parent_title", "child_title"]
	/// For nested: ["grandparent", "parent", "child"]
	pub issue_chain: Vec<String>,
	/// The extension from the path (if provided), or None to use default
	pub extension: Option<Extension>,
}

/// Parse a path for --touch mode
/// Format: workspace/project/issue[.md|.typ] or workspace/project/parent_issue/child_issue[.md|.typ]
/// Extension is optional - if not provided, will use config default
pub fn parse_touch_path(path: &str) -> Result<TouchPath> {
	let path_buf = PathBuf::from(path);

	// Check if path has a valid extension
	let extension = path_buf.extension().and_then(|e| e.to_str()).and_then(|ext| match ext {
		"md" => Some(Extension::Md),
		"typ" => Some(Extension::Typ),
		_ => None,
	});

	// Collect all path components
	let components: Vec<&str> = path_buf.iter().filter_map(|c| c.to_str()).collect();

	// Need at least: workspace/project/issue
	if components.len() < 3 {
		return Err(eyre!("Path must be in format: workspace/project/issue (got {} components)", components.len()));
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
	if extension.is_some()
		&& let Some(last) = issue_chain.last_mut()
	{
		// Strip the extension suffix (e.g., ".md" or ".typ")
		if let Some(stem) = last.rsplit_once('.') {
			*last = stem.0.to_string();
		}
	}

	Ok(TouchPath {
		owner,
		repo,
		issue_chain,
		extension,
	})
}

/// Create a pending issue locally. Will be created on GitHub when the file is closed.
/// For now, sub-issues are not supported in pending mode (parent must exist on GitHub first).
pub fn create_pending_issue(touch_path: &TouchPath, extension: &Extension) -> Result<PathBuf> {
	let owner = &touch_path.owner;
	let repo = &touch_path.repo;

	// For now, only support single-level issues (no sub-issues for pending)
	if touch_path.issue_chain.len() > 1 {
		return Err(eyre!(
			"Sub-issues via --touch require the parent to exist on GitHub first.\n\
			 Create the parent issue first, then use --touch for the sub-issue."
		));
	}

	let issue_title = touch_path.issue_chain.last().unwrap();

	// No issue number yet - will be assigned after GitHub creation
	let issue_file_path = get_issue_file_path(owner, repo, None, issue_title, extension, false, None);

	if let Some(parent) = issue_file_path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	// Create file with no URL (signals pending creation)
	let content = format!("- [ ] {issue_title} <!--  -->\n");
	std::fs::write(&issue_file_path, &content)?;

	// Save metadata (issue_number 0 = pending)
	let meta_entry = IssueMetaEntry {
		issue_number: 0,
		title: issue_title.clone(),
		extension: extension.as_str().to_string(),
		original_issue_body: Some(String::new()),
		original_comments: vec![],
		original_sub_issues: vec![],
		parent_issue: None,
		original_close_state: CloseState::Open,
	};
	save_issue_meta(owner, repo, meta_entry)?;

	println!("Created pending issue: {issue_title}");
	println!("Will be created on GitHub when you close the editor.");

	Ok(issue_file_path)
}

/// Create a new virtual issue locally (no GitHub).
/// Virtual issues have locally-generated issue numbers and are stored in the same format.
pub fn create_virtual_issue(touch_path: &TouchPath, extension: &Extension) -> Result<PathBuf> {
	let owner = &touch_path.owner;
	let repo = &touch_path.repo;

	// Ensure virtual project exists (creates if needed)
	ensure_virtual_project(owner, repo)?;

	// For now, only support single-level issues (no sub-issues for virtual projects)
	if touch_path.issue_chain.len() > 1 {
		// TODO: Support sub-issues for virtual projects
		return Err(eyre!("Sub-issues are not yet supported for virtual projects. Use a flat issue structure."));
	}

	// Get the issue title (last in chain)
	let issue_title = touch_path.issue_chain.last().unwrap();

	// Allocate a virtual issue number (for metadata tracking, not filename)
	let issue_number = allocate_virtual_issue_number(owner, repo)?;

	// Determine file path (no number prefix for virtual issues)
	let issue_file_path = get_issue_file_path(owner, repo, None, issue_title, extension, false, None);

	// Create parent directories
	if let Some(parent) = issue_file_path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	// Create the issue file with basic structure
	// Virtual issues don't have a GitHub URL, so we use a special marker
	let content = format!("- [ ] {issue_title} <!--virtual:{owner}/{repo}#{issue_number}-->\n");

	std::fs::write(&issue_file_path, &content)?;

	// Save metadata
	let meta_entry = IssueMetaEntry {
		issue_number,
		title: issue_title.clone(),
		extension: extension.as_str().to_string(),
		original_issue_body: Some(String::new()),
		original_comments: vec![],
		original_sub_issues: vec![],
		parent_issue: None,
		original_close_state: CloseState::Open,
	};
	save_issue_meta(owner, repo, meta_entry)?;

	println!("Created virtual issue #{issue_number}: {issue_title}");
	println!("Stored at: {:?}", issue_file_path);

	Ok(issue_file_path)
}

/// Try to find an existing local issue file matching the touch path
/// Returns the path if found, None otherwise
pub fn find_local_issue_for_touch(touch_path: &TouchPath, extension: &Extension) -> Option<PathBuf> {
	let issues_base = issues_dir();

	// Path structure: issues/{owner}/{repo}/{number}_-_{title}.{ext}
	let project_dir = issues_base.join(&touch_path.owner).join(&touch_path.repo);
	if !project_dir.exists() {
		return None;
	}

	// Search for files matching the issue title (last in chain)
	let issue_title = touch_path.issue_chain.last()?;
	let ext = extension.as_str();
	// Sanitize and lowercase for comparison
	let sanitized_title_lower = sanitize_title_for_filename(issue_title).to_lowercase();

	// Search using the sanitized title
	if let Ok(matches) = search_issue_files(&sanitized_title_lower) {
		// Filter matches to only those in the correct project directory and with correct extension
		for path in matches {
			// Check if it's in the right project directory
			if !path.starts_with(&project_dir) {
				continue;
			}

			// Check extension matches
			if path.extension().and_then(|e| e.to_str()) != Some(ext) {
				continue;
			}

			// Check the filename contains the sanitized title
			// Filename format: {number}_-_{sanitized_title}.{ext}
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
		assert!(matches!(result.extension, Some(Extension::Md)));
	}

	#[test]
	fn test_parse_touch_path_simple_without_extension() {
		// Simple issue without extension: workspace/project/issue
		let result = parse_touch_path("owner/repo/my-issue").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["my-issue"]);
		assert!(result.extension.is_none());
	}

	#[test]
	fn test_parse_touch_path_sub_issue() {
		// Sub-issue: workspace/project/parent/child.md
		let result = parse_touch_path("owner/repo/parent-issue/child-issue.md").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["parent-issue", "child-issue"]);
		assert!(matches!(result.extension, Some(Extension::Md)));
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
	fn test_parse_touch_path_typst() {
		// Typst file extension
		let result = parse_touch_path("owner/repo/my-issue.typ").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		assert_eq!(result.issue_chain, vec!["my-issue"]);
		assert!(matches!(result.extension, Some(Extension::Typ)));
	}

	#[test]
	fn test_parse_touch_path_unknown_extension_treated_as_no_extension() {
		// Unknown extension is treated as part of the filename (no extension detected)
		let result = parse_touch_path("owner/repo/issue.txt").unwrap();
		assert_eq!(result.owner, "owner");
		assert_eq!(result.repo, "repo");
		// "issue.txt" is treated as the issue title since .txt is not a valid extension
		assert_eq!(result.issue_chain, vec!["issue.txt"]);
		assert!(result.extension.is_none());
	}

	#[test]
	fn test_parse_touch_path_errors() {
		// Too few components
		assert!(parse_touch_path("owner/issue.md").is_err());
		assert!(parse_touch_path("issue.md").is_err());
	}
}
