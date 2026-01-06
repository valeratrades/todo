//! Core stack operations for blocker management.
//!
//! This module provides the fundamental operations on a blocker sequence:
//! - `add`: Push a new blocker onto the stack
//! - `pop`: Remove the last blocker from the stack
//! - `list`: Show all blockers with their headers
//! - `current`: Get the current (last) blocker with its parent context

use color_eyre::eyre::Result;

use super::standard::{LineType, classify_line, format_blocker_content, parse_parent_headers, strip_blocker_prefix};

/// A sequence of blocker content that can be manipulated.
/// This is the core data structure for blocker operations.
#[derive(Clone, Debug)]
pub struct BlockerSequence {
	content: String,
}

impl BlockerSequence {
	/// Create a new BlockerSequence from raw content
	pub fn new(content: String) -> Self {
		Self { content }
	}

	/// Create an empty BlockerSequence
	pub fn empty() -> Self {
		Self { content: String::new() }
	}

	/// Get the raw content
	pub fn content(&self) -> &str {
		&self.content
	}

	/// Take ownership of the content
	pub fn into_content(self) -> String {
		self.content
	}

	/// Get the current (last) blocker line, skipping comments
	pub fn current(&self) -> Option<String> {
		self.content
			.lines()
			.filter(|s| !s.is_empty())
			// Skip comment lines (tab-indented) - only consider content lines
			.filter(|s| !s.starts_with('\t'))
			.last()
			.map(|s| s.to_owned())
	}

	/// Get the current blocker with context prepended (joined by ": ").
	///
	/// `ownership_hierarchy` is a list of parent context items to prepend before the
	/// blocker's own headers. This could be workspace, project, issue title, etc.
	/// The caller is responsible for building this hierarchy based on their context.
	pub fn current_with_context(&self, ownership_hierarchy: &[String]) -> Option<String> {
		let current = self.current()?;
		let stripped = strip_blocker_prefix(&current);

		let parent_headers = parse_parent_headers(&self.content, &current);

		// Build final output: ownership hierarchy + blocker headers + task
		let mut parts: Vec<&str> = ownership_hierarchy.iter().map(|s| s.as_str()).collect();
		parts.extend(parent_headers.iter().map(|s| s.as_str()));

		if parts.is_empty() {
			Some(stripped.to_string())
		} else {
			Some(format!("{}: {stripped}", parts.join(": ")))
		}
	}

	/// Add a content line to the blocker sequence
	pub fn add(&mut self, new_line: &str) -> Result<()> {
		let mut lines: Vec<&str> = self.content.lines().collect();
		lines.push(new_line);
		self.content = format_blocker_content(&lines.join("\n"))?;
		Ok(())
	}

	/// Remove the last content line from the blocker sequence.
	/// Returns the removed line, if any.
	pub fn pop(&mut self) -> Result<Option<String>> {
		let lines: Vec<&str> = self.content.lines().collect();
		let mut content_lines_indices: Vec<usize> = Vec::new();

		// Find indices of all content lines (headers and items, not comments)
		for (idx, line) in lines.iter().enumerate() {
			if let Some(line_type) = classify_line(line)
				&& line_type.is_content()
			{
				content_lines_indices.push(idx);
			}
		}

		// Remove the last content line and its associated comments
		if let Some(&last_content_idx) = content_lines_indices.last() {
			let removed = lines[last_content_idx].to_string();

			// Keep lines before the last content block, exclude the last content line and its comments
			let new_lines: Vec<&str> = lines.iter().enumerate().filter(|(idx, _)| *idx < last_content_idx).map(|(_, line)| *line).collect();

			self.content = format_blocker_content(&new_lines.join("\n"))?;
			Ok(Some(removed))
		} else {
			// No content lines to remove
			self.content = format_blocker_content(&self.content)?;
			Ok(None)
		}
	}

	/// List all content lines (headers and items), returning tuples of (text, is_header)
	pub fn list(&self) -> Vec<(String, bool)> {
		let mut result = Vec::new();

		for line in self.content.lines() {
			// Skip empty lines and comments (tab-indented)
			if line.is_empty() || line.starts_with('\t') {
				continue;
			}

			let line_type = classify_line(line);
			match line_type {
				Some(LineType::Header { text, .. }) => {
					result.push((text, true));
				}
				Some(LineType::Item) => {
					let text = strip_blocker_prefix(line.trim());
					result.push((text.to_string(), false));
				}
				_ => {}
			}
		}

		result
	}

	/// Check if the sequence is empty (no content lines)
	pub fn is_empty(&self) -> bool {
		self.current().is_none()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_current() {
		let seq = BlockerSequence::new("- task 1\n- task 2\n- task 3".to_string());
		assert_eq!(seq.current(), Some("- task 3".to_string()));
	}

	#[test]
	fn test_current_skips_comments() {
		let seq = BlockerSequence::new("- task 1\n\tcomment\n- task 2\n\tanother comment".to_string());
		assert_eq!(seq.current(), Some("- task 2".to_string()));
	}

	#[test]
	fn test_current_with_context_no_hierarchy() {
		let seq = BlockerSequence::new("# Phase 1\n- task 1\n# Phase 2\n- task 2".to_string());
		assert_eq!(seq.current_with_context(&[]), Some("Phase 2: task 2".to_string()));
	}

	#[test]
	fn test_current_with_context_with_hierarchy() {
		let seq = BlockerSequence::new("# Phase 1\n- task 1".to_string());
		let hierarchy = vec!["project".to_string()];
		assert_eq!(seq.current_with_context(&hierarchy), Some("project: Phase 1: task 1".to_string()));
	}

	#[test]
	fn test_current_with_context_multi_level_hierarchy() {
		let seq = BlockerSequence::new("# Section\n- task".to_string());
		let hierarchy = vec!["workspace".to_string(), "project".to_string()];
		assert_eq!(seq.current_with_context(&hierarchy), Some("workspace: project: Section: task".to_string()));
	}

	#[test]
	fn test_add() {
		let mut seq = BlockerSequence::new("- task 1".to_string());
		seq.add("task 2").unwrap();
		assert_eq!(seq.content(), "- task 1\n- task 2");
	}

	#[test]
	fn test_pop() {
		let mut seq = BlockerSequence::new("- task 1\n- task 2".to_string());
		let popped = seq.pop().unwrap();
		assert_eq!(popped, Some("- task 2".to_string()));
		assert_eq!(seq.content(), "- task 1");
	}

	#[test]
	fn test_pop_removes_associated_comments() {
		let mut seq = BlockerSequence::new("- task 1\n\tcomment 1\n- task 2\n\tcomment 2".to_string());
		let popped = seq.pop().unwrap();
		assert_eq!(popped, Some("- task 2".to_string()));
		assert_eq!(seq.content(), "- task 1\n\tcomment 1");
	}

	#[test]
	fn test_pop_empty() {
		let mut seq = BlockerSequence::empty();
		let popped = seq.pop().unwrap();
		assert_eq!(popped, None);
	}

	#[test]
	fn test_list() {
		let seq = BlockerSequence::new("# Header 1\n- task 1\n# Header 2\n- task 2".to_string());
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
		let empty = BlockerSequence::empty();
		assert!(empty.is_empty());

		let with_content = BlockerSequence::new("- task".to_string());
		assert!(!with_content.is_empty());

		// Only comments is still empty
		let only_comments = BlockerSequence::new("\tcomment only".to_string());
		assert!(only_comments.is_empty());
	}
}
