use std::env;
use std::io::Write;

use chrono::{SecondsFormat, Utc};
use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr, eyre};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use urlencoding;

use crate::config::{AppConfig, CACHE_DIR};

static CURRENT_PROJECT_CACHE_FILENAME: &str = "current_project.txt";
static LEGACY_PROJECT_ID: &str = "66d83316b6114535ad872316";

// Helper function to process filename for use as project name
fn process_filename_as_project(relative_path: &str) -> String {
	// Extract filename from path (everything after the last slash, or the whole string if no slash)
	let filename = match relative_path.rfind('/') {
		Some(pos) => &relative_path[pos + 1..],
		None => relative_path,
	};

	// Strip file extension
	let name_without_ext = match filename.rfind('.') {
		Some(pos) => &filename[..pos],
		None => filename,
	};

	// Convert underscores to spaces
	name_without_ext.replace('_', " ")
}

#[derive(Debug, Clone, Args)]
pub struct ClockifyArgs {
	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
	/// Start a new time entry
	Start(StartArgs),
	/// Stop the currently running time entry
	Stop(StopArgs),
	/// List workspaces
	ListWorkspaces,
	/// List projects in a workspace
	ListProjects(ListProjectsArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct StartArgs {
	/// Description for the time entry
	pub description: String,

	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,

	/// Project ID or name (name will be resolved if no exact ID match found)
	#[arg(short = 'p', long)]
	pub project: Option<String>,

	/// Task ID or name (requires --project; name will be resolved)
	#[arg(short = 't', long)]
	pub task: Option<String>,

	/// Comma-separated tag IDs or names (names will be resolved)
	#[arg(short = 'g', long)]
	pub tags: Option<String>,

	/// Mark entry as billable
	#[arg(short = 'b', long, default_value_t = false)]
	pub billable: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct StopArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,
}

#[derive(Parser, Debug, Clone)]
pub struct ListProjectsArgs {
	/// Workspace ID or name (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	pub workspace: Option<String>,
}

#[derive(Deserialize)]
struct User {
	id: String,
	#[serde(rename = "activeWorkspace")]
	active_workspace: String,
}

#[derive(Deserialize)]
struct Workspace {
	id: String,
	name: String,
}

#[derive(Deserialize)]
struct Project {
	id: String,
	name: String,
	#[serde(default)]
	archived: bool,
}

#[derive(Deserialize)]
struct Task {
	id: String,
	name: String,
	#[serde(rename = "projectId")]
	project_id: String,
	#[serde(default)]
	archived: bool,
}

#[derive(Deserialize)]
struct Tag {
	id: String,
	name: String,
	#[serde(default)]
	archived: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewTimeEntry {
	start: String,
	description: String,
	billable: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	project_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	task_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	tag_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct CreatedEntry {
	id: String,
	description: String,
	#[serde(rename = "workspaceId")]
	workspace_id: String,
	#[serde(rename = "projectId")]
	project_id: Option<String>,
	#[serde(rename = "taskId")]
	task_id: Option<String>,
	#[serde(rename = "timeInterval")]
	time_interval: TimeInterval,
}

#[derive(Deserialize)]
struct TimeInterval {
	start: String,
	end: Option<String>,
}

// Public functions for use by other modules
pub async fn start_time_entry(workspace: &str, project: &str, description: String, task: Option<&str>, tags: Option<&str>, billable: bool) -> Result<()> {
	let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;
	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	let workspace_id = resolve_workspace(&client, workspace).await?;
	let project_id = Some(resolve_project(&client, &workspace_id, project).await?);

	let task_id = if let Some(t) = task {
		let pid = project_id.as_ref().ok_or_else(|| eyre!("--task requires --project to be set"))?;
		Some(resolve_task(&client, &workspace_id, pid, t).await?)
	} else {
		None
	};

	let tag_ids = if let Some(t) = tags { Some(resolve_tags(&client, &workspace_id, t).await?) } else { None };

	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

	let payload = NewTimeEntry {
		start: now,
		description,
		billable,
		project_id,
		task_id,
		tag_ids,
	};

	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/time-entries", workspace_id);

	let created: CreatedEntry = client
		.post(url)
		.json(&payload)
		.send()
		.await
		.wrap_err("Failed to create time entry")?
		.error_for_status()
		.wrap_err("Clockify API returned an error creating the time entry")?
		.json()
		.await
		.wrap_err("Failed to parse Clockify response")?;

	println!("Started working on blocker:");
	println!("  id: {}", created.id);
	println!("  description: {}", created.description);
	println!("  start: {}", created.time_interval.start);
	println!("  project: {}", created.project_id.as_deref().unwrap_or("<none>"));
	println!("  task: {}", created.task_id.as_deref().unwrap_or("<none>"));
	println!("  workspace: {}", created.workspace_id);

	Ok(())
}

pub async fn stop_time_entry(workspace: &str) -> Result<()> {
	let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;
	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	let workspace_id = resolve_workspace(&client, workspace).await?;
	stop_current_entry_by_id(&workspace_id).await?;

	Ok(())
}

// New functions with optional parameters for blocker integration
pub async fn start_time_entry_with_defaults(
	workspace: Option<&str>,
	project: Option<&str>,
	description: String,
	task: Option<&str>,
	tags: Option<&str>,
	billable: bool,
	legacy: bool,
	filename: Option<&str>,
) -> Result<()> {
	let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;
	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	// Resolve workspace - use active workspace if not provided
	let workspace_id = match workspace {
		Some(w) => resolve_workspace(&client, w).await?,
		None => get_active_workspace(&client).await?,
	};

	// Handle legacy mode vs normal mode
	let (project_id, final_description) = if legacy {
		// Legacy mode: use hardcoded project ID and prefix description with processed filename
		let project_prefix = match filename {
			Some(fname) => process_filename_as_project(fname),
			None => {
				// Fallback to cached filename if not provided
				let persisted_project_file = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
				match std::fs::read_to_string(&persisted_project_file) {
					Ok(cached_filename) => process_filename_as_project(&cached_filename),
					Err(_) => return Err(eyre!("Legacy mode requires a filename for project prefix")),
				}
			}
		};

		println!("Using legacy mode with project ID: {} and prefix: {}", LEGACY_PROJECT_ID, project_prefix);
		let prefixed_description = format!("{}: {}", project_prefix, description);
		(Some(LEGACY_PROJECT_ID.to_string()), prefixed_description)
	} else {
		// Normal mode: resolve project as before
		let cached_project = if project.is_none() {
			let persisted_project_file = CACHE_DIR.get().unwrap().join(CURRENT_PROJECT_CACHE_FILENAME);
			std::fs::read_to_string(&persisted_project_file).ok()
		} else {
			None
		};

		let processed_project = cached_project.as_ref().map(|filename| process_filename_as_project(filename));

		let project_name = match project {
			Some(p) => p,
			None => match &processed_project {
				Some(processed) => {
					println!("Using cached project (processed from filename): {}", processed);
					processed.as_str()
				}
				None => {
					return Err(eyre!(
						"--project is required for starting time entries. You can set a default with 'todo blocker project <project-name>'"
					));
				}
			},
		};

		let resolved_project_id = resolve_project(&client, &workspace_id, project_name).await?;
		(Some(resolved_project_id), description)
	};

	let task_id = if let Some(t) = task {
		let pid = project_id.as_ref().ok_or_else(|| eyre!("--task requires --project to be set"))?;
		Some(resolve_task(&client, &workspace_id, pid, t).await?)
	} else {
		None
	};

	let tag_ids = if let Some(t) = tags { Some(resolve_tags(&client, &workspace_id, t).await?) } else { None };

	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

	let payload = NewTimeEntry {
		start: now,
		description: final_description,
		billable,
		project_id,
		task_id,
		tag_ids,
	};

	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/time-entries", workspace_id);

	let created: CreatedEntry = client
		.post(url)
		.json(&payload)
		.send()
		.await
		.wrap_err("Failed to create time entry")?
		.error_for_status()
		.wrap_err("Clockify API returned an error creating the time entry")?
		.json()
		.await
		.wrap_err("Failed to parse Clockify response")?;

	println!("Started working on blocker:");
	println!("  id: {}", created.id);
	println!("  description: {}", created.description);
	println!("  start: {}", created.time_interval.start);
	println!("  project: {}", created.project_id.as_deref().unwrap_or("<none>"));
	println!("  task: {}", created.task_id.as_deref().unwrap_or("<none>"));
	println!("  workspace: {}", created.workspace_id);

	Ok(())
}

pub async fn stop_time_entry_with_defaults(workspace: Option<&str>) -> Result<()> {
	let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;
	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	// Resolve workspace - use active workspace if not provided
	let workspace_id = match workspace {
		Some(w) => resolve_workspace(&client, w).await?,
		None => get_active_workspace(&client).await?,
	};

	stop_current_entry_by_id(&workspace_id).await?;

	Ok(())
}

pub fn main(_config: AppConfig, args: ClockifyArgs) -> Result<()> {
	tokio::runtime::Runtime::new()?.block_on(async {
		match args.command {
			Command::ListWorkspaces => {
				list_workspaces().await?;
			}
			Command::ListProjects(list_args) => {
				let workspace_name = list_args.workspace.as_deref().unwrap_or("default");
				list_projects(workspace_name).await?;
			}
			Command::Stop(stop_args) => {
				let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;
				let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

				let workspace_id = match stop_args.workspace {
					Some(w) => resolve_workspace(&client, &w).await?,
					None => get_active_workspace(&client).await?,
				};

				stop_current_entry_by_id(&workspace_id).await?;
			}
			Command::Start(start_args) => {
				let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;
				let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

				let workspace_id = match start_args.workspace {
					Some(w) => resolve_workspace(&client, &w).await?,
					None => get_active_workspace(&client).await?,
				};

				// Require project for creating time entries
				let project = start_args.project.ok_or_else(|| eyre!("--project is required when creating time entries"))?;

				let project_id = Some(resolve_project(&client, &workspace_id, &project).await?);

				let task_id = if let Some(t) = start_args.task {
					let pid = project_id.as_ref().ok_or_else(|| eyre!("--task requires --project to be set"))?;
					Some(resolve_task(&client, &workspace_id, pid, &t).await?)
				} else {
					None
				};

				let tag_ids = if let Some(t) = start_args.tags {
					Some(resolve_tags(&client, &workspace_id, &t).await?)
				} else {
					None
				};

				let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

				let payload = NewTimeEntry {
					start: now,
					description: start_args.description,
					billable: start_args.billable,
					project_id,
					task_id,
					tag_ids,
				};

				let url = format!("https://api.clockify.me/api/v1/workspaces/{}/time-entries", workspace_id);

				let created: CreatedEntry = client
					.post(url)
					.json(&payload)
					.send()
					.await
					.wrap_err("Failed to create time entry")?
					.error_for_status()
					.wrap_err("Clockify API returned an error creating the time entry")?
					.json()
					.await
					.wrap_err("Failed to parse Clockify response")?;

				println!("Started entry:");
				println!("  id: {}", created.id);
				println!("  description: {}", created.description);
				println!("  start: {}", created.time_interval.start);
				println!("  project: {}", created.project_id.as_deref().unwrap_or("<none>"));
				println!("  task: {}", created.task_id.as_deref().unwrap_or("<none>"));
				println!("  workspace: {}", created.workspace_id);
			}
		}

		Ok(())
	})
}

fn make_headers(api_key: &str) -> Result<HeaderMap> {
	let mut h = HeaderMap::new();
	h.insert("X-Api-Key", HeaderValue::from_str(api_key).wrap_err("Invalid CLOCKIFY_API_KEY value")?);
	h.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
	Ok(h)
}

async fn get_active_workspace(client: &reqwest::Client) -> Result<String> {
	let user: User = client
		.get("https://api.clockify.me/api/v1/user")
		.send()
		.await
		.wrap_err("Failed to fetch user")?
		.error_for_status()
		.wrap_err("Clockify API returned an error fetching user")?
		.json()
		.await
		.wrap_err("Failed to parse user response")?;
	Ok(user.active_workspace)
}

async fn resolve_project(client: &reqwest::Client, ws: &str, input: &str) -> Result<String> {
	// If input looks like an ID (UUID-ish), try it directly by fetching it
	if looks_like_id(input) {
		if let Ok(id) = fetch_project_by_id(client, ws, input).await {
			return Ok(id);
		}
	}
	// Otherwise search by name (exact, then case-insensitive substring)
	let url = format!("https://api.clockify.me/api/v1/workspaces/{ws}/projects?archived=false&name={}", urlencoding::encode(input));
	let mut projects: Vec<Project> = client.get(url).send().await?.error_for_status()?.json().await?;

	// Exact match first
	if let Some(p) = projects.iter().find(|p| p.name == input) {
		return Ok(p.id.clone());
	}

	// Exact case-insensitive match
	if let Some(p) = projects.iter().find(|p| p.name.eq_ignore_ascii_case(input)) {
		return Ok(p.id.clone());
	}

	// Case-insensitive substring match
	let input_lower = input.to_lowercase();
	if let Some(p) = projects.iter().find(|p| p.name.to_lowercase().contains(&input_lower)) {
		return Ok(p.id.clone());
	}

	if projects.is_empty() {
		// Fallback: fetch first 200 active projects and try a loose match
		let url = format!("https://api.clockify.me/api/v1/workspaces/{ws}/projects?archived=false&page=1&page-size=200");
		projects = client.get(url).send().await?.error_for_status()?.json().await?;

		// Repeat the same matching logic for the full list
		if let Some(p) = projects.iter().find(|p| p.name == input) {
			return Ok(p.id.clone());
		}
		if let Some(p) = projects.iter().find(|p| p.name.eq_ignore_ascii_case(input)) {
			return Ok(p.id.clone());
		}
		if let Some(p) = projects.iter().find(|p| p.name.to_lowercase().contains(&input_lower)) {
			return Ok(p.id.clone());
		}
	}

	// Project not found - ask user if they want to create it
	println!("Project '{}' not found in Clockify workspace.", input);
	print!("Would you like to create a new Clockify project with this exact name? [y/N]: ");
	Write::flush(&mut std::io::stdout())?;

	let mut response = String::new();
	std::io::stdin().read_line(&mut response)?;
	
	if response.trim().to_lowercase() == "y" || response.trim().to_lowercase() == "yes" {
		let project_id = create_project(client, ws, input).await?;
		println!("Created new project '{}' with ID: {}", input, project_id);
		Ok(project_id)
	} else {
		Err(eyre!("Project not found and user declined to create: {}", input))
	}
}

async fn fetch_project_by_id(client: &reqwest::Client, ws: &str, id: &str) -> Result<String> {
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/projects/{}", ws, id);
	let _p: Project = client.get(url).send().await?.error_for_status()?.json().await?;
	Ok(id.to_string())
}

async fn resolve_task(client: &reqwest::Client, ws: &str, project_id: &str, input: &str) -> Result<String> {
	if looks_like_id(input) {
		if let Ok(id) = fetch_task_by_id(client, ws, project_id, input).await {
			return Ok(id);
		}
	}
	// Clockify tasks listing is per project
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/projects/{}/tasks?page-size=200", ws, project_id);
	let tasks: Vec<Task> = client.get(url).send().await?.error_for_status()?.json().await?;

	if let Some(t) = tasks.iter().find(|t| t.name == input) {
		return Ok(t.id.clone());
	}
	if let Some(t) = tasks.iter().find(|t| t.name.eq_ignore_ascii_case(input) || t.name.contains(input)) {
		return Ok(t.id.clone());
	}
	Err(eyre!("Task not found in project {}: {}", project_id, input))
}

async fn fetch_task_by_id(client: &reqwest::Client, ws: &str, project_id: &str, id: &str) -> Result<String> {
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/projects/{}/tasks/{}", ws, project_id, id);
	let _t: Task = client.get(url).send().await?.error_for_status()?.json().await?;
	Ok(id.to_string())
}

async fn resolve_tags(client: &reqwest::Client, ws: &str, input: &str) -> Result<Vec<String>> {
	// Split by comma and trim
	let wanted: Vec<String> = input.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

	if wanted.is_empty() {
		return Ok(vec![]);
	}

	// First, collect any that already look like IDs and verify them
	let mut ids: Vec<String> = Vec::new();
	let mut names: Vec<String> = Vec::new();
	for w in &wanted {
		if looks_like_id(w) {
			ids.push(w.clone());
		} else {
			names.push(w.clone());
		}
	}

	// Verify ID tags exist
	if !ids.is_empty() {
		let all = fetch_tags(client, ws).await?;
		for id in ids.clone() {
			if !all.iter().any(|t| t.id == id) {
				return Err(eyre!("Tag ID not found: {}", id));
			}
		}
	}

	// Resolve names
	if !names.is_empty() {
		let all = fetch_tags(client, ws).await?;
		for n in names {
			if let Some(t) = all.iter().find(|t| t.name == n) {
				ids.push(t.id.clone());
			} else if let Some(t) = all.iter().find(|t| t.name.eq_ignore_ascii_case(&n) || t.name.contains(&n)) {
				ids.push(t.id.clone());
			} else {
				return Err(eyre!("Tag not found: {}", n));
			}
		}
	}

	Ok(ids)
}

async fn fetch_tags(client: &reqwest::Client, ws: &str) -> Result<Vec<Tag>> {
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/tags?page-size=200", ws);
	let tags: Vec<Tag> = client.get(url).send().await?.error_for_status()?.json().await?;
	Ok(tags.into_iter().filter(|t| !t.archived).collect())
}

async fn create_project(client: &reqwest::Client, ws: &str, name: &str) -> Result<String> {
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/projects", ws);
	
	#[derive(Serialize)]
	struct NewProject {
		name: String,
		color: String,
		billable: bool,
		public: bool,
	}
	
	let new_project = NewProject {
		name: name.to_string(),
		color: "#2196F3".to_string(), // Default blue color
		billable: false,
		public: true,
	};
	
	let created: Project = client
		.post(url)
		.json(&new_project)
		.send()
		.await
		.wrap_err("Failed to create project")?
		.error_for_status()
		.wrap_err("Clockify API returned an error creating the project")?
		.json()
		.await
		.wrap_err("Failed to parse project creation response")?;
	
	Ok(created.id)
}

async fn resolve_workspace(client: &reqwest::Client, input: &str) -> Result<String> {
	// If input looks like an ID, try it directly
	if looks_like_id(input) {
		return Ok(input.to_string());
	}

	// Otherwise search by name
	let workspaces: Vec<Workspace> = client
		.get("https://api.clockify.me/api/v1/workspaces")
		.send()
		.await
		.wrap_err("Failed to fetch workspaces")?
		.error_for_status()
		.wrap_err("Clockify API returned an error fetching workspaces")?
		.json()
		.await
		.wrap_err("Failed to parse workspaces response")?;

	// Exact match first
	if let Some(w) = workspaces.iter().find(|w| w.name == input) {
		return Ok(w.id.clone());
	}

	// Exact case-insensitive match
	if let Some(w) = workspaces.iter().find(|w| w.name.eq_ignore_ascii_case(input)) {
		return Ok(w.id.clone());
	}

	// Case-insensitive substring match
	let input_lower = input.to_lowercase();
	if let Some(w) = workspaces.iter().find(|w| w.name.to_lowercase().contains(&input_lower)) {
		return Ok(w.id.clone());
	}

	Err(eyre!("Workspace not found: {}", input))
}

async fn stop_current_entry_by_id(workspace_id: &str) -> Result<()> {
	let api_key = std::env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;

	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	// Get user ID first
	let user: User = client
		.get("https://api.clockify.me/api/v1/user")
		.send()
		.await
		.wrap_err("Failed to fetch user")?
		.error_for_status()
		.wrap_err("Clockify API returned an error fetching user")?
		.json()
		.await
		.wrap_err("Failed to parse user response")?;

	println!("User ID: {}", user.id);

	// Try the alternative endpoint: get recent time entries and filter for running ones
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/user/{}/time-entries?page-size=10", workspace_id, user.id);
	println!("Checking for recent time entries at: {}", url);

	let entries: Vec<CreatedEntry> = client
		.get(&url)
		.send()
		.await
		.wrap_err("Failed to fetch time entries")?
		.error_for_status()
		.wrap_err("Clockify API returned an error fetching time entries")?
		.json()
		.await
		.wrap_err("Failed to parse time entries response")?;

	println!("Found {} recent entries", entries.len());

	// Find running entry (one without end time)
	let running_entry = entries.iter().find(|entry| entry.time_interval.end.is_none());

	if let Some(entry) = running_entry {
		println!("Found running entry: {} - {}", entry.id, entry.description);
	} else {
		println!("No running time entry found - already stopped");
		return Ok(());
	}

	let entry = running_entry.unwrap();
	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

	// Stop the time entry using the correct endpoint
	let stop_url = format!("https://api.clockify.me/api/v1/workspaces/{}/time-entries/{}", workspace_id, entry.id);
	let stop_payload = serde_json::json!({
		"start": entry.time_interval.start,
		"billable": false,
		"description": entry.description,
		"projectId": entry.project_id,
		"taskId": entry.task_id,
		"end": now
	});

	let _: CreatedEntry = client
		.put(&stop_url)
		.json(&stop_payload)
		.send()
		.await
		.wrap_err("Failed to stop time entry")?
		.error_for_status()
		.wrap_err("Clockify API returned an error stopping the time entry")?
		.json()
		.await
		.wrap_err("Failed to parse stop response")?;

	println!("Stopped time entry: {} - {}", entry.id, entry.description);
	Ok(())
}

async fn list_workspaces() -> Result<()> {
	let api_key = std::env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;

	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	let workspaces: Vec<Workspace> = client
		.get("https://api.clockify.me/api/v1/workspaces")
		.send()
		.await
		.wrap_err("Failed to fetch workspaces")?
		.error_for_status()
		.wrap_err("Clockify API returned an error fetching workspaces")?
		.json()
		.await
		.wrap_err("Failed to parse workspaces response")?;

	println!("Your workspaces:");
	for workspace in workspaces {
		println!("  {} - {}", workspace.id, workspace.name);
	}

	Ok(())
}

async fn list_projects(workspace_input: &str) -> Result<()> {
	let api_key = std::env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;

	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	let workspace_id = if workspace_input == "default" {
		get_active_workspace(&client).await?
	} else {
		resolve_workspace(&client, workspace_input).await?
	};

	// Get all projects in the workspace
	let url = format!("https://api.clockify.me/api/v1/workspaces/{}/projects?archived=false&page-size=200", workspace_id);
	let projects: Vec<Project> = client
		.get(&url)
		.send()
		.await
		.wrap_err("Failed to fetch projects")?
		.error_for_status()
		.wrap_err("Clockify API returned an error fetching projects")?
		.json()
		.await
		.wrap_err("Failed to parse projects response")?;

	println!("Projects in workspace {}:", workspace_id);
	for project in projects {
		println!("  {} - {} (archived: {})", project.id, project.name, project.archived);
	}

	Ok(())
}

fn looks_like_id(s: &str) -> bool {
	// Clockify IDs are usually 24-char hex or UUID. Check a couple of common patterns.
	let is_hex24 = s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit());
	let is_uuid = {
		let parts: Vec<&str> = s.split('-').collect();
		parts.len() == 5 && parts[0].len() == 8 && parts[1].len() == 4 && parts[2].len() == 4 && parts[3].len() == 4 && parts[4].len() == 12
	};
	is_hex24 || is_uuid
}
