//! Pure blocker types for issue files.
//!
//! This module contains the core data structures for blockers without I/O dependencies.
//! These types can be used in both the library and binary contexts.

/// Header level (1-5), where 1 is the highest/largest.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HeaderLevel {
	One,
	Two,
	Three,
	Four,
	Five,
}

impl HeaderLevel {
	/// Get the numeric level (1-5)
	pub fn to_usize(self) -> usize {
		match self {
			HeaderLevel::One => 1,
			HeaderLevel::Two => 2,
			HeaderLevel::Three => 3,
			HeaderLevel::Four => 4,
			HeaderLevel::Five => 5,
		}
	}

	/// Create from numeric level (1-5)
	pub fn from_usize(level: usize) -> Option<Self> {
		match level {
			1 => Some(HeaderLevel::One),
			2 => Some(HeaderLevel::Two),
			3 => Some(HeaderLevel::Three),
			4 => Some(HeaderLevel::Four),
			5 => Some(HeaderLevel::Five),
			_ => None,
		}
	}
}

/// A parsed line in a blocker file.
/// This is the boundary format between raw text and structured blocker data.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
	/// Header with level and text (without # prefix)
	Header { level: HeaderLevel, text: String },
	/// List item content (without - prefix)
	Item(String),
	/// Comment line - tab-indented explanatory text (without leading tab)
	Comment(String),
}

impl Line {
	/// Check if this line is a header
	pub fn is_header(&self) -> bool {
		matches!(self, Line::Header { .. })
	}

	/// Check if this line contributes to the blocker list (headers and items)
	pub fn is_content(&self) -> bool {
		!matches!(self, Line::Comment(_))
	}

	/// Serialize to raw text format
	pub fn to_raw(&self) -> String {
		match self {
			Line::Header { level, text } => format!("{} {text}", "#".repeat(level.to_usize())),
			Line::Item(text) => format!("- {text}"),
			Line::Comment(text) => format!("\t{text}"),
		}
	}
}

/// Parse a line into a structured Line.
/// - Lines starting with tab are Comments (content without leading tab)
/// - Lines starting with 2+ spaces (likely editor-converted tabs) are Comments
/// - Lines starting with # are Headers (levels 1-5, text without # prefix)
/// - Lines starting with - are Items (content without - prefix)
/// - All other non-empty lines are Items (raw content)
/// - Returns None for empty lines
pub fn classify_line(line: &str) -> Option<Line> {
	if line.is_empty() {
		return None;
	}

	// Comment: tab-indented
	if let Some(content) = line.strip_prefix('\t') {
		return Some(Line::Comment(content.to_string()));
	}

	// Comment: 2+ spaces (likely editor tab-to-space conversion)
	// But not if it looks like an indented list item
	if line.starts_with("  ") && !line.trim_start().starts_with('-') {
		let content = line.trim_start();
		return Some(Line::Comment(content.to_string()));
	}

	let trimmed = line.trim();

	// Header: # with space after
	if trimmed.starts_with('#') {
		let mut count = 0;
		for ch in trimmed.chars() {
			if ch == '#' {
				count += 1;
			} else {
				break;
			}
		}

		// Valid header must have space after the # characters
		if count > 0 && trimmed.len() > count {
			let next_char = trimmed.chars().nth(count);
			if next_char == Some(' ') {
				let text = trimmed[count + 1..].to_string();

				// Warn if header is nested too deeply (level > 5)
				if count > 5 {
					eprintln!("Warning: Header level {count} is too deep (max 5 supported). Treating as regular item: {trimmed}");
					return Some(Line::Item(trimmed.to_string()));
				}

				if let Some(level) = HeaderLevel::from_usize(count) {
					return Some(Line::Header { level, text });
				}
			}
		}
	}

	// Item: strip - prefix if present
	let content = trimmed.strip_prefix("- ").unwrap_or(trimmed);
	Some(Line::Item(content.to_string()))
}

/// A single blocker item with optional comments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockerItem {
	pub text: String,
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
				comments: std::mem::take(&mut current_item_comments),
			};
			Self::add_item_to_tree(&mut root, &level_stack, item);
		}

		root
	}

	/// Add an item to the correct position in the tree based on level stack
	fn add_item_to_tree(root: &mut BlockerSequence, level_stack: &[usize], item: BlockerItem) {
		let target = Self::get_sequence_for_level(root, level_stack);
		target.items.push(item);
	}

	/// Add a child sequence to the correct position in the tree
	fn add_child_to_tree(root: &mut BlockerSequence, level_stack: &[usize], seq: BlockerSequence) {
		let target = Self::get_sequence_for_level(root, level_stack);
		target.children.push(seq);
	}

	/// Navigate to the correct sequence based on level stack
	fn get_sequence_for_level<'a>(root: &'a mut BlockerSequence, level_stack: &[usize]) -> &'a mut BlockerSequence {
		let mut current = root;

		// Skip the root level (0) in the stack
		for &level in level_stack.iter().skip(1) {
			// Find the child with this level (it should be the last one added at this level)
			let child_idx = current.children.iter().rposition(|c| c.level == level);
			if let Some(idx) = child_idx {
				current = &mut current.children[idx];
			} else {
				// Should not happen if build_from_lines is correct
				break;
			}
		}

		current
	}

	/// Check if the sequence has no blocker items (headers without items count as empty)
	pub fn is_empty(&self) -> bool {
		self.items.is_empty() && self.children.iter().all(|c| c.is_empty())
	}

	/// Serialize to raw text lines
	pub fn serialize(&self) -> String {
		let mut lines = Vec::new();
		self.serialize_into(&mut lines);
		lines.join("\n")
	}

	/// Internal recursive serialization
	fn serialize_into(&self, lines: &mut Vec<String>) {
		// Items first
		for item in &self.items {
			lines.push(format!("- {}", item.text));
			for comment in &item.comments {
				lines.push(format!("\t{comment}"));
			}
		}

		// Then children
		for child in &self.children {
			// Add header
			if child.level > 0 {
				lines.push(format!("{} {}", "#".repeat(child.level), child.title));
			}
			child.serialize_into(lines);
		}
	}

	/// Get all lines in order (for iteration)
	pub fn lines(&self) -> Vec<Line> {
		let mut result = Vec::new();
		self.collect_lines(&mut result);
		result
	}

	/// Internal recursive line collection
	fn collect_lines(&self, result: &mut Vec<Line>) {
		// Items first
		for item in &self.items {
			result.push(Line::Item(item.text.clone()));
			for comment in &item.comments {
				result.push(Line::Comment(comment.clone()));
			}
		}

		// Then children
		for child in &self.children {
			if child.level > 0 {
				if let Some(level) = HeaderLevel::from_usize(child.level) {
					result.push(Line::Header { level, text: child.title.clone() });
				}
			}
			child.collect_lines(result);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_classify_line() {
		assert_eq!(classify_line(""), None);
		assert_eq!(classify_line("\tComment"), Some(Line::Comment("Comment".to_string())));
		assert_eq!(classify_line("Content"), Some(Line::Item("Content".to_string())));
		assert_eq!(classify_line("  Spaces"), Some(Line::Comment("Spaces".to_string())));
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
	}

	#[test]
	fn test_blocker_sequence_parse() {
		let content = "# Section\n- item 1\n\tcomment\n- item 2";
		let seq = BlockerSequence::parse(content);

		assert_eq!(seq.children.len(), 1);
		assert_eq!(seq.children[0].title, "Section");
		assert_eq!(seq.children[0].items.len(), 2);
		assert_eq!(seq.children[0].items[0].text, "item 1");
		assert_eq!(seq.children[0].items[0].comments, vec!["comment"]);
	}

	#[test]
	fn test_blocker_sequence_serialize() {
		let content = "- item 1\n\tcomment\n# Section\n- item 2";
		let seq = BlockerSequence::parse(content);
		let serialized = seq.serialize();

		assert!(serialized.contains("- item 1"));
		assert!(serialized.contains("\tcomment"));
		assert!(serialized.contains("# Section"));
		assert!(serialized.contains("- item 2"));
	}
}
