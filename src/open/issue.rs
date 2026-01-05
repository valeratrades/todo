//! Core issue data structures and parsing/serialization.

use super::util::{is_blockers_marker, normalize_issue_indentation};
use crate::{
	error::{ParseContext, ParseError},
	github::{self, IssueAction, OriginalSubIssue},
};

/// Metadata for an issue (title line info)
#[derive(Clone, Debug, PartialEq)]
pub struct IssueMeta {
	pub title: String,
	/// GitHub URL, None for new issues
	pub url: Option<String>,
	pub closed: bool,
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

/// A blocker entry - classification + raw content
#[derive(Clone, Debug, PartialEq)]
pub struct Blocker {
	pub line_type: crate::blocker::LineType,
	pub raw: String,
}

/// Complete representation of an issue file
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
	pub meta: IssueMeta,
	pub labels: Vec<String>,
	/// Comments in order. First is always the issue body (serialized without marker).
	pub comments: Vec<Comment>,
	/// Sub-issues in order
	pub children: Vec<Issue>,
	/// Blockers section (empty if none). Parsed exactly like a standalone blockers file.
	pub blockers: Vec<Blocker>,
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
			for blocker in &self.blockers {
				full_body.push_str(&blocker.raw);
				full_body.push('\n');
			}
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
		let mut blockers = Vec::new();
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
			if in_blockers {
				if let Some(line_type) = crate::blocker::classify_line(content) {
					tracing::debug!("[parse] blocker line: {:?} -> {:?}", content, line_type);
					blockers.push(Blocker {
						line_type,
						raw: content.to_string(),
					});
				} else {
					tracing::debug!("[parse] blocker line SKIPPED (classify_line returned None): {:?}", content);
				}
				continue;
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
					blockers: vec![],
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
			blockers,
		})
	}

	/// Parse title line: `- [ ] [label1, label2] Title <!--url-->` or `- [ ] Title <!--immutable url-->`
	/// Returns (IssueMeta, labels)
	fn parse_title_line(line: &str, line_num: usize, ctx: &ParseContext) -> Result<(IssueMeta, Vec<String>), ParseError> {
		let (closed, rest) = if let Some(rest) = line.strip_prefix("- [ ] ") {
			(false, rest)
		} else if let Some(rest) = line.strip_prefix("- [x] ").or_else(|| line.strip_prefix("- [X] ")) {
			(true, rest)
		} else {
			return Err(ParseError::InvalidTitle {
				src: ctx.named_source(),
				span: ctx.line_span(line_num),
				detail: format!("got: {:?}", line),
			});
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

		let (owned, url) = if let Some(url) = inner.strip_prefix("immutable ") {
			(false, Some(url.trim().to_string()))
		} else {
			(true, Some(inner.to_string()))
		};

		Ok((IssueMeta { title, url, closed, owned }, labels))
	}

	/// Parse child/sub-issue title line: `- [x] Title <!--sub url-->` or `- [ ] Title` (new)
	fn parse_child_title_line(line: &str) -> Option<IssueMeta> {
		let (closed, rest) = if let Some(rest) = line.strip_prefix("- [ ] ") {
			(false, rest)
		} else if let Some(rest) = line.strip_prefix("- [x] ").or_else(|| line.strip_prefix("- [X] ")) {
			(true, rest)
		} else {
			return None;
		};

		// Check for sub marker
		if let Some(marker_start) = rest.find("<!--sub ") {
			let marker_end = rest.find("-->")?;
			let title = rest[..marker_start].trim().to_string();
			let url = rest[marker_start + 8..marker_end].trim().to_string();
			Some(IssueMeta {
				title,
				url: Some(url),
				closed,
				owned: true,
			})
		} else if !rest.contains("<!--") {
			let title = rest.trim().to_string();
			if !title.is_empty() {
				Some(IssueMeta {
					title,
					url: None,
					closed,
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

		// Title line: `- [ ] [label1, label2] Title <!-- url -->` or `- [ ] Title <!-- url -->` if no labels
		let checked = if self.meta.closed { "x" } else { " " };
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
			for blocker in &self.blockers {
				out.push_str(&format!("{content_indent}{}\n", blocker.raw));
			}
		}

		// Children (sub-issues) at the very end
		// Closed sub-issues show `<!-- omitted -->` instead of body content
		for child in &self.children {
			let child_checked = if child.meta.closed { "x" } else { " " };
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
			if child.meta.closed {
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
				// New sub-issue - needs to be created
				if let Some(parent_num) = parent_number {
					levels[depth].push(IssueAction::CreateSubIssue {
						child_path: child_path.clone(),
						title: child.meta.title.clone(),
						closed: child.meta.closed,
						parent_issue_number: parent_num,
					});
				}
			} else if let Some(child_url) = &child.meta.url {
				// Existing sub-issue - check if state changed
				if let Some(child_number) = github::extract_issue_number_from_url(child_url)
					&& let Some(orig) = original_sub_issues.iter().find(|o| o.number == child_number)
				{
					let orig_closed = orig.state == "closed";
					if child.meta.closed != orig_closed {
						levels[depth].push(IssueAction::UpdateSubIssueState {
							issue_number: child_number,
							closed: child.meta.closed,
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
}
