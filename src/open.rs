use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use v_utils::prelude::*;

use crate::config::LiveSettings;

/// Returns the base directory for issue storage: XDG_DATA_HOME/todo/issues/
fn issues_dir() -> PathBuf {
	v_utils::xdg_data_dir!("issues")
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Extension {
	#[default]
	Md,
	Typ,
}

impl Extension {
	fn as_str(&self) -> &'static str {
		match self {
			Extension::Md => "md",
			Extension::Typ => "typ",
		}
	}
}

#[derive(Args)]
pub struct OpenArgs {
	/// GitHub issue URL (e.g., https://github.com/owner/repo/issues/123) OR a search pattern for local issue files
	/// With --touch: path format is workspace/project/{issue.md, issue/sub-issue.md}
	pub url_or_pattern: String,

	/// File extension for the output file (overrides config default_extension)
	#[arg(short = 'e', long)]
	pub extension: Option<Extension>,

	/// Render full contents even for closed issues (by default, closed issues show only title with <!-- omitted -->)
	#[arg(long)]
	pub render_closed: bool,

	/// Create or open an issue from a path. Path format: workspace/project/issue[.md|.typ]
	/// For sub-issues: workspace/project/parent/child (parent must exist on GitHub)
	/// If issue already exists locally, opens it. Otherwise creates on GitHub first.
	#[arg(short = 't', long)]
	pub touch: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubIssue {
	number: u64,
	title: String,
	body: Option<String>,
	labels: Vec<GitHubLabel>,
	user: GitHubUser,
	state: String, // "open" or "closed"
}

/// Sub-issue as returned by the GitHub API (same structure as issue for our purposes)
#[derive(Debug, Deserialize)]
struct GitHubSubIssue {
	number: u64,
	title: String,
	state: String, // "open" or "closed"
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
	name: String,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubUser {
	login: String,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubComment {
	id: u64,
	body: Option<String>,
	user: GitHubUser,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OriginalComment {
	id: u64,
	body: Option<String>,
}

impl From<&GitHubComment> for OriginalComment {
	fn from(c: &GitHubComment) -> Self {
		Self { id: c.id, body: c.body.clone() }
	}
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OriginalSubIssue {
	number: u64,
	state: String,
}

impl From<&GitHubSubIssue> for OriginalSubIssue {
	fn from(s: &GitHubSubIssue) -> Self {
		Self {
			number: s.number,
			state: s.state.clone(),
		}
	}
}

/// Target state after user edits - clean representation of what the issue should look like
#[derive(Debug, PartialEq)]
struct TargetState {
	issue_body: String,
	/// Comments in order. None id = new comment to create
	comments: Vec<TargetComment>,
	/// Sub-issues with their checked state (true = closed)
	sub_issues: Vec<TargetSubIssue>,
	/// New sub-issues to create (title only, no number yet)
	new_sub_issues: Vec<NewSubIssue>,
}

#[derive(Debug, PartialEq)]
struct TargetSubIssue {
	number: u64,
	closed: bool,
}

#[derive(Debug, PartialEq)]
struct NewSubIssue {
	title: String,
	closed: bool,
}

#[derive(Debug, PartialEq)]
struct TargetComment {
	id: Option<u64>,
	body: String,
}

/// Parse a GitHub issue URL and extract owner, repo, and issue number.
/// Supports formats like:
/// - https://github.com/owner/repo/issues/123
/// - github.com/owner/repo/issues/123
/// - git@github.com:owner/repo (returns repo info, issue number parsing will fail)
/// - ssh://git@github.com/owner/repo.git (returns repo info, issue number parsing will fail)
fn parse_github_issue_url(url: &str) -> Result<(String, String, u64)> {
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
fn is_github_issue_url(s: &str) -> bool {
	let s = s.trim();
	s.contains("github.com/") && s.contains("/issues/")
}

/// Get the path for an issue file in XDG_DATA.
/// Structure: issues/{owner}/{issue_number}.{ext}
/// If the issue has sub-issues, a directory {owner}/{issue_number}/ will contain them.
fn get_issue_file_path(owner: &str, issue_number: u64, extension: &Extension, parent_issue: Option<u64>) -> PathBuf {
	let base = issues_dir().join(owner);
	match parent_issue {
		None => base.join(format!("{}.{}", issue_number, extension.as_str())),
		Some(parent) => base.join(parent.to_string()).join(format!("{}.{}", issue_number, extension.as_str())),
	}
}

/// Get the directory path for sub-issues of a given issue.
/// Structure: issues/{owner}/{issue_number}/
fn get_sub_issues_dir(owner: &str, issue_number: u64) -> PathBuf {
	issues_dir().join(owner).join(issue_number.to_string())
}

/// Stored metadata about an issue file for syncing
#[derive(Debug, Deserialize, Serialize)]
struct IssueFileMeta {
	owner: String,
	repo: String,
	issue_number: u64,
	extension: String,
	/// Original issue body (for diffing)
	original_issue_body: Option<String>,
	/// Original comments with their IDs
	original_comments: Vec<OriginalComment>,
	/// Original sub-issues with their state
	original_sub_issues: Vec<OriginalSubIssue>,
}

/// Get the metadata file path for an issue (stored alongside the issue file)
fn get_issue_meta_path(issue_file_path: &Path) -> PathBuf {
	let file_name = issue_file_path.file_stem().unwrap_or_default().to_string_lossy();
	issue_file_path.with_file_name(format!(".{}.meta.json", file_name))
}

/// Search for issue files matching a pattern in the issues directory
/// Pattern can be:
/// - Issue number: "123" -> searches for any file named 123.md or 123.typ
/// - Owner pattern: "owner" -> searches in owner/ directory
/// - Owner/number: "owner/123" -> specific issue
fn search_issue_files(pattern: &str) -> Result<Vec<PathBuf>> {
	use std::process::Command;

	let issues_dir = issues_dir();
	if !issues_dir.exists() {
		return Ok(Vec::new());
	}

	// Search for both .md and .typ files
	let output = Command::new("find")
		.args([issues_dir.to_str().unwrap(), "(", "-name", "*.md", "-o", "-name", "*.typ", ")", "-type", "f", "!", "-name", ".*"])
		.output()?;

	if !output.status.success() {
		return Err(eyre!("Failed to search for issue files"));
	}

	let all_files = String::from_utf8(output.stdout)?;
	let mut matches = Vec::new();

	let pattern_lower = pattern.to_lowercase();

	for line in all_files.lines() {
		let file_path = line.trim();
		if file_path.is_empty() {
			continue;
		}

		let path = PathBuf::from(file_path);

		// Get relative path from issues_dir
		let relative = if let Ok(rel) = path.strip_prefix(&issues_dir) {
			rel.to_string_lossy().to_string()
		} else {
			continue;
		};

		let relative_lower = relative.to_lowercase();

		// Check if pattern matches:
		// - The issue number (filename without extension)
		// - The owner (first path component)
		// - The full relative path
		if let Some(file_stem) = path.file_stem() {
			let file_stem_str = file_stem.to_string_lossy().to_lowercase();
			if file_stem_str.contains(&pattern_lower) || relative_lower.contains(&pattern_lower) {
				matches.push(path);
			}
		}
	}

	Ok(matches)
}

/// Use fzf to let user choose from multiple issue file matches
fn choose_issue_with_fzf(matches: &[PathBuf], initial_query: &str) -> Result<Option<PathBuf>> {
	use std::{
		io::Write as IoWrite,
		process::{Command, Stdio},
	};

	let issues_dir = issues_dir();

	// Prepare input for fzf - show relative paths
	let input: String = matches
		.iter()
		.filter_map(|p| p.strip_prefix(&issues_dir).ok().map(|r| r.to_string_lossy().to_string()))
		.collect::<Vec<_>>()
		.join("\n");

	let mut fzf = Command::new("fzf").args(["--query", initial_query]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;

	if let Some(stdin) = fzf.stdin.take() {
		let mut stdin_handle = stdin;
		stdin_handle.write_all(input.as_bytes())?;
	}

	let output = fzf.wait_with_output()?;

	if output.status.success() {
		let chosen = String::from_utf8(output.stdout)?.trim().to_string();
		Ok(Some(issues_dir.join(chosen)))
	} else {
		Ok(None)
	}
}

/// Parse issue metadata from a local issue file's meta file
fn load_issue_meta(issue_file_path: &Path) -> Result<IssueFileMeta> {
	let meta_path = get_issue_meta_path(issue_file_path);
	let content = std::fs::read_to_string(&meta_path).map_err(|_| eyre!("No metadata found for issue file: {:?}", issue_file_path))?;
	let meta: IssueFileMeta = serde_json::from_str(&content)?;
	Ok(meta)
}

/// Save issue metadata alongside the issue file
fn save_issue_meta(issue_file_path: &Path, meta: &IssueFileMeta) -> Result<()> {
	let meta_path = get_issue_meta_path(issue_file_path);
	let content = serde_json::to_string_pretty(meta)?;
	std::fs::write(&meta_path, content)?;
	Ok(())
}

async fn fetch_authenticated_user(settings: &LiveSettings) -> Result<String> {
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

async fn fetch_github_issue(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<GitHubIssue> {
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

async fn fetch_github_comments(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubComment>> {
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

async fn fetch_github_sub_issues(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<Vec<GitHubSubIssue>> {
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

async fn update_github_issue_body(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()> {
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

async fn update_github_issue_state(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64, state: &str) -> Result<()> {
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

async fn update_github_comment(settings: &LiveSettings, owner: &str, repo: &str, comment_id: u64, body: &str) -> Result<()> {
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

async fn create_github_comment(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64, body: &str) -> Result<()> {
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

async fn delete_github_comment(settings: &LiveSettings, owner: &str, repo: &str, comment_id: u64) -> Result<()> {
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
async fn check_collaborator_access(settings: &LiveSettings, owner: &str, repo: &str) -> Result<bool> {
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

/// Response from GitHub when creating an issue
#[derive(Debug, Deserialize)]
struct CreatedIssue {
	number: u64,
	html_url: String,
}

/// Create a new GitHub issue
async fn create_github_issue(settings: &LiveSettings, owner: &str, repo: &str, title: &str, body: &str) -> Result<CreatedIssue> {
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
async fn add_sub_issue(settings: &LiveSettings, owner: &str, repo: &str, parent_issue_number: u64, child_issue_number: u64) -> Result<()> {
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
async fn find_issue_by_title(settings: &LiveSettings, owner: &str, repo: &str, title: &str) -> Result<Option<u64>> {
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
async fn issue_exists(settings: &LiveSettings, owner: &str, repo: &str, issue_number: u64) -> Result<bool> {
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

fn format_issue_as_markdown(issue: &GitHubIssue, comments: &[GitHubComment], sub_issues: &[GitHubSubIssue], owner: &str, repo: &str, current_user: &str, render_closed: bool) -> String {
	let mut content = String::new();

	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	let issue_owned = issue.user.login == current_user;
	let issue_closed = issue.state == "closed";
	let checked = if issue_closed { "x" } else { " " };

	// Issue title as checkbox item with URL inline
	if issue_owned {
		content.push_str(&format!("- [{checked}] {} <!--{}-->\n", issue.title, issue_url));
	} else {
		content.push_str(&format!("- [{checked}] {} <!--immutable {}-->\n", issue.title, issue_url));
	}

	// If issue is closed and render_closed is false, omit contents
	if issue_closed && !render_closed {
		content.push_str("\t<!-- omitted -->\n");
		return content;
	}

	// Labels if any (indented under the issue)
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("\t**Labels:** {}\n", labels.join(", ")));
	}

	// Body (indented under the issue)
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			content.push('\n');
			if issue_owned {
				for line in body.lines() {
					content.push_str(&format!("\t{}\n", line));
				}
			} else {
				// Double indent for immutable body
				for line in body.lines() {
					content.push_str(&format!("\t\t{}\n", line));
				}
			}
		}
	}

	// Sub-issues (indented under the issue, after body)
	if !sub_issues.is_empty() {
		content.push('\n');
		for sub in sub_issues {
			let sub_url = format!("https://github.com/{owner}/{repo}/issues/{}", sub.number);
			let sub_checked = if sub.state == "closed" { "x" } else { " " };
			// Sub-issues are read-only (we fetch their title/state, but don't manage them here)
			content.push_str(&format!("\t- [{sub_checked}] {} <!--sub {}-->\n", sub.title, sub_url));
		}
	}

	// Comments (indented under the issue)
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		let comment_owned = comment.user.login == current_user;

		content.push('\n');
		if comment_owned {
			content.push_str(&format!("\t<!--{}-->\n", comment_url));
		} else {
			content.push_str(&format!("\t<!--immutable {}-->\n", comment_url));
		}

		if let Some(body) = &comment.body {
			if !body.is_empty() {
				if comment_owned {
					for line in body.lines() {
						content.push_str(&format!("\t{}\n", line));
					}
				} else {
					// Double indent for immutable comments
					for line in body.lines() {
						content.push_str(&format!("\t\t{}\n", line));
					}
				}
			}
		}
	}

	content
}

fn convert_markdown_to_typst(body: &str) -> String {
	body.lines()
		.map(|line| {
			// Convert markdown headers to typst
			if let Some(rest) = line.strip_prefix("### ") {
				format!("=== {}", rest)
			} else if let Some(rest) = line.strip_prefix("## ") {
				format!("== {}", rest)
			} else if let Some(rest) = line.strip_prefix("# ") {
				format!("= {}", rest)
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

fn format_issue_as_typst(issue: &GitHubIssue, comments: &[GitHubComment], sub_issues: &[GitHubSubIssue], owner: &str, repo: &str, current_user: &str, render_closed: bool) -> String {
	let mut content = String::new();

	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	let issue_owned = issue.user.login == current_user;
	let issue_closed = issue.state == "closed";
	let checked = if issue_closed { "x" } else { " " };

	// Issue title as checkbox item with URL inline (using typst comment syntax)
	if issue_owned {
		content.push_str(&format!("- [{checked}] {} // {}\n", issue.title, issue_url));
	} else {
		content.push_str(&format!("- [{checked}] {} // immutable {}\n", issue.title, issue_url));
	}

	// If issue is closed and render_closed is false, omit contents
	if issue_closed && !render_closed {
		content.push_str("\t// omitted\n");
		return content;
	}

	// Labels if any (indented under the issue)
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("\t*Labels:* {}\n", labels.join(", ")));
	}

	// Body - convert markdown to typst basics (indented under the issue)
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			content.push('\n');
			let converted = convert_markdown_to_typst(body);
			if issue_owned {
				for line in converted.lines() {
					content.push_str(&format!("\t{}\n", line));
				}
			} else {
				// Double indent for immutable body
				for line in converted.lines() {
					content.push_str(&format!("\t\t{}\n", line));
				}
			}
		}
	}

	// Sub-issues (indented under the issue, after body)
	if !sub_issues.is_empty() {
		content.push('\n');
		for sub in sub_issues {
			let sub_url = format!("https://github.com/{owner}/{repo}/issues/{}", sub.number);
			let sub_checked = if sub.state == "closed" { "x" } else { " " };
			content.push_str(&format!("\t- [{sub_checked}] {} // sub {}\n", sub.title, sub_url));
		}
	}

	// Comments (indented under the issue)
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		let comment_owned = comment.user.login == current_user;

		content.push('\n');
		if comment_owned {
			content.push_str(&format!("\t// {}\n", comment_url));
		} else {
			content.push_str(&format!("\t// immutable {}\n", comment_url));
		}

		if let Some(body) = &comment.body {
			if !body.is_empty() {
				let converted = convert_markdown_to_typst(body);
				if comment_owned {
					for line in converted.lines() {
						content.push_str(&format!("\t{}\n", line));
					}
				} else {
					// Double indent for immutable comments
					for line in converted.lines() {
						content.push_str(&format!("\t\t{}\n", line));
					}
				}
			}
		}
	}

	content
}

/// Helper to strip leading tab from each line (for immutable sections that user might have edited)
fn unindent_body(body: &str) -> String {
	body.lines().map(|line| line.strip_prefix('\t').unwrap_or(line)).collect::<Vec<_>>().join("\n")
}

/// Marker type for parsed markdown markers
#[derive(Debug, PartialEq)]
enum MdMarkerType {
	/// Issue URL (from first line `- [ ] Title <!--url-->`)
	Issue { is_immutable: bool, url: String },
	/// Sub-issue (`- [x] Title <!--sub url-->`)
	SubIssue { number: u64, closed: bool },
	/// Comment URL (`<!--url#issuecomment-id-->`)
	Comment { is_immutable: bool, url: String, id: u64 },
	/// New comment marker (`<!--new comment-->`)
	NewComment,
}

/// Check if a line is a new sub-issue (checkbox without any marker).
/// Format: `\t- [ ] Title` or `\t- [x] Title` (must be indented with one tab)
/// Returns Some((title, closed)) if it's a new sub-issue, None otherwise.
fn parse_new_sub_issue_line(line: &str) -> Option<(String, bool)> {
	// Must start with exactly one tab (indented under the main issue)
	let stripped = line.strip_prefix('\t')?;

	// Must not have further indentation (not a nested item)
	if stripped.starts_with('\t') || stripped.starts_with(' ') {
		return None;
	}

	// Must be a checkbox item
	let (closed, rest) = if let Some(rest) = stripped.strip_prefix("- [ ] ") {
		(false, rest)
	} else if let Some(rest) = stripped.strip_prefix("- [x] ").or_else(|| stripped.strip_prefix("- [X] ")) {
		(true, rest)
	} else {
		return None;
	};

	// Must NOT have any marker (<!--...-->)
	if rest.contains("<!--") {
		return None;
	}

	let title = rest.trim().to_string();
	if title.is_empty() {
		return None;
	}

	Some((title, closed))
}

/// Parse a markdown HTML comment marker from anywhere in a line.
/// Returns the marker type if found, None otherwise.
fn parse_md_marker(line: &str) -> Option<MdMarkerType> {
	// Find the marker in the line
	let start = line.find("<!--")?;
	let end = line.find("-->")?;
	if end <= start {
		return None;
	}

	let inner = line[start + 4..end].trim();

	if inner == "new comment" {
		return Some(MdMarkerType::NewComment);
	}

	// Check for sub-issue marker: `- [x] Title <!--sub url-->`
	if let Some(url) = inner.strip_prefix("sub ") {
		// Extract issue number from URL (last path segment)
		let number = url.trim().rsplit('/').next().and_then(|s| s.parse::<u64>().ok())?;
		// Check if checkbox is checked by looking for `- [x]` before the marker
		let prefix = &line[..start];
		let closed = prefix.contains("[x]") || prefix.contains("[X]");
		return Some(MdMarkerType::SubIssue { number, closed });
	}

	// Check for immutable marker
	let (is_immutable, url) = if let Some(url) = inner.strip_prefix("immutable ") {
		(true, url.trim())
	} else if inner.starts_with("https://github.com/") {
		(false, inner)
	} else {
		return None;
	};

	// Determine if this is issue or comment by URL
	if url.contains("#issuecomment-") {
		let id = url.split("#issuecomment-").last().and_then(|s| s.parse::<u64>().ok())?;
		Some(MdMarkerType::Comment {
			is_immutable,
			url: url.to_string(),
			id,
		})
	} else {
		Some(MdMarkerType::Issue { is_immutable, url: url.to_string() })
	}
}

/// Parse markdown content into target state.
/// New format: `- [ ] Title <!--url-->` on first line, content indented with tabs.
/// Sub-issues are parsed for state changes.
/// New comments are marked with `<!--new comment-->`.
/// New sub-issues are checkbox lines without URL markers.
fn parse_markdown_target(content: &str) -> TargetState {
	let mut issue_body = String::new();
	let mut comments: Vec<TargetComment> = Vec::new();
	let mut sub_issues: Vec<TargetSubIssue> = Vec::new();
	let mut new_sub_issues: Vec<NewSubIssue> = Vec::new();
	let mut current_body = String::new();
	let mut current_comment_id: Option<u64> = None;
	let mut current_is_immutable = false;
	let mut is_new_comment = false;
	let mut seen_issue_marker = false;
	let mut in_labels_line = false;
	let mut in_issue_body = false;

	for line in content.lines() {
		// Strip one level of indentation for content parsing
		let stripped_line = line.strip_prefix('\t').unwrap_or(line);

		// Check for new sub-issues first (checkbox lines without markers)
		// Only check after we've seen the issue marker (so we're in the issue body area)
		if seen_issue_marker {
			if let Some((title, closed)) = parse_new_sub_issue_line(line) {
				new_sub_issues.push(NewSubIssue { title, closed });
				continue;
			}
		}

		// Check for markers in the line
		if let Some(marker) = parse_md_marker(line) {
			match marker {
				MdMarkerType::Issue { is_immutable, .. } => {
					// First line: `- [ ] Title <!--url-->`
					seen_issue_marker = true;
					current_is_immutable = is_immutable;
					in_issue_body = true;
					current_body.clear();
					continue;
				}
				MdMarkerType::SubIssue { number, closed } => {
					// Track sub-issue state
					sub_issues.push(TargetSubIssue { number, closed });
					continue;
				}
				MdMarkerType::Comment { is_immutable, id, .. } => {
					// Flush previous section
					let body = unindent_body(&current_body).trim().to_string();

					if in_issue_body && issue_body.is_empty() {
						if !current_is_immutable {
							issue_body = body;
						}
					} else if let Some(prev_id) = current_comment_id {
						if !current_is_immutable {
							comments.push(TargetComment { id: Some(prev_id), body });
						}
					} else if is_new_comment && !body.is_empty() {
						comments.push(TargetComment { id: None, body });
					}

					current_comment_id = Some(id);
					current_is_immutable = is_immutable;
					is_new_comment = false;
					in_issue_body = false;
					current_body.clear();
					in_labels_line = false;
					continue;
				}
				MdMarkerType::NewComment => {
					// Flush previous section
					let body = unindent_body(&current_body).trim().to_string();

					if in_issue_body && issue_body.is_empty() {
						if !current_is_immutable {
							issue_body = body;
						}
					} else if let Some(id) = current_comment_id {
						if !current_is_immutable {
							comments.push(TargetComment { id: Some(id), body });
						}
					} else if is_new_comment && !body.is_empty() {
						comments.push(TargetComment { id: None, body });
					}

					current_comment_id = None;
					current_is_immutable = false;
					is_new_comment = true;
					in_issue_body = false;
					current_body.clear();
					in_labels_line = false;
					continue;
				}
			}
		}

		// Skip labels line (indented)
		if stripped_line.starts_with("**Labels:**") {
			in_labels_line = true;
			continue;
		}

		// After labels line, skip one empty line
		if in_labels_line && stripped_line.is_empty() {
			in_labels_line = false;
			continue;
		}

		// Accumulate body content (keep original line with indentation for proper unindent later)
		current_body.push_str(line);
		current_body.push('\n');
	}

	// Flush final section
	let body = unindent_body(&current_body).trim().to_string();
	if !seen_issue_marker {
		// No issue marker at all - treat everything as issue body
		issue_body = body;
	} else if in_issue_body && issue_body.is_empty() {
		// Was collecting issue body
		if !current_is_immutable {
			issue_body = body;
		}
	} else if let Some(id) = current_comment_id {
		// Last section was a tracked comment
		if !current_is_immutable {
			comments.push(TargetComment { id: Some(id), body });
		}
	} else if is_new_comment && !body.is_empty() {
		// Last section was a new comment
		comments.push(TargetComment { id: None, body });
	}

	TargetState {
		issue_body,
		comments,
		sub_issues,
		new_sub_issues,
	}
}

/// Marker type for parsed typst markers
#[derive(Debug, PartialEq)]
enum TypstMarkerType {
	/// Issue URL (from first line `- [ ] Title // url`)
	Issue { is_immutable: bool, url: String },
	/// Sub-issue (`- [x] Title // sub url`)
	SubIssue { number: u64, closed: bool },
	/// Comment URL (`// url#issuecomment-id`)
	Comment { is_immutable: bool, url: String, id: u64 },
	/// New comment marker (`// new comment`)
	NewComment,
}

/// Check if a line is a new sub-issue in typst format (checkbox without any marker).
/// Format: `\t- [ ] Title` or `\t- [x] Title` (must be indented with one tab, no // marker)
/// Returns Some((title, closed)) if it's a new sub-issue, None otherwise.
fn parse_new_sub_issue_line_typst(line: &str) -> Option<(String, bool)> {
	// Must start with exactly one tab (indented under the main issue)
	let stripped = line.strip_prefix('\t')?;

	// Must not have further indentation (not a nested item)
	if stripped.starts_with('\t') || stripped.starts_with(' ') {
		return None;
	}

	// Must be a checkbox item
	let (closed, rest) = if let Some(rest) = stripped.strip_prefix("- [ ] ") {
		(false, rest)
	} else if let Some(rest) = stripped.strip_prefix("- [x] ").or_else(|| stripped.strip_prefix("- [X] ")) {
		(true, rest)
	} else {
		return None;
	};

	// Must NOT have any marker (// followed by something)
	if rest.contains(" // ") {
		return None;
	}

	let title = rest.trim().to_string();
	if title.is_empty() {
		return None;
	}

	Some((title, closed))
}

/// Parse a typst comment marker from anywhere in a line.
/// Returns the marker type if found, None otherwise.
fn parse_typst_marker(line: &str) -> Option<TypstMarkerType> {
	// Find the marker in the line (// at the end for inline, or at start for standalone)
	let (prefix, inner) = if let Some(pos) = line.find(" // ") {
		// Inline marker: `- [ ] Title // url`
		(&line[..pos], line[pos + 4..].trim())
	} else if line.trim().starts_with("// ") {
		// Standalone marker: `// url`
		("", line.trim().strip_prefix("// ")?.trim())
	} else {
		return None;
	};

	if inner == "new comment" {
		return Some(TypstMarkerType::NewComment);
	}

	// Check for sub-issue marker: `- [x] Title // sub url`
	if let Some(url) = inner.strip_prefix("sub ") {
		// Extract issue number from URL (last path segment)
		let number = url.trim().rsplit('/').next().and_then(|s| s.parse::<u64>().ok())?;
		// Check if checkbox is checked by looking for `- [x]` before the marker
		let closed = prefix.contains("[x]") || prefix.contains("[X]");
		return Some(TypstMarkerType::SubIssue { number, closed });
	}

	// Check for immutable marker
	let (is_immutable, url) = if let Some(url) = inner.strip_prefix("immutable ") {
		(true, url.trim())
	} else if inner.starts_with("https://github.com/") {
		(false, inner)
	} else {
		return None;
	};

	// Determine if this is issue or comment by URL
	if url.contains("#issuecomment-") {
		let id = url.split("#issuecomment-").last().and_then(|s| s.parse::<u64>().ok())?;
		Some(TypstMarkerType::Comment {
			is_immutable,
			url: url.to_string(),
			id,
		})
	} else {
		Some(TypstMarkerType::Issue { is_immutable, url: url.to_string() })
	}
}

/// Parse typst content into target state.
/// New format: `- [ ] Title // url` on first line, content indented with tabs.
/// Sub-issues are parsed for state changes.
/// New comments are marked with `// new comment`.
/// New sub-issues are checkbox lines without URL markers.
fn parse_typst_target(content: &str) -> TargetState {
	let mut issue_body = String::new();
	let mut comments: Vec<TargetComment> = Vec::new();
	let mut sub_issues: Vec<TargetSubIssue> = Vec::new();
	let mut new_sub_issues: Vec<NewSubIssue> = Vec::new();
	let mut current_body = String::new();
	let mut current_comment_id: Option<u64> = None;
	let mut current_is_immutable = false;
	let mut is_new_comment = false;
	let mut seen_issue_marker = false;
	let mut in_labels_line = false;
	let mut in_issue_body = false;

	for line in content.lines() {
		// Strip one level of indentation for content parsing
		let stripped_line = line.strip_prefix('\t').unwrap_or(line);

		// Check for new sub-issues first (checkbox lines without markers)
		// Only check after we've seen the issue marker (so we're in the issue body area)
		if seen_issue_marker {
			if let Some((title, closed)) = parse_new_sub_issue_line_typst(line) {
				new_sub_issues.push(NewSubIssue { title, closed });
				continue;
			}
		}

		// Check for markers in the line
		if let Some(marker) = parse_typst_marker(line) {
			match marker {
				TypstMarkerType::Issue { is_immutable, .. } => {
					// First line: `- [ ] Title // url`
					seen_issue_marker = true;
					current_is_immutable = is_immutable;
					in_issue_body = true;
					current_body.clear();
					continue;
				}
				TypstMarkerType::SubIssue { number, closed } => {
					// Track sub-issue state
					sub_issues.push(TargetSubIssue { number, closed });
					continue;
				}
				TypstMarkerType::Comment { is_immutable, id, .. } => {
					// Flush previous section
					let body = unindent_body(&current_body).trim().to_string();

					if in_issue_body && issue_body.is_empty() {
						if !current_is_immutable {
							issue_body = body;
						}
					} else if let Some(prev_id) = current_comment_id {
						if !current_is_immutable {
							comments.push(TargetComment { id: Some(prev_id), body });
						}
					} else if is_new_comment && !body.is_empty() {
						comments.push(TargetComment { id: None, body });
					}

					current_comment_id = Some(id);
					current_is_immutable = is_immutable;
					is_new_comment = false;
					in_issue_body = false;
					current_body.clear();
					in_labels_line = false;
					continue;
				}
				TypstMarkerType::NewComment => {
					// Flush previous section
					let body = unindent_body(&current_body).trim().to_string();

					if in_issue_body && issue_body.is_empty() {
						if !current_is_immutable {
							issue_body = body;
						}
					} else if let Some(id) = current_comment_id {
						if !current_is_immutable {
							comments.push(TargetComment { id: Some(id), body });
						}
					} else if is_new_comment && !body.is_empty() {
						comments.push(TargetComment { id: None, body });
					}

					current_comment_id = None;
					current_is_immutable = false;
					is_new_comment = true;
					in_issue_body = false;
					current_body.clear();
					in_labels_line = false;
					continue;
				}
			}
		}

		// Skip labels line (indented)
		if stripped_line.starts_with("*Labels:*") {
			in_labels_line = true;
			continue;
		}

		// After labels line, skip one empty line
		if in_labels_line && stripped_line.is_empty() {
			in_labels_line = false;
			continue;
		}

		// Accumulate body content (keep original line with indentation for proper unindent later)
		current_body.push_str(line);
		current_body.push('\n');
	}

	// Flush final section
	let body = unindent_body(&current_body).trim().to_string();
	if !seen_issue_marker {
		// No issue marker at all - treat everything as issue body
		issue_body = body;
	} else if in_issue_body && issue_body.is_empty() {
		// Was collecting issue body
		if !current_is_immutable {
			issue_body = body;
		}
	} else if let Some(id) = current_comment_id {
		// Last section was a tracked comment
		if !current_is_immutable {
			comments.push(TargetComment { id: Some(id), body });
		}
	} else if is_new_comment && !body.is_empty() {
		// Last section was a new comment
		comments.push(TargetComment { id: None, body });
	}

	TargetState {
		issue_body,
		comments,
		sub_issues,
		new_sub_issues,
	}
}

/// Sync changes from a local issue file back to GitHub using stored metadata.
async fn sync_local_issue_to_github(settings: &LiveSettings, meta: &IssueFileMeta, edited_content: &str) -> Result<()> {
	// Step 1: Parse into target state
	let target = match meta.extension.as_str() {
		"md" => parse_markdown_target(edited_content),
		"typ" => parse_typst_target(edited_content),
		_ => return Err(eyre!("Unsupported extension: {}", meta.extension)),
	};

	let mut updates = 0;
	let mut creates = 0;
	let mut deletes = 0;

	// Step 2a: Check issue body
	let original_body = meta.original_issue_body.as_deref().unwrap_or("");
	if target.issue_body != original_body {
		println!("Updating issue body...");
		update_github_issue_body(settings, &meta.owner, &meta.repo, meta.issue_number, &target.issue_body).await?;
		updates += 1;
	}

	// Step 2b: Collect which original comment IDs are present in target
	let target_ids: std::collections::HashSet<u64> = target.comments.iter().filter_map(|c| c.id).collect();
	let original_ids: std::collections::HashSet<u64> = meta.original_comments.iter().map(|c| c.id).collect();

	// Delete comments that were removed (marker line deleted)
	for orig in &meta.original_comments {
		if !target_ids.contains(&orig.id) {
			println!("Deleting comment {}...", orig.id);
			delete_github_comment(settings, &meta.owner, &meta.repo, orig.id).await?;
			deletes += 1;
		}
	}

	// Update existing comments and create new ones
	for tc in &target.comments {
		match tc.id {
			Some(id) if original_ids.contains(&id) => {
				// Existing comment - check if changed
				let original = meta.original_comments.iter().find(|c| c.id == id).and_then(|c| c.body.as_deref()).unwrap_or("");
				if tc.body != original {
					println!("Updating comment {}...", id);
					update_github_comment(settings, &meta.owner, &meta.repo, id, &tc.body).await?;
					updates += 1;
				}
			}
			Some(id) => {
				// ID present but not in original - shouldn't happen, treat as update attempt
				eprintln!("Warning: comment {} not found in original, skipping", id);
			}
			None => {
				// New comment
				if !tc.body.is_empty() {
					println!("Creating new comment...");
					create_github_comment(settings, &meta.owner, &meta.repo, meta.issue_number, &tc.body).await?;
					creates += 1;
				}
			}
		}
	}

	// Step 2c: Check sub-issue state changes
	let mut sub_issue_updates = 0;
	for ts in &target.sub_issues {
		// Find original state
		if let Some(orig) = meta.original_sub_issues.iter().find(|o| o.number == ts.number) {
			let orig_closed = orig.state == "closed";
			if ts.closed != orig_closed {
				let new_state = if ts.closed { "closed" } else { "open" };
				println!("Updating sub-issue #{} to {}...", ts.number, new_state);
				update_github_issue_state(settings, &meta.owner, &meta.repo, ts.number, new_state).await?;
				sub_issue_updates += 1;
			}
		}
	}

	// Step 2d: Create new sub-issues
	let mut new_sub_issues_created = 0;
	for ns in &target.new_sub_issues {
		println!("Creating sub-issue '{}'...", ns.title);

		// Create the issue on GitHub
		let created = create_github_issue(settings, &meta.owner, &meta.repo, &ns.title, "").await?;
		println!("Created sub-issue #{}: {}", created.number, created.html_url);

		// Add as sub-issue to the parent
		add_sub_issue(settings, &meta.owner, &meta.repo, meta.issue_number, created.number).await?;

		// If the new sub-issue should be closed, close it
		if ns.closed {
			update_github_issue_state(settings, &meta.owner, &meta.repo, created.number, "closed").await?;
		}

		new_sub_issues_created += 1;
	}

	let total = updates + creates + deletes + sub_issue_updates + new_sub_issues_created;
	if total == 0 {
		println!("No changes detected.");
	} else {
		let mut parts = Vec::new();
		if updates > 0 {
			parts.push(format!("{} updated", updates));
		}
		if creates > 0 {
			parts.push(format!("{} created", creates));
		}
		if deletes > 0 {
			parts.push(format!("{} deleted", deletes));
		}
		if sub_issue_updates > 0 {
			parts.push(format!("{} sub-issues updated", sub_issue_updates));
		}
		if new_sub_issues_created > 0 {
			parts.push(format!("{} sub-issues created", new_sub_issues_created));
		}
		println!("Synced to GitHub: {}", parts.join(", "));
	}

	Ok(())
}

/// Fetch an issue and all its sub-issues recursively, writing them to XDG_DATA.
/// Returns the path to the main issue file.
async fn fetch_and_store_issue(
	settings: &LiveSettings,
	owner: &str,
	repo: &str,
	issue_number: u64,
	extension: &Extension,
	render_closed: bool,
	parent_issue: Option<u64>,
) -> Result<PathBuf> {
	// Fetch issue data in parallel
	let (current_user, issue, comments, sub_issues) = tokio::try_join!(
		fetch_authenticated_user(settings),
		fetch_github_issue(settings, owner, repo, issue_number),
		fetch_github_comments(settings, owner, repo, issue_number),
		fetch_github_sub_issues(settings, owner, repo, issue_number),
	)?;

	// Determine file path
	let issue_file_path = get_issue_file_path(owner, issue_number, extension, parent_issue);

	// Create parent directories
	if let Some(parent) = issue_file_path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	// Format content
	let content = match extension {
		Extension::Md => format_issue_as_markdown(&issue, &comments, &sub_issues, owner, repo, &current_user, render_closed),
		Extension::Typ => format_issue_as_typst(&issue, &comments, &sub_issues, owner, repo, &current_user, render_closed),
	};

	// Write issue file
	std::fs::write(&issue_file_path, &content)?;

	// Save metadata for syncing
	let meta = IssueFileMeta {
		owner: owner.to_string(),
		repo: repo.to_string(),
		issue_number,
		extension: extension.as_str().to_string(),
		original_issue_body: issue.body.clone(),
		original_comments: comments.iter().map(OriginalComment::from).collect(),
		original_sub_issues: sub_issues.iter().map(OriginalSubIssue::from).collect(),
	};
	save_issue_meta(&issue_file_path, &meta)?;

	// If there are sub-issues, create a directory and recursively fetch them
	if !sub_issues.is_empty() {
		let sub_dir = get_sub_issues_dir(owner, issue_number);
		std::fs::create_dir_all(&sub_dir)?;

		for sub in &sub_issues {
			// Recursively fetch sub-issue (use Box::pin for recursive async)
			Box::pin(fetch_and_store_issue(settings, owner, repo, sub.number, extension, render_closed, Some(issue_number))).await?;
		}
	}

	Ok(issue_file_path)
}

/// Open a local issue file, let user edit, then sync changes back to GitHub.
async fn open_local_issue(settings: &LiveSettings, issue_file_path: &Path) -> Result<()> {
	// Load metadata
	let meta = load_issue_meta(issue_file_path)?;

	// Read current content for comparison
	let original_content = std::fs::read_to_string(issue_file_path)?;

	// Open in editor (blocks until editor closes)
	v_utils::io::open(issue_file_path)?;

	// Read edited content
	let edited_content = std::fs::read_to_string(issue_file_path)?;

	// Check if content changed and sync back to GitHub
	if edited_content != original_content {
		sync_local_issue_to_github(settings, &meta, &edited_content).await?;

		// Re-fetch and update local file and metadata to reflect the synced state
		println!("Refreshing local issue file from GitHub...");
		let extension = match meta.extension.as_str() {
			"typ" => Extension::Typ,
			_ => Extension::Md,
		};

		// Determine if this is a sub-issue by checking the path structure
		let parent_issue = issue_file_path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).and_then(|s| s.parse::<u64>().ok());

		fetch_and_store_issue(settings, &meta.owner, &meta.repo, meta.issue_number, &extension, false, parent_issue).await?;
	} else {
		println!("No changes made.");
	}

	Ok(())
}

/// Parsed touch path components
/// Format: workspace/project/issue[.md|.typ] or workspace/project/parent/child[.md|.typ] (for sub-issues)
#[derive(Debug)]
struct TouchPath {
	owner: String,
	repo: String,
	/// Chain of issue titles (parent issues first, the target issue last)
	/// For a simple issue: ["issue_title"]
	/// For a sub-issue: ["parent_title", "child_title"]
	/// For nested: ["grandparent", "parent", "child"]
	issue_chain: Vec<String>,
	/// The extension from the path (if provided), or None to use default
	extension: Option<Extension>,
}

/// Parse a path for --touch mode
/// Format: workspace/project/issue[.md|.typ] or workspace/project/parent_issue/child_issue[.md|.typ]
/// Extension is optional - if not provided, will use config default
fn parse_touch_path(path: &str) -> Result<TouchPath> {
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
	if extension.is_some() {
		if let Some(last) = issue_chain.last_mut() {
			// Strip the extension suffix (e.g., ".md" or ".typ")
			if let Some(stem) = last.rsplit_once('.') {
				*last = stem.0.to_string();
			}
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
async fn create_issue_on_github(settings: &LiveSettings, touch_path: &TouchPath, extension: &Extension) -> Result<PathBuf> {
	let owner = &touch_path.owner;
	let repo = &touch_path.repo;

	// Step 1: Check collaborator access
	println!("Checking collaborator access to {}/{}...", owner, repo);
	let has_access = check_collaborator_access(settings, owner, repo).await?;
	if !has_access {
		return Err(eyre!("You don't have collaborator (write) access to {}/{}. Cannot create issues.", owner, repo));
	}
	println!("Access confirmed.");

	// Step 2: Validate parent issues exist (all except the last one in the chain)
	let mut parent_issue_numbers: Vec<u64> = Vec::new();

	if touch_path.issue_chain.len() > 1 {
		println!("Validating parent issue chain...");
		for (i, parent_title) in touch_path.issue_chain[..touch_path.issue_chain.len() - 1].iter().enumerate() {
			// Try to find by title first
			let issue_number = find_issue_by_title(settings, owner, repo, parent_title).await?;

			match issue_number {
				Some(num) => {
					println!("  Found parent issue #{}: {}", num, parent_title);
					parent_issue_numbers.push(num);
				}
				None => {
					// If not found by title, try parsing as issue number
					if let Ok(num) = parent_title.parse::<u64>() {
						if issue_exists(settings, owner, repo, num).await? {
							println!("  Found parent issue #{}", num);
							parent_issue_numbers.push(num);
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
	println!("Creating issue '{}'...", new_issue_title);
	let created = create_github_issue(settings, owner, repo, new_issue_title, "").await?;
	println!("Created issue #{}: {}", created.number, created.html_url);

	// Step 5: If there are parent issues, add as sub-issue to the immediate parent
	if let Some(&parent_number) = parent_issue_numbers.last() {
		println!("Adding as sub-issue to #{}...", parent_number);
		add_sub_issue(settings, owner, repo, parent_number, created.number).await?;
		println!("Sub-issue relationship created.");
	}

	// Step 6: Fetch and store the newly created issue locally (like normal flow)
	let parent_issue = parent_issue_numbers.last().copied();
	let issue_file_path = fetch_and_store_issue(settings, owner, repo, created.number, extension, false, parent_issue).await?;

	println!("Stored issue at: {:?}", issue_file_path);

	Ok(issue_file_path)
}

/// Try to find an existing local issue file matching the touch path
/// Returns the path if found, None otherwise
fn find_local_issue_for_touch(touch_path: &TouchPath, extension: &Extension) -> Option<PathBuf> {
	let issues_dir = issues_dir();

	// Build the expected path based on the issue chain
	// For a simple issue: issues/{owner}/{issue_title}.{ext}
	// For sub-issues, we need to find by searching since we don't know the issue numbers

	// First, try to find by searching in the owner directory
	let owner_dir = issues_dir.join(&touch_path.owner);
	if !owner_dir.exists() {
		return None;
	}

	// Search for files matching the issue title (last in chain)
	let issue_title = touch_path.issue_chain.last()?;
	let ext = extension.as_str();

	// Use search_issue_files to find matches
	// We search for the issue title and filter by extension
	if let Ok(matches) = search_issue_files(issue_title) {
		// Filter matches to only those in the correct owner directory and with correct extension
		for path in matches {
			// Check if it's in the right owner directory
			if !path.starts_with(&owner_dir) {
				continue;
			}

			// Check extension matches
			if path.extension().and_then(|e| e.to_str()) != Some(ext) {
				continue;
			}

			// Check the filename matches (without extension)
			if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
				if stem == issue_title {
					return Some(path);
				}
			}
		}
	}

	None
}

/// Get the effective extension from args, config, or default
fn get_effective_extension(args_extension: Option<Extension>, settings: &LiveSettings) -> Extension {
	// Priority: CLI arg > config > default (md)
	if let Some(ext) = args_extension {
		return ext;
	}

	if let Ok(config) = settings.config() {
		if let Some(open_config) = &config.open {
			return match open_config.default_extension.as_str() {
				"typ" => Extension::Typ,
				_ => Extension::Md,
			};
		}
	}

	Extension::Md
}

pub async fn open_command(settings: &LiveSettings, args: OpenArgs) -> Result<()> {
	let input = args.url_or_pattern.trim();
	let extension = get_effective_extension(args.extension, settings);

	// Handle --touch mode
	if args.touch {
		let touch_path = parse_touch_path(input)?;

		// Determine the extension to use
		let effective_ext = touch_path.extension.unwrap_or(extension);

		// First, try to find an existing local issue file
		if let Some(existing_path) = find_local_issue_for_touch(&touch_path, &effective_ext) {
			println!("Found existing issue: {:?}", existing_path);
			open_local_issue(settings, &existing_path).await?;
			return Ok(());
		}

		// Not found locally - create on GitHub
		let issue_file_path = create_issue_on_github(settings, &touch_path, &effective_ext).await?;

		// Open the local issue file for editing
		open_local_issue(settings, &issue_file_path).await?;
		return Ok(());
	}

	// Check if input is a GitHub issue URL specifically (not just any GitHub URL)
	if is_github_issue_url(input) {
		// GitHub URL mode: fetch issue and store in XDG_DATA
		let (owner, repo, issue_number) = parse_github_issue_url(input)?;

		println!("Fetching issue #{} from {}/{}...", issue_number, owner, repo);

		// Fetch and store issue (and sub-issues) in XDG_DATA
		let issue_file_path = fetch_and_store_issue(settings, &owner, &repo, issue_number, &extension, args.render_closed, None).await?;

		println!("Stored issue at: {:?}", issue_file_path);

		// Open the local issue file for editing
		open_local_issue(settings, &issue_file_path).await?;
	} else {
		// Local search mode: find and open existing issue file
		let matches = search_issue_files(input)?;

		let issue_file_path = match matches.len() {
			0 => {
				return Err(eyre!("No issue files found matching pattern: {}", input));
			}
			1 => {
				println!("Found: {:?}", matches[0]);
				matches[0].clone()
			}
			_ => {
				println!("Found {} matches. Opening fzf to choose:", matches.len());
				match choose_issue_with_fzf(&matches, input)? {
					Some(path) => path,
					None => return Err(eyre!("No issue selected")),
				}
			}
		};

		// Open the local issue file for editing
		open_local_issue(settings, &issue_file_path).await?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use insta::assert_snapshot;

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

	fn make_user(login: &str) -> GitHubUser {
		GitHubUser { login: login.to_string() }
	}

	fn make_issue(number: u64, title: &str, body: Option<&str>, labels: Vec<&str>, user: &str, state: &str) -> GitHubIssue {
		GitHubIssue {
			number,
			title: title.to_string(),
			body: body.map(|s| s.to_string()),
			labels: labels.into_iter().map(|name| GitHubLabel { name: name.to_string() }).collect(),
			user: make_user(user),
			state: state.to_string(),
		}
	}

	#[test]
	fn test_format_issue_as_markdown_owned() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec!["bug", "help wanted"], "me", "open");

		let md = format_issue_as_markdown(&issue, &[], &[], "owner", "repo", "me", false);
		assert_snapshot!(md, @r"
		- [ ] Test Issue <!--https://github.com/owner/repo/issues/123-->
			**Labels:** bug, help wanted

			Issue body text
		");
	}

	#[test]
	fn test_format_issue_as_markdown_not_owned() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec![], "other", "open");

		let md = format_issue_as_markdown(&issue, &[], &[], "owner", "repo", "me", false);
		assert_snapshot!(md, @r"
		- [ ] Test Issue <!--immutable https://github.com/owner/repo/issues/123-->

				Issue body text
		");
	}

	#[test]
	fn test_format_issue_as_markdown_closed_omitted() {
		let issue = make_issue(123, "Closed Issue", Some("Issue body text"), vec![], "me", "closed");

		// Default: closed issues have omitted contents
		let md = format_issue_as_markdown(&issue, &[], &[], "owner", "repo", "me", false);
		assert_snapshot!(md, @r"
		- [x] Closed Issue <!--https://github.com/owner/repo/issues/123-->
			<!-- omitted -->
		");
	}

	#[test]
	fn test_format_issue_as_markdown_closed_rendered() {
		let issue = make_issue(123, "Closed Issue", Some("Issue body text"), vec![], "me", "closed");

		// With render_closed: full contents shown
		let md = format_issue_as_markdown(&issue, &[], &[], "owner", "repo", "me", true);
		assert_snapshot!(md, @r"
		- [x] Closed Issue <!--https://github.com/owner/repo/issues/123-->

			Issue body text
		");
	}

	#[test]
	fn test_format_issue_as_markdown_with_sub_issues() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec![], "me", "open");
		let sub_issues = vec![
			GitHubSubIssue {
				number: 124,
				title: "Open sub-issue".to_string(),
				state: "open".to_string(),
			},
			GitHubSubIssue {
				number: 125,
				title: "Closed sub-issue".to_string(),
				state: "closed".to_string(),
			},
		];

		let md = format_issue_as_markdown(&issue, &[], &sub_issues, "owner", "repo", "me", false);
		assert_snapshot!(md, @r"
		- [ ] Test Issue <!--https://github.com/owner/repo/issues/123-->

			Issue body text

			- [ ] Open sub-issue <!--sub https://github.com/owner/repo/issues/124-->
			- [x] Closed sub-issue <!--sub https://github.com/owner/repo/issues/125-->
		");
	}

	#[test]
	fn test_format_issue_as_markdown_mixed_ownership() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec![], "other", "open");
		let comments = vec![
			GitHubComment {
				id: 1001,
				body: Some("First comment".to_string()),
				user: make_user("me"),
			},
			GitHubComment {
				id: 1002,
				body: Some("Second comment".to_string()),
				user: make_user("other"),
			},
		];

		let md = format_issue_as_markdown(&issue, &comments, &[], "owner", "repo", "me", false);
		assert_snapshot!(md, @r"
		- [ ] Test Issue <!--immutable https://github.com/owner/repo/issues/123-->

				Issue body text

			<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
			First comment

			<!--immutable https://github.com/owner/repo/issues/123#issuecomment-1002-->
				Second comment
		");
	}

	#[test]
	fn test_format_issue_as_typst_owned() {
		let issue = make_issue(123, "Test Issue", Some("## Subheading\nBody text"), vec!["enhancement"], "me", "open");

		let typ = format_issue_as_typst(&issue, &[], &[], "owner", "repo", "me", false);
		assert_snapshot!(typ, @r"
		- [ ] Test Issue // https://github.com/owner/repo/issues/123
			*Labels:* enhancement

			== Subheading
			Body text
		");
	}

	#[test]
	fn test_format_issue_as_typst_not_owned() {
		let issue = make_issue(456, "Typst Issue", Some("Body"), vec![], "other", "open");
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
			user: make_user("other"),
		}];

		let typ = format_issue_as_typst(&issue, &comments, &[], "testowner", "testrepo", "me", false);
		assert_snapshot!(typ, @r"
		- [ ] Typst Issue // immutable https://github.com/testowner/testrepo/issues/456

				Body

			// immutable https://github.com/testowner/testrepo/issues/456#issuecomment-2001
				A comment
		");
	}

	#[test]
	fn test_format_issue_as_typst_closed_omitted() {
		let issue = make_issue(123, "Closed Issue", Some("Body text"), vec![], "me", "closed");

		let typ = format_issue_as_typst(&issue, &[], &[], "owner", "repo", "me", false);
		assert_snapshot!(typ, @r"
		- [x] Closed Issue // https://github.com/owner/repo/issues/123
			// omitted
		");
	}

	#[test]
	fn test_parse_markdown_roundtrip() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec![], "me", "open");
		let comments = vec![
			GitHubComment {
				id: 1001,
				body: Some("First comment".to_string()),
				user: make_user("me"),
			},
			GitHubComment {
				id: 1002,
				body: Some("Second comment".to_string()),
				user: make_user("me"),
			},
		];

		let md = format_issue_as_markdown(&issue, &comments, &[], "owner", "repo", "me", false);
		let target = parse_markdown_target(&md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "Issue body text",
		    comments: [
		        TargetComment {
		            id: Some(
		                1001,
		            ),
		            body: "First comment",
		        },
		        TargetComment {
		            id: Some(
		                1002,
		            ),
		            body: "Second comment",
		        },
		    ],
		    sub_issues: [],
		    new_sub_issues: [],
		}
		"#);
	}

	#[test]
	fn test_parse_typst_roundtrip() {
		let issue = make_issue(456, "Typst Issue", Some("Body text"), vec![], "me", "open");
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
			user: make_user("me"),
		}];

		let typ = format_issue_as_typst(&issue, &comments, &[], "testowner", "testrepo", "me", false);
		let target = parse_typst_target(&typ);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "Body text",
		    comments: [
		        TargetComment {
		            id: Some(
		                2001,
		            ),
		            body: "A comment",
		        },
		    ],
		    sub_issues: [],
		    new_sub_issues: [],
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_deleted_comment() {
		// When comment marker is deleted, content merges into previous section
		let md = "- [ ] Test Issue <!--https://github.com/owner/repo/issues/123-->

\tIssue body text

\t<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
\tFirst comment
\tThis used to be comment 1002 but marker was deleted
";
		let target = parse_markdown_target(md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "Issue body text",
		    comments: [
		        TargetComment {
		            id: Some(
		                1001,
		            ),
		            body: "First comment\nThis used to be comment 1002 but marker was deleted",
		        },
		    ],
		    sub_issues: [],
		    new_sub_issues: [],
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_new_comment() {
		// New comments are marked with <!--new comment-->
		let md = "- [ ] Test Issue <!--https://github.com/owner/repo/issues/123-->

\tIssue body text

\t<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
\tFirst comment

\t<!--new comment-->
\tThis is a new comment I'm adding
";
		let target = parse_markdown_target(md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "Issue body text",
		    comments: [
		        TargetComment {
		            id: Some(
		                1001,
		            ),
		            body: "First comment",
		        },
		        TargetComment {
		            id: None,
		            body: "This is a new comment I'm adding",
		        },
		    ],
		    sub_issues: [],
		    new_sub_issues: [],
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_immutable_ignored() {
		// Immutable sections should not appear in parsed target
		// When the issue is immutable but a comment is editable, we capture the comment
		let md = "- [ ] Test Issue <!--immutable https://github.com/owner/repo/issues/123-->

\t\tImmutable issue body (indented)

\t<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
\tMy editable comment

\t<!--immutable https://github.com/owner/repo/issues/123#issuecomment-1002-->
\t\tSomeone else's comment (indented)
";
		let target = parse_markdown_target(md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "",
		    comments: [
		        TargetComment {
		            id: Some(
		                1001,
		            ),
		            body: "My editable comment",
		        },
		    ],
		    sub_issues: [],
		    new_sub_issues: [],
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_with_sub_issues_state_capture() {
		// Sub-issue state is captured for syncing (checking/unchecking closes/reopens them)
		let md = "- [ ] Test Issue <!--https://github.com/owner/repo/issues/123-->

\tIssue body text

\t- [ ] Sub-issue 1 <!--sub https://github.com/owner/repo/issues/124-->
\t- [x] Sub-issue 2 <!--sub https://github.com/owner/repo/issues/125-->

\t<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
\tA comment
";
		let target = parse_markdown_target(md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "Issue body text",
		    comments: [
		        TargetComment {
		            id: Some(
		                1001,
		            ),
		            body: "A comment",
		        },
		    ],
		    sub_issues: [
		        TargetSubIssue {
		            number: 124,
		            closed: false,
		        },
		        TargetSubIssue {
		            number: 125,
		            closed: true,
		        },
		    ],
		    new_sub_issues: [],
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_new_sub_issues() {
		// New sub-issues are checkbox lines without URL markers
		let md = "- [ ] Test Issue <!--https://github.com/owner/repo/issues/123-->

\tIssue body text

\t- [ ] Existing sub-issue <!--sub https://github.com/owner/repo/issues/124-->
\t- [ ] New sub-issue to create
\t- [x] Another new sub-issue (already done)

\t<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
\tA comment
";
		let target = parse_markdown_target(md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "Issue body text",
		    comments: [
		        TargetComment {
		            id: Some(
		                1001,
		            ),
		            body: "A comment",
		        },
		    ],
		    sub_issues: [
		        TargetSubIssue {
		            number: 124,
		            closed: false,
		        },
		    ],
		    new_sub_issues: [
		        NewSubIssue {
		            title: "New sub-issue to create",
		            closed: false,
		        },
		        NewSubIssue {
		            title: "Another new sub-issue (already done)",
		            closed: true,
		        },
		    ],
		}
		"#);
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
