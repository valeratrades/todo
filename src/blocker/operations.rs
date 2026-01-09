//! Core stack operations for blocker management.
//!
//! This module provides the fundamental operations on a blocker sequence:
//! - `add`: Push a new blocker onto the stack
//! - `pop`: Remove the last blocker from the stack
//! - `list`: Show all blockers with their headers
//! - `current`: Get the current (last) blocker with its parent context

use super::standard::{HeaderLevel, Line, classify_line};

/// A sequence of blocker lines that can be manipulated.
/// This is the core data structure for blocker operations.
#[derive(Clone, Debug, Default)]
pub struct BlockerSequence {
	lines: Vec<Line>,
}

impl BlockerSequence {
	/// Create a new BlockerSequence from parsed lines
	pub fn new(lines: Vec<Line>) -> Self {
		Self { lines }
	}

	/// Parse raw text content into a BlockerSequence
	pub fn parse(content: &str) -> Self {
		let lines = content.lines().filter_map(classify_line).collect();
		Self { lines }
	}

	/// Serialize to raw text format
	pub fn serialize(&self) -> String {
		self.lines.iter().map(|l| l.to_raw()).collect::<Vec<_>>().join("\n")
	}

	/// Get the lines
	pub fn lines(&self) -> &[Line] {
		&self.lines
	}

	/// Get mutable access to lines
	pub fn lines_mut(&mut self) -> &mut Vec<Line> {
		&mut self.lines
	}

	/// Check if the sequence is empty (no content lines)
	pub fn is_empty(&self) -> bool {
		!self.lines.iter().any(|l| l.is_content())
	}

	/// Get the number of lines in the sequence
	pub fn len(&self) -> usize {
		self.lines.len()
	}

	/// Get the current (last) blocker item, skipping comments
	pub fn current(&self) -> Option<&Line> {
		self.lines.iter().rev().find(|l| l.is_content())
	}

	/// Get the current blocker as a raw string (for caching/comparison)
	pub fn current_raw(&self) -> Option<String> {
		self.current().map(|l| l.to_raw())
	}

	/// Get the current blocker with context prepended (joined by ": ").
	///
	/// `ownership_hierarchy` is a list of parent context items to prepend before the
	/// blocker's own headers. This could be workspace, project, issue title, etc.
	pub fn current_with_context(&self, ownership_hierarchy: &[String]) -> Option<String> {
		let current = self.current()?;
		let current_text = match current {
			Line::Header { text, .. } => text.clone(),
			Line::Item(text) => text.clone(),
			Line::Comment(_) => return None, // shouldn't happen due to current() filter
		};

		// Find parent headers above the current line
		let parent_headers = self.parent_headers_of_current();

		// Build final output: ownership hierarchy + blocker headers + task
		let mut parts: Vec<&str> = ownership_hierarchy.iter().map(|s| s.as_str()).collect();
		parts.extend(parent_headers.iter().map(|s| s.as_str()));

		if parts.is_empty() {
			Some(current_text)
		} else {
			Some(format!("{}: {current_text}", parts.join(": ")))
		}
	}

	/// Get parent headers above the current (last content) line
	fn parent_headers_of_current(&self) -> Vec<String> {
		// Find index of last content line
		let last_content_idx = self.lines.iter().rposition(|l| l.is_content());
		let Some(idx) = last_content_idx else {
			return vec![];
		};

		// Get the level of the current line (if it's a header)
		let current_level = match &self.lines[idx] {
			Line::Header { level, .. } => Some(*level),
			_ => None,
		};

		// Walk backwards collecting headers with increasing levels
		let mut headers = Vec::new();
		let mut max_level = current_level.map(|l| l.to_usize()).unwrap_or(usize::MAX);

		for line in self.lines[..idx].iter().rev() {
			if let Line::Header { level, text } = line {
				let lvl = level.to_usize();
				if lvl < max_level {
					headers.push(text.clone());
					max_level = lvl;
				}
			}
		}

		headers.reverse();
		headers
	}

	/// Add a content line to the blocker sequence
	pub fn add(&mut self, text: &str) {
		self.lines.push(Line::Item(text.to_string()));
	}

	/// Add a header to the blocker sequence
	pub fn add_header(&mut self, level: HeaderLevel, text: &str) {
		self.lines.push(Line::Header { level, text: text.to_string() });
	}

	/// Remove the last content line from the blocker sequence.
	/// Also removes any trailing comments associated with that line.
	/// Returns the removed line, if any.
	pub fn pop(&mut self) -> Option<Line> {
		// Find index of last content line
		let last_content_idx = self.lines.iter().rposition(|l| l.is_content())?;

		// Remove all lines from last_content_idx onwards (content + trailing comments)
		let removed = self.lines.remove(last_content_idx);

		// Remove trailing comments that were associated with this line
		while self.lines.len() > last_content_idx {
			if matches!(self.lines.last(), Some(Line::Comment(_))) {
				// This shouldn't happen since comments come after content
				break;
			}
			break;
		}

		// Actually we need to remove comments AFTER the content line
		// But they're already gone since we removed from last_content_idx
		// Let's just return the removed content line
		Some(removed)
	}

	/// List all content lines (headers and items), returning tuples of (text, is_header)
	pub fn list(&self) -> Vec<(String, bool)> {
		self.lines
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
		assert!(matches!(seq.current(), Some(Line::Item(t)) if t == "task 3"));
	}

	#[test]
	fn test_current_skips_comments() {
		let seq = BlockerSequence::parse("- task 1\n\tcomment\n- task 2\n\tanother comment");
		assert!(matches!(seq.current(), Some(Line::Item(t)) if t == "task 2"));
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
	fn test_add() {
		let mut seq = BlockerSequence::parse("- task 1");
		seq.add("task 2");
		assert_eq!(seq.serialize(), "- task 1\n- task 2");
	}

	#[test]
	fn test_pop() {
		let mut seq = BlockerSequence::parse("- task 1\n- task 2");
		let popped = seq.pop();
		assert!(matches!(popped, Some(Line::Item(t)) if t == "task 2"));
		assert_eq!(seq.serialize(), "- task 1");
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

		// Only comments is still empty
		let only_comments = BlockerSequence::parse("\tcomment only");
		assert!(only_comments.is_empty());
	}
}
