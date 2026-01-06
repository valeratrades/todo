//! Integration tests entry point, following https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html

use std::sync::OnceLock;

static BINARY_COMPILED: OnceLock<()> = OnceLock::new();

/// Compile the binary before running any tests
pub fn ensure_binary_compiled() {
	BINARY_COMPILED.get_or_init(|| {
		let status = std::process::Command::new("cargo").args(["build"]).status().expect("Failed to execute cargo build");

		if !status.success() {
			panic!("Failed to build binary");
		}
	});
}

mod blocker_format;
mod blocker_project_resolution;
mod fixtures;
