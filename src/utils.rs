use dirs;
use serde::{Deserialize, Deserializer};
use std::convert::AsRef;
use std::str::FromStr;
use std::{path::Path, path::PathBuf, process::Command};

#[derive(Clone, Debug)]
pub struct ExpandedPath(pub PathBuf);
impl<'de> Deserialize<'de> for ExpandedPath {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let path = String::deserialize(deserializer)?;
		Ok(ExpandedPath(expand_tilde(&path)))
	}
}
impl FromStr for ExpandedPath {
	type Err = std::io::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(ExpandedPath(expand_tilde(s)))
	}
}
fn expand_tilde(path: &str) -> PathBuf {
	if path.starts_with("~") {
		let home_dir = dirs::home_dir().unwrap();
		match path.len() {
			l if l < 2 => {
				return home_dir;
			}
			l if l > 2 => {
				return home_dir.join(&path[2..]);
			}
			_ => panic!("Incorrect Path"),
		}
	}
	PathBuf::from(path)
}
impl std::fmt::Display for ExpandedPath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0.display())
	}
}
impl AsRef<Path> for ExpandedPath {
	fn as_ref(&self) -> &Path {
		self.0.as_ref()
	}
}

pub fn open(path: PathBuf) {
	Command::new("sh")
		.arg("-c")
		.arg(format!("$EDITOR {}", path.display()))
		.status()
		.expect("$EDITOR env variable is not defined");
}
