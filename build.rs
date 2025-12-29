use std::process::Command;

fn main() {
	// Embed git commit hash
	let output = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().unwrap();
	let git_hash = String::from_utf8(output.stdout).unwrap();
	println!("cargo:rustc-env=GIT_HASH={}", git_hash.trim());

	// Embed log directives if .cargo/log_directives exists
	println!("cargo:rerun-if-changed=.cargo/log_directives");
	if let Ok(directives) = std::fs::read_to_string(".cargo/log_directives") {
		let directives = directives.trim();
		if !directives.is_empty() {
			println!("cargo:rustc-env=LOG_DIRECTIVES={directives}");
		}
	}
}
