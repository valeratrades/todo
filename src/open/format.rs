//! Issue formatting for rendering GitHub issues to local files.

use super::{
	files::{find_sub_issue_file, read_sub_issue_body_from_file},
	util::{Extension, convert_markdown_to_typst, extract_checkbox_title},
};
use crate::{
	github::{GitHubComment, GitHubIssue},
	marker::Marker,
};

#[expect(clippy::too_many_arguments)]
pub fn format_issue(issue: &GitHubIssue, comments: &[GitHubComment], sub_issues: &[GitHubIssue], owner: &str, repo: &str, current_user: &str, render_closed: bool, ext: Extension) -> String {
	let mut content = String::new();

	let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
	let issue_owned = issue.user.login == current_user;
	let issue_closed = issue.state == "closed";
	let checked = if issue_closed { "x" } else { " " };

	// Title line: `- [ ] [label1, label2] Title <!-- url -->` or `- [ ] Title <!-- url -->` if no labels
	let title_marker = Marker::IssueUrl {
		url: issue_url.clone(),
		immutable: !issue_owned,
	};
	let labels_part = if issue.labels.is_empty() {
		String::new()
	} else {
		let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
		format!("[{}] ", labels.join(", "))
	};
	content.push_str(&format!("- [{checked}] {labels_part}{} {}\n", issue.title, title_marker.encode(ext)));

	// If issue is closed and render_closed is false, omit contents
	if issue_closed && !render_closed {
		content.push_str(&format!("\t{}\n", Marker::Omitted.encode(ext)));
		return content;
	}

	// Body (description - indented under the issue)
	// Skip checkbox lines that match sub-issue titles (they'll be shown via sub-issues list)
	if let Some(body) = &issue.body
		&& !body.is_empty()
	{
		let sub_issue_titles: Vec<&str> = sub_issues.iter().map(|s| s.title.as_str()).collect();
		let mut skip_until_non_indented = false;

		let body_text = match ext {
			Extension::Md => body.clone(),
			Extension::Typ => convert_markdown_to_typst(body),
		};

		for line in body_text.lines() {
			// Check if this line is a checkbox that matches a sub-issue
			if let Some(title) = extract_checkbox_title(line)
				&& sub_issue_titles.contains(&title.as_str())
			{
				skip_until_non_indented = true;
				continue;
			}

			if skip_until_non_indented {
				if line.starts_with('\t') || line.starts_with("    ") || line.is_empty() {
					continue;
				}
				skip_until_non_indented = false;
			}

			if issue_owned {
				content.push_str(&format!("\t{line}\n"));
			} else {
				// Double indent for immutable body
				content.push_str(&format!("\t\t{line}\n"));
			}
		}
	}

	// Comments (part of body, indented under the issue)
	for comment in comments {
		let comment_url = format!("https://github.com/{owner}/{repo}/issues/{}#issuecomment-{}", issue.number, comment.id);
		let comment_owned = comment.user.login == current_user;

		// Add empty line (with indent for LSP) before comment marker if previous line has content (markdown only)
		if ext == Extension::Md && content.lines().last().is_some_and(|l| !l.trim().is_empty()) {
			content.push_str("\t\n");
		}

		let comment_marker = Marker::Comment {
			url: comment_url,
			id: comment.id,
			immutable: !comment_owned,
		};
		content.push_str(&format!("\t{}\n", comment_marker.encode(ext)));

		if let Some(body) = &comment.body
			&& !body.is_empty()
		{
			let body_text = match ext {
				Extension::Md => body.clone(),
				Extension::Typ => convert_markdown_to_typst(body),
			};
			if comment_owned {
				for line in body_text.lines() {
					content.push_str(&format!("\t{line}\n"));
				}
			} else {
				// Double indent for immutable comments
				for line in body_text.lines() {
					content.push_str(&format!("\t\t{line}\n"));
				}
			}
		}
	}

	// Sub-issues at the very end - embed their body content
	// Prefer local file contents over GitHub body when available
	// Closed sub-issues show `<!-- omitted -->` instead of body content
	for sub in sub_issues {
		// Add empty line (with indent for LSP) before each sub-issue if previous line has content (markdown only)
		if ext == Extension::Md && content.lines().last().is_some_and(|l| !l.trim().is_empty()) {
			content.push_str("\t\n");
		}

		let sub_url = format!("https://github.com/{owner}/{repo}/issues/{}", sub.number);
		let sub_closed = sub.state == "closed";
		let sub_checked = if sub_closed { "x" } else { " " };
		let sub_marker = Marker::SubIssue { url: sub_url };
		content.push_str(&format!("\t- [{sub_checked}] {} {}\n", sub.title, sub_marker.encode(ext)));

		// Closed sub-issues show omitted marker instead of content
		if sub_closed {
			content.push_str(&format!("\t\t{}\n", Marker::Omitted.encode(ext)));
			continue;
		}

		// Try to read local file contents for this sub-issue
		let local_body = find_sub_issue_file(owner, repo, issue.number, &issue.title, sub.number).and_then(|path| read_sub_issue_body_from_file(&path));

		// Use local file contents if available, otherwise fall back to GitHub body
		let body_to_embed = local_body.as_deref().or(sub.body.as_deref());

		if let Some(body) = body_to_embed
			&& !body.is_empty()
		{
			let body_text = match ext {
				Extension::Md => body.to_string(),
				Extension::Typ => convert_markdown_to_typst(body),
			};
			for line in body_text.lines() {
				content.push_str(&format!("\t\t{line}\n"));
			}
		}
	}

	content
}

#[cfg(test)]
mod tests {
	use insta::assert_snapshot;

	use super::*;
	use crate::github::{GitHubLabel, GitHubUser};

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
	fn test_format_issue_md_owned() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec!["bug", "help wanted"], "me", "open");

		let md = format_issue(&issue, &[], &[], "owner", "repo", "me", false, Extension::Md);
		assert_snapshot!(md, @"
		- [ ] [bug, help wanted] Test Issue <!-- https://github.com/owner/repo/issues/123 -->
			Issue body text
		");
	}

	#[test]
	fn test_format_issue_md_not_owned() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec![], "other", "open");

		let md = format_issue(&issue, &[], &[], "owner", "repo", "me", false, Extension::Md);
		assert_snapshot!(md, @"
		- [ ] Test Issue <!--immutable https://github.com/owner/repo/issues/123 -->
				Issue body text
		");
	}

	#[test]
	fn test_format_issue_md_closed_omitted() {
		let issue = make_issue(123, "Closed Issue", Some("Issue body text"), vec![], "me", "closed");

		// Default: closed issues have omitted contents
		let md = format_issue(&issue, &[], &[], "owner", "repo", "me", false, Extension::Md);
		assert_snapshot!(md, @"
		- [x] Closed Issue <!-- https://github.com/owner/repo/issues/123 -->
			<!-- omitted -->
		");
	}

	#[test]
	fn test_format_issue_md_closed_rendered() {
		let issue = make_issue(123, "Closed Issue", Some("Issue body text"), vec![], "me", "closed");

		// With render_closed: full contents shown
		let md = format_issue(&issue, &[], &[], "owner", "repo", "me", true, Extension::Md);
		assert_snapshot!(md, @"
		- [x] Closed Issue <!-- https://github.com/owner/repo/issues/123 -->
			Issue body text
		");
	}

	#[test]
	fn test_format_issue_md_with_sub_issues() {
		let issue = make_issue(123, "Test Issue", Some("Issue body text"), vec![], "me", "open");
		let sub_issues = vec![
			make_issue(124, "Open sub-issue", Some("Sub-issue body content"), vec![], "me", "open"),
			make_issue(125, "Closed sub-issue", None, vec![], "me", "closed"),
		];

		let md = format_issue(&issue, &[], &sub_issues, "owner", "repo", "me", false, Extension::Md);
		assert_snapshot!(md, @"
		- [ ] Test Issue <!-- https://github.com/owner/repo/issues/123 -->
			Issue body text
			
			- [ ] Open sub-issue <!--sub https://github.com/owner/repo/issues/124 -->
				Sub-issue body content
			
			- [x] Closed sub-issue <!--sub https://github.com/owner/repo/issues/125 -->
				<!-- omitted -->
		");
	}

	#[test]
	fn test_format_issue_md_mixed_ownership() {
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

		let md = format_issue(&issue, &comments, &[], "owner", "repo", "me", false, Extension::Md);
		assert_snapshot!(md, @"
		- [ ] Test Issue <!--immutable https://github.com/owner/repo/issues/123 -->
				Issue body text
			
			<!-- https://github.com/owner/repo/issues/123#issuecomment-1001 -->
			First comment
			
			<!--immutable https://github.com/owner/repo/issues/123#issuecomment-1002 -->
				Second comment
		");
	}

	#[test]
	fn test_format_issue_typ_owned() {
		let issue = make_issue(123, "Test Issue", Some("## Subheading\nBody text"), vec!["enhancement"], "me", "open");

		let typ = format_issue(&issue, &[], &[], "owner", "repo", "me", false, Extension::Typ);
		assert_snapshot!(typ, @"
		- [ ] [enhancement] Test Issue // https://github.com/owner/repo/issues/123
			== Subheading
			Body text
		");
	}

	#[test]
	fn test_format_issue_typ_not_owned() {
		let issue = make_issue(456, "Typst Issue", Some("Body"), vec![], "other", "open");
		let comments = vec![GitHubComment {
			id: 2001,
			body: Some("A comment".to_string()),
			user: make_user("other"),
		}];

		let typ = format_issue(&issue, &comments, &[], "testowner", "testrepo", "me", false, Extension::Typ);
		assert_snapshot!(typ, @"
		- [ ] Typst Issue // immutable https://github.com/testowner/testrepo/issues/456
				Body
			// immutable https://github.com/testowner/testrepo/issues/456#issuecomment-2001
				A comment
		");
	}

	#[test]
	fn test_format_issue_typ_closed_omitted() {
		let issue = make_issue(123, "Closed Issue", Some("Body text"), vec![], "me", "closed");

		let typ = format_issue(&issue, &[], &[], "owner", "repo", "me", false, Extension::Typ);
		assert_snapshot!(typ, @"
		- [x] Closed Issue // https://github.com/owner/repo/issues/123
			// omitted
		");
	}

	/// Test the correct rendering order for issues:
	/// 1. Title line with labels inline
	/// 2. Body (first comment)
	/// 3. Comments (with markers)
	/// 4. Blockers section
	/// 5. Sub-issues at the very end
	#[test]
	fn test_issue_render_order_complete() {
		let issue = make_issue(123, "Main Issue", Some("This is the body text.\nWith multiple lines."), vec!["bug", "priority"], "me", "open");
		let comments = vec![
			GitHubComment {
				id: 1001,
				body: Some("First comment from me".to_string()),
				user: make_user("me"),
			},
			GitHubComment {
				id: 1002,
				body: Some("Second comment from other".to_string()),
				user: make_user("other"),
			},
		];
		let sub_issues = vec![
			make_issue(124, "Sub Issue One", Some("Sub issue body"), vec![], "me", "open"),
			make_issue(125, "Sub Issue Two", None, vec![], "me", "closed"),
		];

		let md = format_issue(&issue, &comments, &sub_issues, "owner", "repo", "me", false, Extension::Md);

		// Expected order:
		// 1. Title line with labels: `- [ ] [label1, label2] Title <!-- url -->`
		// 2. Body (description + comments + blockers)
		// 3. Sub-issues at the very end
		assert_snapshot!(md, @"
		- [ ] [bug, priority] Main Issue <!-- https://github.com/owner/repo/issues/123 -->
			This is the body text.
			With multiple lines.
			
			<!-- https://github.com/owner/repo/issues/123#issuecomment-1001 -->
			First comment from me
			
			<!--immutable https://github.com/owner/repo/issues/123#issuecomment-1002 -->
				Second comment from other
			
			- [ ] Sub Issue One <!--sub https://github.com/owner/repo/issues/124 -->
				Sub issue body
			
			- [x] Sub Issue Two <!--sub https://github.com/owner/repo/issues/125 -->
				<!-- omitted -->
		");
	}
}
