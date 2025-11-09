//! Integration tests entry point, following https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html

use std::sync::OnceLock;

static BINARY_COMPILED: OnceLock<()> = OnceLock::new();

/// Compile the binary with is_integration_test feature before running any tests
fn ensure_binary_compiled() {
	BINARY_COMPILED.get_or_init(|| {
		let status = std::process::Command::new("cargo")
			.args(["build", "--features", "is_integration_test"])
			.status()
			.expect("Failed to execute cargo build");

		if !status.success() {
			panic!("Failed to build binary with is_integration_test feature");
		}
	});
}

mod blocker_format;
// mod config_warnings; //dbg: temporarily disabled, as we're failing to make directory for `STATE_DIR` while in nixos eval env
