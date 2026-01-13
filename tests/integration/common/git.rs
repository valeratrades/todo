//! Git extension for TestContext.

use std::path::PathBuf;

use v_fixtures::fs_standards::git::Git;

use super::TestContext;

/// Extension trait adding git operations to TestContext.
pub trait GitExt {
	/// Initialize git in the issues directory.
	fn init_git(&self) -> Git;

	/// Initialize git in a specific subdirectory of the data directory.
	fn init_git_in(&self, subdir: &str) -> Git;
}

impl GitExt for TestContext {
	fn init_git(&self) -> Git {
		Git::init(self.xdg.data_dir().join("issues"))
	}

	fn init_git_in(&self, subdir: &str) -> Git {
		Git::init(self.xdg.data_dir().join(subdir))
	}
}

/// Helper to get the issues directory path for a given owner/repo.
pub fn issues_dir_path(owner: &str, repo: &str) -> PathBuf {
	PathBuf::from(format!("issues/{owner}/{repo}"))
}
