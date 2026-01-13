use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use v_utils::prelude::*;

use crate::config::LiveSettings;

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubIssue {
	pub number: u64,
	pub title: String,
	pub body: Option<String>,
	pub labels: Vec<GitHubLabel>,
	pub user: GitHubUser,
	pub state: String, // "open" or "closed" //TODO!!!!: add an actual enum
	/// Reason for the state (e.g., "completed", "not_planned", "duplicate")
	/// Only present for closed issues.
	pub state_reason: Option<String>,
	/// Last time the issue was updated (ISO 8601 format)
	pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize)]
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
pub struct OriginalSubIssue {
	pub number: u64,
	pub state: String,
}

impl From<&GitHubIssue> for OriginalSubIssue {
	fn from(s: &GitHubIssue) -> Self {
		Self {
			number: s.number,
			state: s.state.clone(),
		}
	}
}

/// Response from GitHub when creating an issue
#[derive(Debug, Deserialize)]
pub struct CreatedIssue {
	pub id: u64,
	pub number: u64,
	pub html_url: String,
}

/// Index path to locate an issue in the tree (e.g., [0, 2] = first child's third child)
pub type IssuePath = Vec<usize>;

/// An action that needs to be performed on GitHub
#[derive(Debug)]
pub enum IssueAction {
	/// Create a new issue, optionally as a sub-issue of a parent
	CreateIssue {
		/// Path to this issue in the tree (empty for root)
		path: IssuePath,
		/// Title for the new issue
		title: String,
		/// Body for the new issue
		body: String,
		/// Whether it should be closed after creation
		closed: bool,
		/// Parent issue number if this is a sub-issue
		parent: Option<u64>,
	},
	/// Update an existing issue's state (open/closed)
	UpdateIssueState { issue_number: u64, closed: bool },
}

//==============================================================================
// GitHub Client Trait
//==============================================================================

/// Trait defining all GitHub API operations.
/// This allows for both real API calls and mock implementations for testing.
#[async_trait]
pub trait GitHubClient: Send + Sync {
	/// Fetch the authenticated user's login name
	async fn fetch_authenticated_user(&self) -> Result<String>;

	/// Fetch a single issue by number
	async fn fetch_issue(&self, owner: &str, repo: &str, issue_number: u64) -> Result<GitHubIssue>;

	/// Fetch all comments on an issue
	async fn fetch_comments(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubComment>>;

	/// Fetch all sub-issues of an issue
	async fn fetch_sub_issues(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubIssue>>;

	/// Update an issue's body
	async fn update_issue_body(&self, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()>;

	/// Update an issue's state (open/closed)
	async fn update_issue_state(&self, owner: &str, repo: &str, issue_number: u64, state: &str) -> Result<()>;

	/// Update a comment's body
	async fn update_comment(&self, owner: &str, repo: &str, comment_id: u64, body: &str) -> Result<()>;

	/// Create a new comment on an issue
	async fn create_comment(&self, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()>;

	/// Delete a comment
	async fn delete_comment(&self, owner: &str, repo: &str, comment_id: u64) -> Result<()>;

	/// Check if the authenticated user has collaborator (push/write) access
	async fn check_collaborator_access(&self, owner: &str, repo: &str) -> Result<bool>;

	/// Create a new issue
	async fn create_issue(&self, owner: &str, repo: &str, title: &str, body: &str) -> Result<CreatedIssue>;

	/// Add a sub-issue to a parent issue
	/// Note: `child_issue_id` is the resource ID (not the issue number)
	async fn add_sub_issue(&self, owner: &str, repo: &str, parent_issue_number: u64, child_issue_id: u64) -> Result<()>;

	/// Find an issue by exact title match
	async fn find_issue_by_title(&self, owner: &str, repo: &str, title: &str) -> Result<Option<u64>>;

	/// Check if an issue exists by number
	async fn issue_exists(&self, owner: &str, repo: &str, issue_number: u64) -> Result<bool>;

	/// Fetch the parent issue of a sub-issue (returns None if issue has no parent)
	async fn fetch_parent_issue(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Option<GitHubIssue>>;
}

//==============================================================================
// Real GitHub Client Implementation
//==============================================================================

/// Real GitHub API client that makes HTTP requests
pub struct RealGitHubClient {
	http_client: Client,
	github_token: String,
}

impl RealGitHubClient {
	pub fn new(settings: &LiveSettings) -> Result<Self> {
		let config = settings.config()?;
		let milestones_config = config
			.milestones
			.as_ref()
			.ok_or_else(|| eyre!("milestones config section is required for GitHub token. Add [milestones] section with github_token to your config"))?;

		Ok(Self {
			http_client: Client::new(),
			github_token: milestones_config.github_token.clone(),
		})
	}

	fn auth_header(&self) -> String {
		format!("token {}", self.github_token)
	}
}

#[async_trait]
impl GitHubClient for RealGitHubClient {
	async fn fetch_authenticated_user(&self) -> Result<String> {
		let res = self
			.http_client
			.get("https://api.github.com/user")
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to fetch authenticated user: {status} - {body}");
		}

		let user = res.json::<GitHubUser>().await?;
		Ok(user.login)
	}

	async fn fetch_issue(&self, owner: &str, repo: &str, issue_number: u64) -> Result<GitHubIssue> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to fetch issue: {status} - {body}");
		}

		let issue = res.json::<GitHubIssue>().await?;
		Ok(issue)
	}

	async fn fetch_comments(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubComment>> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/comments");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to fetch comments: {status} - {body}");
		}

		let comments = res.json::<Vec<GitHubComment>>().await?;
		Ok(comments)
	}

	async fn fetch_sub_issues(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubIssue>> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/sub_issues");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		if !res.status().is_success() {
			// Sub-issues API might not be available or issue has no sub-issues
			// Return empty vec instead of erroring
			return Ok(Vec::new());
		}

		let sub_issues = res.json::<Vec<GitHubIssue>>().await?;
		Ok(sub_issues)
	}

	async fn update_issue_body(&self, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

		let res = self
			.http_client
			.patch(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.header("Content-Type", "application/json")
			.json(&serde_json::json!({ "body": body }))
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to update issue body: {status} - {body}");
		}

		Ok(())
	}

	async fn update_issue_state(&self, owner: &str, repo: &str, issue_number: u64, state: &str) -> Result<()> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

		let res = self
			.http_client
			.patch(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.header("Content-Type", "application/json")
			.json(&serde_json::json!({ "state": state }))
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to update issue state: {status} - {body}");
		}

		Ok(())
	}

	async fn update_comment(&self, owner: &str, repo: &str, comment_id: u64, body: &str) -> Result<()> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}");

		let res = self
			.http_client
			.patch(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.header("Content-Type", "application/json")
			.json(&serde_json::json!({ "body": body }))
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to update comment: {status} - {body}");
		}

		Ok(())
	}

	async fn create_comment(&self, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/comments");

		let res = self
			.http_client
			.post(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.header("Content-Type", "application/json")
			.json(&serde_json::json!({ "body": body }))
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to create comment: {status} - {body}");
		}

		Ok(())
	}

	async fn delete_comment(&self, owner: &str, repo: &str, comment_id: u64) -> Result<()> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/comments/{comment_id}");

		let res = self
			.http_client
			.delete(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to delete comment: {status} - {body}");
		}

		Ok(())
	}

	async fn check_collaborator_access(&self, owner: &str, repo: &str) -> Result<bool> {
		// Get the authenticated user's login
		let current_user = self.fetch_authenticated_user().await?;

		// Check if user is a collaborator with write access
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/collaborators/{current_user}/permission");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
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

	async fn create_issue(&self, owner: &str, repo: &str, title: &str, body: &str) -> Result<CreatedIssue> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues");

		let res = self
			.http_client
			.post(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.header("Content-Type", "application/json")
			.json(&serde_json::json!({ "title": title, "body": body }))
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to create issue: {status} - {body}");
		}

		let issue = res.json::<CreatedIssue>().await?;
		Ok(issue)
	}

	async fn add_sub_issue(&self, owner: &str, repo: &str, parent_issue_number: u64, child_issue_id: u64) -> Result<()> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{parent_issue_number}/sub_issues");

		let res = self
			.http_client
			.post(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.header("Content-Type", "application/json")
			.json(&serde_json::json!({ "sub_issue_id": child_issue_id }))
			.send()
			.await?;

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to add sub-issue: {status} - {body}");
		}

		Ok(())
	}

	async fn find_issue_by_title(&self, owner: &str, repo: &str, title: &str) -> Result<Option<u64>> {
		// Search for issues with this title (search in open and closed)
		let encoded_title = urlencoding::encode(title);
		let api_url = format!("https://api.github.com/search/issues?q=repo:{owner}/{repo}+in:title+{encoded_title}");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
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

	async fn issue_exists(&self, owner: &str, repo: &str, issue_number: u64) -> Result<bool> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		Ok(res.status().is_success())
	}

	async fn fetch_parent_issue(&self, owner: &str, repo: &str, issue_number: u64) -> Result<Option<GitHubIssue>> {
		let api_url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/parent");

		let res = self
			.http_client
			.get(&api_url)
			.header("User-Agent", "Rust GitHub Client")
			.header("Authorization", self.auth_header())
			.send()
			.await?;

		if res.status() == reqwest::StatusCode::NOT_FOUND {
			// Issue has no parent
			return Ok(None);
		}

		if !res.status().is_success() {
			let status = res.status();
			let body = res.text().await.unwrap_or_default();
			bail!("Failed to fetch parent issue: {status} - {body}");
		}

		let parent = res.json::<GitHubIssue>().await?;
		Ok(Some(parent))
	}
}

//==============================================================================
// Convenience type alias for boxed client
//==============================================================================

pub type BoxedGitHubClient = Arc<dyn GitHubClient>;

/// Create a GitHub client from settings.
/// Returns an error if GitHub token is not configured.
pub fn create_client(settings: &LiveSettings) -> Result<BoxedGitHubClient> {
	Ok(Arc::new(RealGitHubClient::new(settings)?))
}

//==============================================================================
// Utility functions (URL parsing, etc.) - These don't need the trait
//==============================================================================

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
		bail!(
			"SSH URL format doesn't support issue numbers. Use HTTPS format: https://github.com/{}/issues/NUMBER",
			path.strip_suffix(".git").unwrap_or(path)
		);
	}

	// Try ssh:// format: ssh://git@github.com/owner/repo.git
	if let Some(path) = url.strip_prefix("ssh://git@github.com/") {
		bail!(
			"SSH URL format doesn't support issue numbers. Use HTTPS format: https://github.com/{}/issues/NUMBER",
			path.strip_suffix(".git").unwrap_or(path)
		);
	}

	// Remove protocol prefix if present (https://, http://)
	let path = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);

	// Remove github.com prefix
	let path = path.strip_prefix("github.com/").ok_or_else(|| eyre!("URL must be a GitHub URL: {url}"))?;

	// Split by /
	let parts: Vec<&str> = path.split('/').collect();

	if parts.len() < 4 || parts[2] != "issues" {
		bail!("Invalid GitHub issue URL format. Expected: https://github.com/owner/repo/issues/123");
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

/// Extract issue number from a GitHub URL
pub fn extract_issue_number_from_url(url: &str) -> Option<u64> {
	// URL format: https://github.com/owner/repo/issues/123
	url.split('/').next_back().and_then(|s| s.parse().ok())
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
