//! Touch mode: create or open issues from paths.

use std::path::PathBuf;

use v_utils::prelude::*;

use super::{
	fetch::fetch_and_store_issue,
	files::{issues_dir, sanitize_title_for_filename, search_issue_files},
	util::Extension,
};
use crate::github::BoxedGitHubClient;

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

/// Handle creating a new issue on GitHub
pub async fn create_issue_on_github(gh: &BoxedGitHubClient, touch_path: &TouchPath, extension: &Extension) -> Result<PathBuf> {
	let owner = &touch_path.owner;
	let repo = &touch_path.repo;

	// Step 1: Check collaborator access
	println!("Checking collaborator access to {owner}/{repo}...");
	let has_access = gh.check_collaborator_access(owner, repo).await?;
	if !has_access {
		return Err(eyre!("You don't have collaborator (write) access to {}/{}. Cannot create issues.", owner, repo));
	}
	println!("Access confirmed.");

	// Step 2: Validate parent issues exist (all except the last one in the chain)
	// Store both number and title for each parent
	let mut parent_issues: Vec<(u64, String)> = Vec::new();

	if touch_path.issue_chain.len() > 1 {
		println!("Validating parent issue chain...");
		for (i, parent_title) in touch_path.issue_chain[..touch_path.issue_chain.len() - 1].iter().enumerate() {
			// Try to find by title first
			let issue_number = gh.find_issue_by_title(owner, repo, parent_title).await?;

			match issue_number {
				Some(num) => {
					println!("  Found parent issue #{num}: {parent_title}");
					parent_issues.push((num, parent_title.clone()));
				}
				None => {
					// If not found by title, try parsing as issue number
					if let Ok(num) = parent_title.parse::<u64>() {
						if gh.issue_exists(owner, repo, num).await? {
							println!("  Found parent issue #{num}");
							// Fetch the actual title from GitHub
							let issue = gh.fetch_issue(owner, repo, num).await?;
							parent_issues.push((num, issue.title));
						} else {
							return Err(eyre!(
								"Parent issue '{}' (position {} in chain) does not exist on GitHub. Please create parent issues first.",
								parent_title,
								i + 1
							));
						}
					} else {
						return Err(eyre!(
							"Parent issue '{}' (position {} in chain) not found on GitHub. Please create parent issues first.",
							parent_title,
							i + 1
						));
					}
				}
			}
		}
	}

	// Step 3: Get the issue title (last in chain)
	let new_issue_title = touch_path.issue_chain.last().unwrap();

	// Step 4: Create the issue on GitHub (with empty body - user will edit after)
	println!("Creating issue '{new_issue_title}'...");
	let created = gh.create_issue(owner, repo, new_issue_title, "").await?;
	println!("Created issue #{}: {}", created.number, created.html_url);

	// Step 5: If there are parent issues, add as sub-issue to the immediate parent
	if let Some((parent_number, _)) = parent_issues.last() {
		println!("Adding as sub-issue to #{parent_number}...");
		gh.add_sub_issue(owner, repo, *parent_number, created.id).await?;
		println!("Sub-issue relationship created.");
	}

	// Step 6: Fetch and store the newly created issue locally (like normal flow)
	let parent_issue = parent_issues.last().cloned();
	let issue_file_path = fetch_and_store_issue(gh, owner, repo, created.number, extension, false, parent_issue).await?;

	println!("Stored issue at: {:?}", issue_file_path);

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
