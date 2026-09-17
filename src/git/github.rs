use anyhow::Result;
use std::path::Path;
use tracing::debug;

use super::repo::{Git2Repository, GitRepository};
use super::types::{GitHubLink, LineRange};

pub trait PermalinkGenerator {
    /// Generate a permalink, optionally labelling it with the enclosing symbol name.
    fn generate_labeled(
        &self,
        file: &Path,
        start_line: u32,
        end_line: Option<u32>,
        symbol: Option<&str>,
    ) -> Result<GitHubLink>;

    fn generate(&self, file: &Path, start_line: u32, end_line: Option<u32>) -> Result<GitHubLink> {
        self.generate_labeled(file, start_line, end_line, None)
    }
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

    fn format_lines(lines: &LineRange) -> String {
        match lines.end {
            Some(end) if end != lines.start => format!("{}-{}", lines.start, end),
            _ => lines.start.to_string(),
        }
    }

    /// Human readable label: `symbol (basename:lines)` when the caller knows the
    /// enclosing symbol, otherwise the repo relative path with the line range.
    fn build_label(relative_path: &str, lines: &LineRange, symbol: Option<&str>) -> String {
        let lines = Self::format_lines(lines);
        match symbol {
            Some(symbol) if !symbol.trim().is_empty() => {
                let basename = relative_path.rsplit('/').next().unwrap_or(relative_path);
                format!("{} ({}:{})", symbol.trim(), basename, lines)
            }
            _ => format!("{}:{}", relative_path, lines),
        }
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
    fn generate_labeled(
        &self,
        file: &Path,
        start_line: u32,
        end_line: Option<u32>,
        symbol: Option<&str>,
    ) -> Result<GitHubLink> {
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

        let label = Self::build_label(&relative_path, &lines, symbol);
        let markdown = format!("[{}]({})", label, url);

        Ok(GitHubLink {
            url,
            markdown,
            label,
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
    fn test_build_label_without_symbol() {
        let lines = LineRange {
            start: 129,
            end: Some(133),
        };
        assert_eq!(
            GitHubPermalinkGenerator::build_label("pdp/src/services/check.rs", &lines, None),
            "pdp/src/services/check.rs:129-133"
        );

        let lines = LineRange {
            start: 129,
            end: None,
        };
        assert_eq!(
            GitHubPermalinkGenerator::build_label("pdp/src/services/check.rs", &lines, None),
            "pdp/src/services/check.rs:129"
        );
    }

    #[test]
    fn test_build_label_with_symbol() {
        let lines = LineRange {
            start: 129,
            end: Some(133),
        };
        assert_eq!(
            GitHubPermalinkGenerator::build_label(
                "pdp/src/services/check.rs",
                &lines,
                Some("CheckPermissionsService::check_permissions")
            ),
            "CheckPermissionsService::check_permissions (check.rs:129-133)"
        );
    }

    #[test]
    fn test_build_label_ignores_blank_symbol() {
        let lines = LineRange {
            start: 1,
            end: Some(1),
        };
        assert_eq!(
            GitHubPermalinkGenerator::build_label("src/main.rs", &lines, Some("  ")),
            "src/main.rs:1"
        );
    }

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
