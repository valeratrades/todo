use clap::ValueEnum;

pub mod issue;

// Re-export all public types from issue module at crate root for convenience
pub use issue::{
	BlockerItem, BlockerSequence, CloseState, Comment, CommentIdentity, DisplayFormat, FetchedIssue, HeaderLevel, Issue, IssueIdentity, IssueLink, IssueMeta, Line, Marker, ParseError,
	classify_line, is_blockers_marker, join_with_blockers, normalize_issue_indentation, split_blockers,
};

/// File extension/type for issue files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Extension {
	#[default]
	Md,
	Typ,
}

impl Extension {
	pub fn as_str(&self) -> &'static str {
		match self {
			Extension::Md => "md",
			Extension::Typ => "typ",
		}
	}
}

/// A header with a level and content, format-aware for serialization.
///
/// Markdown format: `# Content`, `## Content`, etc.
/// Typst format: `= Content`, `== Content`, etc.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Header {
	pub level: usize,
	pub content: String,
}

impl Header {
	/// Create a new header with the given level and content.
	/// Level must be >= 1.
	pub fn new(level: usize, content: impl Into<String>) -> Self {
		debug_assert!(level >= 1, "Header level must be >= 1");
		Self {
			level: level.max(1),
			content: content.into(),
		}
	}

	/// Decode a header from a line string based on the file extension.
	/// Returns None if the line is not a valid header.
	pub fn decode(s: &str, ext: Extension) -> Option<Self> {
		let trimmed = s.trim();

		match ext {
			Extension::Md => {
				// Markdown: # Content, ## Content, etc.
				if !trimmed.starts_with('#') {
					return None;
				}
				let mut level = 0;
				for ch in trimmed.chars() {
					if ch == '#' {
						level += 1;
					} else {
						break;
					}
				}
				// Valid header must have space after the # characters
				if level > 0 && trimmed.len() > level {
					let rest = &trimmed[level..];
					if let Some(stripped) = rest.strip_prefix(' ') {
						return Some(Self {
							level,
							content: stripped.to_string(),
						});
					}
				}
				None
			}
			Extension::Typ => {
				// Typst: = Content, == Content, etc.
				if !trimmed.starts_with('=') {
					return None;
				}
				let mut level = 0;
				for ch in trimmed.chars() {
					if ch == '=' {
						level += 1;
					} else {
						break;
					}
				}
				// Valid header must have space after the = characters
				if level > 0 && trimmed.len() > level {
					let rest = &trimmed[level..];
					if let Some(stripped) = rest.strip_prefix(' ') {
						return Some(Self {
							level,
							content: stripped.to_string(),
						});
					}
				}
				None
			}
		}
	}

	/// Encode the header to a string based on the file extension.
	pub fn encode(&self, ext: Extension) -> String {
		match ext {
			Extension::Md => format!("{} {}", "#".repeat(self.level), self.content),
			Extension::Typ => format!("{} {}", "=".repeat(self.level), self.content),
		}
	}

	/// Check if this header's content matches the given text (case-insensitive).
	pub fn content_eq_ignore_case(&self, text: &str) -> bool {
		self.content.eq_ignore_ascii_case(text)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_header_new() {
		let header = Header::new(2, "Test Content");
		assert_eq!(header.level, 2);
		assert_eq!(header.content, "Test Content");
	}

	#[test]
	fn test_header_decode_md() {
		// Basic markdown headers
		assert_eq!(
			Header::decode("# Heading 1", Extension::Md),
			Some(Header {
				level: 1,
				content: "Heading 1".to_string()
			})
		);
		assert_eq!(
			Header::decode("## Heading 2", Extension::Md),
			Some(Header {
				level: 2,
				content: "Heading 2".to_string()
			})
		);
		assert_eq!(
			Header::decode("### Heading 3", Extension::Md),
			Some(Header {
				level: 3,
				content: "Heading 3".to_string()
			})
		);

		// With leading/trailing whitespace
		assert_eq!(
			Header::decode("  # Trimmed  ", Extension::Md),
			Some(Header {
				level: 1,
				content: "Trimmed".to_string()
			})
		);

		// Invalid: no space after #
		assert_eq!(Header::decode("#NoSpace", Extension::Md), None);

		// Invalid: not a header
		assert_eq!(Header::decode("Just text", Extension::Md), None);
		assert_eq!(Header::decode("- List item", Extension::Md), None);
	}

	#[test]
	fn test_header_decode_typ() {
		// Basic typst headers
		assert_eq!(
			Header::decode("= Heading 1", Extension::Typ),
			Some(Header {
				level: 1,
				content: "Heading 1".to_string()
			})
		);
		assert_eq!(
			Header::decode("== Heading 2", Extension::Typ),
			Some(Header {
				level: 2,
				content: "Heading 2".to_string()
			})
		);
		assert_eq!(
			Header::decode("=== Heading 3", Extension::Typ),
			Some(Header {
				level: 3,
				content: "Heading 3".to_string()
			})
		);

		// With leading/trailing whitespace
		assert_eq!(
			Header::decode("  = Trimmed  ", Extension::Typ),
			Some(Header {
				level: 1,
				content: "Trimmed".to_string()
			})
		);

		// Invalid: no space after =
		assert_eq!(Header::decode("=NoSpace", Extension::Typ), None);

		// Invalid: not a header
		assert_eq!(Header::decode("Just text", Extension::Typ), None);
		assert_eq!(Header::decode("- List item", Extension::Typ), None);
	}

	#[test]
	fn test_header_encode_md() {
		assert_eq!(Header::new(1, "Test").encode(Extension::Md), "# Test");
		assert_eq!(Header::new(2, "Test").encode(Extension::Md), "## Test");
		assert_eq!(Header::new(3, "Test").encode(Extension::Md), "### Test");
	}

	#[test]
	fn test_header_encode_typ() {
		assert_eq!(Header::new(1, "Test").encode(Extension::Typ), "= Test");
		assert_eq!(Header::new(2, "Test").encode(Extension::Typ), "== Test");
		assert_eq!(Header::new(3, "Test").encode(Extension::Typ), "=== Test");
	}

	#[test]
	fn test_header_roundtrip() {
		// Markdown roundtrip
		for level in 1..=6 {
			let original = Header::new(level, "Content");
			let encoded = original.encode(Extension::Md);
			let decoded = Header::decode(&encoded, Extension::Md).unwrap();
			assert_eq!(original, decoded);
		}

		// Typst roundtrip
		for level in 1..=6 {
			let original = Header::new(level, "Content");
			let encoded = original.encode(Extension::Typ);
			let decoded = Header::decode(&encoded, Extension::Typ).unwrap();
			assert_eq!(original, decoded);
		}
	}

	#[test]
	fn test_header_content_eq_ignore_case() {
		let header = Header::new(1, "Blockers");
		assert!(header.content_eq_ignore_case("blockers"));
		assert!(header.content_eq_ignore_case("BLOCKERS"));
		assert!(header.content_eq_ignore_case("Blockers"));
		assert!(!header.content_eq_ignore_case("Blocker"));
	}

	#[test]
	fn test_header_cross_format_conversion() {
		// Decode from markdown, encode to typst
		let md_header = Header::decode("## Section", Extension::Md).unwrap();
		assert_eq!(md_header.encode(Extension::Typ), "== Section");

		// Decode from typst, encode to markdown
		let typ_header = Header::decode("=== Subsection", Extension::Typ).unwrap();
		assert_eq!(typ_header.encode(Extension::Md), "### Subsection");
	}
}
