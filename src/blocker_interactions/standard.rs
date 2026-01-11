//! Extended parsing primitives and formatting for blocker content.
//!
//! This module provides additional functions for understanding blocker file syntax:
//! - `format_blocker_content`: Normalize blocker content to standard format
//! - `normalize_content_by_extension`: Handle different file types (md, typst)
//! - `typst_to_markdown`: Convert Typst syntax to markdown
//!
//! Core types (HeaderLevel, Line, classify_line) are in the library crate.

use std::path::Path;

use color_eyre::eyre::{Result, eyre};
// Re-export from library for internal use
pub use todo::{Line, classify_line};

/// Check if the content is semantically empty (only comments or whitespace, no actual content)
pub fn is_semantically_empty(content: &str) -> bool {
	content.lines().filter_map(classify_line).all(|line_type| !line_type.is_content())
}

/// Format blocker list content according to standardization rules:
/// 1. Lines not starting with `^#* ` get prefixed with `- ` (markdown list format)
/// 2. Always have 1 empty line above `^#* ` lines (unless the line above also starts with `#`)
/// 3. Remove all other empty lines for standardization
/// 4. Comment lines (tab-indented) are preserved and must follow Content or Comment lines
/// 5. Code blocks (``` ... ```) within comments can contain blank lines
pub fn format_blocker_content(content: &str) -> Result<String> {
	let lines: Vec<&str> = content.lines().collect();

	// First pass: validate that comments don't follow empty lines (outside of code blocks)
	let mut in_code_block = false;
	for (idx, line) in lines.iter().enumerate() {
		// Track code block state - code blocks in comments are tab-indented with ```
		let trimmed = line.trim_start_matches('\t').trim_start();
		if trimmed.starts_with("```") {
			in_code_block = !in_code_block;
		}

		// Skip validation inside code blocks - blank lines are allowed there
		if in_code_block {
			continue;
		}

		if let Some(Line::Comment(_)) = classify_line(line) {
			// Check if previous line was empty
			if idx > 0 && lines[idx - 1].is_empty() {
				return Err(eyre!(
					"Comment line at position {} cannot follow an empty line. Comments must follow content or other comments.",
					idx + 1
				));
			}
			// Check if it's the first line
			if idx == 0 {
				return Err(eyre!(
					"Comment line at position {} cannot be first line. Comments must follow content or other comments.",
					idx + 1
				));
			}
		}
	}

	let mut formatted_lines: Vec<String> = Vec::new();
	let mut in_code_block = false;

	for line in lines.iter() {
		// Track code block state for formatting
		let trimmed_for_code = line.trim_start_matches('\t').trim_start();
		if trimmed_for_code.starts_with("```") {
			in_code_block = !in_code_block;
		}

		let line_type = classify_line(line);

		match line_type {
			None => {
				// Preserve empty lines inside code blocks, skip others
				if in_code_block {
					formatted_lines.push(String::new());
				}
				continue;
			}
			Some(Line::Comment(content)) => {
				// Use the parsed content, which has the leading tab/spaces stripped
				formatted_lines.push(format!("\t{content}"));
			}
			Some(Line::Header { level, text }) => {
				// Check if we need an empty line before this header
				if !formatted_lines.is_empty() {
					let last_line = formatted_lines.last().unwrap();
					let prev_line_type = classify_line(last_line);

					// Add empty line based on header level relationship:
					// - No space if previous is larger rank (smaller level value) than current
					// - Space if previous is same or lower rank (same/larger level value) than current
					// - Space if previous line is not a header
					let needs_space = match prev_line_type {
						Some(Line::Header { level: prev_level, .. }) => {
							// Using derived Ord: One < Two < Three < Four < Five
							prev_level >= level // same or lower rank (e.g., ## after # or ##)
						}
						_ => true, // previous line is not a header
					};

					if needs_space {
						formatted_lines.push(String::new());
					}
				}

				// Reconstruct the header line
				let header_prefix = "#".repeat(level.to_usize());
				formatted_lines.push(format!("{header_prefix} {text}"));
			}
			Some(Line::Item(text)) => {
				// Use the parsed text and format with proper "- " prefix
				formatted_lines.push(format!("- {text}"));
			}
		}
	}

	Ok(formatted_lines.join("\n"))
}

/// Normalize content based on file extension.
/// Converts file-specific syntax to a canonical markdown-like format:
/// - .md: pass through as-is
/// - .typ: convert Typst syntax to markdown (= to #, etc.)
/// - other: pass through as-is
pub fn normalize_content_by_extension(content: &str, file_path: &Path) -> Result<String> {
	let extension = file_path.extension().and_then(|e| e.to_str());

	match extension {
		Some("md") => Ok(content.to_string()),
		Some("typ") => typst_to_markdown(content),
		_ => Ok(content.to_string()),
	}
}

/// Convert Typst syntax to markdown format.
/// Typst uses = for headings (more = means deeper), we convert to # (more # means deeper)
/// Typst list syntax is similar to markdown (- for bullets)
pub fn typst_to_markdown(content: &str) -> Result<String> {
	use typst::syntax::{SyntaxKind, ast::AstNode, parse};

	// Parse the Typst source into a syntax tree
	let syntax_node = parse(content);

	// Walk the syntax tree and convert to markdown
	let mut markdown_lines: Vec<String> = Vec::new();

	// Traverse the syntax tree
	for child in syntax_node.children() {
		// Skip pure whitespace nodes (space, parbreak)
		if matches!(child.kind(), SyntaxKind::Space | SyntaxKind::Parbreak) {
			// Check if this is a significant parbreak (actual empty line in source)
			let text = child.text();
			if text.matches('\n').count() > 1 {
				// Multiple newlines = intentional empty line
				markdown_lines.push(String::new());
			}
			continue;
		}

		// Get the text content of this node
		let node_text = child.clone().into_text();

		// Try to interpret as Heading
		if let Some(heading) = typst::syntax::ast::Heading::from_untyped(child) {
			let level_num = heading.depth().get();
			// Extract just the body text (without the = prefix)
			let body_text = heading.body().to_untyped().clone().into_text();
			let trimmed_body = body_text.trim();
			// Convert Typst heading (= foo) to markdown heading (# foo)
			markdown_lines.push(format!("{} {trimmed_body}", "#".repeat(level_num)));
			continue;
		}

		// Try to interpret as ListItem (bullet list)
		// Typst uses "- item" which is identical to markdown, so just keep it
		if let Some(_list_item) = typst::syntax::ast::ListItem::from_untyped(child) {
			let trimmed = node_text.trim();
			if !trimmed.is_empty() {
				markdown_lines.push(trimmed.to_string());
			}
			continue;
		}

		// Try to interpret as EnumItem (numbered list)
		// Convert numbered lists to markdown-style items with "- " prefix
		if let Some(_enum_item) = typst::syntax::ast::EnumItem::from_untyped(child) {
			let trimmed = node_text.trim();
			if !trimmed.is_empty() {
				// For numbered items, just treat as regular items
				// Strip the number/+ prefix and convert to -
				let item_text = if let Some(stripped) = trimmed.strip_prefix('+') {
					stripped.trim()
				} else {
					// Handle numbered format like "1. item"
					if let Some(pos) = trimmed.find('.') { trimmed[pos + 1..].trim() } else { trimmed }
				};
				markdown_lines.push(format!("- {item_text}"));
			}
			continue;
		}

		// For other content (paragraphs, text), keep as-is if non-empty
		let trimmed = node_text.trim();
		if !trimmed.is_empty() {
			markdown_lines.push(trimmed.to_string());
		}
	}

	Ok(markdown_lines.join("\n"))
}

#[cfg(test)]
mod tests {
	use todo::HeaderLevel;

	use super::*;

	#[test]
	fn test_classify_line() {
		assert_eq!(classify_line(""), None);
		assert_eq!(classify_line("\tComment"), Some(Line::Comment("Comment".to_string())));
		assert_eq!(classify_line("Content"), Some(Line::Item("Content".to_string())));
		// Lines with 2+ leading spaces are now treated as comments (likely tab-to-space conversion)
		assert_eq!(classify_line("  Spaces not tab"), Some(Line::Comment("Spaces not tab".to_string())));
		// But space-indented list items (with -) are still items
		assert_eq!(classify_line("  - Indented list item"), Some(Line::Item("Indented list item".to_string())));
		assert_eq!(
			classify_line("# Header 1"),
			Some(Line::Header {
				level: HeaderLevel::One,
				text: "Header 1".to_string()
			})
		);
		assert_eq!(
			classify_line("## Header 2"),
			Some(Line::Header {
				level: HeaderLevel::Two,
				text: "Header 2".to_string()
			})
		);
		assert_eq!(
			classify_line("### Header 3"),
			Some(Line::Header {
				level: HeaderLevel::Three,
				text: "Header 3".to_string()
			})
		);
		assert_eq!(
			classify_line("#### Header 4"),
			Some(Line::Header {
				level: HeaderLevel::Four,
				text: "Header 4".to_string()
			})
		);
		assert_eq!(
			classify_line("##### Header 5"),
			Some(Line::Header {
				level: HeaderLevel::Five,
				text: "Header 5".to_string()
			})
		);
		assert_eq!(classify_line("#NoSpace"), Some(Line::Item("#NoSpace".to_string()))); // Invalid header
		assert_eq!(classify_line("###### Header 6"), Some(Line::Item("###### Header 6".to_string()))); // Level 6 not supported, treated as item
	}

	#[test]
	fn test_comment_validation_errors() {
		// Comment as first line
		assert!(format_blocker_content("\tComment").is_err());
		// Comment after empty line
		assert!(format_blocker_content("- Task\n\n\tComment").is_err());
	}

	#[test]
	fn test_comment_preservation() {
		// Single and multiple comments
		let input = "- Task 1\n\tComment 1\n- Task 2\n\tComment A\n\tComment B";
		let expected = "- Task 1\n\tComment 1\n- Task 2\n\tComment A\n\tComment B";
		assert_eq!(format_blocker_content(input).unwrap(), expected);
	}

	#[test]
	fn test_header_empty_line_rules() {
		// No empty line when going from larger rank (smaller #) to lower rank (more #)
		assert_eq!(format_blocker_content("# H1\n## H2").unwrap(), "# H1\n## H2");
		assert_eq!(format_blocker_content("# H1\n### H3").unwrap(), "# H1\n### H3");
		assert_eq!(format_blocker_content("## H2\n### H3").unwrap(), "## H2\n### H3");

		// Empty line when going from same rank to same rank
		assert_eq!(format_blocker_content("# H1\n# H2").unwrap(), "# H1\n\n# H2");
		assert_eq!(format_blocker_content("## H2a\n## H2b").unwrap(), "## H2a\n\n## H2b");

		// Empty line when going from lower rank (more #) to higher rank (fewer #)
		assert_eq!(format_blocker_content("## H2\n# H1").unwrap(), "## H2\n\n# H1");
		assert_eq!(format_blocker_content("### H3\n# H1").unwrap(), "### H3\n\n# H1");
		assert_eq!(format_blocker_content("### H3\n## H2").unwrap(), "### H3\n\n## H2");

		// Empty line before header after item
		assert_eq!(format_blocker_content("item\n\n# Header").unwrap(), "- item\n\n# Header");

		// Valid header needs space: # vs #NoSpace
		assert_eq!(format_blocker_content("#NoSpace").unwrap(), "- #NoSpace");
	}

	#[test]
	fn test_empty_lines_removed() {
		// Multiple empty lines collapsed
		let input = "item 1\n\n\nitem 2\n\n\n\nitem 3";
		assert_eq!(format_blocker_content(input).unwrap(), "- item 1\n- item 2\n- item 3");
	}

	#[test]
	fn test_space_indented_comments_converted_to_tabs() {
		// Comments with leading spaces (e.g., from editor tab-to-space conversion) should be converted to tab-indented
		let input = "- Task 1\n    Comment with 4 spaces\n- Task 2";
		let expected = "- Task 1\n\tComment with 4 spaces\n- Task 2";
		assert_eq!(format_blocker_content(input).unwrap(), expected);

		// Multiple space-indented comments
		let input2 = "- Task 1\n    Comment 1\n    Comment 2\n- Task 2";
		let expected2 = "- Task 1\n\tComment 1\n\tComment 2\n- Task 2";
		assert_eq!(format_blocker_content(input2).unwrap(), expected2);

		// Mixed: some tabs, some spaces (should normalize to tabs)
		let input3 = "- Task 1\n\tTab comment\n    Space comment\n- Task 2";
		let expected3 = "- Task 1\n\tTab comment\n\tSpace comment\n- Task 2";
		assert_eq!(format_blocker_content(input3).unwrap(), expected3);

		// Comments with varying amounts of leading spaces (2+ spaces)
		let input4 = "- Task 1\n  Comment with 2 spaces\n   Comment with 3 spaces\n      Comment with 6 spaces";
		let expected4 = "- Task 1\n\tComment with 2 spaces\n\tComment with 3 spaces\n\tComment with 6 spaces";
		assert_eq!(format_blocker_content(input4).unwrap(), expected4);

		// Space-indented comments after headers
		let input5 = "# Section 1\n- Task 1\n    Comment about task 1";
		let expected5 = "# Section 1\n- Task 1\n\tComment about task 1";
		assert_eq!(format_blocker_content(input5).unwrap(), expected5);
	}

	#[test]
	fn test_space_indented_comments_edge_cases() {
		// Single space should NOT be treated as comment (too ambiguous)
		let input = "- Task 1\n Content with one space";
		let expected = "- Task 1\n- Content with one space";
		assert_eq!(format_blocker_content(input).unwrap(), expected);

		// Space-indented list items (with -) should remain as items, not become comments
		let input2 = "- Task 1\n  - Subtask with 2 spaces and dash";
		let expected2 = "- Task 1\n- Subtask with 2 spaces and dash";
		assert_eq!(format_blocker_content(input2).unwrap(), expected2);

		// Idempotency: formatting space-indented comments twice should yield same result
		let input3 = "- Task 1\n    Comment";
		let formatted_once = format_blocker_content(input3).unwrap();
		let formatted_twice = format_blocker_content(&formatted_once).unwrap();
		assert_eq!(formatted_once, formatted_twice);
		assert_eq!(formatted_once, "- Task 1\n\tComment");
	}

	#[test]
	fn test_line_methods() {
		let h1 = Line::Header {
			level: HeaderLevel::One,
			text: "Test".to_string(),
		};
		let h2 = Line::Header {
			level: HeaderLevel::Two,
			text: "Test".to_string(),
		};
		let item = Line::Item("Task".to_string());
		let comment = Line::Comment("Note".to_string());

		// Test is_header
		assert!(h1.is_header());
		assert!(h2.is_header());
		assert!(!item.is_header());
		assert!(!comment.is_header());

		// Test is_content
		assert!(h1.is_content());
		assert!(h2.is_content());
		assert!(item.is_content());
		assert!(!comment.is_content());

		// Test to_raw
		assert_eq!(h1.to_raw(), "# Test");
		assert_eq!(h2.to_raw(), "## Test");
		assert_eq!(item.to_raw(), "- Task");
		assert_eq!(comment.to_raw(), "\tNote");
	}

	#[test]
	fn test_header_level_ordering() {
		// Test that HeaderLevel has proper ordering (One < Two < Three < Four < Five)
		assert!(HeaderLevel::One < HeaderLevel::Two);
		assert!(HeaderLevel::Two < HeaderLevel::Three);
		assert!(HeaderLevel::Three < HeaderLevel::Four);
		assert!(HeaderLevel::Four < HeaderLevel::Five);

		// Test to_usize
		assert_eq!(HeaderLevel::One.to_usize(), 1);
		assert_eq!(HeaderLevel::Two.to_usize(), 2);
		assert_eq!(HeaderLevel::Three.to_usize(), 3);
		assert_eq!(HeaderLevel::Four.to_usize(), 4);
		assert_eq!(HeaderLevel::Five.to_usize(), 5);

		// Test from_usize
		assert_eq!(HeaderLevel::from_usize(1), Some(HeaderLevel::One));
		assert_eq!(HeaderLevel::from_usize(2), Some(HeaderLevel::Two));
		assert_eq!(HeaderLevel::from_usize(3), Some(HeaderLevel::Three));
		assert_eq!(HeaderLevel::from_usize(4), Some(HeaderLevel::Four));
		assert_eq!(HeaderLevel::from_usize(5), Some(HeaderLevel::Five));
		assert_eq!(HeaderLevel::from_usize(6), None);
		assert_eq!(HeaderLevel::from_usize(0), None);
	}

	#[test]
	fn test_typst_to_markdown_headings() {
		// Test Typst heading conversion (= to #)
		let typst_input = "= Level 1\n== Level 2\n=== Level 3";
		let expected = "# Level 1\n## Level 2\n### Level 3";
		assert_eq!(typst_to_markdown(typst_input).unwrap(), expected);
	}

	#[test]
	fn test_typst_to_markdown_lists() {
		// Test Typst bullet list (same as markdown)
		let typst_input = "- First item\n- Second item";
		let expected = "- First item\n- Second item";
		assert_eq!(typst_to_markdown(typst_input).unwrap(), expected);
	}

	#[test]
	fn test_typst_to_markdown_enum_lists() {
		// Test Typst numbered list conversion
		let typst_input = "+ First\n+ Second";
		let markdown = typst_to_markdown(typst_input).unwrap();
		// Should convert to markdown list items
		assert!(markdown.contains("- First"));
		assert!(markdown.contains("- Second"));
	}

	#[test]
	fn test_typst_to_markdown_mixed() {
		// Test mixed content
		let typst_input = "= Project\n- task 1\n- task 2";
		let markdown = typst_to_markdown(typst_input).unwrap();
		assert!(markdown.contains("# Project"));
		assert!(markdown.contains("- task 1"));
		assert!(markdown.contains("- task 2"));
	}

	#[test]
	fn test_normalize_content_markdown() {
		use std::path::PathBuf;
		let content = "# Header\n- item";
		let path = PathBuf::from("test.md");
		// For .md files, content should pass through unchanged
		assert_eq!(normalize_content_by_extension(content, &path).unwrap(), content);
	}

	#[test]
	fn test_normalize_content_typst() {
		use std::path::PathBuf;
		let content = "= Header\n- item";
		let path = PathBuf::from("test.typ");
		// For .typ files, should convert to markdown
		let result = normalize_content_by_extension(content, &path).unwrap();
		assert!(result.contains("# Header"));
		assert!(result.contains("- item"));
	}

	#[test]
	fn test_normalize_content_plain() {
		use std::path::PathBuf;
		let content = "plain text\nmore text";
		let path = PathBuf::from("test.txt");
		// For other extensions, content should pass through unchanged
		assert_eq!(normalize_content_by_extension(content, &path).unwrap(), content);
	}

	#[test]
	fn test_is_semantically_empty() {
		// Empty string is semantically empty
		assert!(is_semantically_empty(""));

		// Only whitespace is semantically empty
		assert!(is_semantically_empty("   \n\n  \n"));

		// Only comments is semantically empty
		assert!(is_semantically_empty("\tComment 1\n\tComment 2"));

		// Comments and whitespace is semantically empty
		assert!(is_semantically_empty("\tComment\n\n\tAnother comment\n"));

		// Any content makes it not empty
		assert!(!is_semantically_empty("- Task 1"));
		assert!(!is_semantically_empty("# Header"));
		assert!(!is_semantically_empty("\tComment\n- Task"));
		assert!(!is_semantically_empty("# Header\n\tComment"));
	}

	#[test]
	fn test_format_idempotent_with_same_level_headers_at_end() {
		// Bug: when opening and closing a file, we fail to add spaces between
		// the headers of the same level at the end
		let input = "- move these todos over into a persisted directory\n\tcomment\n- move all typst projects\n- rewrite custom.sh\n\tcomment\n\n# marketmonkey\n- go in-depth on possibilities\n\n# SocialNetworks in rust\n- test twitter\n\n## yt\n- test\n\n# math tools\n## gauss\n- finish it\n- move gaussian pivot over in there\n\n# git lfs: docs, music, etc\n# eww: don't restore if outdated\n# todo: blocker: doesn't add spaces between same level headers";

		// First format
		let formatted_once = format_blocker_content(input).unwrap();

		// Simulate file write and read (write doesn't add trailing newline, read doesn't care)
		// This is what happens in handle_background_blocker_check
		let formatted_twice = format_blocker_content(&formatted_once).unwrap();

		// Check that there are spaces between same-level headers at the end
		assert!(
			formatted_once.contains("# git lfs: docs, music, etc\n\n# eww: don't restore if outdated"),
			"Missing space between first two headers"
		);
		assert!(
			formatted_once.contains("# eww: don't restore if outdated\n\n# todo: blocker: doesn't add spaces between same level headers"),
			"Missing space between last two headers"
		);

		// Should be idempotent
		assert_eq!(formatted_once, formatted_twice, "Formatting should be idempotent");
	}
}
