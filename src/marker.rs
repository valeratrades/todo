//! Marker types for issue file format.
//!
//! Markers are HTML comments or special syntax that encode metadata in issue files.
//! This module provides decoding and encoding for all marker types, ensuring
//! consistent handling of whitespace and formatting.

use std::fmt;

use todo::Extension;

/// A marker that can appear in issue files.
/// All markers normalize whitespace on decode and encode with consistent spacing.
#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
	/// Issue URL marker: `<!-- url -->` or `<!--immutable url -->`
	IssueUrl { url: String, immutable: bool },
	/// Sub-issue marker: `<!--sub url -->`
	SubIssue { url: String },
	/// Comment marker: `<!-- url#issuecomment-123 -->` or `<!--immutable url#issuecomment-123 -->`
	Comment { url: String, id: u64, immutable: bool },
	/// New comment marker: `<!-- new comment -->`
	NewComment,
	/// Blockers section marker: `# Blockers` (md) or `// blockers` (typst)
	/// Must be the entire line content (after stripping leading whitespace).
	BlockersSection,
	/// Omitted content marker: `<!-- omitted -->`
	Omitted,
	/// Omitted with hint: `<!-- omitted (use --render-closed to unfold) -->`
	OmittedWithHint,
}

impl Marker {
	/// Decode a marker from a string.
	/// For BlockersSection, the entire line (after trimming) must match.
	/// For other markers, decodes the HTML comment content.
	/// Returns None if the string doesn't contain a recognized marker.
	pub fn decode(s: &str, _ext: Extension) -> Option<Self> {
		let trimmed = s.trim();

		// Check for markdown header blockers marker: # Blockers, ## Blockers, etc.
		// Must be the entire line content
		if let Some(rest) = trimmed.to_ascii_lowercase().strip_prefix('#') {
			let rest = rest.trim_start_matches('#');
			if rest.trim().trim_end_matches(':') == "blockers" {
				return Some(Marker::BlockersSection);
			}
		}

		// Check for typst blockers marker - must be exact match
		if trimmed.eq_ignore_ascii_case("// blockers") || trimmed.eq_ignore_ascii_case("// blocker") {
			return Some(Marker::BlockersSection);
		}

		// Check for HTML comment markers
		if !trimmed.starts_with("<!--") || !trimmed.ends_with("-->") {
			return None;
		}

		// Extract inner content, normalizing whitespace
		let inner = trimmed.strip_prefix("<!--")?.strip_suffix("-->")?.trim();
		let lower = inner.to_ascii_lowercase();

		// Blockers (legacy) - entire comment must be just "blockers"
		if lower == "blockers" || lower == "blocker" {
			return Some(Marker::BlockersSection);
		}

		// New comment
		if lower == "new comment" {
			return Some(Marker::NewComment);
		}

		// Omitted variants
		if lower == "omitted" {
			return Some(Marker::Omitted);
		}
		if lower.starts_with("omitted") && lower.contains("render-closed") {
			return Some(Marker::OmittedWithHint);
		}

		// Check for immutable prefix
		let (immutable, rest) = if let Some(rest) = inner.strip_prefix("immutable ").or_else(|| inner.strip_prefix("immutable\t")) {
			(true, rest.trim())
		} else {
			(false, inner)
		};

		// Sub-issue marker
		if let Some(url) = rest.strip_prefix("sub ").or_else(|| rest.strip_prefix("sub\t")) {
			return Some(Marker::SubIssue { url: url.trim().to_string() });
		}

		// Comment marker (contains #issuecomment-)
		if rest.contains("#issuecomment-") {
			let id = rest.split("#issuecomment-").nth(1).and_then(|s| {
				// Take only digits
				let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
				digits.parse().ok()
			})?;
			return Some(Marker::Comment {
				url: rest.to_string(),
				id,
				immutable,
			});
		}

		// Issue URL marker (anything else is treated as a URL)
		if !rest.is_empty() {
			return Some(Marker::IssueUrl { url: rest.to_string(), immutable });
		}

		None
	}

	/// Encode the marker to a string with consistent formatting.
	/// Uses `<!-- content -->` format for markdown, `// content` for typst.
	pub fn encode(&self, ext: Extension) -> String {
		match ext {
			Extension::Md => match self {
				Marker::IssueUrl { url, immutable } =>
					if *immutable {
						format!("<!--immutable {url} -->")
					} else {
						format!("<!-- {url} -->")
					},
				Marker::SubIssue { url } => format!("<!--sub {url} -->"),
				Marker::Comment { url, immutable, .. } =>
					if *immutable {
						format!("<!--immutable {url} -->")
					} else {
						format!("<!-- {url} -->")
					},
				Marker::NewComment => "<!-- new comment -->".to_string(),
				Marker::BlockersSection => "# Blockers".to_string(),
				Marker::Omitted => "<!-- omitted -->".to_string(),
				Marker::OmittedWithHint => "<!-- omitted (use --render-closed to unfold) -->".to_string(),
			},
			Extension::Typ => match self {
				Marker::IssueUrl { url, immutable } =>
					if *immutable {
						format!("// immutable {url}")
					} else {
						format!("// {url}")
					},
				Marker::SubIssue { url } => format!("// sub {url}"),
				Marker::Comment { url, immutable, .. } =>
					if *immutable {
						format!("// immutable {url}")
					} else {
						format!("// {url}")
					},
				Marker::NewComment => "// new comment".to_string(),
				Marker::BlockersSection => "// blockers".to_string(),
				Marker::Omitted => "// omitted".to_string(),
				Marker::OmittedWithHint => "// omitted (use --render-closed to unfold)".to_string(),
			},
		}
	}
}

impl fmt::Display for Marker {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.encode(Extension::Md))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_decode_issue_url() {
		// With spaces
		assert_eq!(
			Marker::decode("<!-- https://github.com/owner/repo/issues/123 -->", Extension::Md),
			Some(Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: false
			})
		);
		// Without spaces
		assert_eq!(
			Marker::decode("<!--https://github.com/owner/repo/issues/123-->", Extension::Md),
			Some(Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: false
			})
		);
		// Immutable
		assert_eq!(
			Marker::decode("<!--immutable https://github.com/owner/repo/issues/123-->", Extension::Md),
			Some(Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: true
			})
		);
	}

	#[test]
	fn test_decode_sub_issue() {
		assert_eq!(
			Marker::decode("<!--sub https://github.com/owner/repo/issues/124-->", Extension::Md),
			Some(Marker::SubIssue {
				url: "https://github.com/owner/repo/issues/124".to_string()
			})
		);
		assert_eq!(
			Marker::decode("<!--sub https://github.com/owner/repo/issues/124 -->", Extension::Md),
			Some(Marker::SubIssue {
				url: "https://github.com/owner/repo/issues/124".to_string()
			})
		);
	}

	#[test]
	fn test_decode_comment() {
		assert_eq!(
			Marker::decode("<!--https://github.com/owner/repo/issues/123#issuecomment-456-->", Extension::Md),
			Some(Marker::Comment {
				url: "https://github.com/owner/repo/issues/123#issuecomment-456".to_string(),
				id: 456,
				immutable: false
			})
		);
		assert_eq!(
			Marker::decode("<!-- https://github.com/owner/repo/issues/123#issuecomment-456 -->", Extension::Md),
			Some(Marker::Comment {
				url: "https://github.com/owner/repo/issues/123#issuecomment-456".to_string(),
				id: 456,
				immutable: false
			})
		);
		assert_eq!(
			Marker::decode("<!--immutable https://github.com/owner/repo/issues/123#issuecomment-456-->", Extension::Md),
			Some(Marker::Comment {
				url: "https://github.com/owner/repo/issues/123#issuecomment-456".to_string(),
				id: 456,
				immutable: true
			})
		);
	}

	#[test]
	fn test_decode_blockers_section() {
		// Markdown header (preferred)
		assert_eq!(Marker::decode("# Blockers", Extension::Md), Some(Marker::BlockersSection));
		assert_eq!(Marker::decode("## Blockers", Extension::Md), Some(Marker::BlockersSection));
		assert_eq!(Marker::decode("### Blockers:", Extension::Md), Some(Marker::BlockersSection));
		// With leading/trailing whitespace
		assert_eq!(Marker::decode("  # Blockers  ", Extension::Md), Some(Marker::BlockersSection));
		// Legacy HTML comment
		assert_eq!(Marker::decode("<!--blockers-->", Extension::Md), Some(Marker::BlockersSection));
		assert_eq!(Marker::decode("<!-- blockers -->", Extension::Md), Some(Marker::BlockersSection));
		assert_eq!(Marker::decode("<!--blocker-->", Extension::Md), Some(Marker::BlockersSection));
		// Typst
		assert_eq!(Marker::decode("// blockers", Extension::Typ), Some(Marker::BlockersSection));
		assert_eq!(Marker::decode("// blocker", Extension::Typ), Some(Marker::BlockersSection));

		// Should NOT match if there's other content on the line
		assert_ne!(Marker::decode("# Blockers and more", Extension::Md), Some(Marker::BlockersSection));
		assert_ne!(Marker::decode("Some text # Blockers", Extension::Md), Some(Marker::BlockersSection));
	}

	#[test]
	fn test_decode_new_comment() {
		assert_eq!(Marker::decode("<!--new comment-->", Extension::Md), Some(Marker::NewComment));
		assert_eq!(Marker::decode("<!-- new comment -->", Extension::Md), Some(Marker::NewComment));
	}

	#[test]
	fn test_decode_omitted() {
		assert_eq!(Marker::decode("<!-- omitted -->", Extension::Md), Some(Marker::Omitted));
		assert_eq!(Marker::decode("<!--omitted-->", Extension::Md), Some(Marker::Omitted));
		assert_eq!(Marker::decode("<!-- omitted (use --render-closed to unfold) -->", Extension::Md), Some(Marker::OmittedWithHint));
	}

	#[test]
	fn test_encode() {
		assert_eq!(
			Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: false
			}
			.encode(Extension::Md),
			"<!-- https://github.com/owner/repo/issues/123 -->"
		);
		assert_eq!(
			Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: true
			}
			.encode(Extension::Md),
			"<!--immutable https://github.com/owner/repo/issues/123 -->"
		);
		assert_eq!(
			Marker::SubIssue {
				url: "https://github.com/owner/repo/issues/124".to_string()
			}
			.encode(Extension::Md),
			"<!--sub https://github.com/owner/repo/issues/124 -->"
		);
		assert_eq!(Marker::BlockersSection.encode(Extension::Md), "# Blockers");
		assert_eq!(Marker::BlockersSection.encode(Extension::Typ), "// blockers");
		assert_eq!(Marker::NewComment.encode(Extension::Md), "<!-- new comment -->");
		assert_eq!(Marker::Omitted.encode(Extension::Md), "<!-- omitted -->");
	}

	#[test]
	fn test_roundtrip() {
		let markers = vec![
			Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: false,
			},
			Marker::IssueUrl {
				url: "https://github.com/owner/repo/issues/123".to_string(),
				immutable: true,
			},
			Marker::SubIssue {
				url: "https://github.com/owner/repo/issues/124".to_string(),
			},
			Marker::Comment {
				url: "https://github.com/owner/repo/issues/123#issuecomment-456".to_string(),
				id: 456,
				immutable: false,
			},
			Marker::NewComment,
			Marker::Omitted,
			Marker::OmittedWithHint,
		];

		for marker in markers {
			let encoded = marker.encode(Extension::Md);
			let decoded = Marker::decode(&encoded, Extension::Md).unwrap_or_else(|| panic!("Failed to decode: {encoded}"));
			assert_eq!(marker, decoded, "Roundtrip failed for {:?}", marker);
		}
	}
}
