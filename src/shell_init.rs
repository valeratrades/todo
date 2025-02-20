use clap::{Args, CommandFactory};
use clap_complete::Shell as ClapShell;
use derive_more::derive::{Display, FromStr};

use crate::config::{AppConfig, EXE_NAME};

#[derive(Clone, Debug, Args)]
pub struct ShellInitArgs {
	shell: Shell,
}
#[derive(Debug, Clone, Copy, Display, FromStr)]
enum Shell {
	Dash,
	Bash,
	Zsh,
	Fish,
}

impl Shell {
	fn aliases(&self, exe_name: &str) -> String {
		format!(
			r#"
# {exe_name}s-manual
alias tm="{exe_name} manual"
alias tmc="tm ev -c --" # `--` allows for negative values (as otherwise they are interpreted as flags)
alias tmr="tm ev -r --"

# {exe_name}s-{exe_name}s
alias tdo="{exe_name} open"
alias tda="{exe_name} add"

# {exe_name}s-blocker
alias tdp="{exe_name} blocker pop"
alias tdb="{exe_name} blocker add"
alias tdbs="{exe_name} blocker list"
alias tdbo="{exe_name} blocker open"
"#
		)
	}

	fn to_clap_shell(self) -> ClapShell {
		match self {
			Shell::Dash => ClapShell::Bash, // Dash uses Bash completions
			Shell::Bash => ClapShell::Bash,
			Shell::Zsh => ClapShell::Zsh,
			Shell::Fish => ClapShell::Fish,
		}
	}

	fn completions(&self) -> String {
		let mut cmd = crate::Cli::command(); // Generate the Clap `Command` for your app
		let mut buffer = Vec::new();
		let shell = self.to_clap_shell();
		clap_complete::generate(shell, &mut cmd, EXE_NAME, &mut buffer);

		String::from_utf8(buffer).unwrap_or_else(|_| String::from("# Failed to generate completions"))
	}

	fn hooks(&self) -> String {
		"".to_owned()
	}
}

pub fn output(_settings: AppConfig, args: ShellInitArgs) {
	let shell = args.shell;
	let s = format!(
		r#"{}
{}
{}"#,
		shell.aliases(EXE_NAME),
		shell.completions(),
		shell.hooks()
	);

	println!("{s}");
}
