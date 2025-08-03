// src/main.rs
use std::env;

use chrono::{SecondsFormat, Utc};
use clap::Parser;
use color_eyre::eyre::{Result, WrapErr, eyre};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use urlencoding;

#[derive(Parser, Debug)]
#[command(name = "clockify-start", about = "Start a running Clockify time entry from the CLI")]
struct Args {
	/// Description for the time entry
	#[arg(short, long, default_value = "")]
	desc: String,

	/// Workspace ID (if omitted, use the user's active workspace)
	#[arg(short = 'w', long)]
	workspace: Option<String>,

	/// Project ID or name (name will be resolved if no exact ID match found)
	#[arg(short = 'p', long)]
	project: Option<String>,

	/// Task ID or name (requires --project; name will be resolved)
	#[arg(short = 't', long)]
	task: Option<String>,

	/// Comma-separated tag IDs or names (names will be resolved)
	#[arg(short = 'g', long)]
	tags: Option<String>,

	/// Mark entry as billable
	#[arg(short = 'b', long, default_value_t = false)]
	billable: bool,

	/// List all workspaces
	#[arg(long)]
	list_workspaces: bool,
}

#[derive(Deserialize)]
struct User {
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

#[tokio::main]
async fn main() -> Result<()> {
	color_eyre::install()?;
	let args = Args::parse();

	if args.list_workspaces {
		list_workspaces().await?;
		return Ok(());
	}

	let api_key = env::var("CLOCKIFY_API_KEY").wrap_err("Set CLOCKIFY_API_KEY in your environment with a valid API token")?;

	let client = reqwest::Client::builder().default_headers(make_headers(&api_key)?).build()?;

	let workspace_id = match args.workspace {
		Some(w) => w,
		None => get_active_workspace(&client).await?,
	};

	let project_id = if let Some(p) = args.project {
		Some(resolve_project(&client, &workspace_id, &p).await?)
	} else {
		None
	};

	let task_id = if let Some(t) = args.task {
		let pid = project_id.as_ref().ok_or_else(|| eyre!("--task requires --project to be set"))?;
		Some(resolve_task(&client, &workspace_id, pid, &t).await?)
	} else {
		None
	};

	let tag_ids = if let Some(t) = args.tags {
		Some(resolve_tags(&client, &workspace_id, &t).await?)
	} else {
		None
	};

	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

	let payload = NewTimeEntry {
		start: now,
		description: args.desc,
		billable: args.billable,
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

	Ok(())
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

	if let Some(p) = projects.iter().find(|p| p.name == input) {
		return Ok(p.id.clone());
	}
	if projects.is_empty() {
		// Fallback: fetch first 200 active projects and try a loose match
		let url = format!("https://api.clockify.me/api/v1/workspaces/{ws}/projects?archived=false&page=1&page-size=200");
		projects = client.get(url).send().await?.error_for_status()?.json().await?;
		if let Some(p) = projects.iter().find(|p| p.name.eq_ignore_ascii_case(input) || p.name.contains(input)) {
			return Ok(p.id.clone());
		}
		return Err(eyre!("Project not found: {input}"));
	}
	Ok(projects.remove(0).id)
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

fn looks_like_id(s: &str) -> bool {
	// Clockify IDs are usually 24-char hex or UUID. Check a couple of common patterns.
	let is_hex24 = s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit());
	let is_uuid = {
		let parts: Vec<&str> = s.split('-').collect();
		parts.len() == 5 && parts[0].len() == 8 && parts[1].len() == 4 && parts[2].len() == 4 && parts[3].len() == 4 && parts[4].len() == 12
	};
	is_hex24 || is_uuid
}
