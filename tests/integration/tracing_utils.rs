//! Utilities for capturing and verifying tracing output in integration tests.
//!
//! When tests spawn the todo binary with `TODO_TRACE_FILE` set, trace events are
//! written in JSON format to that file. These utilities help parse and verify
//! those traces.
//!
//! The mock implementations emit `tracing::info!` events with target "mock_github"
//! that include method names and arguments. These can be verified using `has_mock_call`.

use std::{fs, path::PathBuf};

use serde::Deserialize;

/// A single trace event from the JSON log
#[derive(Debug, Deserialize)]
pub struct TraceEvent {
	/// The timestamp of the event
	pub timestamp: String,
	/// The log level (DEBUG, INFO, WARN, ERROR)
	pub level: String,
	/// The target module (e.g., "mock_github")
	pub target: String,
	/// The fields logged with the event (includes message and any other fields)
	pub fields: TraceFields,
}

#[derive(Debug, Deserialize)]
pub struct TraceFields {
	/// The message field from the trace event
	pub message: Option<String>,
	/// Owner field (for GitHub API calls)
	pub owner: Option<String>,
	/// Repo field (for GitHub API calls)
	pub repo: Option<String>,
	/// Issue number field
	pub issue_number: Option<u64>,
	/// Title field (for create_issue, find_issue_by_title)
	pub title: Option<String>,
	/// State field (for update_issue_state)
	pub state: Option<String>,
	/// Comment ID field
	pub comment_id: Option<u64>,
	/// Parent issue number (for add_sub_issue)
	pub parent_issue_number: Option<u64>,
	/// Child issue ID (for add_sub_issue)
	pub child_issue_id: Option<u64>,
}

/// Parsed trace log that provides verification methods
pub struct TraceLog {
	events: Vec<TraceEvent>,
}

impl TraceLog {
	/// Read and parse a trace log file
	pub fn from_file(path: &PathBuf) -> Self {
		let content = fs::read_to_string(path).unwrap_or_default();
		let events: Vec<TraceEvent> = content.lines().filter(|line| !line.is_empty()).filter_map(|line| serde_json::from_str(line).ok()).collect();

		Self { events }
	}

	/// Check if a mock method was called (by looking for info events with target "mock_github")
	pub fn has_mock_call(&self, method_name: &str) -> bool {
		self.events
			.iter()
			.any(|e| e.target == "mock_github" && e.fields.message.as_ref().is_some_and(|m| m == method_name))
	}

	/// Check if a mock method was called with specific arguments
	pub fn has_mock_call_with(&self, method_name: &str, owner: &str, repo: &str) -> bool {
		self.events.iter().any(|e| {
			e.target == "mock_github"
				&& e.fields.message.as_ref().is_some_and(|m| m == method_name)
				&& e.fields.owner.as_ref().is_some_and(|o| o == owner)
				&& e.fields.repo.as_ref().is_some_and(|r| r == repo)
		})
	}

	/// Check if a mock method was called with specific arguments including issue_number
	pub fn has_mock_call_with_issue(&self, method_name: &str, owner: &str, repo: &str, issue_number: u64) -> bool {
		self.events.iter().any(|e| {
			e.target == "mock_github"
				&& e.fields.message.as_ref().is_some_and(|m| m == method_name)
				&& e.fields.owner.as_ref().is_some_and(|o| o == owner)
				&& e.fields.repo.as_ref().is_some_and(|r| r == repo)
				&& e.fields.issue_number == Some(issue_number)
		})
	}

	/// Get all mock call events for debugging
	pub fn mock_calls(&self) -> Vec<&TraceEvent> {
		self.events.iter().filter(|e| e.target == "mock_github").collect()
	}

	/// Get the raw content of the trace file for debugging
	pub fn raw_content(path: &PathBuf) -> String {
		fs::read_to_string(path).unwrap_or_default()
	}
}

/// Assert that a mock method was called
#[macro_export]
macro_rules! assert_traced {
	($log:expr, $method:expr) => {
		assert!(
			$log.has_mock_call($method),
			"Expected mock call '{}' to be traced, but it wasn't. Mock calls:\n{:#?}",
			$method,
			$log.mock_calls()
		);
	};
	($log:expr, $method:expr, $owner:expr, $repo:expr) => {
		assert!(
			$log.has_mock_call_with($method, $owner, $repo),
			"Expected mock call '{}' with owner='{}' repo='{}' to be traced, but it wasn't. Mock calls:\n{:#?}",
			$method,
			$owner,
			$repo,
			$log.mock_calls()
		);
	};
	($log:expr, $method:expr, $owner:expr, $repo:expr, $issue_number:expr) => {
		assert!(
			$log.has_mock_call_with_issue($method, $owner, $repo, $issue_number),
			"Expected mock call '{}' with owner='{}' repo='{}' issue_number={} to be traced, but it wasn't. Mock calls:\n{:#?}",
			$method,
			$owner,
			$repo,
			$issue_number,
			$log.mock_calls()
		);
	};
}
