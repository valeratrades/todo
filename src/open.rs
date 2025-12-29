use clap::{Args, ValueEnum};
use reqwest::Client;
use serde::Deserialize;
use v_utils::prelude::*;

use crate::config::LiveSettings;

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

fn format_issue_as_markdown(issue: &GitHubIssue) -> String {
	let mut content = String::new();

	// Title as H1
	content.push_str(&format!("# {}\n\n", issue.title));

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

	content
}

fn format_issue_as_typst(issue: &GitHubIssue) -> String {
	let mut content = String::new();

	// Title as Typst heading
	content.push_str(&format!("= {}\n\n", issue.title));

	// Labels if any
	if !issue.labels.is_empty() {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		content.push_str(&format!("*Labels:* {}\n\n", labels.join(", ")));
	}

	// Body - convert markdown to typst basics
	if let Some(body) = &issue.body {
		if !body.is_empty() {
			// Basic conversions: ## -> ==, ### -> ===, **text** -> *text*, etc.
			let converted = body
				.lines()
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
				.join("\n");

			content.push_str(&converted);
			if !converted.ends_with('\n') {
				content.push('\n');
			}
		}
	}

	content
}

pub async fn open_command(settings: &LiveSettings, args: OpenArgs) -> Result<()> {
	use std::path::Path;

	let (owner, repo, issue_number) = parse_github_issue_url(&args.url)?;

	println!("Fetching issue #{} from {}/{}...", issue_number, owner, repo);

	let issue = fetch_github_issue(settings, &owner, &repo, issue_number).await?;

	// Format content based on extension
	let content = match args.extension {
		Extension::Md => format_issue_as_markdown(&issue),
		Extension::Typ => format_issue_as_typst(&issue),
	};

	// Create temp file
	let tmp_path = format!("/tmp/issue_{}_{}_{}_{}.{}", owner, repo, issue_number, issue.number, args.extension.as_str());
	std::fs::write(&tmp_path, &content)?;

	// Open in editor
	v_utils::io::open(Path::new(&tmp_path))?;

	// Clean up temp file after editor closes
	let _ = std::fs::remove_file(&tmp_path);

	Ok(())
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
	fn test_format_issue_as_markdown() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body text".to_string()),
			labels: vec![GitHubLabel { name: "bug".to_string() }, GitHubLabel { name: "help wanted".to_string() }],
		};

		let md = format_issue_as_markdown(&issue);
		assert!(md.contains("# Test Issue"));
		assert!(md.contains("**Labels:** bug, help wanted"));
		assert!(md.contains("Issue body text"));
	}

	#[test]
	fn test_format_issue_as_typst() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("## Subheading\nBody text".to_string()),
			labels: vec![GitHubLabel { name: "enhancement".to_string() }],
		};

		let typ = format_issue_as_typst(&issue);
		assert!(typ.contains("= Test Issue"));
		assert!(typ.contains("*Labels:* enhancement"));
		assert!(typ.contains("== Subheading"));
		assert!(typ.contains("Body text"));
	}
}
