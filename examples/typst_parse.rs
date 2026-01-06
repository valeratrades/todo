//! Example: Parse any text file as typst (with preprocessing) and pretty-print the AST
//!
//! Run with: cargo run --example typst_parse [path]

use std::{env, fs};

use typst_syntax::parse;

fn main() {
	let path = env::args()
		.nth(1)
		.unwrap_or_else(|| "/home/v/.local/share/todo/issues/valeratrades/todo/46_-_git_issues_editor.md".into());

	let content = fs::read_to_string(&path).expect("Failed to read file");

	println!("=== Parsing: {path} ===\n");

	let preprocessed = preprocess_to_typst(&content);

	println!("=== Preprocessed ===\n{preprocessed}\n");
	println!("=== AST ===\n");

	let root = parse(&preprocessed);

	let errors = root.errors();
	if !errors.is_empty() {
		eprintln!("=== Parse errors ===");
		for err in errors {
			eprintln!("  {err:?}");
		}
		std::process::exit(1);
	}

	println!("{root:#?}");
}

/// Preprocess raw text into valid typst
fn preprocess_to_typst(input: &str) -> String {
	let mut result = String::with_capacity(input.len());

	for line in input.lines() {
		let preprocessed = preprocess_line(line);
		result.push_str(&preprocessed);
		result.push('\n');
	}

	result
}

fn preprocess_line(line: &str) -> String {
	let trimmed = line.trim_start();
	let indent = &line[..line.len() - trimmed.len()];

	// Markdown checkboxes: `- [ ]` or `- [x]`
	if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
		format!("{indent}- #checkbox(false) {rest}")
	} else if let Some(rest) = trimmed.strip_prefix("- [x] ") {
		format!("{indent}- #checkbox(true) {rest}")
	} else if let Some(rest) = trimmed.strip_prefix("- [X] ") {
		format!("{indent}- #checkbox(true) {rest}")
	} else {
		line.to_string()
	}
}
