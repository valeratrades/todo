//! Project and issue metadata persistence.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use todo::CloseState;
use v_utils::prelude::*;

use super::files::get_project_dir;
use crate::github::{OriginalComment, OriginalSubIssue};

/// Stored metadata for a single issue
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IssueMetaEntry {
	pub issue_number: u64,
	pub title: String,
	pub extension: String,
	/// Original issue body (for diffing)
	pub original_issue_body: Option<String>,
	/// Original comments with their IDs
	pub original_comments: Vec<OriginalComment>,
	/// Original sub-issues with their state
	pub original_sub_issues: Vec<OriginalSubIssue>,
	/// Parent issue number if this is a sub-issue
	pub parent_issue: Option<u64>,
	/// Original issue close state (for detecting close/reopen)
	#[serde(default)]
	pub original_close_state: CloseState,
}

/// Project-level metadata file containing all issues
/// Stored at: issues/{owner}/{repo}/.meta.json
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProjectMeta {
	pub owner: String,
	pub repo: String,
	/// Map from issue number to its metadata
	pub issues: HashMap<u64, IssueMetaEntry>,
	/// Virtual project: has no GitHub remote, all operations are offline-only.
	/// Issue numbers are locally generated (starting from 1).
	#[serde(default)]
	pub virtual_project: bool,
	/// Next issue number for virtual projects (auto-incremented)
	#[serde(default)]
	pub next_virtual_issue_number: u64,
}

/// Get the metadata file path for a project
pub fn get_project_meta_path(owner: &str, repo: &str) -> PathBuf {
	get_project_dir(owner, repo).join(".meta.json")
}

/// Load project metadata, creating empty if not exists
pub fn load_project_meta(owner: &str, repo: &str) -> ProjectMeta {
	let meta_path = get_project_meta_path(owner, repo);
	match std::fs::read_to_string(&meta_path) {
		Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| ProjectMeta {
			owner: owner.to_string(),
			repo: repo.to_string(),
			issues: HashMap::new(),
			virtual_project: false,
			next_virtual_issue_number: 0,
		}),
		Err(_) => ProjectMeta {
			owner: owner.to_string(),
			repo: repo.to_string(),
			issues: HashMap::new(),
			virtual_project: false,
			next_virtual_issue_number: 0,
		},
	}
}

/// Save project metadata
pub fn save_project_meta(meta: &ProjectMeta) -> Result<()> {
	let meta_path = get_project_meta_path(&meta.owner, &meta.repo);
	if let Some(parent) = meta_path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let content = serde_json::to_string_pretty(meta)?;
	std::fs::write(&meta_path, content)?;
	Ok(())
}

/// Get metadata for a specific issue from the project meta
pub fn get_issue_meta(owner: &str, repo: &str, issue_number: u64) -> Option<IssueMetaEntry> {
	let project_meta = load_project_meta(owner, repo);
	project_meta.issues.get(&issue_number).cloned()
}

/// Save metadata for a specific issue to the project meta
pub fn save_issue_meta(owner: &str, repo: &str, entry: IssueMetaEntry) -> Result<()> {
	let mut project_meta = load_project_meta(owner, repo);
	project_meta.issues.insert(entry.issue_number, entry);
	save_project_meta(&project_meta)
}

/// Load metadata from an issue file path by extracting the issue number.
/// Handles both flat format ({number}_-_{title}.ext) and directory format ({number}_-_{title}/__main__.ext).
pub fn load_issue_meta_from_path(issue_file_path: &std::path::Path) -> Result<IssueMetaEntry> {
	use super::files::{MAIN_ISSUE_FILENAME, extract_owner_repo_from_path};

	let (owner, repo) = extract_owner_repo_from_path(issue_file_path)?;

	// Extract issue number from filename or parent directory name
	let filename = issue_file_path.file_name().and_then(|n| n.to_str()).ok_or_else(|| eyre!("Invalid issue file path"))?;

	// Handle .bak suffix for closed issues
	let filename = filename.strip_suffix(".bak").unwrap_or(filename);

	// Check if this is a __main__ file (directory format)
	let name_to_parse = if filename.starts_with(MAIN_ISSUE_FILENAME) {
		// Get issue number from parent directory name instead
		// Parent directory format: {number}_-_{title}
		issue_file_path
			.parent()
			.and_then(|p| p.file_name())
			.and_then(|n| n.to_str())
			.ok_or_else(|| eyre!("Could not extract parent directory for __main__ file"))?
	} else {
		filename
	};

	// Extract issue number from name format: {number}_-_{title} or {number}.{ext}
	let issue_number: u64 = name_to_parse
		.split("_-_")
		.next()
		.or_else(|| name_to_parse.split('.').next())
		.ok_or_else(|| eyre!("Could not extract issue number from filename"))?
		.parse()?;

	get_issue_meta(&owner, &repo, issue_number).ok_or_else(|| eyre!("No metadata found for issue #{issue_number}"))
}

/// Check if a project is virtual (has no GitHub remote)
pub fn is_virtual_project(owner: &str, repo: &str) -> bool {
	load_project_meta(owner, repo).virtual_project
}

/// Allocate the next issue number for a virtual project.
/// Returns the allocated number and saves the updated meta.
pub fn allocate_virtual_issue_number(owner: &str, repo: &str) -> Result<u64> {
	let mut project_meta = load_project_meta(owner, repo);
	if !project_meta.virtual_project {
		return Err(eyre!("Cannot allocate virtual issue number for non-virtual project {owner}/{repo}"));
	}

	// Ensure we start from 1
	if project_meta.next_virtual_issue_number == 0 {
		project_meta.next_virtual_issue_number = 1;
	}

	let issue_number = project_meta.next_virtual_issue_number;
	project_meta.next_virtual_issue_number += 1;
	save_project_meta(&project_meta)?;

	Ok(issue_number)
}

/// Create or load a virtual project meta. If project doesn't exist, creates it as virtual.
/// If it exists and is not virtual, returns an error.
pub fn ensure_virtual_project(owner: &str, repo: &str) -> Result<ProjectMeta> {
	let meta_path = get_project_meta_path(owner, repo);
	if meta_path.exists() {
		let project_meta = load_project_meta(owner, repo);
		if !project_meta.virtual_project {
			return Err(eyre!("Project {owner}/{repo} exists but is not a virtual project"));
		}
		Ok(project_meta)
	} else {
		// Create new virtual project
		let project_meta = ProjectMeta {
			owner: owner.to_string(),
			repo: repo.to_string(),
			issues: HashMap::new(),
			virtual_project: true,
			next_virtual_issue_number: 1,
		};
		save_project_meta(&project_meta)?;
		Ok(project_meta)
	}
}
