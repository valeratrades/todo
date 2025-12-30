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
}

#[derive(Debug, Deserialize)]
struct GitHubIssue {
	number: u64,
	title: String,
	body: Option<String>,
	labels: Vec<GitHubLabel>,
	user: GitHubUser,
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

/// Target state after user edits - clean representation of what the issue should look like
#[derive(Debug, PartialEq)]
struct TargetState {
	issue_body: String,
	/// Comments in order. None id = new comment to create
	comments: Vec<TargetComment>,
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

/// Indent every line of text with a tab
fn indent_body(body: &str) -> String {
	body.lines().map(|line| format!("\t{line}")).collect::<Vec<_>>().join("\n")
}

fn format_issue_as_markdown(issue: &GitHubIssue, comments: &[GitHubComment], owner: &str, repo: &str, current_user: &str) -> String {
	let mut content = String::new();

	// Title as H1
	content.push_str(&format!("# {}\n", issue.title));
	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	let issue_owned = issue.user.login == current_user;

	if issue_owned {
		content.push_str(&format!("<!--{}-->\n\n", issue_url));
	} else {
		content.push_str(&format!("<!--immutable {}-->\n\n", issue_url));
	}

	// Labels if any
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("**Labels:** {}\n\n", labels.join(", ")));
	}

	// Body
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			if issue_owned {
				content.push_str(body);
			} else {
				content.push_str(&indent_body(body));
			}
			if !body.ends_with('\n') {
				content.push('\n');
			}
		}
	}

	// Comments
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		let comment_owned = comment.user.login == current_user;

		if comment_owned {
			content.push_str(&format!("\n<!--{}-->\n", comment_url));
		} else {
			content.push_str(&format!("\n<!--immutable {}-->\n", comment_url));
		}

		if let Some(body) = &comment.body {
			if !body.is_empty() {
				if comment_owned {
					content.push_str(body);
				} else {
					content.push_str(&indent_body(body));
				}
				if !body.ends_with('\n') {
					content.push('\n');
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

fn format_issue_as_typst(issue: &GitHubIssue, comments: &[GitHubComment], owner: &str, repo: &str, current_user: &str) -> String {
	let mut content = String::new();

	// Title as Typst heading
	content.push_str(&format!("= {}\n", issue.title));
	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	let issue_owned = issue.user.login == current_user;

	if issue_owned {
		content.push_str(&format!("// {}\n\n", issue_url));
	} else {
		content.push_str(&format!("// immutable {}\n\n", issue_url));
	}

	// Labels if any
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("*Labels:* {}\n\n", labels.join(", ")));
	}

	// Body - convert markdown to typst basics
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			let converted = convert_markdown_to_typst(body);
			if issue_owned {
				content.push_str(&converted);
			} else {
				content.push_str(&indent_body(&converted));
			}
			if !converted.ends_with('\n') {
				content.push('\n');
			}
		}
	}

	// Comments
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		let comment_owned = comment.user.login == current_user;

		if comment_owned {
			content.push_str(&format!("\n// {}\n", comment_url));
		} else {
			content.push_str(&format!("\n// immutable {}\n", comment_url));
		}

		if let Some(body) = &comment.body {
			if !body.is_empty() {
				let converted = convert_markdown_to_typst(body);
				if comment_owned {
					content.push_str(&converted);
				} else {
					content.push_str(&indent_body(&converted));
				}
				if !converted.ends_with('\n') {
					content.push('\n');
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

/// Parse a markdown HTML comment marker line.
/// Returns (is_immutable, url) if it's a valid marker, None otherwise.
fn parse_md_marker(line: &str) -> Option<(bool, &str)> {
	if !line.starts_with("<!--") || !line.ends_with("-->") {
		return None;
	}
	let inner = line.trim_start_matches("<!--").trim_end_matches("-->").trim();
	if inner == "new comment" {
		return None; // Special marker handled separately
	}
	if let Some(url) = inner.strip_prefix("immutable ") {
		Some((true, url.trim()))
	} else if inner.starts_with("https://github.com/") {
		Some((false, inner))
	} else {
		None
	}
}

/// Parse markdown content into target state.
/// Content is split by markers. Immutable sections are ignored for sync.
/// New comments are marked with `<!--new comment-->`.
fn parse_markdown_target(content: &str) -> TargetState {
	let mut issue_body = String::new();
	let mut comments: Vec<TargetComment> = Vec::new();
	let mut current_body = String::new();
	let mut current_comment_id: Option<u64> = None;
	let mut current_is_immutable = false;
	let mut is_new_comment = false;
	let mut seen_issue_marker = false;
	let mut in_labels_line = false;

	for line in content.lines() {
		// Check for new comment marker
		if line == "<!--new comment-->" {
			// Flush previous section
			let body = if current_is_immutable { unindent_body(&current_body) } else { current_body.clone() };
			let body = body.trim().to_string();
			if seen_issue_marker {
				if comments.is_empty() && issue_body.is_empty() {
					issue_body = body;
				} else if let Some(id) = current_comment_id {
					if !current_is_immutable {
						comments.push(TargetComment { id: Some(id), body });
					}
				} else if is_new_comment && !body.is_empty() {
					comments.push(TargetComment { id: None, body });
				}
			}

			current_comment_id = None;
			current_is_immutable = false;
			is_new_comment = true;
			current_body.clear();
			in_labels_line = false;
			continue;
		}

		// Check for marker (issue or comment URL)
		if let Some((is_immutable, url)) = parse_md_marker(line) {
			// Flush previous section
			let body = if current_is_immutable { unindent_body(&current_body) } else { current_body.clone() };
			let body = body.trim().to_string();

			if seen_issue_marker {
				if comments.is_empty() && issue_body.is_empty() {
					// Previous was issue body
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
			}

			// Determine if this is issue or comment by URL
			if url.contains("#issuecomment-") {
				// It's a comment
				current_comment_id = url.split("#issuecomment-").last().and_then(|s| s.parse::<u64>().ok());
				is_new_comment = false;
			} else {
				// It's the issue marker
				seen_issue_marker = true;
				current_comment_id = None;
				is_new_comment = false;
			}
			current_is_immutable = is_immutable;
			current_body.clear();
			in_labels_line = false;
			continue;
		}

		// Skip title line (# Title) - only before issue marker
		if line.starts_with("# ") && !seen_issue_marker {
			continue;
		}

		// Skip labels line
		if line.starts_with("**Labels:**") {
			in_labels_line = true;
			continue;
		}

		// After labels line, skip one empty line
		if in_labels_line && line.is_empty() {
			in_labels_line = false;
			continue;
		}

		current_body.push_str(line);
		current_body.push('\n');
	}

	// Flush final section
	let body = if current_is_immutable { unindent_body(&current_body) } else { current_body.clone() };
	let body = body.trim().to_string();
	if !seen_issue_marker {
		// No issue marker at all - treat everything as issue body
		issue_body = body;
	} else if comments.is_empty() && issue_body.is_empty() {
		// Only issue body, no comments
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

	TargetState { issue_body, comments }
}

/// Parse a typst comment marker line.
/// Returns (is_immutable, url) if it's a valid marker, None otherwise.
fn parse_typst_marker(line: &str) -> Option<(bool, &str)> {
	if !line.starts_with("// ") {
		return None;
	}
	let inner = line.trim_start_matches("// ").trim();
	if inner == "new comment" {
		return None; // Special marker handled separately
	}
	if let Some(url) = inner.strip_prefix("immutable ") {
		Some((true, url.trim()))
	} else if inner.starts_with("https://github.com/") {
		Some((false, inner))
	} else {
		None
	}
}

/// Parse typst content into target state.
/// New comments are marked with `// new comment`.
fn parse_typst_target(content: &str) -> TargetState {
	let mut issue_body = String::new();
	let mut comments: Vec<TargetComment> = Vec::new();
	let mut current_body = String::new();
	let mut current_comment_id: Option<u64> = None;
	let mut current_is_immutable = false;
	let mut is_new_comment = false;
	let mut seen_issue_marker = false;
	let mut in_labels_line = false;

	for line in content.lines() {
		// Check for new comment marker
		if line == "// new comment" {
			// Flush previous section
			let body = if current_is_immutable { unindent_body(&current_body) } else { current_body.clone() };
			let body = body.trim().to_string();
			if seen_issue_marker {
				if comments.is_empty() && issue_body.is_empty() {
					issue_body = body;
				} else if let Some(id) = current_comment_id {
					if !current_is_immutable {
						comments.push(TargetComment { id: Some(id), body });
					}
				} else if is_new_comment && !body.is_empty() {
					comments.push(TargetComment { id: None, body });
				}
			}

			current_comment_id = None;
			current_is_immutable = false;
			is_new_comment = true;
			current_body.clear();
			in_labels_line = false;
			continue;
		}

		// Check for marker (issue or comment URL)
		if let Some((is_immutable, url)) = parse_typst_marker(line) {
			// Flush previous section
			let body = if current_is_immutable { unindent_body(&current_body) } else { current_body.clone() };
			let body = body.trim().to_string();

			if seen_issue_marker {
				if comments.is_empty() && issue_body.is_empty() {
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
			}

			// Determine if this is issue or comment by URL
			if url.contains("#issuecomment-") {
				current_comment_id = url.split("#issuecomment-").last().and_then(|s| s.parse::<u64>().ok());
				is_new_comment = false;
			} else {
				seen_issue_marker = true;
				current_comment_id = None;
				is_new_comment = false;
			}
			current_is_immutable = is_immutable;
			current_body.clear();
			in_labels_line = false;
			continue;
		}

		// Skip title line (= Title) - only before issue marker
		if line.starts_with("= ") && !seen_issue_marker {
			continue;
		}

		// Skip labels line
		if line.starts_with("*Labels:*") {
			in_labels_line = true;
			continue;
		}

		// After labels line, skip one empty line
		if in_labels_line && line.is_empty() {
			in_labels_line = false;
			continue;
		}

		current_body.push_str(line);
		current_body.push('\n');
	}

	// Flush final section
	let body = if current_is_immutable { unindent_body(&current_body) } else { current_body.clone() };
	let body = body.trim().to_string();
	if !seen_issue_marker {
		issue_body = body;
	} else if comments.is_empty() && issue_body.is_empty() {
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

	TargetState { issue_body, comments }
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

	let total = updates + creates + deletes;
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
	let (current_user, issue, comments) = tokio::try_join!(
		fetch_authenticated_user(settings),
		fetch_github_issue(settings, &owner, &repo, issue_number),
		fetch_github_comments(settings, &owner, &repo, issue_number),
	)?;

	// Format content based on extension
	let content = match args.extension {
		Extension::Md => format_issue_as_markdown(&issue, &comments, &owner, &repo, &current_user),
		Extension::Typ => format_issue_as_typst(&issue, &comments, &owner, &repo, &current_user),
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

	#[test]
	fn test_format_issue_as_markdown_owned() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![GitHubLabel { name: "bug".to_string() }, GitHubLabel { name: "help wanted".to_string() }],
			user: make_user("me"),
		};

		let md = format_issue_as_markdown(&issue, &[], "owner", "repo", "me");
		assert_snapshot!(md, @"
		# Test Issue
		<!--https://github.com/owner/repo/issues/123-->

		**Labels:** bug, help wanted

		Issue body text
		");
	}

	#[test]
	fn test_format_issue_as_markdown_not_owned() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![],
			user: make_user("other"),
		};

		let md = format_issue_as_markdown(&issue, &[], "owner", "repo", "me");
		assert_snapshot!(md, @"
		# Test Issue
		<!--immutable https://github.com/owner/repo/issues/123-->

			Issue body text
		");
	}

	#[test]
	fn test_format_issue_as_markdown_mixed_ownership() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![],
			user: make_user("other"),
		};
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

		let md = format_issue_as_markdown(&issue, &comments, "owner", "repo", "me");
		assert_snapshot!(md, @"
		# Test Issue
		<!--immutable https://github.com/owner/repo/issues/123-->

			Issue body text

		<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
		First comment

		<!--immutable https://github.com/owner/repo/issues/123#issuecomment-1002-->
			Second comment
		");
	}

	#[test]
	fn test_format_issue_as_typst_owned() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("## Subheading\nBody text".to_string()),
			labels: vec![GitHubLabel { name: "enhancement".to_string() }],
			user: make_user("me"),
		};

		let typ = format_issue_as_typst(&issue, &[], "owner", "repo", "me");
		assert_snapshot!(typ, @"
		= Test Issue
		// https://github.com/owner/repo/issues/123

		*Labels:* enhancement

		== Subheading
		Body text
		");
	}

	#[test]
	fn test_format_issue_as_typst_not_owned() {
		let issue = GitHubIssue {
			number: 456,
			title: "Typst Issue".to_string(),
			body: Some("Body".to_string()),
			labels: vec![],
			user: make_user("other"),
		};
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
			user: make_user("other"),
		}];

		let typ = format_issue_as_typst(&issue, &comments, "testowner", "testrepo", "me");
		assert_snapshot!(typ, @"
		= Typst Issue
		// immutable https://github.com/testowner/testrepo/issues/456

			Body

		// immutable https://github.com/testowner/testrepo/issues/456#issuecomment-2001
			A comment
		");
	}

	#[test]
	fn test_parse_markdown_roundtrip() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![],
			user: make_user("me"),
		};
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

		let md = format_issue_as_markdown(&issue, &comments, "owner", "repo", "me");
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
		}
		"#);
	}

	#[test]
	fn test_parse_typst_roundtrip() {
		let issue = GitHubIssue {
			number: 456,
			title: "Typst Issue".to_string(),
			body: Some("Body text".to_string()),
			labels: vec![],
			user: make_user("me"),
		};
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
			user: make_user("me"),
		}];

		let typ = format_issue_as_typst(&issue, &comments, "testowner", "testrepo", "me");
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
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_deleted_comment() {
		// When comment marker is deleted, content merges into previous section
		let md = r#"# Test Issue
<!--https://github.com/owner/repo/issues/123-->

Issue body text

<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
First comment
This used to be comment 1002 but marker was deleted
"#;
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
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_new_comment() {
		// New comments are marked with <!--new comment-->
		let md = r#"# Test Issue
<!--https://github.com/owner/repo/issues/123-->

Issue body text

<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
First comment

<!--new comment-->
This is a new comment I'm adding
"#;
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
		}
		"#);
	}

	#[test]
	fn test_parse_markdown_immutable_ignored() {
		// Immutable sections should not appear in parsed target
		let md = r#"# Test Issue
<!--immutable https://github.com/owner/repo/issues/123-->

	Immutable issue body (indented)

<!--https://github.com/owner/repo/issues/123#issuecomment-1001-->
My editable comment

<!--immutable https://github.com/owner/repo/issues/123#issuecomment-1002-->
	Someone else's comment (indented)
"#;
		let target = parse_markdown_target(md);
		assert_snapshot!(format!("{target:#?}"), @r#"
		TargetState {
		    issue_body: "My editable comment",
		    comments: [],
		}
		"#);
	}
}
