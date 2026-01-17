//! Github synchronization for Issues.
//!
//! This module contains all Github-specific logic for Issues:
//! - Converting Github API responses to Issue

use jiff::Timestamp;
use todo::{BlockerSequence, CloseState, Comment, CommentIdentity, Issue, IssueContents, IssueIdentity, IssueLink, IssueMeta, split_blockers};

use crate::github::{GithubComment, GithubIssue};

/// Extension trait for Github-specific Issue operations.
/// These methods are only available in the binary, not the library.
pub trait IssueGithubExt {
	/// Construct an Issue directly from Github API data.
	fn from_github(issue: &GithubIssue, comments: &[GithubComment], sub_issues: &[GithubIssue], owner: &str, repo: &str, current_user: &str) -> Issue;
}

impl IssueGithubExt for Issue {
	fn from_github(issue: &GithubIssue, comments: &[GithubComment], sub_issues: &[GithubIssue], owner: &str, repo: &str, _current_user: &str) -> Issue {
		let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
		let close_state = CloseState::from_github(&issue.state, issue.state_reason.as_deref());

		let link = IssueLink::parse(&issue_url).expect("just constructed valid URL");
		let identity = IssueIdentity::Created {
			user: issue.user.login.clone(),
			link,
		};
		let labels: Vec<String> = issue.labels.iter().map(|l| l.name.clone()).collect();

		// Parse timestamp from Github's ISO 8601 format
		let last_contents_change = issue.updated_at.parse::<Timestamp>().ok();

		let meta = IssueMeta { identity, last_contents_change };

		// Build comments: body is first comment
		// Split out blockers from body (they're appended during sync)
		let mut issue_comments = Vec::new();
		let raw_body = issue.body.as_deref().unwrap_or("");
		let (body, blockers) = split_blockers(raw_body);
		issue_comments.push(Comment {
			identity: CommentIdentity::Body,
			body: todo::Events::parse(&body),
		});

		// Actual comments
		for c in comments {
			issue_comments.push(Comment {
				identity: CommentIdentity::Created {
					user: c.user.login.clone(),
					id: c.id,
				},
				body: todo::Events::parse(c.body.as_deref().unwrap_or("")),
			});
		}

		// Build children from sub-issues (shallow - just metadata)
		// Filter out duplicates - they shouldn't appear in local representation
		let children: Vec<Issue> = sub_issues
			.iter()
			.filter(|si| !CloseState::is_duplicate_reason(si.state_reason.as_deref()))
			.map(|si| {
				let child_url = format!("https://github.com/{owner}/{repo}/issues/{}", si.number);
				let child_link = IssueLink::parse(&child_url).expect("just constructed valid URL");
				let child_identity = IssueIdentity::Created {
					user: si.user.login.clone(),
					link: child_link,
				};
				let child_close_state = CloseState::from_github(&si.state, si.state_reason.as_deref());
				let child_timestamp = si.updated_at.parse::<Timestamp>().ok();
				let child_labels: Vec<String> = si.labels.iter().map(|l| l.name.clone()).collect();
				Issue {
					meta: IssueMeta {
						identity: child_identity,
						last_contents_change: child_timestamp,
					},
					contents: IssueContents {
						title: si.title.clone(),
						labels: child_labels,
						state: child_close_state,
						comments: vec![Comment {
							identity: CommentIdentity::Body,
							body: todo::Events::parse(si.body.as_deref().unwrap_or("")),
						}],
						blockers: BlockerSequence::default(),
					},
					children: Vec::new(),
				}
			})
			.collect();

		Issue {
			meta,
			contents: IssueContents {
				title: issue.title.clone(),
				labels,
				state: close_state,
				comments: issue_comments,
				blockers,
			},
			children,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::github::{GithubLabel, GithubUser};

	#[test]
	fn test_from_github() {
		let issue = GithubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body".to_string()),
			labels: vec![GithubLabel { name: "bug".to_string() }],
			user: GithubUser { login: "me".to_string() },
			state: "open".to_string(),
			state_reason: None,
			updated_at: "2024-01-15T12:00:00Z".to_string(),
		};

		let comments = vec![GithubComment {
			id: 456,
			body: Some("A comment".to_string()),
			user: GithubUser { login: "other".to_string() },
		}];

		let sub_issues = vec![GithubIssue {
			number: 124,
			title: "Sub Issue".to_string(),
			body: Some("Sub body".to_string()),
			labels: vec![],
			user: GithubUser { login: "me".to_string() },
			state: "closed".to_string(),
			state_reason: Some("completed".to_string()),
			updated_at: "2024-01-15T12:00:00Z".to_string(),
		}];

		let result = Issue::from_github(&issue, &comments, &sub_issues, "owner", "repo", "me");

		assert_eq!(result.contents.title, "Test Issue");
		assert_eq!(result.meta.identity.url_str(), Some("https://github.com/owner/repo/issues/123"));
		assert_eq!(result.contents.state, CloseState::Open);
		assert_eq!(result.meta.identity.user(), Some("me"));
		assert_eq!(result.contents.labels, vec!["bug".to_string()]);

		// Body + 1 comment
		assert_eq!(result.contents.comments.len(), 2);
		assert_eq!(result.contents.comments[0].body.plain_text(), "Issue body");
		assert_eq!(result.contents.comments[0].identity, CommentIdentity::Body);
		assert_eq!(result.contents.comments[1].identity.id(), Some(456));
		assert_eq!(result.contents.comments[1].body.plain_text(), "A comment");
		assert_eq!(result.contents.comments[1].identity.user(), Some("other")); // different user

		// Sub-issue
		assert_eq!(result.children.len(), 1);
		assert_eq!(result.children[0].contents.title, "Sub Issue");
		assert_eq!(result.children[0].contents.state, CloseState::Closed);
	}

	#[test]
	fn test_partial_eq() {
		let make_issue = |body: &str, state: &str| -> Issue {
			let gh_issue = GithubIssue {
				number: 1,
				title: "Test".to_string(),
				body: Some(body.to_string()),
				labels: vec![],
				user: GithubUser { login: "me".to_string() },
				state: state.to_string(),
				state_reason: None,
				updated_at: "2024-01-15T12:00:00Z".to_string(),
			};
			Issue::from_github(&gh_issue, &[], &[], "o", "r", "me")
		};

		let issue1 = make_issue("body", "open");
		let issue2 = make_issue("body", "open");
		let issue3 = make_issue("different", "open");
		let issue4 = make_issue("body", "closed");

		assert_eq!(issue1, issue2);
		assert_ne!(issue1, issue3); // different body
		assert_ne!(issue1, issue4); // different state
	}
}
