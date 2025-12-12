use std::path::PathBuf;

use super::types::{JumpLinkKind, JumpRequest};

pub trait JumpLinkParser {
    fn parse(&self, text: &str) -> Option<JumpRequest>;
}

#[derive(Default)]
pub struct LinkParser;

impl LinkParser {
    fn extract_link_target(&self, text: &str) -> String {
        let trimmed = text.trim().trim_matches(['"', '\'']);

        if let Some(start) = trimmed.find("](") {
            let after = &trimmed[start + 2..];
            if let Some(end) = after.find(')') {
                return after[..end].to_string();
            }
        }

        trimmed
            .trim_start_matches('(')
            .trim_end_matches(')')
            .to_string()
    }

    fn parse_line_fragment(&self, fragment: &str) -> (Option<u32>, Option<u32>) {
        let normalized = fragment.trim_start_matches('#').trim();
        let normalized = normalized.trim_start_matches(['L', 'l']);

        if normalized.is_empty() {
            return (None, None);
        }

        let mut parts = normalized.splitn(2, '-');
        let start = parts.next().and_then(|s| s.parse::<u32>().ok());
        let end = parts
            .next()
            .map(|s| s.trim_start_matches(['L', 'l']))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    s.parse::<u32>().ok()
                }
            });

        (start, end)
    }

    fn looks_like_line(&self, fragment: &str) -> bool {
        let normalized = fragment.trim_start_matches(['L', 'l']);
        !normalized.is_empty()
            && normalized
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == '#')
    }

    fn split_path_and_line(&self, target: &str) -> (String, Option<String>) {
        if let Some((path, fragment)) = target.split_once('#') {
            return (path.to_string(), Some(fragment.to_string()));
        }

        if let Some((path, fragment)) = target.rsplit_once(':') {
            if self.looks_like_line(fragment) {
                return (path.to_string(), Some(fragment.to_string()));
            }
        }

        (target.to_string(), None)
    }

    fn parse_github(&self, target: &str) -> Option<JumpRequest> {
        if !target.contains("github.com") || !target.contains("/blob/") {
            return None;
        }

        let (link_part, fragment) = match target.split_once('#') {
            Some((path, frag)) => (path, Some(frag)),
            None => (target, None),
        };

        // Extract repo name from github.com/owner/repo/blob/...
        let repo_name = link_part
            .split("github.com/")
            .nth(1)
            .and_then(|s| s.split('/').nth(1))
            .map(|s| s.to_string());

        let (_, rest) = link_part.split_once("/blob/")?;
        let (revision, remaining) = rest.split_once('/')?;

        let path_part = remaining.split('?').next().unwrap_or(remaining);
        let (line, end_line) = fragment
            .map(|frag| self.parse_line_fragment(frag))
            .unwrap_or((None, None));

        Some(JumpRequest {
            kind: JumpLinkKind::Github,
            path: PathBuf::from(path_part),
            line,
            end_line,
            revision: Some(revision.to_string()),
            repo_name,
        })
    }

    fn parse_file_url(&self, target: &str) -> Option<JumpRequest> {
        let path_str = target.strip_prefix("file://")?;
        let (path_part, fragment) = self.split_path_and_line(path_str);

        let (line, end_line) = fragment
            .as_deref()
            .map(|frag| self.parse_line_fragment(frag))
            .unwrap_or((None, None));

        Some(JumpRequest {
            kind: JumpLinkKind::Absolute,
            path: PathBuf::from(path_part),
            line,
            end_line,
            revision: None,
            repo_name: None,
        })
    }

    fn parse_file(&self, target: &str) -> Option<JumpRequest> {
        let (path_part, fragment) = self.split_path_and_line(target);
        if path_part.is_empty() {
            return None;
        }

        let path = PathBuf::from(&path_part);
        let kind = if path.is_absolute() {
            JumpLinkKind::Absolute
        } else {
            JumpLinkKind::Relative
        };

        let (line, end_line) = fragment
            .as_deref()
            .map(|frag| self.parse_line_fragment(frag))
            .unwrap_or((None, None));

        Some(JumpRequest {
            kind,
            path,
            line,
            end_line,
            revision: None,
            repo_name: None,
        })
    }
}

impl JumpLinkParser for LinkParser {
    fn parse(&self, text: &str) -> Option<JumpRequest> {
        let target = self.extract_link_target(text);

        if let Some(req) = self.parse_github(&target) {
            return Some(req);
        }

        if let Some(req) = self.parse_file_url(&target) {
            return Some(req);
        }

        self.parse_file(&target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_markdown_github_link_with_range() {
        let parser = LinkParser;
        let input = "[main](https://github.com/FlakM/jump/blob/main/src/main.rs#L10-L12)";

        let request = parser.parse(input).expect("Expected parsed request");

        assert_eq!(request.kind, JumpLinkKind::Github);
        assert_eq!(request.path, PathBuf::from("src/main.rs"));
        assert_eq!(request.line, Some(10));
        assert_eq!(request.end_line, Some(12));
        assert_eq!(request.revision.as_deref(), Some("main"));
        assert_eq!(request.repo_name.as_deref(), Some("jump"));
    }

    #[test]
    fn parses_relative_path_with_line() {
        let parser = LinkParser;
        let input = "../src/lib.rs:42";

        let request = parser.parse(input).expect("Expected parsed request");

        assert_eq!(request.kind, JumpLinkKind::Relative);
        assert_eq!(request.path, PathBuf::from("../src/lib.rs"));
        assert_eq!(request.line, Some(42));
        assert_eq!(request.end_line, None);
        assert!(request.revision.is_none());
        assert!(request.repo_name.is_none());
    }

    #[test]
    fn parses_absolute_path_with_fragment() {
        let parser = LinkParser;
        let input = "/tmp/project/src/lib.rs#L5";

        let request = parser.parse(input).expect("Expected parsed request");

        assert_eq!(request.kind, JumpLinkKind::Absolute);
        assert_eq!(request.path, PathBuf::from("/tmp/project/src/lib.rs"));
        assert_eq!(request.line, Some(5));
        assert_eq!(request.end_line, None);
        assert!(request.revision.is_none());
        assert!(request.repo_name.is_none());
    }

    #[test]
    fn parses_file_url_with_line() {
        let parser = LinkParser;
        let input = "file:///home/user/project/src/main.rs#L11";

        let request = parser.parse(input).expect("Expected parsed request");

        assert_eq!(request.kind, JumpLinkKind::Absolute);
        assert_eq!(
            request.path,
            PathBuf::from("/home/user/project/src/main.rs")
        );
        assert_eq!(request.line, Some(11));
        assert_eq!(request.end_line, None);
    }

    #[test]
    fn parses_markdown_link_with_file_url() {
        let parser = LinkParser;
        let input = "[fn jump::main](file:///home/user/project/src/main.rs#L11)";

        let request = parser.parse(input).expect("Expected parsed request");

        assert_eq!(request.kind, JumpLinkKind::Absolute);
        assert_eq!(
            request.path,
            PathBuf::from("/home/user/project/src/main.rs")
        );
        assert_eq!(request.line, Some(11));
    }
}
