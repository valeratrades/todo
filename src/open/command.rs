//! Main command entry point for the open subcommand.

use std::path::Path;

use clap::Args;
use v_utils::prelude::*;

use super::{
	fetch::fetch_and_store_issue,
	files::{choose_issue_with_fzf, search_issue_files},
	meta::is_virtual_project,
	sync::open_local_issue,
	touch::{create_pending_issue, create_virtual_issue, find_local_issue_for_touch, parse_touch_path},
	util::Extension,
};
use crate::{
	config::LiveSettings,
	github::{self, BoxedGitHubClient},
};

/// Open a GitHub issue in $EDITOR.
///
/// Issue files support a blockers section for tracking sub-tasks. Add a `# Blockers` marker
/// (or `// blockers` for Typst) in the issue body. Content after this marker until the next sub-issue
/// or comment is treated as blockers, using the same format as standalone blocker files.
///
/// Shorthand: Use `!b` on its own line to auto-expand to `# Blockers` (or `// blockers` for Typst).
#[derive(Args)]
pub struct OpenArgs {
	/// GitHub issue URL (e.g., https://github.com/owner/repo/issues/123) OR a search pattern for local issue files
	/// With --touch: path format is workspace/project/{issue.md, issue/sub-issue.md}
	/// If omitted, opens fzf on all local issue files.
	pub url_or_pattern: Option<String>,

	/// File extension for the output file (overrides config default_extension)
	#[arg(short = 'e', long)]
	pub extension: Option<Extension>,

	/// Render full contents even for closed issues (by default, closed issues show only title with <!-- omitted -->)
	#[arg(long)]
	pub render_closed: bool,

	/// Create or open an issue from a path. Path format: workspace/project/issue[.md|.typ]
	/// For sub-issues: workspace/project/parent/child (parent must exist on GitHub)
	/// If issue already exists locally, opens it. Otherwise creates on GitHub first.
	#[arg(short, long)]
	pub touch: bool,

	/// Open the most recently modified issue file
	#[arg(short, long)]
	pub last: bool,

	/// Skip all network operations - edit locally only, don't sync to GitHub
	#[arg(long)]
	pub offline: bool,

	/// Fetch latest from GitHub before opening. If remote differs from local,
	/// prompts: [s]kip (use local), [o]verwrite (use remote), [m]erge (attempt merge)
	#[arg(short, long)]
	pub pull: bool,
}

/// Get the effective extension from args, config, or default
fn get_effective_extension(args_extension: Option<Extension>, settings: &LiveSettings) -> Extension {
	// Priority: CLI arg > config > default (md)
	if let Some(ext) = args_extension {
		return ext;
	}

	if let Ok(config) = settings.config()
		&& let Some(open_config) = &config.open
	{
		return match open_config.default_extension.as_str() {
			"typ" => Extension::Typ,
			_ => Extension::Md,
		};
	}

	Extension::Md
}

pub async fn open_command(settings: &LiveSettings, gh: BoxedGitHubClient, args: OpenArgs, global_offline: bool) -> Result<()> {
	let input = args.url_or_pattern.as_deref().unwrap_or("").trim();
	let extension = get_effective_extension(args.extension, settings);
	// Combine global --offline with subcommand --offline
	let offline = global_offline || args.offline;

	// Handle --last mode: open the most recently modified issue file
	if args.last {
		let all_files = search_issue_files("")?;
		if all_files.is_empty() {
			return Err(eyre!("No issue files found. Use a GitHub URL to fetch an issue first."));
		}
		// Files are already sorted by modification time (most recent first)
		open_local_issue(&gh, &all_files[0], offline).await?;
		return Ok(());
	}

	// Handle --touch mode
	if args.touch {
		let touch_path = parse_touch_path(input)?;

		// Determine the extension to use
		let effective_ext = touch_path.extension.unwrap_or(extension);

		// Check if the project is virtual
		let project_is_virtual = is_virtual_project(&touch_path.owner, &touch_path.repo);

		// First, try to find an existing local issue file
		if let Some(existing_path) = find_local_issue_for_touch(&touch_path, &effective_ext) {
			println!("Found existing issue: {:?}", existing_path);
			let effective_offline = offline || project_is_virtual;
			open_local_issue(&gh, &existing_path, effective_offline).await?;
			return Ok(());
		}

		// Not found locally - create a local file
		let issue_file_path = if project_is_virtual {
			// Virtual project: stays local forever
			println!("Project {}/{} is virtual (no GitHub remote)", touch_path.owner, touch_path.repo);
			create_virtual_issue(&touch_path, &effective_ext)?
		} else {
			// Real project: create pending issue (will be created on GitHub when editor closes)
			create_pending_issue(&touch_path, &effective_ext)?
		};

		// Open for editing - sync will create on GitHub if pending
		let effective_offline = offline || project_is_virtual;
		open_local_issue(&gh, &issue_file_path, effective_offline).await?;
		return Ok(());
	}

	// Check if input is a GitHub issue URL specifically (not just any GitHub URL)
	if github::is_github_issue_url(input) {
		// GitHub URL mode: fetch issue and store in XDG_DATA (can't be offline)
		if offline {
			return Err(eyre!("Cannot fetch issue from URL in offline mode"));
		}

		let (owner, repo, issue_number) = github::parse_github_issue_url(input)?;

		println!("Fetching issue #{issue_number} from {owner}/{repo}...");

		// Fetch and store issue (and sub-issues) in XDG_DATA
		let issue_file_path = fetch_and_store_issue(&gh, &owner, &repo, issue_number, &extension, args.render_closed, None).await?;

		println!("Stored issue at: {:?}", issue_file_path);

		// Open the local issue file for editing
		open_local_issue(&gh, &issue_file_path, offline).await?;
		return Ok(());
	}

	// Check if input is an existing file path (absolute or relative)
	let input_path = Path::new(input);
	if input_path.exists() && input_path.is_file() {
		// Direct file path - open it
		open_local_issue(&gh, input_path, offline).await?;
		return Ok(());
	}

	// Local search mode: find and open existing issue file
	let matches = search_issue_files(input)?;

	let issue_file_path = match matches.len() {
		0 => {
			// No matches - open fzf with all files and use input as initial query
			let all_files = search_issue_files("")?;
			if all_files.is_empty() {
				return Err(eyre!("No issue files found. Use a GitHub URL to fetch an issue first."));
			}
			match choose_issue_with_fzf(&all_files, input)? {
				Some(path) => path,
				None => return Err(eyre!("No issue selected")),
			}
		}
		1 => matches[0].clone(),
		_ => {
			// Multiple matches - open fzf to choose
			match choose_issue_with_fzf(&matches, input)? {
				Some(path) => path,
				None => return Err(eyre!("No issue selected")),
			}
		}
	};

	// Open the local issue file for editing
	open_local_issue(&gh, &issue_file_path, offline).await?;

	Ok(())
}
