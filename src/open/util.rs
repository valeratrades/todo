//! Small utility functions for issue processing.

pub use todo::Extension;

use crate::marker::Marker;

/// Check if a line is a blockers section marker.
/// Recognized formats (case-insensitive):
/// - `# Blockers` (preferred for .md, what `!b` expands to)
/// - `<!--blockers-->` (legacy, still supported)
/// - `#{1,6} Blockers` (any header level)
/// - `**Blockers**` (with optional trailing `:`)
/// - `// blockers` (typst, what `!b` expands to for .typ)
pub fn is_blockers_marker(line: &str) -> bool {
	// Use Marker enum for standard formats
	if matches!(Marker::decode(line, Extension::Md), Some(Marker::BlockersSection)) {
		return true;
	}
	// Also support **Blockers** format (not in Marker enum as it's non-standard)
	let lower = line.trim().to_ascii_lowercase();
	lower.starts_with("**blockers**")
}

/// Normalize issue content by converting space-based indentation to tab-based.
/// This ensures content edited in editors that convert tabs to spaces can be properly parsed.
///
/// The function detects the first non-marker indented line to determine the space-per-indent ratio,
/// then normalizes all indentation to tabs.
pub fn normalize_issue_indentation(content: &str) -> String {
	// Detect spaces-per-indent by finding the first indented line that starts with spaces
	let spaces_per_indent = content
		.lines()
		.find_map(|line| {
			if line.starts_with(' ') && !line.trim().is_empty() {
				// Count leading spaces
				let spaces = line.len() - line.trim_start_matches(' ').len();
				// Common indent sizes: 2, 3, 4, 8
				if spaces >= 2 {
					Some(spaces.min(8)) // Cap at 8 to avoid issues with very deep indents
				} else {
					None
				}
			} else {
				None
			}
		})
		.unwrap_or(4); // Default to 4 spaces per indent if we can't detect

	content
		.lines()
		.map(|line| {
			if line.is_empty() {
				return String::new();
			}

			// Count leading whitespace and convert to appropriate number of tabs
			let mut chars = line.chars().peekable();
			let mut space_count = 0;
			let mut tab_count = 0;

			while let Some(&ch) = chars.peek() {
				match ch {
					'\t' => {
						tab_count += 1;
						chars.next();
					}
					' ' => {
						space_count += 1;
						chars.next();
					}
					_ => break,
				}
			}

			// Convert spaces to tabs
			let extra_tabs = space_count / spaces_per_indent;
			let total_tabs = tab_count + extra_tabs;
			let remaining_spaces = space_count % spaces_per_indent;

			// Reconstruct the line with normalized indentation
			let rest: String = chars.collect();
			let mut result = "\t".repeat(total_tabs);
			// Keep any remaining spaces (less than one indent level)
			result.push_str(&" ".repeat(remaining_spaces));
			result.push_str(&rest);
			result
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// Extract title from a checkbox line if it matches the pattern `- [ ] Title` or `- [x] Title`
/// Returns the title without the checkbox prefix
pub fn extract_checkbox_title(line: &str) -> Option<String> {
	let trimmed = line.trim();
	if !trimmed.starts_with("- [") {
		return None;
	}
	// Match `- [ ] ` or `- [x] `
	let rest = trimmed.strip_prefix("- [ ] ").or_else(|| trimmed.strip_prefix("- [x] "))?;
	// Title is everything before any HTML comment marker
	let title = if let Some(idx) = rest.find("<!--") { rest[..idx].trim() } else { rest.trim() };
	if title.is_empty() { None } else { Some(title.to_string()) }
}

/// Expand `!b` shorthand to the full blockers marker.
/// Matches lines that are just `!b` or `!B` (with any indentation).
/// For .md files: expands to `# Blockers`
/// For .typ files: expands to `// blockers`
pub fn expand_blocker_shorthand(content: &str, extension: &Extension) -> String {
	let replacement = match extension {
		Extension::Md => "# Blockers",
		Extension::Typ => "// blockers",
	};

	content
		.lines()
		.map(|line| {
			let trimmed = line.trim();
			if trimmed.eq_ignore_ascii_case("!b") {
				// Preserve the original indentation
				let indent = &line[..line.len() - trimmed.len()];
				format!("{indent}{replacement}")
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// Convert markdown headers to typst format
pub fn convert_markdown_to_typst(body: &str) -> String {
	body.lines()
		.map(|line| {
			// Convert markdown headers to typst
			if let Some(rest) = line.strip_prefix("### ") {
				format!("=== {rest}")
			} else if let Some(rest) = line.strip_prefix("## ") {
				format!("== {rest}")
			} else if let Some(rest) = line.strip_prefix("# ") {
				format!("= {rest}")
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}
