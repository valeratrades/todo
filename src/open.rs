use clap::{Args, ValueEnum};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use v_utils::prelude::*;

use crate::config::LiveSettings;

static ISSUE_EDIT_LOCK_FILENAME: &str = "issue_edit.lock";

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
	/// GitHub issue URL (e.g., https://github.com/owner/repo/issues/123)
	pub url: String,

	/// File extension for the output file
	#[arg(short = 'e', long, default_value = "md")]
	pub extension: Extension,

	/// Render full contents even for closed issues (by default, closed issues show only title with <!-- omitted -->)
	#[arg(long)]
	pub render_closed: bool,
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

/// Lock file data stored during edit session
#[derive(Debug, Deserialize, Serialize)]
struct EditLock {
	owner: String,
	repo: String,
	issue_number: u64,
	extension: String,
	tmp_path: String,
	/// Original issue body (for diffing)
	original_issue_body: Option<String>,
	/// Original comments with their IDs
	original_comments: Vec<OriginalComment>,
	/// Original sub-issues with their state
	original_sub_issues: Vec<OriginalSubIssue>,
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
}

#[derive(Debug, PartialEq)]
struct TargetSubIssue {
	number: u64,
	closed: bool,
}

#[derive(Debug, PartialEq)]
struct TargetComment {
	id: Option<u64>,
	body: String,
}

/// Parse a GitHub issue URL and extract owner, repo, and issue number
/// Supports formats like:
/// - https://github.com/owner/repo/issues/123
/// - github.com/owner/repo/issues/123
fn parse_github_issue_url(url: &str) -> Result<(String, String, u64)> {
	let url = url.trim();

	// Remove protocol prefix if present
	let path = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);

	// Remove github.com prefix
	let path = path.strip_prefix("github.com/").ok_or_else(|| eyre!("URL must be a GitHub issue URL: {}", url))?;

	// Split by /
	let parts: Vec<&str> = path.split('/').collect();

	// Expected format: owner/repo/issues/number
	if parts.len() < 4 || parts[2] != "issues" {
		return Err(eyre!("Invalid GitHub issue URL format. Expected: https://github.com/owner/repo/issues/123"));
	}

	let owner = parts[0].to_string();
	let repo = parts[1].to_string();
	let issue_number: u64 = parts[3].parse().map_err(|_| eyre!("Invalid issue number: {}", parts[3]))?;

	Ok((owner, repo, issue_number))
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
fn parse_markdown_target(content: &str) -> TargetState {
	let mut issue_body = String::new();
	let mut comments: Vec<TargetComment> = Vec::new();
	let mut sub_issues: Vec<TargetSubIssue> = Vec::new();
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

	TargetState { issue_body, comments, sub_issues }
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
fn parse_typst_target(content: &str) -> TargetState {
	let mut issue_body = String::new();
	let mut comments: Vec<TargetComment> = Vec::new();
	let mut sub_issues: Vec<TargetSubIssue> = Vec::new();
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

	TargetState { issue_body, comments, sub_issues }
}

/// Sync changes from edited file back to GitHub.
/// Two-step process:
/// 1. Parse edited content into target state
/// 2. Diff against original and apply create/update/delete operations
async fn sync_changes_to_github(settings: &LiveSettings, lock: &EditLock, edited_content: &str) -> Result<()> {
	// Step 1: Parse into target state
	let target = match lock.extension.as_str() {
		"md" => parse_markdown_target(edited_content),
		"typ" => parse_typst_target(edited_content),
		_ => return Err(eyre!("Unsupported extension: {}", lock.extension)),
	};

	let mut updates = 0;
	let mut creates = 0;
	let mut deletes = 0;

	// Step 2a: Check issue body
	let original_body = lock.original_issue_body.as_deref().unwrap_or("");
	if target.issue_body != original_body {
		println!("Updating issue body...");
		update_github_issue_body(settings, &lock.owner, &lock.repo, lock.issue_number, &target.issue_body).await?;
		updates += 1;
	}

	// Step 2b: Collect which original comment IDs are present in target
	let target_ids: std::collections::HashSet<u64> = target.comments.iter().filter_map(|c| c.id).collect();
	let original_ids: std::collections::HashSet<u64> = lock.original_comments.iter().map(|c| c.id).collect();

	// Delete comments that were removed (marker line deleted)
	for orig in &lock.original_comments {
		if !target_ids.contains(&orig.id) {
			println!("Deleting comment {}...", orig.id);
			delete_github_comment(settings, &lock.owner, &lock.repo, orig.id).await?;
			deletes += 1;
		}
	}

	// Update existing comments and create new ones
	for tc in &target.comments {
		match tc.id {
			Some(id) if original_ids.contains(&id) => {
				// Existing comment - check if changed
				let original = lock.original_comments.iter().find(|c| c.id == id).and_then(|c| c.body.as_deref()).unwrap_or("");
				if tc.body != original {
					println!("Updating comment {}...", id);
					update_github_comment(settings, &lock.owner, &lock.repo, id, &tc.body).await?;
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
					create_github_comment(settings, &lock.owner, &lock.repo, lock.issue_number, &tc.body).await?;
					creates += 1;
				}
			}
		}
	}

	// Step 2c: Check sub-issue state changes
	let mut sub_issue_updates = 0;
	for ts in &target.sub_issues {
		// Find original state
		if let Some(orig) = lock.original_sub_issues.iter().find(|o| o.number == ts.number) {
			let orig_closed = orig.state == "closed";
			if ts.closed != orig_closed {
				let new_state = if ts.closed { "closed" } else { "open" };
				println!("Updating sub-issue #{} to {}...", ts.number, new_state);
				update_github_issue_state(settings, &lock.owner, &lock.repo, ts.number, new_state).await?;
				sub_issue_updates += 1;
			}
		}
	}

	let total = updates + creates + deletes + sub_issue_updates;
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
		println!("Synced to GitHub: {}", parts.join(", "));
	}

	Ok(())
}

pub async fn open_command(settings: &LiveSettings, args: OpenArgs) -> Result<()> {
	use std::path::Path;

	let lock_path = v_utils::xdg_state_file!(ISSUE_EDIT_LOCK_FILENAME);

	let (owner, repo, issue_number) = parse_github_issue_url(&args.url)?;

	println!("Fetching issue #{} from {}/{}...", issue_number, owner, repo);

	// Fetch in parallel
	let (current_user, issue, comments, sub_issues) = tokio::try_join!(
		fetch_authenticated_user(settings),
		fetch_github_issue(settings, &owner, &repo, issue_number),
		fetch_github_comments(settings, &owner, &repo, issue_number),
		fetch_github_sub_issues(settings, &owner, &repo, issue_number),
	)?;

	// Format content based on extension
	let content = match args.extension {
		Extension::Md => format_issue_as_markdown(&issue, &comments, &sub_issues, &owner, &repo, &current_user, args.render_closed),
		Extension::Typ => format_issue_as_typst(&issue, &comments, &sub_issues, &owner, &repo, &current_user, args.render_closed),
	};

	// Create temp file
	let tmp_path = format!("/tmp/issue_{}_{}_{}_{}.{}", owner, repo, issue_number, issue.number, args.extension.as_str());
	std::fs::write(&tmp_path, &content)?;

	// Create lock file with original content for diffing
	let lock = EditLock {
		owner: owner.clone(),
		repo: repo.clone(),
		issue_number,
		extension: args.extension.as_str().to_string(),
		tmp_path: tmp_path.clone(),
		original_issue_body: issue.body.clone(),
		original_comments: comments.iter().map(OriginalComment::from).collect(),
		original_sub_issues: sub_issues.iter().map(OriginalSubIssue::from).collect(),
	};
	std::fs::write(&lock_path, serde_json::to_string_pretty(&lock)?)?;

	// Open in editor (blocks until editor closes)
	v_utils::io::open(Path::new(&tmp_path))?;

	// Read edited content
	let edited_content = std::fs::read_to_string(&tmp_path)?;

	// Check if content changed and sync back to GitHub
	if edited_content != content {
		sync_changes_to_github(settings, &lock, &edited_content).await?;
	} else {
		println!("No changes made.");
	}

	// Clean up
	let _ = std::fs::remove_file(&tmp_path);
	let _ = std::fs::remove_file(&lock_path);

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
		}
		"#);
	}
}
