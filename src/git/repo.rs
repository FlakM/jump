use anyhow::{Context, Result};
use git2::Repository;
use std::path::{Path, PathBuf};
use tracing::debug;

use super::types::RepoInfo;

pub trait GitRepository {
    fn find_root(&self, from: &Path) -> Result<PathBuf>;
    fn get_remote_url(&self, remote: &str) -> Result<String>;
    fn get_current_revision(&self) -> Result<String>;
    fn get_relative_path(&self, absolute: &Path) -> Result<PathBuf>;
}

pub struct Git2Repository {
    repo: Repository,
}

impl Git2Repository {
    pub fn discover(path: &Path) -> Result<Self> {
        let repo = Repository::discover(path)
            .with_context(|| format!("Failed to discover git repository from {:?}", path))?;

        debug!("Discovered git repository at: {:?}", repo.path());

        Ok(Self { repo })
    }

    pub fn get_info(&self, remote_name: &str) -> Result<RepoInfo> {
        let root = self.find_root(self.repo.workdir().context("Not a working directory")?)?;
        let remote_url = self.get_remote_url(remote_name)?;
        let revision = self.get_current_revision()?;

        Ok(RepoInfo {
            root,
            remote_url,
            revision,
        })
    }
}

impl GitRepository for Git2Repository {
    fn find_root(&self, _from: &Path) -> Result<PathBuf> {
        self.repo
            .workdir()
            .map(|p| p.to_path_buf())
            .context("Repository has no working directory")
    }

    fn get_remote_url(&self, remote: &str) -> Result<String> {
        let remote = self
            .repo
            .find_remote(remote)
            .with_context(|| format!("Remote '{}' not found", remote))?;

        let url = remote
            .url()
            .context("Remote URL is not valid UTF-8")?
            .to_string();

        debug!("Found remote URL: {}", url);

        Ok(normalize_git_url(&url))
    }

    fn get_current_revision(&self) -> Result<String> {
        let head = self.repo.head().context("Failed to get HEAD")?;
        let commit = head
            .peel_to_commit()
            .context("Failed to peel HEAD to commit")?;
        let oid = commit.id().to_string();

        debug!("Current revision: {}", oid);

        Ok(oid)
    }

    fn get_relative_path(&self, absolute: &Path) -> Result<PathBuf> {
        let root = self.find_root(absolute)?;
        let relative = absolute
            .strip_prefix(&root)
            .with_context(|| {
                format!(
                    "Path {:?} is not within repository root {:?}",
                    absolute, root
                )
            })?
            .to_path_buf();

        Ok(relative)
    }
}

fn normalize_git_url(url: &str) -> String {
    // Convert SSH URLs to HTTPS
    // git@github.com:user/repo.git -> https://github.com/user/repo
    if let Some(ssh_url) = url.strip_prefix("git@") {
        if let Some((host, path)) = ssh_url.split_once(':') {
            let path = path.strip_suffix(".git").unwrap_or(path);
            return format!("https://{}/{}", host, path);
        }
    }

    // Strip .git suffix from HTTPS URLs
    url.strip_suffix(".git").unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_git_url() {
        assert_eq!(
            normalize_git_url("git@github.com:FlakM/jump.git"),
            "https://github.com/FlakM/jump"
        );

        assert_eq!(
            normalize_git_url("https://github.com/FlakM/jump.git"),
            "https://github.com/FlakM/jump"
        );

        assert_eq!(
            normalize_git_url("https://github.com/FlakM/jump"),
            "https://github.com/FlakM/jump"
        );
    }
}
