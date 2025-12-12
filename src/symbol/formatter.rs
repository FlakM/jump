use super::types::SymbolInfo;
use crate::git::GitHubLink;
use anyhow::Result;

#[derive(Debug, Clone)]
pub enum LinkType {
    Local,
    GitHub,
    Relative,
}

pub trait ReferenceFormatter {
    fn format_markdown(
        &self,
        symbol: &SymbolInfo,
        github_link: Option<&GitHubLink>,
    ) -> Result<String>;
}

#[derive(Default)]
pub struct MarkdownFormatter;

impl MarkdownFormatter {
    fn format_symbol_name(symbol: &SymbolInfo) -> String {
        if let Some(name) = &symbol.qualified_name {
            if let Some(kind) = &symbol.kind {
                format!("{} {}", kind, name)
            } else {
                name.clone()
            }
        } else if let Some(kind) = &symbol.kind {
            kind.clone()
        } else {
            "unknown".to_string()
        }
    }

    fn format_local_link(uri: &str, line: Option<u32>) -> String {
        match line {
            Some(l) => format!("{}#L{}", uri, l),
            None => uri.to_string(),
        }
    }
}

impl ReferenceFormatter for MarkdownFormatter {
    fn format_markdown(
        &self,
        symbol: &SymbolInfo,
        github_link: Option<&GitHubLink>,
    ) -> Result<String> {
        let label = Self::format_symbol_name(symbol);

        let url = if let Some(gh_link) = github_link {
            gh_link.url.clone()
        } else if let Some(def_uri) = &symbol.definition_uri {
            Self::format_local_link(def_uri, symbol.definition_line)
        } else {
            return Ok(label);
        };

        Ok(format!("[{}]({})", label, url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitHubLink, LineRange};

    #[test]
    fn test_format_markdown_with_github_link() {
        let symbol = SymbolInfo {
            qualified_name: Some("std::path::PathBuf".to_string()),
            kind: Some("struct".to_string()),
            definition_uri: Some("file:///path/to/file.rs".to_string()),
            definition_line: Some(42),
        };

        let github_link = GitHubLink {
            url: "https://github.com/rust-lang/rust/blob/abc123/library/std/src/path.rs#L42"
                .to_string(),
            markdown: "[library/std/src/path.rs#L42](https://github.com/rust-lang/rust/blob/abc123/library/std/src/path.rs#L42)".to_string(),
            relative_path: "library/std/src/path.rs".to_string(),
            revision: "abc123".to_string(),
            lines: LineRange {
                start: 42,
                end: None,
            },
            provider: "github".to_string(),
        };

        let formatter = MarkdownFormatter;
        let result = formatter
            .format_markdown(&symbol, Some(&github_link))
            .unwrap();

        assert_eq!(
            result,
            "[struct std::path::PathBuf](https://github.com/rust-lang/rust/blob/abc123/library/std/src/path.rs#L42)"
        );
    }

    #[test]
    fn test_format_markdown_with_local_link() {
        let symbol = SymbolInfo {
            qualified_name: Some("crate::symbol::SymbolInfo".to_string()),
            kind: Some("struct".to_string()),
            definition_uri: Some("file:///home/user/project/src/symbol/types.rs".to_string()),
            definition_line: Some(4),
        };

        let formatter = MarkdownFormatter;
        let result = formatter.format_markdown(&symbol, None).unwrap();

        assert_eq!(
            result,
            "[struct crate::symbol::SymbolInfo](file:///home/user/project/src/symbol/types.rs#L4)"
        );
    }

    #[test]
    fn test_format_markdown_no_definition() {
        let symbol = SymbolInfo {
            qualified_name: Some("crate::foo::Bar".to_string()),
            kind: Some("function".to_string()),
            definition_uri: None,
            definition_line: None,
        };

        let formatter = MarkdownFormatter;
        let result = formatter.format_markdown(&symbol, None).unwrap();

        assert_eq!(result, "function crate::foo::Bar");
    }
}
