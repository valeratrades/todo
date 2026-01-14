//! Main command entry point for the open subcommand.

use std::path::Path;

use clap::Args;
use todo::Extension;
use v_utils::prelude::*;

use super::{
	fetch::fetch_and_store_issue,
	files::{choose_issue_with_fzf, search_issue_files},
	meta::is_virtual_project,
	sync::{MergeMode, Side, SyncOptions, open_local_issue},
	touch::{create_pending_issue, create_virtual_issue, find_local_issue_for_touch, parse_touch_path},
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

	/// Create or open an issue from a path. Path format: workspace/project/issue[.md|.typ]
	/// For sub-issues: workspace/project/parent/child (parent must exist on GitHub)
	/// If issue already exists locally, opens it. Otherwise creates on GitHub first.
	#[arg(short, long)]
	pub touch: bool,

	/// Open the most recently modified issue file
	#[arg(short, long)]
	pub last: bool,

	/// Skip all network operations - edit locally only, don't sync to GitHub
	#[arg(short, long)]
	pub offline: bool,

	/// Fetch latest from GitHub before opening. If remote differs from local,
	/// prompts: [s]kip (use local), [o]verwrite (use remote), [m]erge (attempt merge)
	#[arg(short, long)]
	pub pull: bool,

	/// Use the current blocker issue file (from `todo blocker set`)
	/// If no pattern provided, opens the current blocker issue.
	#[arg(short, long)]
	pub blocker: bool,

	/// Like --blocker, but also sets the opened issue as active if different from current.
	/// Opens the current blocker issue (or pattern match), and if that issue belongs to
	/// a different project than the currently active one, sets it as the active project.
	#[arg(long)]
	pub blocker_set: bool,

	/// Force through conflicts by taking the source side.
	/// When opening via local path: takes local version.
	/// When opening via GitHub URL: takes remote version.
	#[arg(short, long)]
	pub force: bool,

	/// Reset to source state, ignoring any local/remote changes.
	/// Overwrites everything with current source without syncing.
	/// When opening via local path: keeps local as-is (skips sync).
	/// When opening via GitHub URL: overwrites local with remote.
	#[arg(short, long)]
	pub reset: bool,
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
	let extension = get_effective_extension(args.extension, settings);
	// Combine global --offline with subcommand --offline
	let offline = global_offline || args.offline;

	// Build merge mode from args
	let build_merge_mode = |prefer: Side| -> Option<MergeMode> {
		if args.reset {
			Some(MergeMode::Reset { prefer })
		} else if args.force {
			Some(MergeMode::Force { prefer })
		} else {
			None
		}
	};

	// Helper to create sync opts for local source (used in multiple branches)
	let local_sync_opts = || SyncOptions::new(build_merge_mode(Side::Local), args.pull);

	// Handle --blocker and --blocker-set modes: use current blocker issue file if no pattern provided
	let use_blocker_mode = args.blocker || args.blocker_set;
	let input = if use_blocker_mode && args.url_or_pattern.is_none() {
		// Get current blocker issue path
		let blocker_path = crate::blocker_interactions::integration::get_current_blocker_issue().ok_or_else(|| eyre!("No blocker issue set. Use `todo blocker set <pattern>` first."))?;
		blocker_path.to_string_lossy().to_string()
	} else {
		args.url_or_pattern.as_deref().unwrap_or("").trim().to_string()
	};
	let input = input.as_str();

	// Resolve the issue file path and sync options based on mode
	let (issue_file_path, sync_opts, effective_offline) = if args.last {
		// Handle --last mode: open the most recently modified issue file
		let all_files = search_issue_files("")?;
		if all_files.is_empty() {
			bail!("No issue files found. Use a GitHub URL to fetch an issue first.");
		}
		// Files are already sorted by modification time (most recent first)
		(all_files[0].clone(), local_sync_opts(), offline)
	} else if args.touch {
		// Handle --touch mode
		let touch_path = parse_touch_path(input)?;

		// Determine the extension to use
		let effective_ext = touch_path.extension.unwrap_or(extension);

		// Check if the project is virtual
		let project_is_virtual = is_virtual_project(&touch_path.owner, &touch_path.repo);
		let effective_offline = offline || project_is_virtual;

		// First, try to find an existing local issue file
		let issue_file_path = if let Some(existing_path) = find_local_issue_for_touch(&touch_path, &effective_ext) {
			println!("Found existing issue: {existing_path:?}");
			existing_path
		} else if project_is_virtual {
			// Virtual project: stays local forever
			println!("Project {}/{} is virtual (no GitHub remote)", touch_path.owner, touch_path.repo);
			create_virtual_issue(&touch_path, &effective_ext)?
		} else {
			// Real project: create pending issue (will be created on GitHub when editor closes)
			create_pending_issue(&touch_path, &effective_ext)?
		};

		(issue_file_path, local_sync_opts(), effective_offline)
	} else if github::is_github_issue_url(input) {
		// GitHub URL mode: fetch issue and store in XDG_DATA (can't be offline)
		if offline {
			bail!("Cannot fetch issue from URL in offline mode");
		}

		let (owner, repo, issue_number) = github::parse_github_issue_url(input)?;

		println!("Fetching issue #{issue_number} from {owner}/{repo}...");

		// Fetch and store issue (and sub-issues) in XDG_DATA
		// This fetch IS the "take remote" action for --force/--reset with remote source
		let issue_file_path = fetch_and_store_issue(&gh, &owner, &repo, issue_number, &extension, None).await?;

		println!("Stored issue at: {issue_file_path:?}");

		// Commit the fetched state as the consensus baseline for post-editor sync
		// This ensures that changes made during editing can be properly detected
		use super::git::commit_issue_changes;
		commit_issue_changes(&issue_file_path, &owner, &repo, issue_number, Some("initial fetch"))?;

		// For remote source: the fetch IS the sync action.
		// Post-editor sync uses Normal mode (changes get synced normally).
		// Note: --reset and --force are consumed by the fetch itself.
		let remote_sync_opts = SyncOptions::new(None, false);
		(issue_file_path, remote_sync_opts, offline)
	} else {
		// Check if input is an existing file path (absolute or relative)
		let input_path = Path::new(input);
		if input_path.exists() && input_path.is_file() {
			// Direct file path - open it
			(input_path.to_path_buf(), local_sync_opts(), offline)
		} else {
			// Local search mode: find and open existing issue file
			let matches = search_issue_files(input)?;

			let issue_file_path = match matches.len() {
				0 => {
					// No matches - open fzf with all files and use input as initial query
					let all_files = search_issue_files("")?;
					if all_files.is_empty() {
						bail!("No issue files found. Use a GitHub URL to fetch an issue first.");
					}
					match choose_issue_with_fzf(&all_files, input)? {
						Some(path) => path,
						None => bail!("No issue selected"),
					}
				}
				1 => matches[0].clone(),
				_ => {
					// Multiple matches - open fzf to choose
					match choose_issue_with_fzf(&matches, input)? {
						Some(path) => path,
						None => bail!("No issue selected"),
					}
				}
			};

			(issue_file_path, local_sync_opts(), offline)
		}
	};

	// Open the local issue file for editing
	// If using blocker mode, open at the last blocker position
	open_local_issue(&gh, &issue_file_path, effective_offline, sync_opts, use_blocker_mode).await?;

	// If --blocker-set was used, set this issue as the current blocker issue
	if args.blocker_set {
		crate::blocker_interactions::integration::set_current_blocker_issue(&issue_file_path)?;
		println!("Set current blocker issue to: {}", issue_file_path.display());
	}

	Ok(())
}
