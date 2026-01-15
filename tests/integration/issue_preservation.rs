//! Integration tests for issue content preservation through edit/sync cycles.
//!
//! Tests that nested issues, blockers, and other content survive the
//! parse -> edit -> serialize -> sync cycle intact.

use std::path::Path;

use todo::Issue;

use crate::common::{TestContext, git::GitExt};

fn parse(content: &str) -> Issue {
	Issue::parse(content, Path::new("test.md")).expect("failed to parse test issue")
}

#[test]
fn test_comments_with_ids_sync_correctly() {
	let ctx = TestContext::new("");

	// Issue with a comment that has an ID
	let issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tbody text\n\
		 \n\
		 \t<!-- https://github.com/o/r/issues/1#issuecomment-12345 -->\n\
		 \tThis is my comment\n",
	);

	ctx.consensus(&issue);
	ctx.remote(&issue);

	let path = ctx.issue_path(&issue);
	let (status, stdout, stderr) = ctx.open(&path).args(&["--force"]).run();
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	// This should NOT fail with "comment X not found in consensus"
	assert!(status.success(), "sync failed: {stderr}");
}

#[test]
fn test_nested_issues_preserved_through_sync() {
	let ctx = TestContext::new("");

	let issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t- [ ] b <!--sub https://github.com/o/r/issues/2 -->\n\
		 \t\tnested body b\n\
		 \n\
		 \t- [ ] c <!--sub https://github.com/o/r/issues/3 -->\n\
		 \t\tnested body c\n",
	);

	let path = ctx.consensus(&issue);
	ctx.remote(&issue);

	let (status, stdout, stderr) = ctx.run_open(&path);
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	let final_content = std::fs::read_to_string(&path).unwrap();
	assert!(final_content.contains("nested body b"), "nested issue b body lost");
	assert!(final_content.contains("nested body c"), "nested issue c body lost");
}

#[test]
fn test_mixed_open_closed_nested_issues_preserved() {
	let ctx = TestContext::new("");

	let issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t- [ ] b <!--sub https://github.com/o/r/issues/2 -->\n\
		 \t\topen nested body\n\
		 \n\
		 \t- [x] c <!--sub https://github.com/o/r/issues/3 -->\n\
		 \t\t<!--omitted {{{always-->\n\
		 \t\tclosed nested body\n\
		 \t\t<!--,}}}-->\n",
	);

	let path = ctx.consensus(&issue);
	ctx.remote(&issue);

	let (status, stdout, stderr) = ctx.run_open(&path);
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	let final_content = std::fs::read_to_string(&path).unwrap();
	assert!(final_content.contains("open nested body"), "open nested issue body lost");
	assert!(final_content.contains("- [x] c"), "closed nested issue state lost");
}

#[test]
fn test_blockers_preserved_through_sync() {
	let ctx = TestContext::new("");

	let issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t# Blockers\n\
		 \t- first blocker\n\
		 \t- second blocker\n",
	);

	let path = ctx.consensus(&issue);
	ctx.remote(&issue);

	let (status, stdout, stderr) = ctx.run_open(&path);
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	let final_content = std::fs::read_to_string(&path).unwrap();
	assert!(final_content.contains("# Blockers"), "blockers section lost");
	assert!(final_content.contains("first blocker"), "first blocker lost");
	assert!(final_content.contains("second blocker"), "second blocker lost");
}

#[test]
fn test_blockers_added_during_edit_preserved() {
	let ctx = TestContext::new("");

	// Initial state: no blockers
	let initial_issue = parse("- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\tlorem ipsum\n");

	let path = ctx.consensus(&initial_issue);
	ctx.remote(&initial_issue);

	// User adds blockers during edit
	let edited_issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t# Blockers\n\
		 \t- new blocker added\n",
	);

	let (status, stdout, stderr) = ctx.open(&path).edit(&edited_issue).run();
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	let final_content = std::fs::read_to_string(&path).unwrap();
	assert!(final_content.contains("# Blockers"), "blockers section not preserved");
	assert!(final_content.contains("new blocker added"), "added blocker lost");
}

#[test]
fn test_blockers_with_headers_preserved() {
	let ctx = TestContext::new("");

	let issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t# Blockers\n\
		 \t# phase 1\n\
		 \t- task alpha\n\
		 \t- task beta\n\
		 \n\
		 \t# phase 2\n\
		 \t- task gamma\n",
	);

	let path = ctx.consensus(&issue);
	ctx.remote(&issue);

	let (status, stdout, stderr) = ctx.run_open(&path);
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	let final_content = std::fs::read_to_string(&path).unwrap();
	assert!(final_content.contains("# phase 1"), "phase 1 header lost");
	assert!(final_content.contains("# phase 2"), "phase 2 header lost");
	assert!(final_content.contains("task alpha"), "task alpha lost");
	assert!(final_content.contains("task gamma"), "task gamma lost");
}

#[test]
fn test_nested_issues_and_blockers_together() {
	let ctx = TestContext::new("");

	let issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t# Blockers\n\
		 \t- blocker one\n\
		 \t- blocker two\n\
		 \n\
		 \t- [ ] b <!--sub https://github.com/o/r/issues/2 -->\n\
		 \t\tnested body\n",
	);

	ctx.consensus(&issue);
	ctx.remote(&issue);

	let path = ctx.issue_path(&issue);
	let (status, stdout, stderr) = ctx.run_open(&path);
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	// File moves to directory format when nested issues exist
	let final_path = ctx.issue_path_after_sync("o", "r", 1, "a", true);
	let final_content = std::fs::read_to_string(&final_path).unwrap();
	assert!(final_content.contains("# Blockers"), "blockers section lost");
	assert!(final_content.contains("blocker one"), "blocker one lost");
	assert!(final_content.contains("nested body"), "nested issue body lost");
}

#[test]
fn test_closing_nested_issue_adds_fold_markers() {
	let ctx = TestContext::new("");

	// Start with open nested issue
	let initial_issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t- [ ] b <!--sub https://github.com/o/r/issues/2 -->\n\
		 \t\tnested body content\n",
	);

	ctx.consensus(&initial_issue);
	ctx.remote(&initial_issue);

	let path = ctx.issue_path(&initial_issue);

	// User closes nested issue during edit
	let edited_issue = parse(
		"- [ ] a <!-- https://github.com/o/r/issues/1 -->\n\
		 \tlorem ipsum\n\
		 \n\
		 \t- [x] b <!--sub https://github.com/o/r/issues/2 -->\n\
		 \t\tnested body content\n",
	);

	let (status, stdout, stderr) = ctx.open(&path).edit(&edited_issue).run();
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	// File moves to directory format when nested issues exist
	let final_path = ctx.issue_path_after_sync("o", "r", 1, "a", true);
	let final_content = std::fs::read_to_string(&final_path).unwrap();
	assert!(final_content.contains("- [x] b"), "nested issue not marked closed");
	assert!(final_content.contains("<!--omitted"), "fold marker not added for closed nested issue");
}
