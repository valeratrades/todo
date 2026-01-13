//! Integration tests for issue content preservation through edit/sync cycles.
//!
//! Tests that nested issues, blockers, and other content survive the
//! parse -> edit -> serialize -> sync cycle intact.

use todo::{Issue, ParseContext};

use crate::common::{TestContext, git::GitExt};

const OWNER: &str = "A";
const REPO: &str = "B";

fn parse(content: &str) -> Issue {
	let ctx = ParseContext::new(content.to_string(), "test.md".to_string());
	Issue::parse(content, &ctx).expect("failed to parse test issue")
}

#[test]
fn test_nested_issues_preserved_through_sync() {
	let ctx = TestContext::new("");

	let content = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	- [ ] b <!--sub https://github.com/{OWNER}/{REPO}/issues/2 -->
		nested body b

	- [ ] c <!--sub https://github.com/{OWNER}/{REPO}/issues/3 -->
		nested body c
"#
	);
	let issue = parse(&content);

	let path = ctx.setup_issue(OWNER, REPO, 1, &issue);
	ctx.setup_remote_with_children(OWNER, REPO, 1, &issue, &[2, 3]);

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

	let content = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	- [ ] b <!--sub https://github.com/{OWNER}/{REPO}/issues/2 -->
		open nested body

	- [x] c <!--sub https://github.com/{OWNER}/{REPO}/issues/3 -->
		<!--omitted {{{{{{always-->
		closed nested body
		<!--,}}}}}}-->
"#
	);
	let issue = parse(&content);

	let path = ctx.setup_issue(OWNER, REPO, 1, &issue);
	ctx.setup_remote_with_children(OWNER, REPO, 1, &issue, &[2, 3]);

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

	let content = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	# Blockers
	- first blocker
	- second blocker
"#
	);
	let issue = parse(&content);

	let path = ctx.setup_issue(OWNER, REPO, 1, &issue);
	ctx.setup_remote(OWNER, REPO, 1, &issue);

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
	let initial = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum
"#
	);
	let initial_issue = parse(&initial);

	let path = ctx.setup_issue(OWNER, REPO, 1, &initial_issue);
	ctx.setup_remote(OWNER, REPO, 1, &initial_issue);

	// User adds blockers during edit
	let edited = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	# Blockers
	- new blocker added
"#
	);
	let edited_issue = parse(&edited);

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

	let content = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	# Blockers
	# phase 1
	- task alpha
	- task beta

	# phase 2
	- task gamma
"#
	);
	let issue = parse(&content);

	let path = ctx.setup_issue(OWNER, REPO, 1, &issue);
	ctx.setup_remote(OWNER, REPO, 1, &issue);

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

	let content = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	# Blockers
	- blocker one
	- blocker two

	- [ ] b <!--sub https://github.com/{OWNER}/{REPO}/issues/2 -->
		nested body
"#
	);
	let issue = parse(&content);

	let path = ctx.setup_issue(OWNER, REPO, 1, &issue);
	ctx.setup_remote_with_children(OWNER, REPO, 1, &issue, &[2]);

	let (status, stdout, stderr) = ctx.run_open(&path);
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	// File moves to directory format when nested issues exist
	let final_path = ctx.issue_path_after_sync(OWNER, REPO, 1, "a", true);
	let final_content = std::fs::read_to_string(&final_path).unwrap();
	assert!(final_content.contains("# Blockers"), "blockers section lost");
	assert!(final_content.contains("blocker one"), "blocker one lost");
	assert!(final_content.contains("nested body"), "nested issue body lost");
}

#[test]
fn test_closing_nested_issue_adds_fold_markers() {
	let ctx = TestContext::new("");

	// Start with open nested issue
	let initial = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	- [ ] b <!--sub https://github.com/{OWNER}/{REPO}/issues/2 -->
		nested body content
"#
	);
	let initial_issue = parse(&initial);

	let path = ctx.setup_issue(OWNER, REPO, 1, &initial_issue);
	ctx.setup_remote_with_children(OWNER, REPO, 1, &initial_issue, &[2]);

	// User closes nested issue during edit
	let edited = format!(
		r#"- [ ] a <!-- https://github.com/{OWNER}/{REPO}/issues/1 -->
	lorem ipsum

	- [x] b <!--sub https://github.com/{OWNER}/{REPO}/issues/2 -->
		nested body content
"#
	);
	let edited_issue = parse(&edited);

	let (status, stdout, stderr) = ctx.open(&path).edit(&edited_issue).run();
	eprintln!("stdout: {stdout}\nstderr: {stderr}");

	assert!(status.success(), "stderr: {stderr}");

	// File moves to directory format when nested issues exist
	let final_path = ctx.issue_path_after_sync(OWNER, REPO, 1, "a", true);
	let final_content = std::fs::read_to_string(&final_path).unwrap();
	assert!(final_content.contains("- [x] b"), "nested issue not marked closed");
	assert!(final_content.contains("<!--omitted"), "fold marker not added for closed nested issue");
}
