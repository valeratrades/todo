//! Small utility functions for issue processing.

pub use todo::{Extension, Header};

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
/// For .typ files: expands to `= Blockers`
pub fn expand_blocker_shorthand(content: &str, extension: &Extension) -> String {
	let blockers_header = Header::new(1, "Blockers");
	let replacement = blockers_header.encode(*extension);

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
			// Try to decode as markdown header and re-encode as typst
			if let Some(header) = Header::decode(line, Extension::Md) {
				header.encode(Extension::Typ)
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}
