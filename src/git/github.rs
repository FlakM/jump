use anyhow::Result;
use std::path::Path;
use tracing::debug;

use super::repo::{Git2Repository, GitRepository};
use super::types::{GitHubLink, LineRange};

pub trait PermalinkGenerator {
    fn generate(&self, file: &Path, start_line: u32, end_line: Option<u32>) -> Result<GitHubLink>;
}

pub struct GitHubPermalinkGenerator {
    repo: Git2Repository,
    remote_name: String,
}

impl GitHubPermalinkGenerator {
    pub fn new(file: &Path, remote_name: Option<String>) -> Result<Self> {
        let repo = Git2Repository::discover(file)?;
        let remote_name = remote_name.unwrap_or_else(|| "origin".to_string());

        Ok(Self { repo, remote_name })
    }

    fn is_github_url(url: &str) -> bool {
        url.contains("github.com")
    }

    fn build_github_url(
        base_url: &str,
        revision: &str,
        relative_path: &str,
        lines: &LineRange,
    ) -> String {
        let line_fragment = match lines.end {
            Some(end) if end != lines.start => format!("#L{}-L{}", lines.start, end),
            _ => format!("#L{}", lines.start),
        };

        format!(
            "{}/blob/{}/{}{}",
            base_url, revision, relative_path, line_fragment
        )
    }
}

impl PermalinkGenerator for GitHubPermalinkGenerator {
    fn generate(&self, file: &Path, start_line: u32, end_line: Option<u32>) -> Result<GitHubLink> {
        let info = self.repo.get_info(&self.remote_name)?;

        let relative_path = self
            .repo
            .get_relative_path(file)?
            .to_string_lossy()
            .to_string();

        debug!(
            "Generating permalink for {} at {}:{}",
            relative_path,
            start_line,
            end_line.unwrap_or(start_line)
        );

        let provider = if Self::is_github_url(&info.remote_url) {
            "github"
        } else {
            "unknown"
        };

        let lines = LineRange {
            start: start_line,
            end: end_line,
        };

        let url = Self::build_github_url(&info.remote_url, &info.revision, &relative_path, &lines);

        let line_fragment = match lines.end {
            Some(end) if end != lines.start => format!("#L{}-L{}", lines.start, end),
            _ => format!("#L{}", lines.start),
        };
        let markdown = format!("[{}{}]({})", relative_path, line_fragment, url);

        Ok(GitHubLink {
            url,
            markdown,
            relative_path,
            revision: info.revision,
            lines,
            provider: provider.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_github_url() {
        let base = "https://github.com/FlakM/jump";
        let rev = "abc123";
        let path = "src/main.rs";

        // Single line
        let lines = LineRange {
            start: 10,
            end: None,
        };
        let url = GitHubPermalinkGenerator::build_github_url(base, rev, path, &lines);
        assert_eq!(
            url,
            "https://github.com/FlakM/jump/blob/abc123/src/main.rs#L10"
        );

        // Range
        let lines = LineRange {
            start: 10,
            end: Some(15),
        };
        let url = GitHubPermalinkGenerator::build_github_url(base, rev, path, &lines);
        assert_eq!(
            url,
            "https://github.com/FlakM/jump/blob/abc123/src/main.rs#L10-L15"
        );

        // Same line (start == end)
        let lines = LineRange {
            start: 10,
            end: Some(10),
        };
        let url = GitHubPermalinkGenerator::build_github_url(base, rev, path, &lines);
        assert_eq!(
            url,
            "https://github.com/FlakM/jump/blob/abc123/src/main.rs#L10"
        );
    }
}
