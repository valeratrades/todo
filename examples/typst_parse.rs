//! Example: Parse any text file as typst (with preprocessing) and pretty-print the AST
//!
//! Run with: cargo run --example typst_parse [path]

use std::{env, fs, path::Path};

use typst_syntax::parse;

fn main() {
	let path = env::args().nth(1).unwrap_or_else(|| "examples/test_issue.typ".into());

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

	let ast_str = format!("{root:#?}");
	println!("{ast_str}");

	// Write AST to .rs file next to input
	let input_path = Path::new(&path);
	let ast_path = input_path.with_extension("ast.rs");
	fs::write(&ast_path, &ast_str).expect("Failed to write AST file");
	eprintln!("\nWrote AST to {}", ast_path.display());
}

/// Preprocess raw text into valid typst
fn preprocess_to_typst(input: &str) -> String {
	let mut result = String::with_capacity(input.len());

	for line in input.lines() {
		//let preprocessed = preprocess_line(line);
		let preprocessed = line.to_string();
		result.push_str(&preprocessed);
		result.push('\n');
	}

	result
}
