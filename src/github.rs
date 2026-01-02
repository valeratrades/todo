use reqwest::Client;
use serde::{Deserialize, Serialize};
use v_utils::prelude::*;

use crate::config::LiveSettings;

#[derive(Debug, Deserialize)]
pub struct GitHubIssue {
	pub number: u64,
	pub title: String,
	pub body: Option<String>,
	pub labels: Vec<GitHubLabel>,
	pub user: GitHubUser,
	pub state: String, // "open" or "closed" //TODO!!!!: add an actual enum
}

/// Sub-issue as returned by the GitHub API (same structure as issue for our purposes)
#[derive(Debug, Deserialize)]
pub struct GitHubSubIssue {
	pub number: u64,
	pub title: String,
	pub state: String, // "open" or "closed"
}

#[derive(Debug, Deserialize)]
pub struct GitHubLabel {
	pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubUser {
	pub login: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubComment {
	pub id: u64,
	pub body: Option<String>,
	pub user: GitHubUser,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OriginalComment {
	pub id: u64,
	pub body: Option<String>,
}

impl From<&GitHubComment> for OriginalComment {
	fn from(c: &GitHubComment) -> Self {
		Self { id: c.id, body: c.body.clone() }
	}
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OriginalSubIssue {
	pub number: u64,
	pub state: String,
}

impl From<&GitHubSubIssue> for OriginalSubIssue {
	fn from(s: &GitHubSubIssue) -> Self {
		Self {
			number: s.number,
			state: s.state.clone(),
		}
	}
}

/// Response from GitHub when creating an issue
#[derive(Debug, Deserialize)]
pub struct CreatedIssue {
	pub number: u64,
	pub html_url: String,
}

/// Parse a GitHub issue URL and extract owner, repo, and issue number.
/// Supports formats like:
/// - https://github.com/owner/repo/issues/123
/// - github.com/owner/repo/issues/123
/// - git@github.com:owner/repo (returns repo info, issue number parsing will fail)
/// - ssh://git@github.com/owner/repo.git (returns repo info, issue number parsing will fail)
pub fn parse_github_issue_url(url: &str) -> Result<(String, String, u64)> {
	let url = url.trim();

	// Try SSH format first: git@github.com:owner/repo.git or git@github.com:owner/repo
	// SSH URLs don't support issue numbers directly, but we parse them for consistency
	if let Some(path) = url.strip_prefix("git@github.com:") {
		// SSH format doesn't have issue numbers - this is an error for issue URLs
		return Err(eyre!(
			"SSH URL format doesn't support issue numbers. Use HTTPS format: https://github.com/{}/issues/NUMBER",
			path.strip_suffix(".git").unwrap_or(path)
		));
	}

	// Try ssh:// format: ssh://git@github.com/owner/repo.git
	if let Some(path) = url.strip_prefix("ssh://git@github.com/") {
		return Err(eyre!(
			"SSH URL format doesn't support issue numbers. Use HTTPS format: https://github.com/{}/issues/NUMBER",
			path.strip_suffix(".git").unwrap_or(path)
		));
	}

	// Remove protocol prefix if present (https://, http://)
	let path = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);

	// Remove github.com prefix
	let path = path.strip_prefix("github.com/").ok_or_else(|| eyre!("URL must be a GitHub URL: {}", url))?;

	// Split by /
	let parts: Vec<&str> = path.split('/').collect();

	if parts.len() < 4 || parts[2] != "issues" {
		return Err(eyre!("Invalid GitHub issue URL format. Expected: https://github.com/owner/repo/issues/123"));
	}

	let owner = parts[0].to_string();
	let repo = parts[1].to_string();
	let issue_number: u64 = parts[3].parse().map_err(|_| eyre!("Invalid issue number: {}", parts[3]))?;

	Ok((owner, repo, issue_number))
}

/// Check if a string looks like a GitHub issue URL specifically
pub fn is_github_issue_url(s: &str) -> bool {
	let s = s.trim();
	s.contains("github.com/") && s.contains("/issues/")
}

pub async fn fetch_authenticated_user(settings: &LiveSettings) -> Result<String> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let client = Client::new();
	let res = client
		.get("https://api.github.com/user")
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to fetch authenticated user: {} - {}", status, body));
	}

	let user = res.json::<GitHubUser>().await?;
	Ok(user.login)
}

pub async fn fetch_github_issue(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<GitHubIssue> {
	let config = settings.config()?;
	let milestones_config = config
		.milestones
		.as_ref()
		.ok_or_else(|| eyre!("milestones config section is required for GitHub token. Add [milestones] section with github_token to your config"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

	let client = Client::new();
	let res = client
		.get(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to fetch issue: {} - {}", status, body));
	}

	let issue = res.json::<GitHubIssue>().await?;
	Ok(issue)
}

pub async fn fetch_github_comments(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubComment>> {
	let config = settings.config()?;
	let milestones_config = config
		.milestones
		.as_ref()
		.ok_or_else(|| eyre!("milestones config section is required for GitHub token. Add [milestones] section with github_token to your config"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/comments");

	let client = Client::new();
	let res = client
		.get(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to fetch comments: {} - {}", status, body));
	}

	let comments = res.json::<Vec<GitHubComment>>().await?;
	Ok(comments)
}

pub async fn fetch_github_sub_issues(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubSubIssue>> {
	let config = settings.config()?;
	let milestones_config = config
		.milestones
		.as_ref()
		.ok_or_else(|| eyre!("milestones config section is required for GitHub token. Add [milestones] section with github_token to your config"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/sub_issues");

	let client = Client::new();
	let res = client
		.get(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		// Sub-issues API might not be available or issue has no sub-issues
		// Return empty vec instead of erroring
		return Ok(Vec::new());
	}

	let sub_issues = res.json::<Vec<GitHubSubIssue>>().await?;
	Ok(sub_issues)
}

pub async fn update_github_issue_body(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

	let client = Client::new();
	let res = client
		.patch(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "body": body }))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to update issue body: {} - {}", status, body));
	}

	Ok(())
}

pub async fn update_github_issue_state(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64, state: &str) -> Result<()> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

	let client = Client::new();
	let res = client
		.patch(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "state": state }))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to update issue state: {} - {}", status, body));
	}

	Ok(())
}

pub async fn update_github_comment(settings: &LiveSettings, owner: &str, repo: &str, comment_id: u64, body: &str) -> Result<()> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}");

	let client = Client::new();
	let res = client
		.patch(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "body": body }))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to update comment: {} - {}", status, body));
	}

	Ok(())
}

pub async fn create_github_comment(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/comments");

	let client = Client::new();
	let res = client
		.post(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "body": body }))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to create comment: {} - {}", status, body));
	}

	Ok(())
}

pub async fn delete_github_comment(settings: &LiveSettings, owner: &str, repo: &str, comment_id: u64) -> Result<()> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}");

	let client = Client::new();
	let res = client
		.delete(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to delete comment: {} - {}", status, body));
	}

	Ok(())
}

/// Check if the authenticated user has collaborator (push/write) access to a repository
pub async fn check_collaborator_access(settings: &LiveSettings, owner: &str, repo: &str) -> Result<bool> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	// Get the authenticated user's login
	let current_user = fetch_authenticated_user(settings).await?;

	// Check if user is a collaborator with write access
	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/collaborators/{current_user}/permission");

	let client = Client::new();
	let res = client
		.get(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		// If we can't check, assume no access
		return Ok(false);
	}

	#[derive(Deserialize)]
	struct PermissionResponse {
		permission: String,
	}

	let perm: PermissionResponse = res.json().await?;
	// "admin", "write", "read", "none"
	Ok(perm.permission == "admin" || perm.permission == "write")
}

/// Create a new GitHub issue
pub async fn create_github_issue(settings: &LiveSettings, owner: &str, repo: &str, title: &str, body: &str) -> Result<CreatedIssue> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues");

	let client = Client::new();
	let res = client
		.post(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "title": title, "body": body }))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to create issue: {} - {}", status, body));
	}

	let issue = res.json::<CreatedIssue>().await?;
	Ok(issue)
}

/// Add a sub-issue to a parent issue using GitHub's sub-issues API
pub async fn add_sub_issue(settings: &LiveSettings, owner: &str, repo: &str, parent_issue_number: u64, child_issue_number: u64) -> Result<()> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{parent_issue_number}/sub_issues");

	let client = Client::new();
	let res = client
		.post(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({ "sub_issue_id": child_issue_number }))
		.send()
		.await?;

	if !res.status().is_success() {
		let status = res.status();
		let body = res.text().await.unwrap_or_default();
		return Err(eyre!("Failed to add sub-issue: {} - {}", status, body));
	}

	Ok(())
}

/// Check if a GitHub issue exists and return its number if found by searching open issues with exact title match
pub async fn find_issue_by_title(settings: &LiveSettings, owner: &str, repo: &str, title: &str) -> Result<Option<u64>> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	// Search for issues with this title (search in open and closed)
	let encoded_title = urlencoding::encode(title);
	let api_url = format!("https://api.github.com/search/issues?q=repo:{owner}/{repo}+in:title+{encoded_title}");

	let client = Client::new();
	let res = client
		.get(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	if !res.status().is_success() {
		return Ok(None);
	}

	#[derive(Deserialize)]
	struct SearchResult {
		items: Vec<SearchItem>,
	}
	#[derive(Deserialize)]
	struct SearchItem {
		number: u64,
		title: String,
	}

	let result: SearchResult = res.json().await?;

	// Find exact title match
	for item in result.items {
		if item.title == title {
			return Ok(Some(item.number));
		}
	}

	Ok(None)
}

/// Check if an issue exists by number
pub async fn issue_exists(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<bool> {
	let config = settings.config()?;
	let milestones_config = config.milestones.as_ref().ok_or_else(|| eyre!("milestones config section is required for GitHub token"))?;

	let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

	let client = Client::new();
	let res = client
		.get(&api_url)
		.header("User-Agent", "Rust GitHub Client")
		.header("Authorization", format!("token {}", milestones_config.github_token))
		.send()
		.await?;

	Ok(res.status().is_success())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_github_issue_url() {
		// Standard HTTPS URL
		let (owner, repo, num) = parse_github_issue_url("https://github.com/owner/repo/issues/123").unwrap();
		assert_eq!(owner, "owner");
		assert_eq!(repo, "repo");
		assert_eq!(num, 123);

		// Without protocol
		let (owner, repo, num) = parse_github_issue_url("github.com/owner/repo/issues/456").unwrap();
		assert_eq!(owner, "owner");
		assert_eq!(repo, "repo");
		assert_eq!(num, 456);

		// HTTP URL
		let (owner, repo, num) = parse_github_issue_url("http://github.com/owner/repo/issues/789").unwrap();
		assert_eq!(owner, "owner");
		assert_eq!(repo, "repo");
		assert_eq!(num, 789);

		// With trailing whitespace
		let (owner, repo, num) = parse_github_issue_url("  https://github.com/owner/repo/issues/123  ").unwrap();
		assert_eq!(owner, "owner");
		assert_eq!(repo, "repo");
		assert_eq!(num, 123);
	}

	#[test]
	fn test_parse_github_issue_url_errors() {
		// Not a GitHub URL
		assert!(parse_github_issue_url("https://gitlab.com/owner/repo/issues/123").is_err());

		// Not an issues URL
		assert!(parse_github_issue_url("https://github.com/owner/repo/pull/123").is_err());

		// Invalid issue number
		assert!(parse_github_issue_url("https://github.com/owner/repo/issues/abc").is_err());

		// Missing parts
		assert!(parse_github_issue_url("https://github.com/owner").is_err());
	}

	#[test]
	fn test_parse_github_issue_url_ssh_error() {
		// SSH URLs should give a helpful error message
		let result = parse_github_issue_url("git@github.com:owner/repo.git");
		assert!(result.is_err());
		let err = result.unwrap_err().to_string();
		assert!(err.contains("SSH URL format doesn't support issue numbers"));
		assert!(err.contains("owner/repo"));

		// ssh:// format
		let result = parse_github_issue_url("ssh://git@github.com/owner/repo.git");
		assert!(result.is_err());
		let err = result.unwrap_err().to_string();
		assert!(err.contains("SSH URL format doesn't support issue numbers"));
	}

	#[test]
	fn test_is_github_issue_url() {
		// Valid issue URLs
		assert!(is_github_issue_url("https://github.com/owner/repo/issues/123"));
		assert!(is_github_issue_url("github.com/owner/repo/issues/456"));
		assert!(is_github_issue_url("http://github.com/owner/repo/issues/789"));

		// Not issue URLs
		assert!(!is_github_issue_url("https://github.com/owner/repo"));
		assert!(!is_github_issue_url("git@github.com:owner/repo.git"));
		assert!(!is_github_issue_url("https://github.com/owner/repo/pull/123"));
		assert!(!is_github_issue_url("https://gitlab.com/owner/repo/issues/123"));
	}
}
