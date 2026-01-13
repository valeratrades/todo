//! Benchmark markdown parsing/conversion latency
//!
//! Run with: cargo run --example pandoc_bench

use std::{
	io::Write,
	process::{Command, Stdio},
	time::Instant,
};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

fn main() {
	// Generate ~300 lines of markdown
	let mut markdown = String::new();
	for i in 1..=50 {
		markdown.push_str(&format!("## Heading {i}\n\n"));
		markdown.push_str(&format!("- [ ] task item {i} with some text and **bold** content\n"));
		markdown.push_str(&format!("- [x] completed task {i} with `inline code`\n"));
		markdown.push_str(&format!("- regular list item {i}\n\n"));
	}

	let line_count = markdown.lines().count();
	println!("Generated {line_count} lines of markdown\n");

	// Benchmark pulldown-cmark parsing
	println!("=== pulldown-cmark (parse only) ===");
	let start = Instant::now();
	let options = Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
	let parser = Parser::new_ext(&markdown, options);
	let events: Vec<_> = parser.collect();
	let pulldown_parse = start.elapsed();
	println!("Parse time: {pulldown_parse:?}");
	println!("Events: {}", events.len());

	// Benchmark pulldown-cmark → typst conversion
	println!("\n=== pulldown-cmark → typst ===");
	let start = Instant::now();
	let typst_output = md_to_typst_pulldown(&markdown);
	let pulldown_to_typst = start.elapsed();
	println!("Convert time: {pulldown_to_typst:?}");
	println!("Output (first 500 chars):\n{}\n", &typst_output[..500.min(typst_output.len())]);

	// Benchmark pandoc md → typst
	println!("=== pandoc md → typst ===");
	let start = Instant::now();
	let typst = pandoc_convert(&markdown, "markdown", "typst");
	let pandoc_to_typst = start.elapsed();
	println!("Convert time: {pandoc_to_typst:?}");

	if let Ok(ref t) = typst {
		println!("Output (first 500 chars):\n{}\n", &t[..500.min(t.len())]);
	}

	// Summary
	println!("=== Summary for {line_count} lines ===");
	println!("pulldown-cmark parse:     {pulldown_parse:?}");
	println!("pulldown-cmark → typst:   {pulldown_to_typst:?}");
	println!("pandoc md → typst:        {pandoc_to_typst:?}");
	println!("Speedup (pulldown/pandoc): {:.1}x", pandoc_to_typst.as_secs_f64() / pulldown_to_typst.as_secs_f64());
}

fn md_to_typst_pulldown(markdown: &str) -> String {
	let options = Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
	let parser = Parser::new_ext(markdown, options);

	let mut output = String::new();
	let mut list_depth: usize = 0;

	for event in parser {
		match event {
			Event::Start(Tag::Heading { level, .. }) => {
				let marker = "=".repeat(level as usize);
				output.push_str(&marker);
				output.push(' ');
			}
			Event::End(TagEnd::Heading(_)) => {
				output.push('\n');
			}
			Event::Start(Tag::List(_)) => {
				list_depth += 1;
			}
			Event::End(TagEnd::List(_)) => {
				list_depth -= 1;
				if list_depth == 0 {
					output.push('\n');
				}
			}
			Event::Start(Tag::Item) => {
				output.push_str(&"\t".repeat(list_depth.saturating_sub(1)));
				output.push_str("- ");
			}
			Event::End(TagEnd::Item) => {
				output.push('\n');
			}
			Event::TaskListMarker(checked) =>
				if checked {
					output.push_str("#checkbox(true) ");
				} else {
					output.push_str("#checkbox(false) ");
				},
			Event::Start(Tag::Strong) => {
				output.push('*');
			}
			Event::End(TagEnd::Strong) => {
				output.push('*');
			}
			Event::Start(Tag::Emphasis) => {
				output.push('_');
			}
			Event::End(TagEnd::Emphasis) => {
				output.push('_');
			}
			Event::Code(code) => {
				output.push('`');
				output.push_str(&code);
				output.push('`');
			}
			Event::Text(text) => {
				output.push_str(&text);
			}
			Event::SoftBreak | Event::HardBreak => {
				output.push('\n');
			}
			Event::Start(Tag::Paragraph) => {}
			Event::End(TagEnd::Paragraph) => {
				output.push('\n');
			}
			_ => {}
		}
	}

	output
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
