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
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
	name: String,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubComment {
	id: u64,
	body: Option<String>,
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

/// Parsed section from edited file
#[derive(Debug)]
enum ParsedSection {
	IssueBody { body: String },
	Comment { id: u64, body: String },
	NewComment { body: String },
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

fn format_issue_as_markdown(issue: &GitHubIssue, comments: &[GitHubComment], owner: &str, repo: &str) -> String {
	let mut content = String::new();

	// Title as H1
	content.push_str(&format!("# {}\n", issue.title));
	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	content.push_str(&format!("<!--issue: {}-->\n\n", issue_url));

	// Labels if any
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("**Labels:** {}\n\n", labels.join(", ")));
	}

	// Body
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			content.push_str(body);
			if !body.ends_with('\n') {
				content.push('\n');
			}
		}
	}

	// Comments
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		content.push_str(&format!("\n<!--comment: {}-->\n", comment_url));
		if let Some(body) = &comment.body {
			if !body.is_empty() {
				content.push_str(body);
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

fn format_issue_as_typst(issue: &GitHubIssue, comments: &[GitHubComment], owner: &str, repo: &str) -> String {
	let mut content = String::new();

	// Title as Typst heading
	content.push_str(&format!("= {}\n", issue.title));
	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	content.push_str(&format!("// issue: {}\n\n", issue_url));

	// Labels if any
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("*Labels:* {}\n\n", labels.join(", ")));
	}

	// Body - convert markdown to typst basics
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			let converted = convert_markdown_to_typst(body);
			content.push_str(&converted);
			if !converted.ends_with('\n') {
				content.push('\n');
			}
		}
	}

	// Comments
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		// Typst doesn't have HTML comments, use a typst comment instead
		content.push_str(&format!("\n// comment: {}\n", comment_url));
		if let Some(body) = &comment.body {
			if !body.is_empty() {
				let converted = convert_markdown_to_typst(body);
				content.push_str(&converted);
				if !converted.ends_with('\n') {
					content.push('\n');
				}
			}
		}
	}

	content
}

/// Parse markdown content into sections (issue body + comments)
/// Returns a list of sections with their identifiers
fn parse_markdown_sections(content: &str) -> Vec<ParsedSection> {
	let mut sections = Vec::new();
	let mut current_body = String::new();
	let mut current_comment_id: Option<u64> = None;
	let mut seen_issue_marker = false;
	let mut in_labels_line = false;

	for line in content.lines() {
		// Check for issue marker
		if line.starts_with("<!--issue:") && line.ends_with("-->") {
			seen_issue_marker = true;
			continue;
		}

		// Check for comment marker
		if line.starts_with("<!--comment:") && line.ends_with("-->") {
			// Save previous section
			if seen_issue_marker && current_comment_id.is_none() {
				// This was the issue body
				let body = current_body.trim().to_string();
				sections.push(ParsedSection::IssueBody { body });
			} else if let Some(id) = current_comment_id {
				let body = current_body.trim().to_string();
				sections.push(ParsedSection::Comment { id, body });
			}

			// Extract comment ID from URL like: <!--comment: https://github.com/owner/repo/issues/123#issuecomment-456-->
			let url_part = line.trim_start_matches("<!--comment:").trim_end_matches("-->").trim();
			if let Some(id_str) = url_part.split("#issuecomment-").last() {
				if let Ok(id) = id_str.parse::<u64>() {
					current_comment_id = Some(id);
				}
			}
			current_body.clear();
			in_labels_line = false;
			continue;
		}

		// Skip title line (# Title)
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

	// Save the last section
	if seen_issue_marker && current_comment_id.is_none() && sections.is_empty() {
		let body = current_body.trim().to_string();
		sections.push(ParsedSection::IssueBody { body });
	} else if let Some(id) = current_comment_id {
		let body = current_body.trim().to_string();
		sections.push(ParsedSection::Comment { id, body });
	} else if !current_body.trim().is_empty() && !sections.is_empty() {
		// New content at the end (after all existing comments) - treat as new comment
		let body = current_body.trim().to_string();
		sections.push(ParsedSection::NewComment { body });
	}

	sections
}

/// Parse typst content into sections (issue body + comments)
fn parse_typst_sections(content: &str) -> Vec<ParsedSection> {
	let mut sections = Vec::new();
	let mut current_body = String::new();
	let mut current_comment_id: Option<u64> = None;
	let mut seen_issue_marker = false;
	let mut in_labels_line = false;

	for line in content.lines() {
		// Check for issue marker
		if line.starts_with("// issue:") {
			seen_issue_marker = true;
			continue;
		}

		// Check for comment marker
		if line.starts_with("// comment:") {
			// Save previous section
			if seen_issue_marker && current_comment_id.is_none() {
				let body = current_body.trim().to_string();
				sections.push(ParsedSection::IssueBody { body });
			} else if let Some(id) = current_comment_id {
				let body = current_body.trim().to_string();
				sections.push(ParsedSection::Comment { id, body });
			}

			// Extract comment ID from URL like: // comment: https://github.com/owner/repo/issues/123#issuecomment-456
			let url_part = line.trim_start_matches("// comment:").trim();
			if let Some(id_str) = url_part.split("#issuecomment-").last() {
				if let Ok(id) = id_str.parse::<u64>() {
					current_comment_id = Some(id);
				}
			}
			current_body.clear();
			in_labels_line = false;
			continue;
		}

		// Skip title line (= Title)
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

	// Save the last section
	if seen_issue_marker && current_comment_id.is_none() && sections.is_empty() {
		let body = current_body.trim().to_string();
		sections.push(ParsedSection::IssueBody { body });
	} else if let Some(id) = current_comment_id {
		let body = current_body.trim().to_string();
		sections.push(ParsedSection::Comment { id, body });
	} else if !current_body.trim().is_empty() && !sections.is_empty() {
		let body = current_body.trim().to_string();
		sections.push(ParsedSection::NewComment { body });
	}

	sections
}

/// Sync changes from edited file back to GitHub
async fn sync_changes_to_github(settings: &LiveSettings, lock: &EditLock, edited_content: &str) -> Result<()> {
	let sections = match lock.extension.as_str() {
		"md" => parse_markdown_sections(edited_content),
		"typ" => parse_typst_sections(edited_content),
		_ => return Err(eyre!("Unsupported extension: {}", lock.extension)),
	};

	let mut updates_made = 0;

	for section in sections {
		match section {
			ParsedSection::IssueBody { body } => {
				let original = lock.original_issue_body.as_deref().unwrap_or("");
				if body != original {
					println!("Updating issue body...");
					update_github_issue_body(settings, &lock.owner, &lock.repo, lock.issue_number, &body).await?;
					updates_made += 1;
				}
			}
			ParsedSection::Comment { id, body } => {
				// Find original comment
				let original = lock.original_comments.iter().find(|c| c.id == id).and_then(|c| c.body.as_deref()).unwrap_or("");
				if body != original {
					println!("Updating comment {}...", id);
					update_github_comment(settings, &lock.owner, &lock.repo, id, &body).await?;
					updates_made += 1;
				}
			}
			ParsedSection::NewComment { body } =>
				if !body.is_empty() {
					println!("Creating new comment...");
					create_github_comment(settings, &lock.owner, &lock.repo, lock.issue_number, &body).await?;
					updates_made += 1;
				},
		}
	}

	if updates_made == 0 {
		println!("No changes detected.");
	} else {
		println!("Synced {} update(s) to GitHub.", updates_made);
	}

	Ok(())
}

pub async fn open_command(settings: &LiveSettings, args: OpenArgs) -> Result<()> {
	use std::path::Path;

	let lock_path = v_utils::xdg_state_file!(ISSUE_EDIT_LOCK_FILENAME);

	let (owner, repo, issue_number) = parse_github_issue_url(&args.url)?;

	println!("Fetching issue #{} from {}/{}...", issue_number, owner, repo);

	let issue = fetch_github_issue(settings, &owner, &repo, issue_number).await?;
	let comments = fetch_github_comments(settings, &owner, &repo, issue_number).await?;

	// Format content based on extension
	let content = match args.extension {
		Extension::Md => format_issue_as_markdown(&issue, &comments, &owner, &repo),
		Extension::Typ => format_issue_as_typst(&issue, &comments, &owner, &repo),
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

	#[test]
	fn test_format_issue_as_markdown() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![GitHubLabel { name: "bug".to_string() }, GitHubLabel { name: "help wanted".to_string() }],
		};

		let md = format_issue_as_markdown(&issue, &[], "owner", "repo");
		assert_snapshot!(md, @r#"
  # Test Issue
  <!--issue: https://github.com/owner/repo/issues/123-->

  **Labels:** bug, help wanted

  Issue body text
  "#);
	}

	#[test]
	fn test_format_issue_as_markdown_with_comments() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![],
		};
		let comments = vec![
			GitHubComment {
				id: 1001,
				body: Some("First comment".to_string()),
			},
			GitHubComment {
				id: 1002,
				body: Some("Second comment".to_string()),
			},
		];

		let md = format_issue_as_markdown(&issue, &comments, "owner", "repo");
		assert_snapshot!(md, @r#"
  # Test Issue
  <!--issue: https://github.com/owner/repo/issues/123-->

  Issue body text

  <!--comment: https://github.com/owner/repo/issues/123#issuecomment-1001-->
  First comment

  <!--comment: https://github.com/owner/repo/issues/123#issuecomment-1002-->
  Second comment
  "#);
	}

	#[test]
	fn test_format_issue_as_typst() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("## Subheading\nBody text".to_string()),
			labels: vec![GitHubLabel { name: "enhancement".to_string() }],
		};

		let typ = format_issue_as_typst(&issue, &[], "owner", "repo");
		assert_snapshot!(typ, @r#"
  = Test Issue
  // issue: https://github.com/owner/repo/issues/123

  *Labels:* enhancement

  == Subheading
  Body text
  "#);
	}

	#[test]
	fn test_format_issue_as_typst_with_comments() {
		let issue = GitHubIssue {
			number: 456,
			title: "Typst Issue".to_string(),
			body: Some("Body".to_string()),
			labels: vec![],
		};
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
		}];

		let typ = format_issue_as_typst(&issue, &comments, "testowner", "testrepo");
		assert_snapshot!(typ, @r#"
  = Typst Issue
  // issue: https://github.com/testowner/testrepo/issues/456

  Body

  // comment: https://github.com/testowner/testrepo/issues/456#issuecomment-2001
  A comment
  "#);
	}

	#[test]
	fn test_parse_markdown_sections_roundtrip() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![],
		};
		let comments = vec![
			GitHubComment {
				id: 1001,
				body: Some("First comment".to_string()),
			},
			GitHubComment {
				id: 1002,
				body: Some("Second comment".to_string()),
			},
		];

		let md = format_issue_as_markdown(&issue, &comments, "owner", "repo");
		let sections = parse_markdown_sections(&md);
		assert_snapshot!(format!("{sections:#?}"), @r#"
		[
		    IssueBody {
		        body: "Issue body text",
		    },
		    Comment {
		        id: 1001,
		        body: "First comment",
		    },
		    Comment {
		        id: 1002,
		        body: "Second comment",
		    },
		]
		"#);
	}

	#[test]
	fn test_parse_typst_sections_roundtrip() {
		let issue = GitHubIssue {
			number: 456,
			title: "Typst Issue".to_string(),
			body: Some("Body text".to_string()),
			labels: vec![],
		};
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
		}];

		let typ = format_issue_as_typst(&issue, &comments, "testowner", "testrepo");
		let sections = parse_typst_sections(&typ);
		assert_snapshot!(format!("{sections:#?}"), @r#"
		[
		    IssueBody {
		        body: "Body text",
		    },
		    Comment {
		        id: 2001,
		        body: "A comment",
		    },
		]
		"#);
	}
}
