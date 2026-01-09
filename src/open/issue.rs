//! Core issue data structures and parsing/serialization.

use serde::{Deserialize, Serialize};

use super::util::{is_blockers_marker, normalize_issue_indentation};
use crate::{
	error::{ParseContext, ParseError},
	github::{self, GitHubComment, GitHubIssue, IssueAction, OriginalSubIssue},
};

/// Close state of an issue.
/// Maps to GitHub's binary open/closed, but locally supports additional variants.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum CloseState {
	/// Issue is open: `- [ ]`
	#[default]
	Open,
	/// Issue is closed normally: `- [x]`
	Closed,
	/// Issue was closed as not planned: `- [-]`
	/// Treated same as Closed for storage (embedded with .bak)
	NotPlanned,
	/// Issue is a duplicate of another issue: `- [123]`
	/// The number references another issue in the same repo.
	/// These should be removed from local storage entirely.
	Duplicate(u64),
}

impl CloseState {
	/// Returns true if the issue is closed (any close variant)
	pub fn is_closed(&self) -> bool {
		!matches!(self, CloseState::Open)
	}

	/// Returns true if this close state means the issue should be removed from local storage
	pub fn should_remove(&self) -> bool {
		matches!(self, CloseState::Duplicate(_))
	}

	/// Convert to GitHub API state string
	pub fn to_github_state(&self) -> &'static str {
		match self {
			CloseState::Open => "open",
			_ => "closed",
		}
	}

	/// Parse from checkbox content (the character(s) inside `[ ]`)
	pub fn from_checkbox(content: &str) -> Option<Self> {
		let content = content.trim();
		match content {
			"" | " " => Some(CloseState::Open),
			"x" | "X" => Some(CloseState::Closed),
			"-" => Some(CloseState::NotPlanned),
			s => s.parse::<u64>().ok().map(CloseState::Duplicate),
		}
	}

	/// Convert to checkbox character(s) for serialization
	pub fn to_checkbox(&self) -> String {
		match self {
			CloseState::Open => " ".to_string(),
			CloseState::Closed => "x".to_string(),
			CloseState::NotPlanned => "-".to_string(),
			CloseState::Duplicate(n) => n.to_string(),
		}
	}
}

/// Metadata for an issue (title line info)
#[derive(Clone, Debug, PartialEq)]
pub struct IssueMeta {
	pub title: String,
	/// GitHub URL, None for new issues
	pub url: Option<String>,
	pub close_state: CloseState,
	/// Whether owned by current user (false = immutable)
	pub owned: bool,
}

/// A comment in the issue conversation (first one is always the issue body)
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
	/// Comment ID from GitHub URL, None for new comments or issue body
	pub id: Option<u64>,
	pub body: String,
	pub owned: bool,
}

/// Complete representation of an issue file
#[derive(Clone, Debug)]
pub struct Issue {
	pub meta: IssueMeta,
	pub labels: Vec<String>,
	/// Comments in order. First is always the issue body (serialized without marker).
	pub comments: Vec<Comment>,
	/// Sub-issues in order
	pub children: Vec<Issue>,
	/// Blockers section. Uses the blocker module's BlockerSequence directly.
	pub blockers: crate::blocker::BlockerSequence,
}

impl Issue {
	/// Get the full issue body including blockers section.
	/// This is what should be synced to GitHub as the issue body.
	pub fn body(&self) -> String {
		let base_body = self.comments.first().map(|c| c.body.as_str()).unwrap_or("");
		if self.blockers.is_empty() {
			base_body.to_string()
		} else {
			let mut full_body = base_body.to_string();
			full_body.push_str("# Blockers\n");
			full_body.push_str(&self.blockers.serialize());
			full_body.push('\n');
			full_body
		}
	}

	/// Parse markdown content into an Issue.
	/// Returns an error with a detailed message if any part of the file cannot be understood.
	pub fn parse(content: &str, ctx: &ParseContext) -> Result<Self, ParseError> {
		let normalized = normalize_issue_indentation(content);
		let mut lines = normalized.lines().peekable();

		Self::parse_at_depth(&mut lines, 0, 1, ctx)
	}

	/// Parse an issue at given nesting depth (0 = root, 1 = sub-issue, etc.)
	/// `line_num` tracks the current line for error reporting.
	fn parse_at_depth(lines: &mut std::iter::Peekable<std::str::Lines>, depth: usize, line_num: usize, ctx: &ParseContext) -> Result<Self, ParseError> {
		let indent = "\t".repeat(depth);
		let child_indent = "\t".repeat(depth + 1);

		// Parse title line: `- [ ] [label1, label2] Title <!--url-->` or `- [ ] Title <!--immutable url-->`
		let first_line = lines.next().ok_or(ParseError::EmptyFile)?;
		let title_content = first_line.strip_prefix(&indent).ok_or_else(|| ParseError::BadIndentation {
			src: ctx.named_source(),
			span: ctx.line_span(line_num),
			expected_tabs: depth,
		})?;
		let (meta, labels) = Self::parse_title_line(title_content, line_num, ctx)?;

		let mut comments = Vec::new();
		let mut children = Vec::new();
		let mut blocker_lines = Vec::new();
		let mut current_comment_lines: Vec<String> = Vec::new();
		let mut current_comment_meta: Option<(Option<u64>, bool)> = None; // (id, owned)
		let mut in_body = true;
		let mut in_blockers = false;

		// Body is first comment (no marker)
		let mut body_lines: Vec<String> = Vec::new();

		while let Some(&line) = lines.peek() {
			// Check if this line belongs to us (has our indent level or deeper)
			if !line.is_empty() && !line.starts_with(&indent) {
				break; // Less indented = parent's content
			}

			let line = lines.next().unwrap();

			// Empty line handling
			if line.is_empty() {
				if in_blockers {
					// Empty lines in blockers are ignored by classify_line
				} else if current_comment_meta.is_some() {
					current_comment_lines.push(String::new());
				} else if in_body {
					body_lines.push(String::new());
				}
				continue;
			}

			// Strip our indent level to get content
			let content = line.strip_prefix(&child_indent).unwrap_or(line);

			// Check for blockers marker
			if is_blockers_marker(content) {
				// Flush current comment/body
				if in_body {
					in_body = false;
					if !body_lines.is_empty() {
						let body = body_lines.join("\n").trim().to_string();
						comments.push(Comment { id: None, body, owned: meta.owned });
					}
				} else if let Some((id, owned)) = current_comment_meta.take() {
					let body = current_comment_lines.join("\n").trim().to_string();
					comments.push(Comment { id, body, owned });
					current_comment_lines.clear();
				}
				in_blockers = true;
				tracing::debug!("[parse] entering blockers section");
				continue;
			}

			// If in blockers section, parse as blocker lines
			// But stop at sub-issue lines (they end the blockers section)
			if in_blockers {
				// Check if this is a sub-issue line - if so, exit blockers mode and process it below
				if content.starts_with("- [") && Self::parse_child_title_line(content).is_some() {
					in_blockers = false;
					tracing::debug!("[parse] exiting blockers section due to sub-issue: {:?}", content);
					// Fall through to sub-issue processing below
				} else {
					if let Some(line) = crate::blocker::classify_line(content) {
						tracing::debug!("[parse] blocker line: {:?} -> {:?}", content, line);
						blocker_lines.push(line);
					} else {
						tracing::debug!("[parse] blocker line SKIPPED (classify_line returned None): {:?}", content);
					}
					continue;
				}
			}

			// Check for comment marker
			if content.starts_with("<!--") && content.contains("-->") {
				// Flush previous
				if in_body {
					in_body = false;
					let body = body_lines.join("\n").trim().to_string();
					comments.push(Comment { id: None, body, owned: meta.owned });
				} else if let Some((id, owned)) = current_comment_meta.take() {
					let body = current_comment_lines.join("\n").trim().to_string();
					comments.push(Comment { id, body, owned });
					current_comment_lines.clear();
				}

				let inner = content.strip_prefix("<!--").and_then(|s| s.split("-->").next()).unwrap_or("").trim();

				if inner == "new comment" {
					current_comment_meta = Some((None, true));
				} else if inner == "omitted" {
					continue;
				} else if inner.contains("#issuecomment-") {
					let (is_immutable, url) = if let Some(rest) = inner.strip_prefix("immutable ") {
						(true, rest.trim())
					} else {
						(false, inner)
					};
					let id = url.split("#issuecomment-").nth(1).and_then(|s| s.parse().ok());
					current_comment_meta = Some((id, !is_immutable));
				}
				continue;
			}

			// Check for sub-issue line: `- [x] Title <!--sub url-->` or `- [ ] Title` (new)
			if content.starts_with("- [")
				&& let Some(child_meta) = Self::parse_child_title_line(content)
			{
				// Flush current
				if in_body {
					in_body = false;
					let body = body_lines.join("\n").trim().to_string();
					comments.push(Comment { id: None, body, owned: meta.owned });
				} else if let Some((id, owned)) = current_comment_meta.take() {
					let body = current_comment_lines.join("\n").trim().to_string();
					comments.push(Comment { id, body, owned });
					current_comment_lines.clear();
				}

				// Parse child's body content (lines at depth + 2 indentation)
				let child_body_indent = "\t".repeat(depth + 2);
				let mut child_body_lines: Vec<String> = Vec::new();

				while let Some(&next_line) = lines.peek() {
					// Child body lines are at depth + 2 (one more than the child title's depth + 1)
					if next_line.is_empty() {
						// Preserve empty lines within child content
						let _ = lines.next();
						child_body_lines.push(String::new());
					} else if next_line.starts_with(&child_body_indent) {
						let _ = lines.next();
						// Strip the child body indent to get actual content
						let body_content = next_line.strip_prefix(&child_body_indent).unwrap_or("");
						child_body_lines.push(body_content.to_string());
					} else {
						// Not a child body line - break
						break;
					}
				}

				// Trim trailing empty lines
				while child_body_lines.last().is_some_and(|l| l.is_empty()) {
					child_body_lines.pop();
				}

				let child_body = child_body_lines.join("\n").trim().to_string();
				let child_comments = if child_body.is_empty() {
					vec![]
				} else {
					vec![Comment {
						id: None,
						body: child_body,
						owned: child_meta.owned,
					}]
				};

				children.push(Issue {
					meta: child_meta,
					labels: vec![],
					comments: child_comments,
					children: vec![],
					blockers: crate::blocker::BlockerSequence::default(),
				});
				continue;
			}

			// Regular content line
			let content_line = content.strip_prefix('\t').unwrap_or(content); // Extra indent for immutable
			if in_body {
				body_lines.push(content_line.to_string());
			} else if current_comment_meta.is_some() {
				current_comment_lines.push(content_line.to_string());
			}
		}

		// Flush final
		if in_body {
			let body = body_lines.join("\n").trim().to_string();
			comments.push(Comment { id: None, body, owned: meta.owned });
		} else if let Some((id, owned)) = current_comment_meta.take() {
			let body = current_comment_lines.join("\n").trim().to_string();
			comments.push(Comment { id, body, owned });
		}

		Ok(Issue {
			meta,
			labels,
			comments,
			children,
			blockers: crate::blocker::BlockerSequence::from_lines(blocker_lines),
		})
	}

	/// Parse title line: `- [ ] [label1, label2] Title <!--url-->` or `- [ ] Title <!--immutable url-->`
	/// Also supports `- [-]` for not-planned and `- [123]` for duplicates.
	/// Returns (IssueMeta, labels)
	fn parse_title_line(line: &str, line_num: usize, ctx: &ParseContext) -> Result<(IssueMeta, Vec<String>), ParseError> {
		// Parse checkbox: `- [CONTENT] `
		let (close_state, rest) = Self::parse_checkbox_prefix(line).ok_or_else(|| ParseError::InvalidTitle {
			src: ctx.named_source(),
			span: ctx.line_span(line_num),
			detail: format!("got: {:?}", line),
		})?;

		// Check for labels: [label1, label2] at the start
		let (labels, rest) = if rest.starts_with('[') {
			if let Some(bracket_end) = rest.find("] ") {
				let labels_str = &rest[1..bracket_end];
				let labels: Vec<String> = labels_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
				(labels, &rest[bracket_end + 2..])
			} else {
				(vec![], rest)
			}
		} else {
			(vec![], rest)
		};

		let marker_start = rest.find("<!--").ok_or_else(|| ParseError::MissingUrlMarker {
			src: ctx.named_source(),
			span: ctx.line_span(line_num),
		})?;
		let marker_end = rest.find("-->").ok_or_else(|| ParseError::MalformedUrlMarker {
			src: ctx.named_source(),
			span: ctx.line_span(line_num),
		})?;
		if marker_end <= marker_start {
			return Err(ParseError::MalformedUrlMarker {
				src: ctx.named_source(),
				span: ctx.line_span(line_num),
			});
		}

		let title = rest[..marker_start].trim().to_string();
		let inner = rest[marker_start + 4..marker_end].trim();

		let (owned, url) = if let Some(url) = inner.strip_prefix("immutable ") {
			(false, Some(url.trim().to_string()))
		} else {
			(true, Some(inner.to_string()))
		};

		Ok((IssueMeta { title, url, close_state, owned }, labels))
	}

	/// Parse checkbox prefix: `- [CONTENT] ` and return (CloseState, rest of line)
	fn parse_checkbox_prefix(line: &str) -> Option<(CloseState, &str)> {
		// Match `- [` prefix
		let rest = line.strip_prefix("- [")?;

		// Find closing `] `
		let bracket_end = rest.find("] ")?;
		let checkbox_content = &rest[..bracket_end];
		let rest = &rest[bracket_end + 2..];

		let close_state = CloseState::from_checkbox(checkbox_content)?;
		Some((close_state, rest))
	}

	/// Parse child/sub-issue title line: `- [x] Title <!--sub url-->` or `- [ ] Title` (new)
	/// Also supports `- [-]` for not-planned and `- [123]` for duplicates.
	fn parse_child_title_line(line: &str) -> Option<IssueMeta> {
		let (close_state, rest) = Self::parse_checkbox_prefix(line)?;

		// Check for sub marker
		if let Some(marker_start) = rest.find("<!--sub ") {
			let marker_end = rest.find("-->")?;
			let title = rest[..marker_start].trim().to_string();
			let url = rest[marker_start + 8..marker_end].trim().to_string();
			Some(IssueMeta {
				title,
				url: Some(url),
				close_state,
				owned: true,
			})
		} else if !rest.contains("<!--") {
			let title = rest.trim().to_string();
			if !title.is_empty() {
				Some(IssueMeta {
					title,
					url: None,
					close_state,
					owned: true,
				})
			} else {
				None
			}
		} else {
			None
		}
	}

	/// Serialize the issue back to markdown
	pub fn serialize(&self) -> String {
		self.serialize_at_depth(0)
	}

	/// Serialize at given nesting depth
	fn serialize_at_depth(&self, depth: usize) -> String {
		let indent = "\t".repeat(depth);
		let content_indent = "\t".repeat(depth + 1);
		let mut out = String::new();

		// Title line: `- [x] [label1, label2] Title <!-- url -->` or `- [ ] Title <!-- url -->` if no labels
		// Also supports `- [-]` for not-planned and `- [123]` for duplicates.
		let checked = self.meta.close_state.to_checkbox();
		let url_part = self.meta.url.as_deref().unwrap_or("");
		let labels_part = if self.labels.is_empty() { String::new() } else { format!("[{}] ", self.labels.join(", ")) };
		if self.meta.owned {
			out.push_str(&format!("{indent}- [{checked}] {labels_part}{} <!-- {url_part} -->\n", self.meta.title));
		} else {
			out.push_str(&format!("{indent}- [{checked}] {labels_part}{} <!--immutable {url_part} -->\n", self.meta.title));
		}

		// Body (first comment, no marker)
		if let Some(body_comment) = self.comments.first() {
			let comment_indent = if body_comment.owned { &content_indent } else { &format!("{content_indent}\t") };
			if !body_comment.body.is_empty() {
				for line in body_comment.body.lines() {
					out.push_str(&format!("{comment_indent}{line}\n"));
				}
			}
		}

		// Comments (all other comments, part of the description)
		for comment in self.comments.iter().skip(1) {
			let comment_indent = if comment.owned { &content_indent } else { &format!("{content_indent}\t") };

			// Add empty line (with indent for LSP) before comment marker if previous line has content
			if out.lines().last().is_some_and(|l| !l.trim().is_empty()) {
				out.push_str(&format!("{content_indent}\n"));
			}

			// Comment with marker
			if let Some(id) = comment.id {
				let url = self.meta.url.as_deref().unwrap_or("");
				if comment.owned {
					out.push_str(&format!("{content_indent}<!-- {url}#issuecomment-{id} -->\n"));
				} else {
					out.push_str(&format!("{content_indent}<!--immutable {url}#issuecomment-{id} -->\n"));
				}
			} else {
				out.push_str(&format!("{content_indent}<!-- new comment -->\n"));
			}
			if !comment.body.is_empty() {
				for line in comment.body.lines() {
					out.push_str(&format!("{comment_indent}{line}\n"));
				}
			}
		}

		// Blockers (separate section at bottom, before sub-issues)
		if !self.blockers.is_empty() {
			// Add empty line (with indent for LSP) before blockers if previous line has content
			if out.lines().last().is_some_and(|l| !l.trim().is_empty()) {
				out.push_str(&format!("{content_indent}\n"));
			}
			out.push_str(&format!("{content_indent}# Blockers\n"));
			for line in self.blockers.lines() {
				out.push_str(&format!("{content_indent}{}\n", line.to_raw()));
			}
		}

		// Children (sub-issues) at the very end
		// Closed sub-issues show `<!-- omitted -->` instead of body content
		for child in &self.children {
			let child_checked = child.meta.close_state.to_checkbox();
			let child_content_indent = "\t".repeat(depth + 2);

			// Add empty line (with indent for LSP) before each sub-issue if previous line has content
			if out.lines().last().is_some_and(|l| !l.trim().is_empty()) {
				out.push_str(&format!("{content_indent}\n"));
			}

			// Output child title line
			if let Some(url) = &child.meta.url {
				out.push_str(&format!("{content_indent}- [{child_checked}] {} <!--sub {url} -->\n", child.meta.title));
			} else {
				out.push_str(&format!("{content_indent}- [{child_checked}] {}\n", child.meta.title));
			}

			// Closed sub-issues: show omitted marker instead of content
			if child.meta.close_state.is_closed() {
				out.push_str(&format!("{child_content_indent}<!-- omitted -->\n"));
				continue;
			}

			// Output child body (first comment is body)
			if let Some(body_comment) = child.comments.first()
				&& !body_comment.body.is_empty()
			{
				for line in body_comment.body.lines() {
					out.push_str(&format!("{child_content_indent}{line}\n"));
				}
			}
		}

		out
	}

	/// Collect all required GitHub actions, organized by nesting level.
	/// Returns a Vec where index 0 = actions for root issue, index 1 = actions for children, etc.
	/// Each level's actions can be executed in parallel, but levels must be sequential.
	pub fn collect_actions(&self, original_sub_issues: &[OriginalSubIssue]) -> Vec<Vec<IssueAction>> {
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

		self.collect_actions_recursive(&[], original_sub_issues, &mut levels);
		levels
	}

	/// Recursively collect actions from this issue and its children
	fn collect_actions_recursive(&self, current_path: &[usize], original_sub_issues: &[OriginalSubIssue], levels: &mut Vec<Vec<IssueAction>>) {
		let depth = current_path.len();

		// Ensure we have a vec for this level
		while levels.len() <= depth {
			levels.push(Vec::new());
		}

		// Get parent issue number from URL
		let parent_number = self.meta.url.as_ref().and_then(|url| github::extract_issue_number_from_url(url));

		// Check each child for required actions
		for (i, child) in self.children.iter().enumerate() {
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
			child.collect_actions_recursive(&child_path, original_sub_issues, levels);
		}
	}

	/// Get a mutable reference to a child issue by path
	pub fn get_child_mut(&mut self, path: &[usize]) -> Option<&mut Issue> {
		if path.is_empty() {
			return Some(self);
		}
		let mut current = self;
		for &idx in path.iter().take(path.len() - 1) {
			current = current.children.get_mut(idx)?;
		}
		current.children.get_mut(*path.last()?)
	}

	//==========================================================================
	// Construction from GitHub API data
	//==========================================================================

	/// Construct an Issue directly from GitHub API data.
	/// This is the canonical way to create an Issue from remote state.
	///
	/// `current_user` is used to determine ownership (owned vs immutable).
	pub fn from_github(issue: &GitHubIssue, comments: &[GitHubComment], sub_issues: &[GitHubIssue], owner: &str, repo: &str, current_user: &str) -> Self {
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

		// Build children from sub-issues
		let children: Vec<Issue> = sub_issues
			.iter()
			.map(|sub| {
				let sub_url = format!("https://github.com/{owner}/{repo}/issues/{}", sub.number);
				let sub_close_state = if sub.state == "closed" { CloseState::Closed } else { CloseState::Open };
				let sub_owned = sub.user.login == current_user;

				let sub_body = sub.body.as_deref().unwrap_or("").to_string();
				let sub_comments = if sub_body.is_empty() {
					vec![]
				} else {
					vec![Comment {
						id: None,
						body: sub_body,
						owned: sub_owned,
					}]
				};

				Issue {
					meta: IssueMeta {
						title: sub.title.clone(),
						url: Some(sub_url),
						close_state: sub_close_state,
						owned: sub_owned,
					},
					labels: sub.labels.iter().map(|l| l.name.clone()).collect(),
					comments: sub_comments,
					children: vec![],                                     // Sub-issues don't have nested children in this context
					blockers: crate::blocker::BlockerSequence::default(), // Sub-issues don't have blockers
				}
			})
			.collect();

		Issue {
			meta,
			labels,
			comments: issue_comments,
			children,
			blockers: crate::blocker::BlockerSequence::default(), // Blockers are local-only, not from GitHub
		}
	}

	/// Reconstruct an Issue from stored metadata (original state at last fetch).
	/// This is used for comparing against current remote state to detect divergence.
	///
	/// Note: This reconstructs a minimal Issue suitable for comparison.
	/// It won't have blockers (local-only) or full sub-issue content.
	pub fn from_meta(meta: &super::meta::IssueMetaEntry, owner: &str, repo: &str) -> Self {
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
			.map(|sub| {
				let sub_close_state = if sub.state == "closed" { CloseState::Closed } else { CloseState::Open };
				Issue {
					meta: IssueMeta {
						title: String::new(), // Not stored in OriginalSubIssue
						url: Some(format!("https://github.com/{owner}/{repo}/issues/{}", sub.number)),
						close_state: sub_close_state,
						owned: true,
					},
					labels: vec![],
					comments: vec![],
					children: vec![],
					blockers: crate::blocker::BlockerSequence::default(),
				}
			})
			.collect();

		Issue {
			meta: issue_meta,
			labels: vec![], // Not stored in meta
			comments,
			children,
			blockers: crate::blocker::BlockerSequence::default(),
		}
	}
}

/// Semantic equality for divergence detection.
/// Compares the fields that matter for sync: close_state, body, comments, sub-issue states.
/// Ignores local-only fields like blockers and ownership.
impl PartialEq for Issue {
	fn eq(&self, other: &Self) -> bool {
		// Compare close state
		if self.meta.close_state != other.meta.close_state {
			return false;
		}

		// Compare body (first comment)
		let self_body = self.comments.first().map(|c| c.body.as_str()).unwrap_or("");
		let other_body = other.comments.first().map(|c| c.body.as_str()).unwrap_or("");
		if self_body != other_body {
			return false;
		}

		// Compare comments (skip first which is body)
		let self_comments: Vec<_> = self.comments.iter().skip(1).collect();
		let other_comments: Vec<_> = other.comments.iter().skip(1).collect();

		if self_comments.len() != other_comments.len() {
			return false;
		}

		for (sc, oc) in self_comments.iter().zip(other_comments.iter()) {
			if sc.id != oc.id || sc.body != oc.body {
				return false;
			}
		}

		// Compare sub-issue states
		if self.children.len() != other.children.len() {
			return false;
		}

		for (sc, oc) in self.children.iter().zip(other.children.iter()) {
			// Compare by URL (issue number) and state
			if sc.meta.url != oc.meta.url || sc.meta.close_state != oc.meta.close_state {
				return false;
			}
		}

		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_close_state_from_checkbox() {
		assert_eq!(CloseState::from_checkbox(" "), Some(CloseState::Open));
		assert_eq!(CloseState::from_checkbox(""), Some(CloseState::Open));
		assert_eq!(CloseState::from_checkbox("x"), Some(CloseState::Closed));
		assert_eq!(CloseState::from_checkbox("X"), Some(CloseState::Closed));
		assert_eq!(CloseState::from_checkbox("-"), Some(CloseState::NotPlanned));
		assert_eq!(CloseState::from_checkbox("123"), Some(CloseState::Duplicate(123)));
		assert_eq!(CloseState::from_checkbox("42"), Some(CloseState::Duplicate(42)));
		assert_eq!(CloseState::from_checkbox("invalid"), None);
	}

	#[test]
	fn test_close_state_to_checkbox() {
		assert_eq!(CloseState::Open.to_checkbox(), " ");
		assert_eq!(CloseState::Closed.to_checkbox(), "x");
		assert_eq!(CloseState::NotPlanned.to_checkbox(), "-");
		assert_eq!(CloseState::Duplicate(123).to_checkbox(), "123");
	}

	#[test]
	fn test_close_state_is_closed() {
		assert!(!CloseState::Open.is_closed());
		assert!(CloseState::Closed.is_closed());
		assert!(CloseState::NotPlanned.is_closed());
		assert!(CloseState::Duplicate(123).is_closed());
	}

	#[test]
	fn test_close_state_should_remove() {
		assert!(!CloseState::Open.should_remove());
		assert!(!CloseState::Closed.should_remove());
		assert!(!CloseState::NotPlanned.should_remove());
		assert!(CloseState::Duplicate(123).should_remove());
	}

	#[test]
	fn test_close_state_to_github_state() {
		assert_eq!(CloseState::Open.to_github_state(), "open");
		assert_eq!(CloseState::Closed.to_github_state(), "closed");
		assert_eq!(CloseState::NotPlanned.to_github_state(), "closed");
		assert_eq!(CloseState::Duplicate(123).to_github_state(), "closed");
	}

	#[test]
	fn test_parse_checkbox_prefix() {
		// Standard cases
		assert_eq!(Issue::parse_checkbox_prefix("- [ ] rest"), Some((CloseState::Open, "rest")));
		assert_eq!(Issue::parse_checkbox_prefix("- [x] rest"), Some((CloseState::Closed, "rest")));
		assert_eq!(Issue::parse_checkbox_prefix("- [X] rest"), Some((CloseState::Closed, "rest")));

		// New close types
		assert_eq!(Issue::parse_checkbox_prefix("- [-] rest"), Some((CloseState::NotPlanned, "rest")));
		assert_eq!(Issue::parse_checkbox_prefix("- [123] rest"), Some((CloseState::Duplicate(123), "rest")));
		assert_eq!(Issue::parse_checkbox_prefix("- [42] Title here"), Some((CloseState::Duplicate(42), "Title here")));

		// Invalid cases
		assert_eq!(Issue::parse_checkbox_prefix("no checkbox"), None);
		assert_eq!(Issue::parse_checkbox_prefix("- [invalid] rest"), None);
	}

	#[test]
	fn test_parse_and_serialize_not_planned() {
		let content = "- [-] Not planned issue <!-- https://github.com/owner/repo/issues/123 -->\n\tBody text\n";
		let ctx = crate::error::ParseContext::new(content.to_string(), "test".to_string());
		let issue = Issue::parse(content, &ctx).unwrap();

		assert_eq!(issue.meta.close_state, CloseState::NotPlanned);
		assert_eq!(issue.meta.title, "Not planned issue");

		// Verify serialization preserves the state
		let serialized = issue.serialize();
		assert!(serialized.starts_with("- [-] Not planned issue"));
	}

	#[test]
	fn test_parse_and_serialize_duplicate() {
		let content = "- [456] Duplicate issue <!-- https://github.com/owner/repo/issues/123 -->\n\tBody text\n";
		let ctx = crate::error::ParseContext::new(content.to_string(), "test".to_string());
		let issue = Issue::parse(content, &ctx).unwrap();

		assert_eq!(issue.meta.close_state, CloseState::Duplicate(456));
		assert_eq!(issue.meta.title, "Duplicate issue");

		// Verify serialization preserves the state
		let serialized = issue.serialize();
		assert!(serialized.starts_with("- [456] Duplicate issue"));
	}

	#[test]
	fn test_parse_sub_issue_close_types() {
		let content = r#"- [ ] Parent issue <!-- https://github.com/owner/repo/issues/1 -->
	Body

	- [x] Closed sub <!--sub https://github.com/owner/repo/issues/2 -->
		<!-- omitted -->

	- [-] Not planned sub <!--sub https://github.com/owner/repo/issues/3 -->
		<!-- omitted -->

	- [42] Duplicate sub <!--sub https://github.com/owner/repo/issues/4 -->
		<!-- omitted -->
"#;
		let ctx = crate::error::ParseContext::new(content.to_string(), "test".to_string());
		let issue = Issue::parse(content, &ctx).unwrap();

		assert_eq!(issue.children.len(), 3);
		assert_eq!(issue.children[0].meta.close_state, CloseState::Closed);
		assert_eq!(issue.children[1].meta.close_state, CloseState::NotPlanned);
		assert_eq!(issue.children[2].meta.close_state, CloseState::Duplicate(42));
	}

	#[test]
	fn test_from_github() {
		use crate::github::{GitHubLabel, GitHubUser};

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
		use crate::github::GitHubUser;

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
		use super::super::meta::IssueMetaEntry;
		use crate::github::OriginalComment;

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
