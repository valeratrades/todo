//! Core stack operations for blocker management.
//!
//! This module provides the fundamental operations on a blocker sequence:
//! - `add`: Push a new blocker onto the stack
//! - `pop`: Remove the last blocker from the stack
//! - `list`: Show all blockers with their headers
//! - `current`: Get the current (last) blocker with its parent context
//!
//! # Hierarchy Model
//!
//! Blockers are organized in a tree structure where headers define sections.
//! Each header level creates a nested scope that "owns" its contents:
//! - H1 headers own everything until the next H1 (or end)
//! - H2 headers own everything until the next H2/H1 (or end)
//! - Items at the root level (before any header) belong to an implicit H1
//!
//! Two rendering modes are available:
//! - `headers`: Traditional flat format with `# Header` lines
//! - `nested`: Indented format showing hierarchy visually

use clap::ValueEnum;

use super::standard::{HeaderLevel, Line, classify_line};

/// Display format for blocker output
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum DisplayFormat {
	/// Traditional flat format with `# Header` lines
	#[default]
	Headers,
	/// Indented format showing hierarchy visually
	Nested,
}

/// An item in a blocker section
#[derive(Clone, Debug)]
pub struct BlockerItem {
	pub text: String,
	/// Comments attached to this item
	pub comments: Vec<String>,
}

/// A sequence of blocker lines that can be manipulated.
/// Internally represented as a tree, where each sequence can contain child sequences.
#[derive(Clone, Debug, Default)]
pub struct BlockerSequence {
	/// Header text (empty for root)
	pub title: String,
	/// Header level (0 for root, 1-5 for real headers)
	pub level: usize,
	/// Items directly under this sequence (not under any child header)
	pub items: Vec<BlockerItem>,
	/// Child sections (sub-headers)
	pub children: Vec<BlockerSequence>,
}

impl BlockerSequence {
	/// Create a new empty sequence with the given title and level
	pub fn new(title: impl Into<String>, level: usize) -> Self {
		Self {
			title: title.into(),
			level,
			items: Vec::new(),
			children: Vec::new(),
		}
	}

	/// Create an implicit root sequence (level 0)
	pub fn root() -> Self {
		Self::new("", 0)
	}

	/// Create a BlockerSequence from parsed lines
	pub fn from_lines(lines: Vec<Line>) -> Self {
		Self::build_from_lines(&lines)
	}

	/// Parse raw text content into a BlockerSequence
	pub fn parse(content: &str) -> Self {
		let lines: Vec<Line> = content.lines().filter_map(classify_line).collect();
		Self::build_from_lines(&lines)
	}

	/// Build tree from flat line list
	fn build_from_lines(lines: &[Line]) -> Self {
		let mut root = Self::root();
		let mut current_item_comments: Vec<String> = Vec::new();
		let mut pending_item: Option<String> = None;

		// Stack of levels to track where we are
		let mut level_stack: Vec<usize> = vec![0]; // Start at root level

		for line in lines {
			match line {
				Line::Header { level, text } => {
					// Flush pending item
					if let Some(item_text) = pending_item.take() {
						let item = BlockerItem {
							text: item_text,
							comments: std::mem::take(&mut current_item_comments),
						};
						Self::add_item_to_tree(&mut root, &level_stack, item);
					}

					let lvl = level.to_usize();

					// Pop levels that are >= this header's level
					while level_stack.last().is_some_and(|&l| l >= lvl) {
						level_stack.pop();
					}

					// Add new sequence at appropriate position
					let new_seq = Self::new(text.clone(), lvl);
					Self::add_child_to_tree(&mut root, &level_stack, new_seq);

					// Push this level
					level_stack.push(lvl);
				}
				Line::Item(text) => {
					// Flush pending item first
					if let Some(item_text) = pending_item.take() {
						let item = BlockerItem {
							text: item_text,
							comments: std::mem::take(&mut current_item_comments),
						};
						Self::add_item_to_tree(&mut root, &level_stack, item);
					}
					pending_item = Some(text.clone());
				}
				Line::Comment(text) => {
					// Comments belong to the pending item
					current_item_comments.push(text.clone());
				}
			}
		}

		// Flush final pending item
		if let Some(item_text) = pending_item.take() {
			let item = BlockerItem {
				text: item_text,
				comments: current_item_comments,
			};
			Self::add_item_to_tree(&mut root, &level_stack, item);
		}

		root
	}

	/// Add a child sequence to the tree at the position indicated by the level stack
	fn add_child_to_tree(root: &mut Self, level_stack: &[usize], child: Self) {
		let mut current = root;

		for &lvl in level_stack.iter().skip(1) {
			let idx = current.children.iter().rposition(|c| c.level == lvl);
			if let Some(i) = idx {
				current = &mut current.children[i];
			} else {
				break;
			}
		}

		current.children.push(child);
	}

	/// Add an item to the tree at the position indicated by the level stack
	fn add_item_to_tree(root: &mut Self, level_stack: &[usize], item: BlockerItem) {
		let mut current = root;

		for &lvl in level_stack.iter().skip(1) {
			if let Some(idx) = current.children.iter().rposition(|c| c.level == lvl) {
				current = &mut current.children[idx];
			} else {
				break;
			}
		}

		current.items.push(item);
	}

	/// Check if this sequence has any content (items or children with content)
	pub fn is_empty(&self) -> bool {
		self.items.is_empty() && self.children.iter().all(|c| c.is_empty())
	}

	/// Get the number of items in the sequence (recursive)
	pub fn len(&self) -> usize {
		let own_count = self.items.len();
		let child_count: usize = self.children.iter().map(|c| c.len()).sum();
		own_count + child_count
	}

	/// Get the last item in the tree (depth-first, rightmost)
	fn last_item(&self) -> Option<&BlockerItem> {
		// Check children first (rightmost child's last item)
		for child in self.children.iter().rev() {
			if let Some(item) = child.last_item() {
				return Some(item);
			}
		}
		// Then check our own items
		self.items.last()
	}

	/// Get the path of headers leading to the last item
	fn path_to_last(&self) -> Vec<&str> {
		let mut path = Vec::new();
		self.path_to_last_inner(&mut path);
		path
	}

	fn path_to_last_inner<'a>(&'a self, path: &mut Vec<&'a str>) {
		// Check if any child has content
		for child in self.children.iter().rev() {
			if !child.is_empty() {
				if !self.title.is_empty() {
					path.push(&self.title);
				}
				child.path_to_last_inner(path);
				return;
			}
		}
		// If we have items, add our title
		if !self.items.is_empty() && !self.title.is_empty() {
			path.push(&self.title);
		}
	}

	/// Render to flat format with headers (traditional)
	fn render_headers_vec(&self) -> Vec<Line> {
		let mut lines = Vec::new();
		self.render_headers_inner(&mut lines);
		lines
	}

	fn render_headers_inner(&self, lines: &mut Vec<Line>) {
		// Output header if this is not the root
		if self.level > 0 {
			if let Some(header_level) = HeaderLevel::from_usize(self.level) {
				lines.push(Line::Header {
					level: header_level,
					text: self.title.clone(),
				});
			}
		}

		// Output items
		for item in &self.items {
			lines.push(Line::Item(item.text.clone()));
			for comment in &item.comments {
				lines.push(Line::Comment(comment.clone()));
			}
		}

		// Output children
		for child in &self.children {
			child.render_headers_inner(lines);
		}
	}

	/// Render to nested format with indentation
	fn render_nested_vec(&self) -> Vec<String> {
		let mut lines = Vec::new();
		self.render_nested_inner(&mut lines, 0);
		lines
	}

	fn render_nested_inner(&self, lines: &mut Vec<String>, indent: usize) {
		let indent_str = "\t".repeat(indent);

		// Output title if this is not the root
		if self.level > 0 && !self.title.is_empty() {
			lines.push(format!("{indent_str}{}", self.title));
		}

		// Output items (indented one more level if we have a title)
		let item_indent = if self.level > 0 { indent + 1 } else { indent };
		let item_indent_str = "\t".repeat(item_indent);

		for item in &self.items {
			lines.push(format!("{item_indent_str}- {}", item.text));
			for comment in &item.comments {
				lines.push(format!("{item_indent_str}\t{comment}"));
			}
		}

		// Output children
		let child_indent = if self.level > 0 { indent + 1 } else { indent };
		for child in &self.children {
			child.render_nested_inner(lines, child_indent);
		}
	}

	/// Pop the last item from the tree
	fn pop_last(&mut self) -> Option<BlockerItem> {
		// Try children first (rightmost)
		for child in self.children.iter_mut().rev() {
			if let Some(item) = child.pop_last() {
				return Some(item);
			}
		}
		// Then our own items
		self.items.pop()
	}

	/// Serialize to raw text format (headers mode)
	pub fn serialize(&self) -> String {
		self.render_headers_vec().iter().map(|l| l.to_raw()).collect::<Vec<_>>().join("\n")
	}

	/// Get the lines in headers format
	pub fn lines(&self) -> Vec<Line> {
		self.render_headers_vec()
	}

	/// Render in the specified format
	pub fn render(&self, format: DisplayFormat) -> String {
		match format {
			DisplayFormat::Headers => self.serialize(),
			DisplayFormat::Nested => self.render_nested_vec().join("\n"),
		}
	}

	/// Get the current (last) blocker item
	pub fn current(&self) -> Option<&BlockerItem> {
		self.last_item()
	}

	/// Get the current blocker as a raw string (for caching/comparison)
	pub fn current_raw(&self) -> Option<String> {
		self.current().map(|item| format!("- {}", item.text))
	}

	/// Get the current blocker with context prepended (joined by ": ").
	///
	/// `ownership_hierarchy` is a list of parent context items to prepend before the
	/// blocker's own headers. This could be workspace, project, issue title, etc.
	pub fn current_with_context(&self, ownership_hierarchy: &[String]) -> Option<String> {
		let current = self.current()?;

		// Get path of headers to the current item
		let path = self.path_to_last();

		// Build final output: ownership hierarchy + blocker headers + task
		let mut parts: Vec<&str> = ownership_hierarchy.iter().map(|s| s.as_str()).collect();
		parts.extend(path);

		if parts.is_empty() {
			Some(current.text.clone())
		} else {
			Some(format!("{}: {}", parts.join(": "), current.text))
		}
	}

	/// Add a content line to the blocker sequence (at current position)
	pub fn add(&mut self, text: &str) {
		let item = BlockerItem {
			text: text.to_string(),
			comments: Vec::new(),
		};
		// Add to the deepest current section
		self.add_item_to_current(item);
	}

	fn add_item_to_current(&mut self, item: BlockerItem) {
		// Find the deepest non-empty section and add there
		// If all empty, add to root
		fn add_to_deepest(seq: &mut BlockerSequence, item: BlockerItem) -> bool {
			// Try children first (rightmost)
			for child in seq.children.iter_mut().rev() {
				if add_to_deepest(child, item.clone()) {
					return true;
				}
			}
			// If this sequence has items or is non-root, add here
			if !seq.items.is_empty() || seq.level > 0 {
				seq.items.push(item);
				return true;
			}
			false
		}

		if !add_to_deepest(self, item.clone()) {
			// Nothing found, add to root
			self.items.push(item);
		}
	}

	/// Remove the last content line from the blocker sequence.
	/// Returns the removed item text, if any.
	pub fn pop(&mut self) -> Option<String> {
		self.pop_last().map(|item| item.text)
	}

	/// List all content items with their header paths
	pub fn list(&self) -> Vec<(String, bool)> {
		let lines = self.render_headers_vec();
		lines
			.iter()
			.filter_map(|line| match line {
				Line::Header { text, .. } => Some((text.clone(), true)),
				Line::Item(text) => Some((text.clone(), false)),
				Line::Comment(_) => None,
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_and_serialize() {
		let content = "# Header\n- task 1\n\tcomment\n- task 2";
		let seq = BlockerSequence::parse(content);
		assert_eq!(seq.serialize(), content);
	}

	#[test]
	fn test_current() {
		let seq = BlockerSequence::parse("- task 1\n- task 2\n- task 3");
		assert_eq!(seq.current().map(|i| i.text.as_str()), Some("task 3"));
	}

	#[test]
	fn test_current_skips_comments() {
		let seq = BlockerSequence::parse("- task 1\n\tcomment\n- task 2\n\tanother comment");
		assert_eq!(seq.current().map(|i| i.text.as_str()), Some("task 2"));
		// Comments should be attached to the item
		assert_eq!(seq.current().map(|i| i.comments.len()), Some(1));
	}

	#[test]
	fn test_current_with_context_no_hierarchy() {
		let seq = BlockerSequence::parse("# Phase 1\n- task 1\n# Phase 2\n- task 2");
		assert_eq!(seq.current_with_context(&[]), Some("Phase 2: task 2".to_string()));
	}

	#[test]
	fn test_current_with_context_with_hierarchy() {
		let seq = BlockerSequence::parse("# Phase 1\n- task 1");
		let hierarchy = vec!["project".to_string()];
		assert_eq!(seq.current_with_context(&hierarchy), Some("project: Phase 1: task 1".to_string()));
	}

	#[test]
	fn test_current_with_context_multi_level_hierarchy() {
		let seq = BlockerSequence::parse("# Section\n- task");
		let hierarchy = vec!["workspace".to_string(), "project".to_string()];
		assert_eq!(seq.current_with_context(&hierarchy), Some("workspace: project: Section: task".to_string()));
	}

	#[test]
	fn test_nested_headers() {
		let content = "# H1\n## H2\n- task under H2\n# Another H1\n- task under another H1";
		let seq = BlockerSequence::parse(content);

		// Current should be "task under another H1" with path "Another H1"
		assert_eq!(seq.current_with_context(&[]), Some("Another H1: task under another H1".to_string()));
	}

	#[test]
	fn test_deeply_nested() {
		let content = "# Level 1\n## Level 2\n### Level 3\n- deep task";
		let seq = BlockerSequence::parse(content);

		assert_eq!(seq.current_with_context(&[]), Some("Level 1: Level 2: Level 3: deep task".to_string()));
	}

	#[test]
	fn test_add() {
		let mut seq = BlockerSequence::parse("- task 1");
		seq.add("task 2");
		assert_eq!(seq.serialize(), "- task 1\n- task 2");
	}

	#[test]
	fn test_add_to_section() {
		let mut seq = BlockerSequence::parse("# Section\n- task 1");
		seq.add("task 2");
		// Should add under the same section
		assert_eq!(seq.serialize(), "# Section\n- task 1\n- task 2");
	}

	#[test]
	fn test_pop() {
		let mut seq = BlockerSequence::parse("- task 1\n- task 2");
		let popped = seq.pop();
		assert_eq!(popped, Some("task 2".to_string()));
		assert_eq!(seq.serialize(), "- task 1");
	}

	#[test]
	fn test_pop_from_section() {
		let mut seq = BlockerSequence::parse("# Section\n- task 1\n- task 2");
		let popped = seq.pop();
		assert_eq!(popped, Some("task 2".to_string()));
		assert_eq!(seq.serialize(), "# Section\n- task 1");
	}

	#[test]
	fn test_pop_empty() {
		let mut seq = BlockerSequence::default();
		let popped = seq.pop();
		assert!(popped.is_none());
	}

	#[test]
	fn test_list() {
		let seq = BlockerSequence::parse("# Header 1\n- task 1\n# Header 2\n- task 2");
		let list = seq.list();
		assert_eq!(
			list,
			vec![
				("Header 1".to_string(), true),
				("task 1".to_string(), false),
				("Header 2".to_string(), true),
				("task 2".to_string(), false),
			]
		);
	}

	#[test]
	fn test_is_empty() {
		let empty = BlockerSequence::default();
		assert!(empty.is_empty());

		let with_content = BlockerSequence::parse("- task");
		assert!(!with_content.is_empty());

		// Only comments - the tree won't have items because comments need items
		let only_header = BlockerSequence::parse("# Just a header");
		assert!(only_header.is_empty()); // Headers without items are empty
	}

	#[test]
	fn test_render_nested() {
		let seq = BlockerSequence::parse("# Section A\n- task 1\n## Subsection\n- task 2\n# Section B\n- task 3");

		let nested = seq.render(DisplayFormat::Nested);
		let expected = "Section A\n\t- task 1\n\tSubsection\n\t\t- task 2\nSection B\n\t- task 3";
		assert_eq!(nested, expected);
	}

	#[test]
	fn test_render_headers() {
		let content = "# Section A\n- task 1\n## Subsection\n- task 2";
		let seq = BlockerSequence::parse(content);

		let headers = seq.render(DisplayFormat::Headers);
		assert_eq!(headers, content);
	}

	#[test]
	fn test_items_before_first_header() {
		let content = "- root task\n# Section\n- section task";
		let seq = BlockerSequence::parse(content);

		// Should serialize back correctly
		assert_eq!(seq.serialize(), content);

		// Current should be the section task
		assert_eq!(seq.current_with_context(&[]), Some("Section: section task".to_string()));
	}

	#[test]
	fn test_multiple_h1_sections() {
		let content = "# A\n- task a\n# B\n- task b\n# C\n- task c";
		let seq = BlockerSequence::parse(content);

		assert_eq!(seq.current_with_context(&[]), Some("C: task c".to_string()));

		// Pop should remove from C
		let mut seq = seq;
		seq.pop();
		assert_eq!(seq.current_with_context(&[]), Some("B: task b".to_string()));
	}

	#[test]
	fn test_comments_preserved() {
		let content = "# Section\n- task 1\n\tcomment 1\n\tcomment 2\n- task 2";
		let seq = BlockerSequence::parse(content);

		assert_eq!(seq.serialize(), content);
	}
}
