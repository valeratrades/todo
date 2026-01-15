//! Core issue data structures and parsing/serialization.
//!
//! This module contains the pure Issue type with parsing and serialization.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use url::Url;

/// A GitHub issue identifier. Wraps a URL and derives all properties on demand.
/// Format: `https://github.com/{owner}/{repo}/issues/{number}`
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct IssueLink(Url);

impl IssueLink {
	/// Create from a URL. Returns None if not a valid GitHub issue URL.
	pub fn new(url: Url) -> Option<Self> {
		// Validate it's a GitHub issue URL
		if url.host_str() != Some("github.com") {
			return None;
		}
		let segments: Vec<_> = url.path_segments()?.collect();
		// Must be: owner/repo/issues/number
		if segments.len() < 4 || segments[2] != "issues" {
			return None;
		}
		// Number must be valid
		segments[3].parse::<u64>().ok()?;
		Some(Self(url))
	}

	/// Parse from a URL string.
	pub fn parse(url: &str) -> Option<Self> {
		let url = Url::parse(url).ok()?;
		Self::new(url)
	}

	/// Get the underlying URL.
	pub fn url(&self) -> &Url {
		&self.0
	}

	/// Get the owner (first path segment).
	pub fn owner(&self) -> &str {
		self.0.path_segments().unwrap().next().unwrap()
	}

	/// Get the repo (second path segment).
	pub fn repo(&self) -> &str {
		self.0.path_segments().unwrap().nth(1).unwrap()
	}

	/// Get the issue number (fourth path segment).
	pub fn number(&self) -> u64 {
		self.0.path_segments().unwrap().nth(3).unwrap().parse().unwrap()
	}

	/// Build URL string.
	pub fn as_str(&self) -> &str {
		self.0.as_str()
	}
}

impl fmt::Display for IssueLink {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl From<IssueLink> for Url {
	fn from(link: IssueLink) -> Url {
		link.0
	}
}

impl AsRef<Url> for IssueLink {
	fn as_ref(&self) -> &Url {
		&self.0
	}
}

/// Identity of an issue - either linked to GitHub or pending creation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IssueIdentity {
	/// Issue exists on GitHub with this link
	Linked(IssueLink),
	/// Issue is pending creation on GitHub (will be created in post-sync)
	Pending,
}

impl IssueIdentity {
	/// Get the link if this issue is linked to GitHub.
	pub fn link(&self) -> Option<&IssueLink> {
		match self {
			Self::Linked(link) => Some(link),
			Self::Pending => None,
		}
	}

	/// Check if this issue is linked to GitHub.
	pub fn is_linked(&self) -> bool {
		matches!(self, Self::Linked(_))
	}

	/// Check if this issue is pending creation.
	pub fn is_pending(&self) -> bool {
		matches!(self, Self::Pending)
	}

	/// Get the issue number if linked.
	pub fn number(&self) -> Option<u64> {
		self.link().map(|l| l.number())
	}

	/// Get the URL string if linked.
	pub fn url_str(&self) -> Option<&str> {
		self.link().map(|l| l.as_str())
	}
}

/// Identity of a comment - either linked to GitHub or pending creation.
/// Note: The first comment (issue body) is always `Body`, not `Linked` or `Pending`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommentIdentity {
	/// This is the issue body (first comment), not a separate GitHub comment
	Body,
	/// Comment exists on GitHub with this ID
	Linked(u64),
	/// Comment is pending creation on GitHub (will be created in post-sync)
	Pending,
}

impl CommentIdentity {
	/// Get the comment ID if linked.
	pub fn id(&self) -> Option<u64> {
		match self {
			Self::Linked(id) => Some(*id),
			Self::Body | Self::Pending => None,
		}
	}

	/// Check if this is a GitHub comment (not the issue body).
	pub fn is_comment(&self) -> bool {
		!matches!(self, Self::Body)
	}

	/// Check if this comment is pending creation.
	pub fn is_pending(&self) -> bool {
		matches!(self, Self::Pending)
	}
}

/// An issue with its title - used when we need both identity and display name.
/// This is what we have after fetching an issue from GitHub.
#[derive(Clone, Debug)]
pub struct FetchedIssue {
	pub link: IssueLink,
	pub title: String,
}

impl FetchedIssue {
	pub fn new(link: IssueLink, title: impl Into<String>) -> Self {
		Self { link, title: title.into() }
	}

	/// Create from owner, repo, number, and title (constructs the URL internally).
	pub fn from_parts(owner: &str, repo: &str, number: u64, title: impl Into<String>) -> Option<Self> {
		let url_str = format!("https://github.com/{owner}/{repo}/issues/{number}");
		let link = IssueLink::parse(&url_str)?;
		Some(Self { link, title: title.into() })
	}

	/// Convenience: get the issue number
	pub fn number(&self) -> u64 {
		self.link.number()
	}

	/// Convenience: get owner
	pub fn owner(&self) -> &str {
		self.link.owner()
	}

	/// Convenience: get repo
	pub fn repo(&self) -> &str {
		self.link.repo()
	}
}

use super::{
	blocker::{BlockerSequence, classify_line, join_with_blockers},
	error::{ParseContext, ParseError},
	util::{is_blockers_marker, normalize_issue_indentation},
};

/// Result of parsing a checkbox prefix.
enum CheckboxParseResult<'a> {
	/// Successfully parsed checkbox
	Ok(CloseState, &'a str),
	/// Not a checkbox line (doesn't start with `- [`)
	NotCheckbox,
	/// Has checkbox syntax but invalid content (like `[abc]`)
	InvalidContent(String),
}

/// Result of parsing a child title line.
enum ChildTitleParseResult {
	/// Successfully parsed child/sub-issue
	Ok(IssueMeta),
	/// Not a child title line
	NotChildTitle,
	/// Has checkbox syntax but invalid content (like `[abc]`)
	InvalidCheckbox(String),
}

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

	/// Convert to GitHub API state_reason string (for closed issues)
	pub fn to_github_state_reason(&self) -> Option<&'static str> {
		match self {
			CloseState::Open => None,
			CloseState::Closed => Some("completed"),
			CloseState::NotPlanned => Some("not_planned"),
			CloseState::Duplicate(_) => Some("duplicate"),
		}
	}

	/// Create from GitHub API state and state_reason.
	///
	/// # Panics
	/// Panics if state_reason is "duplicate" - duplicates must be filtered before calling this.
	pub fn from_github(state: &str, state_reason: Option<&str>) -> Self {
		assert!(state_reason != Some("duplicate"), "Duplicate issues must be filtered before calling from_github");

		match (state, state_reason) {
			("open", _) => CloseState::Open,
			("closed", Some("not_planned")) => CloseState::NotPlanned,
			("closed", Some("completed") | None) => CloseState::Closed,
			("closed", Some(unknown)) => {
				tracing::warn!("Unknown state_reason '{unknown}', treating as Closed");
				CloseState::Closed
			}
			(unknown, _) => {
				tracing::warn!("Unknown state '{unknown}', treating as Open");
				CloseState::Open
			}
		}
	}

	/// Returns true if this represents a duplicate (should be filtered from fetch results)
	pub fn is_duplicate_reason(state_reason: Option<&str>) -> bool {
		state_reason == Some("duplicate")
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
	/// Issue identity - linked to GitHub or pending creation
	pub identity: IssueIdentity,
	pub close_state: CloseState,
	/// Whether owned by current user (false = immutable)
	pub owned: bool,
}

/// A comment in the issue conversation (first one is always the issue body)
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
	/// Comment identity - body, linked to GitHub, or pending creation
	pub identity: CommentIdentity,
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
	/// Blockers section.
	pub blockers: BlockerSequence,
	/// Timestamp of last content change (body/comments, not children).
	/// Used for sync conflict resolution. None for local-only issues that haven't been synced.
	pub last_contents_change: Option<Timestamp>,
}

impl Issue {
	/// Get the full issue body including blockers section.
	/// This is what should be synced to GitHub as the issue body.
	pub fn body(&self) -> String {
		let base_body = self.comments.first().map(|c| c.body.as_str()).unwrap_or("");
		join_with_blockers(base_body, &self.blockers)
	}

	/// Parse markdown content into an Issue.
	/// Returns an error with a detailed message if any part of the file cannot be understood.
	pub fn parse(content: &str, path: &std::path::Path) -> Result<Self, ParseError> {
		let ctx = ParseContext::new(content.to_string(), path.display().to_string());

		//TODO!!!!!!: make parse aware of Extension
		let extension = path
			.extension()
			.and_then(|e| e.to_str())
			.map(|ext| match ext {
				"md" => crate::Extension::Md,
				"typ" => crate::Extension::Typ,
				_ => crate::Extension::Md,
			})
			.unwrap_or(crate::Extension::Md);

		let normalized = normalize_issue_indentation(content);
		let mut lines = normalized.lines().peekable();

		Self::parse_at_depth(&mut lines, 0, 1, &ctx)
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
		let mut current_comment_meta: Option<(CommentIdentity, bool)> = None; // (identity, owned)
		let mut in_body = true;
		let mut in_blockers = false;
		let mut current_line = line_num;

		// Body is first comment (no marker)
		let mut body_lines: Vec<String> = Vec::new();

		while let Some(&line) = lines.peek() {
			// Check if this line belongs to us (has our indent level or deeper)
			if !line.is_empty() && !line.starts_with(&indent) {
				break; // Less indented = parent's content
			}

			let line = lines.next().unwrap();
			current_line += 1;

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
						comments.push(Comment {
							identity: CommentIdentity::Body,
							body,
							owned: meta.owned,
						});
					}
				} else if let Some((identity, owned)) = current_comment_meta.take() {
					let body = current_comment_lines.join("\n").trim().to_string();
					comments.push(Comment { identity, body, owned });
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
				if content.starts_with("- [") {
					match Self::parse_child_title_line_detailed(content) {
						ChildTitleParseResult::Ok(_) => {
							in_blockers = false;
							tracing::debug!("[parse] exiting blockers section due to sub-issue: {content:?}");
							// Fall through to sub-issue processing below
						}
						ChildTitleParseResult::InvalidCheckbox(invalid_content) => {
							return Err(ParseError::InvalidCheckbox {
								src: ctx.named_source(),
								span: ctx.line_span(current_line),
								content: invalid_content,
							});
						}
						ChildTitleParseResult::NotChildTitle => {
							// Not a sub-issue, continue parsing as blocker
							if let Some(line) = classify_line(content) {
								tracing::debug!("[parse] blocker line: {content:?} -> {line:?}");
								blocker_lines.push(line);
							} else {
								tracing::debug!("[parse] blocker line SKIPPED (classify_line returned None): {content:?}");
							}
							continue;
						}
					}
				} else {
					if let Some(line) = classify_line(content) {
						tracing::debug!("[parse] blocker line: {content:?} -> {line:?}");
						blocker_lines.push(line);
					} else {
						tracing::debug!("[parse] blocker line SKIPPED (classify_line returned None): {content:?}");
					}
					continue;
				}
			}

			// Check for comment marker (including !c shorthand)
			let is_new_comment_shorthand = content.trim().eq_ignore_ascii_case("!c");
			if is_new_comment_shorthand || (content.starts_with("<!--") && content.contains("-->")) {
				// Flush previous
				if in_body {
					in_body = false;
					let body = body_lines.join("\n").trim().to_string();
					comments.push(Comment {
						identity: CommentIdentity::Body,
						body,
						owned: meta.owned,
					});
				} else if let Some((identity, owned)) = current_comment_meta.take() {
					let body = current_comment_lines.join("\n").trim().to_string();
					comments.push(Comment { identity, body, owned });
					current_comment_lines.clear();
				}

				// Handle !c shorthand
				if is_new_comment_shorthand {
					current_comment_meta = Some((CommentIdentity::Pending, true));
					continue;
				}

				let inner = content.strip_prefix("<!--").and_then(|s| s.split("-->").next()).unwrap_or("").trim();

				if inner == "new comment" {
					current_comment_meta = Some((CommentIdentity::Pending, true));
				} else if inner.starts_with("omitted") && inner.contains("{{{") {
					// vim fold start marker - skip it
					continue;
				} else if inner.starts_with(",}}}") {
					// vim fold end marker - skip it
					continue;
				} else if inner.contains("#issuecomment-") {
					let (is_immutable, url) = if let Some(rest) = inner.strip_prefix("immutable ") {
						(true, rest.trim())
					} else {
						(false, inner)
					};
					let identity = url
						.split("#issuecomment-")
						.nth(1)
						.and_then(|s| s.parse().ok())
						.map(CommentIdentity::Linked)
						.unwrap_or(CommentIdentity::Pending);
					current_comment_meta = Some((identity, !is_immutable));
				}
				continue;
			}

			// Check for sub-issue line: `- [x] Title <!--sub url-->` or `- [ ] Title` (new)
			if content.starts_with("- [") {
				let is_child_title = match Self::parse_child_title_line_detailed(content) {
					ChildTitleParseResult::Ok(_) => true,
					ChildTitleParseResult::InvalidCheckbox(invalid_content) => {
						return Err(ParseError::InvalidCheckbox {
							src: ctx.named_source(),
							span: ctx.line_span(current_line),
							content: invalid_content,
						});
					}
					ChildTitleParseResult::NotChildTitle => false,
				};

				if !is_child_title {
					// Not a sub-issue line, treat as regular content
					let content_line = content.strip_prefix('\t').unwrap_or(content);
					if in_body {
						body_lines.push(content_line.to_string());
					} else if current_comment_meta.is_some() {
						current_comment_lines.push(content_line.to_string());
					}
					continue;
				}

				// Flush current
				if in_body {
					in_body = false;
					let body = body_lines.join("\n").trim().to_string();
					comments.push(Comment {
						identity: CommentIdentity::Body,
						body,
						owned: meta.owned,
					});
				} else if let Some((identity, owned)) = current_comment_meta.take() {
					let body = current_comment_lines.join("\n").trim().to_string();
					comments.push(Comment { identity, body, owned });
					current_comment_lines.clear();
				}

				// Collect all lines belonging to this child (at depth+1 and deeper)
				let child_content_indent = "\t".repeat(depth + 2);
				let mut child_lines: Vec<String> = vec![content.to_string()]; // Start with the title line (without parent indent)

				while let Some(&next_line) = lines.peek() {
					if next_line.is_empty() {
						// Preserve empty lines
						let _ = lines.next();
						child_lines.push(String::new());
					} else if next_line.starts_with(&child_content_indent) {
						let _ = lines.next();
						// Strip one level of indent (the child's content indent) to normalize for recursive parsing
						let stripped = next_line.strip_prefix(&child_indent).unwrap_or(next_line);
						child_lines.push(stripped.to_string());
					} else {
						// Not a child content line - break
						break;
					}
				}

				// Trim trailing empty lines
				while child_lines.last().is_some_and(|l| l.is_empty()) {
					child_lines.pop();
				}

				// Recursively parse the child
				let child_content = child_lines.join("\n");
				let mut child_lines_iter = child_content.lines().peekable();
				let child = Self::parse_at_depth(&mut child_lines_iter, 0, current_line, ctx)?;
				children.push(child);
				continue;
			}

			// Regular content line (doesn't start with "- [")
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
			comments.push(Comment {
				identity: CommentIdentity::Body,
				body,
				owned: meta.owned,
			});
		} else if let Some((identity, owned)) = current_comment_meta.take() {
			let body = current_comment_lines.join("\n").trim().to_string();
			comments.push(Comment { identity, body, owned });
		}

		Ok(Issue {
			meta,
			labels,
			comments,
			children,
			blockers: BlockerSequence::from_lines(blocker_lines),
			last_contents_change: None, // Set from GitHub when syncing
		})
	}

	/// Parse title line: `- [ ] [label1, label2] Title <!--url-->` or `- [ ] Title <!--immutable url-->`
	/// Also supports `- [-]` for not-planned and `- [123]` for duplicates.
	/// Returns (IssueMeta, labels)
	fn parse_title_line(line: &str, line_num: usize, ctx: &ParseContext) -> Result<(IssueMeta, Vec<String>), ParseError> {
		// Parse checkbox: `- [CONTENT] `
		let (close_state, rest) = match Self::parse_checkbox_prefix_detailed(line) {
			CheckboxParseResult::Ok(state, rest) => (state, rest),
			CheckboxParseResult::NotCheckbox => {
				return Err(ParseError::InvalidTitle {
					src: ctx.named_source(),
					span: ctx.line_span(line_num),
					detail: format!("got: {line:?}"),
				});
			}
			CheckboxParseResult::InvalidContent(content) => {
				return Err(ParseError::InvalidCheckbox {
					src: ctx.named_source(),
					span: ctx.line_span(line_num),
					content,
				});
			}
		};

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

		let (owned, identity) = if let Some(url) = inner.strip_prefix("immutable ") {
			let url = url.trim();
			let identity = IssueLink::parse(url).map(IssueIdentity::Linked).unwrap_or(IssueIdentity::Pending);
			(false, identity)
		} else if inner.is_empty() {
			// Empty marker `<!--  -->` means pending
			(true, IssueIdentity::Pending)
		} else {
			let identity = IssueLink::parse(inner).map(IssueIdentity::Linked).unwrap_or(IssueIdentity::Pending);
			(true, identity)
		};

		Ok((
			IssueMeta {
				title,
				identity,
				close_state,
				owned,
			},
			labels,
		))
	}

	/// Parse checkbox prefix: `- [CONTENT] ` and return result.
	fn parse_checkbox_prefix_detailed(line: &str) -> CheckboxParseResult<'_> {
		// Match `- [` prefix
		let Some(rest) = line.strip_prefix("- [") else {
			return CheckboxParseResult::NotCheckbox;
		};

		// Find closing `] `
		let Some(bracket_end) = rest.find("] ") else {
			return CheckboxParseResult::NotCheckbox;
		};

		let checkbox_content = &rest[..bracket_end];
		let rest = &rest[bracket_end + 2..];

		match CloseState::from_checkbox(checkbox_content) {
			Some(close_state) => CheckboxParseResult::Ok(close_state, rest),
			None => CheckboxParseResult::InvalidContent(checkbox_content.to_string()),
		}
	}

	/// Parse child/sub-issue title line with detailed result.
	fn parse_child_title_line_detailed(line: &str) -> ChildTitleParseResult {
		let (close_state, rest) = match Self::parse_checkbox_prefix_detailed(line) {
			CheckboxParseResult::Ok(state, rest) => (state, rest),
			CheckboxParseResult::NotCheckbox => return ChildTitleParseResult::NotChildTitle,
			CheckboxParseResult::InvalidContent(content) => return ChildTitleParseResult::InvalidCheckbox(content),
		};

		// Check for sub marker
		if let Some(marker_start) = rest.find("<!--sub ") {
			let Some(marker_end) = rest.find("-->") else {
				return ChildTitleParseResult::NotChildTitle;
			};
			let title = rest[..marker_start].trim().to_string();
			let url = rest[marker_start + 8..marker_end].trim().to_string();
			let identity = IssueLink::parse(&url).map(IssueIdentity::Linked).unwrap_or(IssueIdentity::Pending);
			ChildTitleParseResult::Ok(IssueMeta {
				title,
				identity,
				close_state,
				owned: true,
			})
		} else if !rest.contains("<!--") {
			let title = rest.trim().to_string();
			if !title.is_empty() {
				ChildTitleParseResult::Ok(IssueMeta {
					title,
					identity: IssueIdentity::Pending,
					close_state,
					owned: true,
				})
			} else {
				ChildTitleParseResult::NotChildTitle
			}
		} else {
			ChildTitleParseResult::NotChildTitle
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
		let url_part = self.meta.identity.url_str().unwrap_or("");
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
			match &comment.identity {
				CommentIdentity::Body => {
					// Body shouldn't appear here (it's always first), but handle gracefully
					out.push_str(&format!("{content_indent}<!-- new comment -->\n"));
				}
				CommentIdentity::Linked(id) => {
					let url = self.meta.identity.url_str().unwrap_or("");
					if comment.owned {
						out.push_str(&format!("{content_indent}<!-- {url}#issuecomment-{id} -->\n"));
					} else {
						out.push_str(&format!("{content_indent}<!--immutable {url}#issuecomment-{id} -->\n"));
					}
				}
				CommentIdentity::Pending => {
					out.push_str(&format!("{content_indent}<!-- new comment -->\n"));
				}
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
		// Closed sub-issues wrap body content in vim fold markers
		for child in &self.children {
			let child_checked = child.meta.close_state.to_checkbox();
			let child_content_indent = "\t".repeat(depth + 2);

			// Add empty line (with indent for LSP) before each sub-issue if previous line has content
			if out.lines().last().is_some_and(|l| !l.trim().is_empty()) {
				out.push_str(&format!("{content_indent}\n"));
			}

			// Output child title line
			match &child.meta.identity {
				IssueIdentity::Linked(link) => {
					out.push_str(&format!("{content_indent}- [{child_checked}] {} <!--sub {} -->\n", child.meta.title, link.as_str()));
				}
				IssueIdentity::Pending => {
					out.push_str(&format!("{content_indent}- [{child_checked}] {}\n", child.meta.title));
				}
			}

			// Closed sub-issues: wrap content in vim fold markers
			if child.meta.close_state.is_closed() {
				out.push_str(&format!("{child_content_indent}<!--omitted {{{{{{always-->\n"));
			}

			// Output child body (first comment is body)
			if let Some(body_comment) = child.comments.first()
				&& !body_comment.body.is_empty()
			{
				for line in body_comment.body.lines() {
					out.push_str(&format!("{child_content_indent}{line}\n"));
				}
			}

			// Close vim fold for closed sub-issues
			if child.meta.close_state.is_closed() {
				out.push_str(&format!("{child_content_indent}<!--,}}}}}}-->\n"));
			}
		}

		out
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

	/// Find the position (line, col) of the last blocker item in the serialized content.
	/// Returns None if there are no blockers.
	/// Line numbers are 1-indexed to match editor conventions.
	/// Column points to the first character of the item text (after `- `).
	pub fn find_last_blocker_position(&self) -> Option<(u32, u32)> {
		if self.blockers.is_empty() {
			return None;
		}

		// Serialize and find the last blocker item line
		let serialized = self.serialize();
		let lines: Vec<&str> = serialized.lines().collect();

		// Find where blockers section starts (look for `# Blockers` marker)
		let blockers_start_idx = lines.iter().position(|line| line.trim() == "# Blockers")?;

		// Track the last line that's a blocker item (starts with `- ` but not `- [` which is sub-issue)
		let mut last_item_line_num: Option<u32> = None;
		let mut last_item_col: Option<u32> = None;

		for (offset, line) in lines[blockers_start_idx + 1..].iter().enumerate() {
			let trimmed = line.trim();

			// Check if we've reached sub-issues (they start with `- [`)
			if trimmed.starts_with("- [") {
				break;
			}

			// A blocker item starts with `- ` (but not `- [`)
			if trimmed.starts_with("- ") {
				// Line number is 1-indexed
				let line_num = (blockers_start_idx + 1 + offset + 1) as u32;
				// Column: find where `- ` starts, then add 2 to skip past it
				let dash_pos = line.find("- ").unwrap_or(0);
				let col = (dash_pos + 3) as u32; // +2 for "- ", +1 for 1-indexing
				last_item_line_num = Some(line_num);
				last_item_col = Some(col);
			}
		}

		last_item_line_num.zip(last_item_col)
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
			if sc.identity != oc.identity || sc.body != oc.body {
				return false;
			}
		}

		// Compare sub-issue states
		if self.children.len() != other.children.len() {
			return false;
		}

		for (sc, oc) in self.children.iter().zip(other.children.iter()) {
			// Compare by identity (issue number) and state
			if sc.meta.identity != oc.meta.identity || sc.meta.close_state != oc.meta.close_state {
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
		// Helper to extract (CloseState, rest) from Ok result
		fn extract_ok(result: CheckboxParseResult) -> Option<(CloseState, String)> {
			match result {
				CheckboxParseResult::Ok(state, rest) => Some((state, rest.to_string())),
				_ => None,
			}
		}

		// Standard cases
		assert_eq!(extract_ok(Issue::parse_checkbox_prefix_detailed("- [ ] rest")), Some((CloseState::Open, "rest".to_string())));
		assert_eq!(extract_ok(Issue::parse_checkbox_prefix_detailed("- [x] rest")), Some((CloseState::Closed, "rest".to_string())));
		assert_eq!(extract_ok(Issue::parse_checkbox_prefix_detailed("- [X] rest")), Some((CloseState::Closed, "rest".to_string())));

		// New close types
		assert_eq!(
			extract_ok(Issue::parse_checkbox_prefix_detailed("- [-] rest")),
			Some((CloseState::NotPlanned, "rest".to_string()))
		);
		assert_eq!(
			extract_ok(Issue::parse_checkbox_prefix_detailed("- [123] rest")),
			Some((CloseState::Duplicate(123), "rest".to_string()))
		);
		assert_eq!(
			extract_ok(Issue::parse_checkbox_prefix_detailed("- [42] Title here")),
			Some((CloseState::Duplicate(42), "Title here".to_string()))
		);

		// Not a checkbox line
		assert!(matches!(Issue::parse_checkbox_prefix_detailed("no checkbox"), CheckboxParseResult::NotCheckbox));

		// Invalid checkbox content
		assert!(matches!(
			Issue::parse_checkbox_prefix_detailed("- [invalid] rest"),
			CheckboxParseResult::InvalidContent(s) if s == "invalid"
		));
		assert!(matches!(
			Issue::parse_checkbox_prefix_detailed("- [abc] rest"),
			CheckboxParseResult::InvalidContent(s) if s == "abc"
		));
	}

	#[test]
	fn test_parse_invalid_checkbox_returns_error() {
		use std::path::Path;

		// Invalid checkbox on root issue
		let content = "- [abc] Invalid issue <!-- https://github.com/owner/repo/issues/123 -->\n\tBody\n";
		let result = Issue::parse(content, Path::new("test.md"));
		assert!(matches!(result, Err(ParseError::InvalidCheckbox { content, .. }) if content == "abc"));

		// Invalid checkbox on sub-issue
		let content = "- [ ] Parent <!-- https://github.com/owner/repo/issues/1 -->\n\tBody\n\n\t- [xyz] Bad sub <!--sub https://github.com/owner/repo/issues/2 -->\n";
		let result = Issue::parse(content, Path::new("test.md"));
		assert!(matches!(result, Err(ParseError::InvalidCheckbox { content, .. }) if content == "xyz"));
	}

	#[test]
	fn test_parse_and_serialize_not_planned() {
		use std::path::Path;

		let content = "- [-] Not planned issue <!-- https://github.com/owner/repo/issues/123 -->\n\tBody text\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();

		assert_eq!(issue.meta.close_state, CloseState::NotPlanned);
		assert_eq!(issue.meta.title, "Not planned issue");

		// Verify serialization preserves the state
		let serialized = issue.serialize();
		assert!(serialized.starts_with("- [-] Not planned issue"));
	}

	#[test]
	fn test_parse_and_serialize_duplicate() {
		use std::path::Path;

		let content = "- [456] Duplicate issue <!-- https://github.com/owner/repo/issues/123 -->\n\tBody text\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();

		assert_eq!(issue.meta.close_state, CloseState::Duplicate(456));
		assert_eq!(issue.meta.title, "Duplicate issue");

		// Verify serialization preserves the state
		let serialized = issue.serialize();
		assert!(serialized.starts_with("- [456] Duplicate issue"));
	}

	#[test]
	fn test_parse_sub_issue_close_types() {
		use std::path::Path;

		let content = r#"- [ ] Parent issue <!-- https://github.com/owner/repo/issues/1 -->
	Body

	- [x] Closed sub <!--sub https://github.com/owner/repo/issues/2 -->
		<!--omitted {{{always-->
		closed body
		<!--,}}}-->

	- [-] Not planned sub <!--sub https://github.com/owner/repo/issues/3 -->
		<!--omitted {{{always-->
		not planned body
		<!--,}}}-->

	- [42] Duplicate sub <!--sub https://github.com/owner/repo/issues/4 -->
		<!--omitted {{{always-->
		duplicate body
		<!--,}}}-->
"#;
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();
		insta::assert_snapshot!(issue.serialize(), @"
		- [ ] Parent issue <!-- https://github.com/owner/repo/issues/1 -->
			Body
			
			- [x] Closed sub <!--sub https://github.com/owner/repo/issues/2 -->
				<!--omitted {{{always-->
				closed body
				<!--,}}}-->
			
			- [-] Not planned sub <!--sub https://github.com/owner/repo/issues/3 -->
				<!--omitted {{{always-->
				not planned body
				<!--,}}}-->
			
			- [42] Duplicate sub <!--sub https://github.com/owner/repo/issues/4 -->
				<!--omitted {{{always-->
				duplicate body
				<!--,}}}-->
		");
	}

	#[test]
	fn test_find_last_blocker_position_empty() {
		use std::path::Path;

		let content = "- [ ] Issue <!-- https://github.com/owner/repo/issues/1 -->\n\tBody\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();
		assert!(issue.find_last_blocker_position().is_none());
	}

	#[test]
	fn test_find_last_blocker_position_single_item() {
		use std::path::Path;

		let content = "- [ ] Issue <!-- https://github.com/owner/repo/issues/1 -->\n\tBody\n\n\t# Blockers\n\t- task 1\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();
		let pos = issue.find_last_blocker_position();
		assert!(pos.is_some());
		let (line, col) = pos.unwrap();
		assert_eq!(line, 5); // Line 5: `\t- task 1`
		// Column 4: 1-indexed position of first char after `\t- ` (tab=1, dash=2, space=3, 't'=4)
		assert_eq!(col, 4);
	}

	#[test]
	fn test_find_last_blocker_position_multiple_items() {
		use std::path::Path;

		let content = "- [ ] Issue <!-- https://github.com/owner/repo/issues/1 -->\n\tBody\n\n\t# Blockers\n\t- task 1\n\t- task 2\n\t- task 3\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();
		let pos = issue.find_last_blocker_position();
		assert!(pos.is_some());
		let (line, col) = pos.unwrap();
		assert_eq!(line, 7); // Line 7: `\t- task 3`
		assert_eq!(col, 4);
	}

	#[test]
	fn test_find_last_blocker_position_with_headers() {
		use std::path::Path;

		let content = "- [ ] Issue <!-- https://github.com/owner/repo/issues/1 -->\n\tBody\n\n\t# Blockers\n\t# Phase 1\n\t- task a\n\t# Phase 2\n\t- task b\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();
		let pos = issue.find_last_blocker_position();
		assert!(pos.is_some());
		let (line, col) = pos.unwrap();
		assert_eq!(line, 8); // Line 8: `\t- task b`
		assert_eq!(col, 4);
	}

	#[test]
	fn test_find_last_blocker_position_before_sub_issues() {
		use std::path::Path;

		let content =
			"- [ ] Issue <!-- https://github.com/owner/repo/issues/1 -->\n\tBody\n\n\t# Blockers\n\t- blocker task\n\n\t- [ ] Sub issue <!--sub https://github.com/owner/repo/issues/2 -->\n";
		let issue = Issue::parse(content, Path::new("test.md")).unwrap();
		let pos = issue.find_last_blocker_position();
		assert!(pos.is_some());
		let (line, col) = pos.unwrap();
		assert_eq!(line, 5); // Line 5: `\t- blocker task`, not the sub-issue line
		assert_eq!(col, 4);
	}
}
