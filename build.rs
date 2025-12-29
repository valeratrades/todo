use std::process::Command;

fn main() {
	// Embed git commit hash (fallback to "unknown" if git unavailable, e.g., in Nix sandbox)
	let git_hash = Command::new("git")
		.args(["rev-parse", "--short", "HEAD"])
		.output()
		.ok()
		.and_then(|o| String::from_utf8(o.stdout).ok())
		.map(|s| s.trim().to_owned())
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| "unknown".to_owned());
	println!("cargo:rustc-env=GIT_HASH={git_hash}");

	// Embed log directives if .cargo/log_directives exists
	println!("cargo:rerun-if-changed=.cargo/log_directives");
	if let Ok(directives) = std::fs::read_to_string(".cargo/log_directives") {
		let directives = directives.trim();
		if !directives.is_empty() {
			println!("cargo:rustc-env=LOG_DIRECTIVES={directives}");
		}
	}
}
