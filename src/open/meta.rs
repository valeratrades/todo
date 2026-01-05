//! Project and issue metadata persistence.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
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
	/// Original issue state (for detecting close/reopen)
	#[serde(default)]
	pub original_closed: bool,
}

/// Project-level metadata file containing all issues
/// Stored at: issues/{owner}/{repo}/.meta.json
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProjectMeta {
	pub owner: String,
	pub repo: String,
	/// Map from issue number to its metadata
	pub issues: HashMap<u64, IssueMetaEntry>,
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
		}),
		Err(_) => ProjectMeta {
			owner: owner.to_string(),
			repo: repo.to_string(),
			issues: HashMap::new(),
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

/// Load metadata from an issue file path by extracting the issue number
pub fn load_issue_meta_from_path(issue_file_path: &std::path::Path) -> Result<IssueMetaEntry> {
	use super::files::extract_owner_repo_from_path;

	let (owner, repo) = extract_owner_repo_from_path(issue_file_path)?;

	// Extract issue number from filename
	let filename = issue_file_path.file_name().and_then(|n| n.to_str()).ok_or_else(|| eyre!("Invalid issue file path"))?;

	// Handle .bak suffix for closed issues
	let filename = filename.strip_suffix(".bak").unwrap_or(filename);

	// Extract issue number from filename format: {number}_-_{title}.{ext}
	let issue_number: u64 = filename
		.split("_-_")
		.next()
		.or_else(|| filename.split('.').next())
		.ok_or_else(|| eyre!("Could not extract issue number from filename"))?
		.parse()?;

	get_issue_meta(&owner, &repo, issue_number).ok_or_else(|| eyre!("No metadata found for issue #{issue_number}"))
}
