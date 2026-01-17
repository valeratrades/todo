//! Github synchronization for Issues.
//!
//! This module contains all Github-specific logic for Issues:
//! - Converting Github API responses to Issue
//! - Collecting actions needed to sync Issue to Github
//!
//! Actions are split into two categories:
//! - **Pre-sync**: Creating new issues (without URLs) so they get URLs before comparison
//! - **Post-sync**: Updating existing issue states to match local changes

use jiff::Timestamp;
use todo::{BlockerSequence, CloseState, Comment, CommentIdentity, Issue, IssueIdentity, IssueLink, IssueMeta, split_blockers};

use crate::github::{GithubComment, GithubIssue, IssueAction, OriginalSubIssue};

/// Extension trait for Github-specific Issue operations.
/// These methods are only available in the binary, not the library.
pub trait IssueGithubExt {
	/// Collect actions for creating new issues (pre-sync).
	/// These must run BEFORE sync so new issues get URLs for comparison.
	fn collect_create_actions(&self) -> Vec<Vec<IssueAction>>;

	/// Collect actions for updating existing issue states (post-sync).
	/// These run AFTER sync to push local state changes to remote.
	fn collect_update_actions(&self, consensus_sub_issues: &[OriginalSubIssue]) -> Vec<Vec<IssueAction>>;

	/// Construct an Issue directly from Github API data.
	fn from_github(issue: &GithubIssue, comments: &[GithubComment], sub_issues: &[GithubIssue], owner: &str, repo: &str, current_user: &str) -> Issue;
}

impl IssueGithubExt for Issue {
	fn collect_create_actions(&self) -> Vec<Vec<IssueAction>> {
		let mut levels: Vec<Vec<IssueAction>> = Vec::new();

		// Check if root issue needs to be created (pending = no URL yet)
		if self.meta.identity.is_pending() {
			levels.push(vec![IssueAction::CreateIssue {
				path: vec![],
				title: self.meta.title.clone(),
				body: self.body(),
				closed: self.meta.close_state.is_closed(),
				parent: None,
			}]);
			// Don't collect sub-issue creates yet - they'll be handled after root is created
			return levels;
		}

		// Collect create actions for children recursively
		collect_create_actions_recursive(self, &[], &mut levels);
		levels
	}

	fn collect_update_actions(&self, consensus_sub_issues: &[OriginalSubIssue]) -> Vec<Vec<IssueAction>> {
		let mut levels: Vec<Vec<IssueAction>> = Vec::new();

		// Only collect updates if root exists
		if self.meta.identity.is_linked() {
			collect_update_actions_recursive(self, &[], consensus_sub_issues, &mut levels);
		}

		levels
	}

	fn from_github(issue: &GithubIssue, comments: &[GithubComment], sub_issues: &[GithubIssue], owner: &str, repo: &str, current_user: &str) -> Issue {
		let issue_url = format!("https://github.com/{owner}/{repo}/issues/{}", issue.number);
		let issue_owned = issue.user.login == current_user;
		let close_state = CloseState::from_github(&issue.state, issue.state_reason.as_deref());

		let link = IssueLink::parse(&issue_url).expect("just constructed valid URL");
		let identity = IssueIdentity::Created {
			user: issue.user.login.clone(),
			link,
		};
		let labels: Vec<String> = issue.labels.iter().map(|l| l.name.clone()).collect();
		let meta = IssueMeta {
			title: issue.title.clone(),
			identity,
			close_state,
			owned: issue_owned,
			labels,
		};

		// Parse timestamp from Github's ISO 8601 format
		let last_contents_change = issue.updated_at.parse::<Timestamp>().ok();

		// Build comments: body is first comment
		// Split out blockers from body (they're appended during sync)
		let mut issue_comments = Vec::new();
		let raw_body = issue.body.as_deref().unwrap_or("");
		let (body, blockers) = split_blockers(raw_body);
		issue_comments.push(Comment {
			identity: CommentIdentity::Body,
			body: todo::Events::parse(&body),
			owned: issue_owned,
		});

		// Actual comments
		for c in comments {
			let comment_owned = c.user.login == current_user;
			issue_comments.push(Comment {
				identity: CommentIdentity::Created {
					user: c.user.login.clone(),
					id: c.id,
				},
				body: todo::Events::parse(c.body.as_deref().unwrap_or("")),
				owned: comment_owned,
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
				Issue {
					meta: IssueMeta {
						title: si.title.clone(),
						identity: child_identity,
						close_state: child_close_state,
						owned: si.user.login == current_user,
						labels: si.labels.iter().map(|l| l.name.clone()).collect(),
					},
					contents: Default::default(),
					comments: vec![Comment {
						identity: CommentIdentity::Body,
						body: todo::Events::parse(si.body.as_deref().unwrap_or("")),
						owned: si.user.login == current_user,
					}],
					children: Vec::new(),
					blockers: BlockerSequence::default(),
					last_contents_change: child_timestamp,
				}
			})
			.collect();

		Issue {
			meta,
			contents: Default::default(),
			comments: issue_comments,
			children,
			blockers,
			last_contents_change,
		}
	}
}

/// Recursively collect CREATE actions only (for pre-sync)
fn collect_create_actions_recursive(issue: &Issue, current_path: &[usize], levels: &mut Vec<Vec<IssueAction>>) {
	let depth = current_path.len();

	// Ensure we have a vec for this level
	while levels.len() <= depth {
		levels.push(Vec::new());
	}

	// Get parent issue number from identity
	let parent_number = issue.meta.identity.number();

	// Check each child for create actions
	for (i, child) in issue.children.iter().enumerate() {
		let mut child_path = current_path.to_vec();
		child_path.push(i);

		if child.meta.identity.is_pending() {
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
		}

		// Recursively process child's children (only if child is linked - can't create under non-existent parent)
		if child.meta.identity.is_linked() {
			collect_create_actions_recursive(child, &child_path, levels);
		}
	}
}

/// Recursively collect UPDATE actions only (for post-sync)
fn collect_update_actions_recursive(issue: &Issue, current_path: &[usize], consensus_sub_issues: &[OriginalSubIssue], levels: &mut Vec<Vec<IssueAction>>) {
	let depth = current_path.len();

	// Ensure we have a vec for this level
	while levels.len() <= depth {
		levels.push(Vec::new());
	}

	// Check each child for update actions
	for (i, child) in issue.children.iter().enumerate() {
		let mut child_path = current_path.to_vec();
		child_path.push(i);

		// Only check for updates if child is linked (exists on Github)
		if let Some(child_number) = child.meta.identity.number()
			&& let Some(consensus) = consensus_sub_issues.iter().find(|o| o.number == child_number)
		{
			let consensus_closed = consensus.state == "closed";
			if child.meta.close_state.is_closed() != consensus_closed {
				levels[depth].push(IssueAction::UpdateIssueState {
					issue_number: child_number,
					closed: child.meta.close_state.is_closed(),
				});
			}
		}

		// Recursively process child's children
		collect_update_actions_recursive(child, &child_path, consensus_sub_issues, levels);
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

		assert_eq!(result.meta.title, "Test Issue");
		assert_eq!(result.meta.identity.url_str(), Some("https://github.com/owner/repo/issues/123"));
		assert_eq!(result.meta.close_state, CloseState::Open);
		assert!(result.meta.owned);
		assert_eq!(result.meta.labels, vec!["bug".to_string()]);

		// Body + 1 comment
		assert_eq!(result.comments.len(), 2);
		assert_eq!(result.comments[0].body.plain_text(), "Issue body");
		assert!(result.comments[0].owned);
		assert_eq!(result.comments[1].identity.id(), Some(456));
		assert_eq!(result.comments[1].body.plain_text(), "A comment");
		assert!(!result.comments[1].owned); // different user

		// Sub-issue
		assert_eq!(result.children.len(), 1);
		assert_eq!(result.children[0].meta.title, "Sub Issue");
		assert_eq!(result.children[0].meta.close_state, CloseState::Closed);
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
