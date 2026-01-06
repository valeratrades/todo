//! Line-based parsing primitives for issue files.
//!
//! This module provides a format-agnostic representation of lines in issue files.
//! Both markdown (.md) and typst (.typ) files are normalized into the same `Line` structure,
//! enabling uniform processing by higher-level modules like `blocker`.

use todo::Extension;

use crate::marker::Marker;

/// The type of content on a line.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentType {
	/// A checkbox item: `- [ ] text` or `- [x] text`
	/// The bool indicates whether it's checked.
	Checkbox { checked: bool },
	/// A list item: `- text`
	ListItem,
	/// A header: `# text` (md) or `= text` (typ)
	/// Level is 1-6.
	Header { level: usize },
	/// A marker line (blockers section, comment marker, etc.)
	/// The marker itself is stored in `Line::marker`.
	MarkerLine,
	/// A comment/annotation line (tab-indented explanation text in blockers).
	/// These don't contribute to the blocker list but provide context.
	Comment,
	/// Regular text content (body text, paragraph, etc.)
	Text,
	/// Empty line (blank)
	Empty,
}

/// A parsed line from an issue file.
///
/// This is the fundamental unit of the file format. Everything else builds on top of
/// collections of `Line`s.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
	/// Number of indentation levels (tabs).
	/// 0 = no indent (root level), 1 = one tab, etc.
	pub indent: usize,
	/// The type of content on this line.
	pub content_type: ContentType,
	/// The actual text content (with prefix stripped).
	/// For headers, this is the header text without `#` or `=`.
	/// For checkboxes, this is the text after `- [ ] ` or `- [x] `.
	/// For list items, this is the text after `- `.
	/// For markers, this may be empty (content is in `marker`).
	/// For comments/text, this is the full text.
	pub content: String,
	/// Optional marker decoded from this line.
	/// Present for issue URLs, sub-issue markers, comment markers, blockers sections, etc.
	pub marker: Option<Marker>,
	/// The original raw line (for roundtrip serialization).
	pub raw: String,
}

impl Line {
	/// Parse a single line into a `Line` structure.
	///
	/// The `ext` parameter determines how to interpret format-specific syntax
	/// (e.g., `#` vs `=` for headers).
	pub fn parse(raw: &str, ext: Extension) -> Self {
		// Count leading tabs for indentation
		let indent = raw.chars().take_while(|&c| c == '\t').count();
		let content_after_indent = &raw[indent..];

		// Handle empty lines
		if content_after_indent.trim().is_empty() {
			return Self {
				indent,
				content_type: ContentType::Empty,
				content: String::new(),
				marker: None,
				raw: raw.to_string(),
			};
		}

		// Try to decode as a marker first
		if let Some(marker) = Marker::decode(content_after_indent, ext) {
			return Self {
				indent,
				content_type: ContentType::MarkerLine,
				content: String::new(),
				marker: Some(marker),
				raw: raw.to_string(),
			};
		}

		// Check for checkbox: `- [ ] ` or `- [x] ` or `- [X] `
		let trimmed = content_after_indent.trim_start();
		if let Some((checked, rest)) = Self::parse_checkbox_prefix(trimmed) {
			// Check if there's a marker at the end (issue URL, sub-issue marker)
			let (content, marker) = Self::extract_trailing_marker(rest, ext);
			return Self {
				indent,
				content_type: ContentType::Checkbox { checked },
				content: content.trim().to_string(),
				marker,
				raw: raw.to_string(),
			};
		}

		// Check for list item: `- `
		if let Some(rest) = trimmed.strip_prefix("- ") {
			let (content, marker) = Self::extract_trailing_marker(rest, ext);
			return Self {
				indent,
				content_type: ContentType::ListItem,
				content: content.trim().to_string(),
				marker,
				raw: raw.to_string(),
			};
		}

		// Check for header
		if let Some(header) = todo::Header::decode(trimmed, ext) {
			return Self {
				indent,
				content_type: ContentType::Header { level: header.level },
				content: header.content,
				marker: None,
				raw: raw.to_string(),
			};
		}

		// Check for comment (tab-indented or space-indented text that's not a list/header)
		// In blocker context, lines that start with extra indentation are comments
		if content_after_indent.starts_with('\t') || content_after_indent.starts_with("  ") {
			let comment_text = content_after_indent.trim_start();
			// Don't treat list items as comments
			if !comment_text.starts_with("- ") {
				return Self {
					indent: indent + 1, // Account for the extra indent
					content_type: ContentType::Comment,
					content: comment_text.to_string(),
					marker: None,
					raw: raw.to_string(),
				};
			}
		}

		// Regular text content
		Self {
			indent,
			content_type: ContentType::Text,
			content: content_after_indent.to_string(),
			marker: None,
			raw: raw.to_string(),
		}
	}

	/// Parse a checkbox prefix: `- [ ] ` or `- [x] ` or `- [X] `
	/// Returns (is_checked, rest_of_line)
	fn parse_checkbox_prefix(s: &str) -> Option<(bool, &str)> {
		if let Some(rest) = s.strip_prefix("- [ ] ") {
			return Some((false, rest));
		}
		if let Some(rest) = s.strip_prefix("- [x] ").or_else(|| s.strip_prefix("- [X] ")) {
			return Some((true, rest));
		}
		// Also support other checkbox states for issue close types
		if let Some(rest) = s.strip_prefix("- [-] ") {
			return Some((true, rest)); // Not planned = closed
		}
		// Duplicate reference: `- [123] `
		if s.starts_with("- [") {
			if let Some(bracket_end) = s[3..].find("] ") {
				let content = &s[3..3 + bracket_end];
				if content.chars().all(|c| c.is_ascii_digit()) {
					return Some((true, &s[3 + bracket_end + 2..])); // Duplicate = closed
				}
			}
		}
		None
	}

	/// Extract a trailing marker from content (e.g., `<!-- url -->` at end of line)
	fn extract_trailing_marker(content: &str, ext: Extension) -> (String, Option<Marker>) {
		// Look for HTML comment marker at end
		if let Some(marker_start) = content.rfind("<!--") {
			if content.ends_with("-->") {
				let marker_text = &content[marker_start..];
				if let Some(marker) = Marker::decode(marker_text, ext) {
					return (content[..marker_start].to_string(), Some(marker));
				}
			}
		}
		// Look for typst comment marker at end: ` // url`
		if ext == Extension::Typ {
			if let Some(marker_start) = content.rfind(" // ") {
				let marker_text = &content[marker_start + 1..]; // Skip the leading space
				if let Some(marker) = Marker::decode(marker_text, ext) {
					return (content[..marker_start].to_string(), Some(marker));
				}
			}
		}
		(content.to_string(), None)
	}

	/// Check if this line represents content that contributes to a blocker list.
	/// Headers and list items contribute; comments and empty lines don't.
	pub fn is_blocker_content(&self) -> bool {
		matches!(self.content_type, ContentType::Header { .. } | ContentType::ListItem | ContentType::Checkbox { .. })
	}

	/// Check if this line is a comment (explanatory text, not contributing to blocker list).
	pub fn is_comment(&self) -> bool {
		matches!(self.content_type, ContentType::Comment)
	}

	/// Check if this line is empty.
	pub fn is_empty(&self) -> bool {
		matches!(self.content_type, ContentType::Empty)
	}

	/// Check if this line is a blockers section marker.
	pub fn is_blockers_marker(&self) -> bool {
		matches!(&self.marker, Some(Marker::BlockersSection(_)))
	}
}

/// Parse multiple lines into a Vec<Line>.
pub fn parse_lines(content: &str, ext: Extension) -> Vec<Line> {
	content.lines().map(|line| Line::parse(line, ext)).collect()
}

#[cfg(test)]
mod tests {
	use insta::assert_debug_snapshot;

	use super::*;

	#[test]
	fn test_parse_empty_line() {
		assert_debug_snapshot!(Line::parse("", Extension::Md), @r#"
		Line {
		    indent: 0,
		    content_type: Empty,
		    content: "",
		    marker: None,
		    raw: "",
		}
		"#);
		assert_debug_snapshot!(Line::parse("\t", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: Empty,
		    content: "",
		    marker: None,
		    raw: "\t",
		}
		"#);
	}

	#[test]
	fn test_parse_checkbox_unchecked() {
		assert_debug_snapshot!(Line::parse("- [ ] Task to do", Extension::Md), @r#"
		Line {
		    indent: 0,
		    content_type: Checkbox {
		        checked: false,
		    },
		    content: "Task to do",
		    marker: None,
		    raw: "- [ ] Task to do",
		}
		"#);
	}

	#[test]
	fn test_parse_checkbox_checked() {
		assert_debug_snapshot!(Line::parse("- [x] Completed task", Extension::Md), @r#"
		Line {
		    indent: 0,
		    content_type: Checkbox {
		        checked: true,
		    },
		    content: "Completed task",
		    marker: None,
		    raw: "- [x] Completed task",
		}
		"#);
	}

	#[test]
	fn test_parse_checkbox_with_url_marker() {
		assert_debug_snapshot!(Line::parse("- [ ] Issue title <!-- https://github.com/owner/repo/issues/123 -->", Extension::Md), @r#"
		Line {
		    indent: 0,
		    content_type: Checkbox {
		        checked: false,
		    },
		    content: "Issue title",
		    marker: Some(
		        IssueUrl {
		            url: "https://github.com/owner/repo/issues/123",
		            immutable: false,
		        },
		    ),
		    raw: "- [ ] Issue title <!-- https://github.com/owner/repo/issues/123 -->",
		}
		"#);
	}

	#[test]
	fn test_parse_checkbox_with_sub_marker() {
		assert_debug_snapshot!(Line::parse("\t- [ ] Sub-issue <!--sub https://github.com/owner/repo/issues/124 -->", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: Checkbox {
		        checked: false,
		    },
		    content: "Sub-issue",
		    marker: Some(
		        SubIssue {
		            url: "https://github.com/owner/repo/issues/124",
		        },
		    ),
		    raw: "\t- [ ] Sub-issue <!--sub https://github.com/owner/repo/issues/124 -->",
		}
		"#);
	}

	#[test]
	fn test_parse_list_item() {
		assert_debug_snapshot!(Line::parse("- Simple list item", Extension::Md), @r#"
		Line {
		    indent: 0,
		    content_type: ListItem,
		    content: "Simple list item",
		    marker: None,
		    raw: "- Simple list item",
		}
		"#);
	}

	#[test]
	fn test_parse_list_item_indented() {
		assert_debug_snapshot!(Line::parse("\t- Indented blocker item", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: ListItem,
		    content: "Indented blocker item",
		    marker: None,
		    raw: "\t- Indented blocker item",
		}
		"#);
	}

	#[test]
	fn h1() {
		assert_debug_snapshot!(Line::parse("# Main Header", Extension::Md), @r##"
		Line {
		    indent: 0,
		    content_type: Header {
		        level: 1,
		    },
		    content: "Main Header",
		    marker: None,
		    raw: "# Main Header",
		}
		"##);
	}

	#[test]
	fn md_h1() {
		assert_debug_snapshot!(Line::parse("# Main Header", Extension::Md), @r##"
		Line {
		    indent: 0,
		    content_type: Header {
		        level: 1,
		    },
		    content: "Main Header",
		    marker: None,
		    raw: "# Main Header",
		}
		"##);
	}

	#[test]
	fn md_h2() {
		assert_debug_snapshot!(Line::parse("## Sub Header", Extension::Md), @r###"
		Line {
		    indent: 0,
		    content_type: Header {
		        level: 2,
		    },
		    content: "Sub Header",
		    marker: None,
		    raw: "## Sub Header",
		}
		"###);
	}

	#[test]
	fn typ_h1() {
		assert_debug_snapshot!(Line::parse("= Main Header", Extension::Typ), @r#"
		Line {
		    indent: 0,
		    content_type: Header {
		        level: 1,
		    },
		    content: "Main Header",
		    marker: None,
		    raw: "= Main Header",
		}
		"#);
	}

	#[test]
	fn typ_h2() {
		assert_debug_snapshot!(Line::parse("== Sub Header", Extension::Typ), @r#"
		Line {
		    indent: 0,
		    content_type: Header {
		        level: 2,
		    },
		    content: "Sub Header",
		    marker: None,
		    raw: "== Sub Header",
		}
		"#);
	}

	#[test]
	fn test_parse_header_indented() {
		assert_debug_snapshot!(Line::parse("\t# Blockers", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: MarkerLine,
		    content: "",
		    marker: Some(
		        BlockersSection(
		            Header {
		                level: 1,
		                content: "Blockers",
		            },
		        ),
		    ),
		    raw: "\t# Blockers",
		}
		"#);
	}

	#[test]
	fn test_parse_blockers_marker_legacy() {
		assert_debug_snapshot!(Line::parse("<!--blockers-->", Extension::Md), @r#"
		Line {
		    indent: 0,
		    content_type: MarkerLine,
		    content: "",
		    marker: Some(
		        BlockersSection(
		            Header {
		                level: 1,
		                content: "Blockers",
		            },
		        ),
		    ),
		    raw: "<!--blockers-->",
		}
		"#);
	}

	#[test]
	fn test_parse_blockers_marker_indented() {
		assert_debug_snapshot!(Line::parse("\t<!--blockers-->", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: MarkerLine,
		    content: "",
		    marker: Some(
		        BlockersSection(
		            Header {
		                level: 1,
		                content: "Blockers",
		            },
		        ),
		    ),
		    raw: "\t<!--blockers-->",
		}
		"#);
	}

	#[test]
	fn test_parse_comment_marker() {
		assert_debug_snapshot!(Line::parse("\t<!-- https://github.com/owner/repo/issues/123#issuecomment-456 -->", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: MarkerLine,
		    content: "",
		    marker: Some(
		        Comment {
		            url: "https://github.com/owner/repo/issues/123#issuecomment-456",
		            id: 456,
		            immutable: false,
		        },
		    ),
		    raw: "\t<!-- https://github.com/owner/repo/issues/123#issuecomment-456 -->",
		}
		"#);
	}

	#[test]
	fn test_parse_text_content() {
		assert_debug_snapshot!(Line::parse("\tThis is body text.", Extension::Md), @r#"
		Line {
		    indent: 1,
		    content_type: Text,
		    content: "This is body text.",
		    marker: None,
		    raw: "\tThis is body text.",
		}
		"#);
	}

	#[test]
	fn test_parse_comment_double_indent() {
		// In blocker context, extra indentation indicates a comment
		assert_debug_snapshot!(Line::parse("\t\tThis is a comment on the blocker above", Extension::Md), @r#"
		Line {
		    indent: 2,
		    content_type: Text,
		    content: "This is a comment on the blocker above",
		    marker: None,
		    raw: "\t\tThis is a comment on the blocker above",
		}
		"#);
	}

	#[test]
	fn test_parse_typst_checkbox_with_url() {
		assert_debug_snapshot!(Line::parse("- [ ] Issue title // https://github.com/owner/repo/issues/123", Extension::Typ), @r#"
		Line {
		    indent: 0,
		    content_type: Checkbox {
		        checked: false,
		    },
		    content: "Issue title",
		    marker: Some(
		        IssueUrl {
		            url: "https://github.com/owner/repo/issues/123",
		            immutable: false,
		        },
		    ),
		    raw: "- [ ] Issue title // https://github.com/owner/repo/issues/123",
		}
		"#);
	}

	#[test]
	fn test_parse_lines_full_issue() {
		let content = "- [ ] Issue title <!-- https://github.com/owner/repo/issues/123 -->
\tBody text here.

\t<!--blockers-->
\t# Phase 1
\t- First task
\t\tcomment on first task
\t- Second task

\t# Phase 2
\t- Third task
";
		let lines = parse_lines(content, Extension::Md);
		assert_debug_snapshot!(lines, @r#"
		[
		    Line {
		        indent: 0,
		        content_type: Checkbox {
		            checked: false,
		        },
		        content: "Issue title",
		        marker: Some(
		            IssueUrl {
		                url: "https://github.com/owner/repo/issues/123",
		                immutable: false,
		            },
		        ),
		        raw: "- [ ] Issue title <!-- https://github.com/owner/repo/issues/123 -->",
		    },
		    Line {
		        indent: 1,
		        content_type: Text,
		        content: "Body text here.",
		        marker: None,
		        raw: "\tBody text here.",
		    },
		    Line {
		        indent: 0,
		        content_type: Empty,
		        content: "",
		        marker: None,
		        raw: "",
		    },
		    Line {
		        indent: 1,
		        content_type: MarkerLine,
		        content: "",
		        marker: Some(
		            BlockersSection(
		                Header {
		                    level: 1,
		                    content: "Blockers",
		                },
		            ),
		        ),
		        raw: "\t<!--blockers-->",
		    },
		    Line {
		        indent: 1,
		        content_type: Header {
		            level: 1,
		        },
		        content: "Phase 1",
		        marker: None,
		        raw: "\t# Phase 1",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "First task",
		        marker: None,
		        raw: "\t- First task",
		    },
		    Line {
		        indent: 2,
		        content_type: Text,
		        content: "comment on first task",
		        marker: None,
		        raw: "\t\tcomment on first task",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "Second task",
		        marker: None,
		        raw: "\t- Second task",
		    },
		    Line {
		        indent: 0,
		        content_type: Empty,
		        content: "",
		        marker: None,
		        raw: "",
		    },
		    Line {
		        indent: 1,
		        content_type: Header {
		            level: 1,
		        },
		        content: "Phase 2",
		        marker: None,
		        raw: "\t# Phase 2",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "Third task",
		        marker: None,
		        raw: "\t- Third task",
		    },
		]
		"#);
	}

	#[test]
	fn test_is_blocker_content() {
		let header = Line::parse("# Header", Extension::Md);
		assert!(header.is_blocker_content());

		let list_item = Line::parse("- Item", Extension::Md);
		assert!(list_item.is_blocker_content());

		let checkbox = Line::parse("- [ ] Todo", Extension::Md);
		assert!(checkbox.is_blocker_content());

		let comment = Line::parse("\t\tComment text", Extension::Md);
		assert!(!comment.is_blocker_content());

		let empty = Line::parse("", Extension::Md);
		assert!(!empty.is_blocker_content());
	}

	#[test]
	fn test_real_issue_file_format() {
		// Test parsing a real-world issue file structure
		let content = "- [ ] blocker rewrite <!-- https://github.com/valeratrades/todo/issues/49 -->
\tget all the present functionality + legacy supported, over into integration with issues

\t<!--blockers-->
\t- support for virtual blockers (to keep legacy blocker files usable)
\t- move all primitives into new `blocker.rs`
\t- get clockify integration
\t- rename rewrite to `blocker`, and `blocker` to `blocker-legacy`. See what breaks
\t- ensure existing are working
";
		let lines = parse_lines(content, Extension::Md);
		assert_debug_snapshot!(lines, @r#"
		[
		    Line {
		        indent: 0,
		        content_type: Checkbox {
		            checked: false,
		        },
		        content: "blocker rewrite",
		        marker: Some(
		            IssueUrl {
		                url: "https://github.com/valeratrades/todo/issues/49",
		                immutable: false,
		            },
		        ),
		        raw: "- [ ] blocker rewrite <!-- https://github.com/valeratrades/todo/issues/49 -->",
		    },
		    Line {
		        indent: 1,
		        content_type: Text,
		        content: "get all the present functionality + legacy supported, over into integration with issues",
		        marker: None,
		        raw: "\tget all the present functionality + legacy supported, over into integration with issues",
		    },
		    Line {
		        indent: 0,
		        content_type: Empty,
		        content: "",
		        marker: None,
		        raw: "",
		    },
		    Line {
		        indent: 1,
		        content_type: MarkerLine,
		        content: "",
		        marker: Some(
		            BlockersSection(
		                Header {
		                    level: 1,
		                    content: "Blockers",
		                },
		            ),
		        ),
		        raw: "\t<!--blockers-->",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "support for virtual blockers (to keep legacy blocker files usable)",
		        marker: None,
		        raw: "\t- support for virtual blockers (to keep legacy blocker files usable)",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "move all primitives into new `blocker.rs`",
		        marker: None,
		        raw: "\t- move all primitives into new `blocker.rs`",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "get clockify integration",
		        marker: None,
		        raw: "\t- get clockify integration",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "rename rewrite to `blocker`, and `blocker` to `blocker-legacy`. See what breaks",
		        marker: None,
		        raw: "\t- rename rewrite to `blocker`, and `blocker` to `blocker-legacy`. See what breaks",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "ensure existing are working",
		        marker: None,
		        raw: "\t- ensure existing are working",
		    },
		]
		"#);
	}

	#[test]
	fn test_virtual_issue_with_header_blockers() {
		// Virtual issues use `# Blockers` header instead of `<!--blockers-->`
		let content = "- [ ] todo: issue editor <!--virtual:valera/tooling#5-->
\t# Blockers
\t## blockers not syncing
\t- see where they disappear
\t\twe know they don't get uploaded to the github. And then they get nuked from issue file.
\t\tQ: can a sync from github maybe be overwriting them? Check for that too
\t- put logs on full lifecycle
\t- get logging working
";
		let lines = parse_lines(content, Extension::Md);
		assert_debug_snapshot!(lines, @r#"
		[
		    Line {
		        indent: 0,
		        content_type: Checkbox {
		            checked: false,
		        },
		        content: "todo: issue editor",
		        marker: Some(
		            IssueUrl {
		                url: "virtual:valera/tooling#5",
		                immutable: false,
		            },
		        ),
		        raw: "- [ ] todo: issue editor <!--virtual:valera/tooling#5-->",
		    },
		    Line {
		        indent: 1,
		        content_type: MarkerLine,
		        content: "",
		        marker: Some(
		            BlockersSection(
		                Header {
		                    level: 1,
		                    content: "Blockers",
		                },
		            ),
		        ),
		        raw: "\t# Blockers",
		    },
		    Line {
		        indent: 1,
		        content_type: Header {
		            level: 2,
		        },
		        content: "blockers not syncing",
		        marker: None,
		        raw: "\t## blockers not syncing",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "see where they disappear",
		        marker: None,
		        raw: "\t- see where they disappear",
		    },
		    Line {
		        indent: 2,
		        content_type: Text,
		        content: "we know they don't get uploaded to the github. And then they get nuked from issue file.",
		        marker: None,
		        raw: "\t\twe know they don't get uploaded to the github. And then they get nuked from issue file.",
		    },
		    Line {
		        indent: 2,
		        content_type: Text,
		        content: "Q: can a sync from github maybe be overwriting them? Check for that too",
		        marker: None,
		        raw: "\t\tQ: can a sync from github maybe be overwriting them? Check for that too",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "put logs on full lifecycle",
		        marker: None,
		        raw: "\t- put logs on full lifecycle",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "get logging working",
		        marker: None,
		        raw: "\t- get logging working",
		    },
		]
		"#);
	}

	#[test]
	fn test_typst_issue_file() {
		// Note: Using tab-indentation as the Line parser expects normalized content
		// (real files may use spaces, but normalize_issue_indentation converts to tabs)
		// Typst uses `= Header` syntax, not `# Header`
		let content = "- [ ] dataflow-based_integration_tests // https://github.com/valeratrades/todo/issues/68
\t= Blockers
\t- reimplement all of integration tests here to only ever use basic actions at the boundary:
\t\t- define initial dir state (alongside exact file contents)
\t\t- emulate user opening/closing
";
		let lines = parse_lines(content, Extension::Typ);
		assert_debug_snapshot!(lines, @r#"
		[
		    Line {
		        indent: 0,
		        content_type: Checkbox {
		            checked: false,
		        },
		        content: "dataflow-based_integration_tests",
		        marker: Some(
		            IssueUrl {
		                url: "https://github.com/valeratrades/todo/issues/68",
		                immutable: false,
		            },
		        ),
		        raw: "- [ ] dataflow-based_integration_tests // https://github.com/valeratrades/todo/issues/68",
		    },
		    Line {
		        indent: 1,
		        content_type: MarkerLine,
		        content: "",
		        marker: Some(
		            BlockersSection(
		                Header {
		                    level: 1,
		                    content: "Blockers",
		                },
		            ),
		        ),
		        raw: "\t= Blockers",
		    },
		    Line {
		        indent: 1,
		        content_type: ListItem,
		        content: "reimplement all of integration tests here to only ever use basic actions at the boundary:",
		        marker: None,
		        raw: "\t- reimplement all of integration tests here to only ever use basic actions at the boundary:",
		    },
		    Line {
		        indent: 2,
		        content_type: ListItem,
		        content: "define initial dir state (alongside exact file contents)",
		        marker: None,
		        raw: "\t\t- define initial dir state (alongside exact file contents)",
		    },
		    Line {
		        indent: 2,
		        content_type: ListItem,
		        content: "emulate user opening/closing",
		        marker: None,
		        raw: "\t\t- emulate user opening/closing",
		    },
		]
		"#);
	}
}
