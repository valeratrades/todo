//! Benchmark pandoc markdown ↔ typst conversion latency
//!
//! Run with: cargo run --example pandoc_bench

use std::{
	io::Write,
	process::{Command, Stdio},
	time::Instant,
};

fn main() {
	// Generate ~200 lines of markdown
	let mut markdown = String::new();
	for i in 1..=50 {
		markdown.push_str(&format!("## Heading {i}\n\n"));
		markdown.push_str(&format!("- [ ] task item {i} with some text and **bold** content\n"));
		markdown.push_str(&format!("- [x] completed task {i} with `inline code`\n"));
		markdown.push_str(&format!("- regular list item {i}\n\n"));
	}

	let line_count = markdown.lines().count();
	println!("Generated {line_count} lines of markdown\n");
	println!("=== Markdown (first 500 chars) ===\n{}\n", &markdown[..500.min(markdown.len())]);

	// Benchmark md → typst
	println!("=== Benchmarking md → typst ===");
	let start = Instant::now();
	let typst = pandoc_convert(&markdown, "markdown", "typst");
	let md_to_typst = start.elapsed();
	println!("md → typst: {:?}", md_to_typst);

	let typst = match typst {
		Ok(t) => t,
		Err(e) => {
			eprintln!("Error: {e}");
			return;
		}
	};

	println!("\n=== Typst output (first 500 chars) ===\n{}\n", &typst[..500.min(typst.len())]);

	// Benchmark typst → md
	println!("=== Benchmarking typst → md ===");
	let start = Instant::now();
	let markdown_back = pandoc_convert(&typst, "typst", "markdown");
	let typst_to_md = start.elapsed();
	println!("typst → md: {:?}", typst_to_md);

	match markdown_back {
		Ok(md) => {
			println!("\n=== Markdown roundtrip (first 500 chars) ===\n{}\n", &md[..500.min(md.len())]);
		}
		Err(e) => {
			eprintln!("Error: {e}");
		}
	}

	// Summary
	println!("=== Summary ===");
	println!("Lines:        {line_count}");
	println!("md → typst:   {:?}", md_to_typst);
	println!("typst → md:   {:?}", typst_to_md);
	println!("Total:        {:?}", md_to_typst + typst_to_md);
}

fn pandoc_convert(input: &str, from: &str, to: &str) -> Result<String, String> {
	let mut child = Command::new("pandoc")
		.args(["--from", from, "--to", to])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.map_err(|e| format!("Failed to spawn pandoc: {e}"))?;

	child
		.stdin
		.as_mut()
		.unwrap()
		.write_all(input.as_bytes())
		.map_err(|e| format!("Failed to write to pandoc stdin: {e}"))?;

	let output = child.wait_with_output().map_err(|e| format!("Failed to wait for pandoc: {e}"))?;

	if output.status.success() {
		String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 from pandoc: {e}"))
	} else {
		Err(format!("Pandoc failed: {}", String::from_utf8_lossy(&output.stderr)))
	}
}
