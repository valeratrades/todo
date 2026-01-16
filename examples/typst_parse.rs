//! Example: Parse a typst file and pretty-print the AST
//!
//! Run with: cargo run --example typst_parse [path]

use std::{env, fs, path::Path};

use typst_syntax::parse;

fn main() {
	let path = env::args().nth(1).unwrap_or_else(|| "examples/test_issue.typ".into());

	let content = fs::read_to_string(&path).expect("Failed to read file");

	println!("=== Parsing: {path} ===\n");
	println!("=== AST ===\n");

	let root = parse(&content);

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

	// Write AST to .ast.rs.bak file next to input
	let input_path = Path::new(&path);
	let ast_path = input_path.with_extension("ast.rs.bak");
	fs::write(&ast_path, &ast_str).expect("Failed to write AST file");
	eprintln!("\nWrote AST to {}", ast_path.display());
}
