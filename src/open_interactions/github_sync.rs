//! GitHub synchronization for Issues.
//!
//! This module contains all GitHub-specific logic for Issues:
//! - Converting GitHub API responses to Issue
//! - Converting stored metadata to Issue
//! - Collecting actions needed to sync Issue to GitHub

use todo::{BlockerSequence, CloseState, Comment, Issue, IssueMeta};

use super::meta::IssueMetaEntry;
use crate::github::{self, GitHubComment, GitHubIssue, IssueAction, OriginalSubIssue};

/// Extension trait for GitHub-specific Issue operations.
/// These methods are only available in the binary, not the library.
pub trait IssueGitHubExt {
	/// Collect all required GitHub actions, organized by nesting level.
	fn collect_actions(&self, original_sub_issues: &[OriginalSubIssue]) -> Vec<Vec<IssueAction>>;

	/// Construct an Issue directly from GitHub API data.
	fn from_github(issue: &GitHubIssue, comments: &[GitHubComment], sub_issues: &[GitHubIssue], owner: &str, repo: &str, current_user: &str) -> Issue;

	/// Construct an Issue from stored metadata (the "original" consensus state).
	fn from_meta(meta: &IssueMetaEntry, owner: &str, repo: &str) -> Issue;
}

impl IssueGitHubExt for Issue {
	fn collect_actions(&self, original_sub_issues: &[OriginalSubIssue]) -> Vec<Vec<IssueAction>> {
		let mut levels: Vec<Vec<IssueAction>> = Vec::new();

		// Check if root issue needs to be created (no URL = pending creation from --touch)
		if self.meta.url.is_none() {
			levels.push(vec![IssueAction::CreateIssue {
				path: vec![],
				title: self.meta.title.clone(),
				body: self.body(),
				closed: self.meta.close_state.is_closed(),
				parent: None,
			}]);
			// Don't collect sub-issue actions yet - they'll be handled after root is created
			return levels;
		}

		collect_actions_recursive(self, &[], original_sub_issues, &mut levels);
		levels
	}

	fn from_github(issue: &GitHubIssue, comments: &[GitHubComment], sub_issues: &[GitHubIssue], owner: &str, repo: &str, current_user: &str) -> Issue {
		let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
		let issue_owned = issue.user.login == current_user;
		let close_state = if issue.state == "closed" { CloseState::Closed } else { CloseState::Open };

		let meta = IssueMeta {
			title: issue.title.clone(),
			url: Some(issue_url.clone()),
			close_state,
			owned: issue_owned,
		};

		let labels: Vec<String> = issue.labels.iter().map(|l| l.name.clone()).collect();

		// Build comments: body is first comment
		let mut issue_comments = Vec::new();

		// Body as first comment
		let body = issue.body.as_deref().unwrap_or("").to_string();
		issue_comments.push(Comment { id: None, body, owned: issue_owned });

		// Actual comments
		for c in comments {
			let comment_owned = c.user.login == current_user;
			issue_comments.push(Comment {
				id: Some(c.id),
				body: c.body.as_deref().unwrap_or("").to_string(),
				owned: comment_owned,
			});
		}

		// Build children from sub-issues (shallow - just metadata)
		let children: Vec<Issue> = sub_issues
			.iter()
			.map(|si| {
				let child_url = format!("https://github.com/{owner}/{repo}/issues/{}", si.number);
				let child_close_state = if si.state == "closed" { CloseState::Closed } else { CloseState::Open };
				Issue {
					meta: IssueMeta {
						title: si.title.clone(),
						url: Some(child_url),
						close_state: child_close_state,
						owned: si.user.login == current_user,
					},
					labels: si.labels.iter().map(|l| l.name.clone()).collect(),
					comments: vec![Comment {
						id: None,
						body: si.body.as_deref().unwrap_or("").to_string(),
						owned: si.user.login == current_user,
					}],
					children: Vec::new(),
					blockers: BlockerSequence::default(),
				}
			})
			.collect();

		Issue {
			meta,
			labels,
			comments: issue_comments,
			children,
			blockers: BlockerSequence::default(),
		}
	}

	fn from_meta(meta: &IssueMetaEntry, owner: &str, repo: &str) -> Issue {
		let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", meta.issue_number);

		let issue_meta = IssueMeta {
			title: meta.title.clone(),
			url: Some(issue_url.clone()),
			close_state: meta.original_close_state.clone(),
			owned: true, // Doesn't matter for comparison
		};

		// Build comments from original_comments
		let mut comments = Vec::new();

		// Body as first comment
		comments.push(Comment {
			id: None,
			body: meta.original_issue_body.clone().unwrap_or_default(),
			owned: true,
		});

		// Original comments
		for oc in &meta.original_comments {
			comments.push(Comment {
				id: Some(oc.id),
				body: oc.body.clone().unwrap_or_default(),
				owned: true,
			});
		}

		// Build children from original_sub_issues (minimal - just state for comparison)
		let children: Vec<Issue> = meta
			.original_sub_issues
			.iter()
			.map(|osi| {
				let child_url = format!("https://github.com/{owner}/{repo}/issues/{}", osi.number);
				let child_close_state = if osi.state == "closed" { CloseState::Closed } else { CloseState::Open };
				Issue {
					meta: IssueMeta {
						title: String::new(), // Not stored in OriginalSubIssue
						url: Some(child_url),
						close_state: child_close_state,
						owned: true,
					},
					labels: Vec::new(),
					comments: Vec::new(),
					children: Vec::new(),
					blockers: BlockerSequence::default(),
				}
			})
			.collect();

		Issue {
			meta: issue_meta,
			labels: Vec::new(),
			comments,
			children,
			blockers: BlockerSequence::default(),
		}
	}
}

/// Recursively collect actions from this issue and its children
fn collect_actions_recursive(issue: &Issue, current_path: &[usize], original_sub_issues: &[OriginalSubIssue], levels: &mut Vec<Vec<IssueAction>>) {
	let depth = current_path.len();

	// Ensure we have a vec for this level
	while levels.len() <= depth {
		levels.push(Vec::new());
	}

	// Get parent issue number from URL
	let parent_number = issue.meta.url.as_ref().and_then(|url| github::extract_issue_number_from_url(url));

	// Check each child for required actions
	for (i, child) in issue.children.iter().enumerate() {
		let mut child_path = current_path.to_vec();
		child_path.push(i);

		if child.meta.url.is_none() {
			// New issue - needs to be created
			if let Some(parent_num) = parent_number {
				levels[depth].push(IssueAction::CreateIssue {
					path: child_path.clone(),
					title: child.meta.title.clone(),
					body: String::new(),
					closed: child.meta.close_state.is_closed(),
					parent: Some(parent_num),
				});
			}
		} else if let Some(child_url) = &child.meta.url {
			// Existing issue - check if state changed
			if let Some(child_number) = github::extract_issue_number_from_url(child_url)
				&& let Some(orig) = original_sub_issues.iter().find(|o| o.number == child_number)
			{
				let orig_closed = orig.state == "closed";
				if child.meta.close_state.is_closed() != orig_closed {
					levels[depth].push(IssueAction::UpdateIssueState {
						issue_number: child_number,
						closed: child.meta.close_state.is_closed(),
					});
				}
			}
		}

		// Recursively process child's children
		collect_actions_recursive(child, &child_path, original_sub_issues, levels);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::github::{GitHubLabel, GitHubUser, OriginalComment};

	#[test]
	fn test_from_github() {
		let issue = GitHubIssue {
			number: 123,
			title: "Test Issue".to_string(),
			body: Some("Issue body".to_string()),
			labels: vec![GitHubLabel { name: "bug".to_string() }],
			user: GitHubUser { login: "me".to_string() },
			state: "open".to_string(),
		};

		let comments = vec![GitHubComment {
			id: 456,
			body: Some("A comment".to_string()),
			user: GitHubUser { login: "other".to_string() },
		}];

		let sub_issues = vec![GitHubIssue {
			number: 124,
			title: "Sub Issue".to_string(),
			body: Some("Sub body".to_string()),
			labels: vec![],
			user: GitHubUser { login: "me".to_string() },
			state: "closed".to_string(),
		}];

		let result = Issue::from_github(&issue, &comments, &sub_issues, "owner", "repo", "me");

		assert_eq!(result.meta.title, "Test Issue");
		assert_eq!(result.meta.url, Some("https://github.com/owner/repo/issues/123".to_string()));
		assert_eq!(result.meta.close_state, CloseState::Open);
		assert!(result.meta.owned);
		assert_eq!(result.labels, vec!["bug".to_string()]);

		// Body + 1 comment
		assert_eq!(result.comments.len(), 2);
		assert_eq!(result.comments[0].body, "Issue body");
		assert!(result.comments[0].owned);
		assert_eq!(result.comments[1].id, Some(456));
		assert_eq!(result.comments[1].body, "A comment");
		assert!(!result.comments[1].owned); // different user

		// Sub-issue
		assert_eq!(result.children.len(), 1);
		assert_eq!(result.children[0].meta.title, "Sub Issue");
		assert_eq!(result.children[0].meta.close_state, CloseState::Closed);
	}

	#[test]
	fn test_partial_eq() {
		let make_issue = |body: &str, state: &str| -> Issue {
			let gh_issue = GitHubIssue {
				number: 1,
				title: "Test".to_string(),
				body: Some(body.to_string()),
				labels: vec![],
				user: GitHubUser { login: "me".to_string() },
				state: state.to_string(),
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

	#[test]
	fn test_from_meta_roundtrip() {
		let meta = IssueMetaEntry {
			issue_number: 42,
			title: "Meta Issue".to_string(),
			extension: "md".to_string(),
			original_issue_body: Some("Original body".to_string()),
			original_comments: vec![OriginalComment {
				id: 100,
				body: Some("Original comment".to_string()),
			}],
			original_sub_issues: vec![OriginalSubIssue {
				number: 43,
				state: "open".to_string(),
			}],
			parent_issue: None,
			original_close_state: CloseState::Open,
		};

		let issue = Issue::from_meta(&meta, "owner", "repo");

		assert_eq!(issue.meta.title, "Meta Issue");
		assert_eq!(issue.meta.close_state, CloseState::Open);
		assert_eq!(issue.comments.len(), 2); // body + 1 comment
		assert_eq!(issue.comments[0].body, "Original body");
		assert_eq!(issue.comments[1].id, Some(100));
		assert_eq!(issue.children.len(), 1);
		assert_eq!(issue.children[0].meta.close_state, CloseState::Open);
	}
}
