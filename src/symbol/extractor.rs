use serde_json::Value;
use std::path::Path;

use super::types::SymbolInfo;

pub trait SymbolExtractor {
    fn extract_qualified_name(&self, hover_result: &Value) -> Option<String>;
    fn extract_hover_text(&self, hover_result: &Value) -> Option<String>;
    fn extract_symbol_info(&self, hover: &Value, definition: &Value) -> SymbolInfo;
}

pub fn detect_language(path: &Path) -> &'static str {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext {
            "rs" => "rust",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            "py" | "pyw" | "pyi" => "python",
            "go" => "go",
            "java" => "java",
            "kt" | "kts" => "kotlin",
            "c" | "h" => "c",
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
            "cs" => "csharp",
            "rb" => "ruby",
            "php" => "php",
            "swift" => "swift",
            "scala" => "scala",
            "lua" => "lua",
            "sh" | "bash" | "zsh" => "shellscript",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "md" | "markdown" => "markdown",
            "html" | "htm" => "html",
            "css" => "css",
            "scss" | "sass" => "scss",
            "vue" => "vue",
            "svelte" => "svelte",
            "zig" => "zig",
            "nix" => "nix",
            _ => "plaintext",
        })
        .unwrap_or("plaintext")
}

pub fn create_extractor(language: &str) -> Box<dyn SymbolExtractor> {
    match language {
        "rust" => Box::new(RustSymbolExtractor),
        _ => Box::new(GenericSymbolExtractor::new(language)),
    }
}

#[derive(Default)]
pub struct RustSymbolExtractor;

impl RustSymbolExtractor {
    fn strip_markdown_code_blocks(text: &str) -> String {
        let mut lines = Vec::new();
        let mut in_code_block = false;

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                in_code_block = !in_code_block;
                continue;
            }
            if in_code_block && !trimmed.is_empty() {
                lines.push(trimmed);
            }
        }

        if lines.is_empty() {
            text.to_string()
        } else {
            lines.join("\n")
        }
    }

    fn clean_symbol_name(name: &str) -> &str {
        let name = if let Some(idx) = name.find('<') {
            &name[..idx]
        } else {
            name
        };
        name.trim_end_matches(['{', ';', ':', '(', ','])
    }

    fn extract_kind_and_name<'a>(&self, line: &'a str) -> Option<(&'a str, &'a str)> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }

        let kind = parts[0];
        match kind {
            "struct" | "enum" | "trait" | "const" | "static" => {
                let name = Self::clean_symbol_name(parts[1]);
                Some((kind, name))
            }
            "type" => {
                let name_part = parts[1];
                let name = if name_part.contains('<') {
                    name_part.split('<').next()?
                } else {
                    name_part.trim_end_matches(['{', ';'])
                };
                Some((kind, name))
            }
            "fn" | "async" => {
                let name_idx = if kind == "async" && parts.get(1) == Some(&"fn") {
                    2
                } else {
                    1
                };
                let name = parts.get(name_idx)?.split('(').next()?.split('<').next()?;
                Some(("fn", name))
            }
            _ => None,
        }
    }

    fn extract_let_binding<'a>(&self, line: &'a str) -> Option<(&'a str, &'a str)> {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("let ")?;
        let mut parts = rest.splitn(2, ':');
        let name = parts.next()?.trim();
        let ty = parts.next()?.trim();
        if !name.is_empty() && !name.contains(' ') && !ty.is_empty() {
            Some((name, ty))
        } else {
            None
        }
    }

    fn extract_field<'a>(&self, line: &'a str) -> Option<&'a str> {
        let trimmed = line.trim();
        if trimmed.contains(':') && !trimmed.starts_with("fn ") && !trimmed.starts_with("pub fn ") {
            let before_colon = trimmed.split(':').next()?.trim();
            let field_name = before_colon.strip_prefix("pub ").unwrap_or(before_colon);
            if !field_name.is_empty() && !field_name.contains(' ') {
                return Some(field_name);
            }
        }
        None
    }

    fn extract_kind_from_hover(&self, hover_text: &str) -> Option<String> {
        let is_markdown = hover_text.contains("```");
        let text = if is_markdown {
            Self::strip_markdown_code_blocks(hover_text)
        } else {
            hover_text.to_string()
        };

        for line in text.lines().skip(1) {
            let trimmed = line.trim();
            let check_line = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
            if let Some((kind, _name)) = self.extract_kind_and_name(check_line) {
                return Some(kind.to_string());
            }
            if self.extract_field(trimmed).is_some() {
                return Some("field".to_string());
            }
        }
        None
    }
}

impl SymbolExtractor for RustSymbolExtractor {
    fn extract_qualified_name(&self, hover_result: &Value) -> Option<String> {
        let contents = hover_result.get("contents")?;

        let raw_value = match contents {
            Value::Object(obj) => obj.get("value")?.as_str()?,
            Value::String(s) => s.as_str(),
            _ => return None,
        };

        let is_markdown = contents
            .get("kind")
            .and_then(|k| k.as_str())
            .map(|k| k == "markdown")
            .unwrap_or_else(|| raw_value.contains("```"));

        let value_str = if is_markdown {
            Self::strip_markdown_code_blocks(raw_value)
        } else {
            raw_value.to_string()
        };

        let lines: Vec<&str> = value_str.lines().collect();
        if lines.is_empty() {
            return None;
        }

        let first_line = lines[0].trim();

        // Handle let bindings: "let name: Type"
        if let Some((name, ty)) = self.extract_let_binding(first_line) {
            return Some(format!("let {}: {}", name, ty));
        }

        let module_path = first_line;
        let has_module_path = module_path.contains("::");

        // For local items (no module path), try to extract from first line directly
        if !has_module_path {
            let check_line = first_line.strip_prefix("pub ").unwrap_or(first_line);
            if let Some((kind, name)) = self.extract_kind_and_name(check_line) {
                // Local functions get () suffix since kind won't be shown separately
                let suffix = if kind == "fn" { "()" } else { "" };
                return Some(format!("{}{}", name, suffix));
            }
        }

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let without_pub = trimmed.strip_prefix("pub ").unwrap_or(trimmed);

            if let Some((_kind, name)) = self.extract_kind_and_name(without_pub) {
                if idx > 0 || trimmed.starts_with("pub ") {
                    // Module-qualified items don't get () suffix - kind is shown separately
                    return Some(format!("{}::{}", module_path, name));
                }
            } else if idx > 0 {
                if let Some(field_name) = self.extract_field(trimmed) {
                    return Some(format!("{}::{}", module_path, field_name));
                }
            }
        }

        None
    }

    fn extract_hover_text(&self, hover_result: &Value) -> Option<String> {
        let contents = hover_result.get("contents")?;

        fn normalize(value: &Value) -> String {
            match value {
                Value::String(s) => s.clone(),
                Value::Array(arr) => arr.iter().map(normalize).collect::<Vec<_>>().join("\n"),
                Value::Object(obj) => {
                    if let Some(Value::String(v)) = obj.get("value") {
                        v.clone()
                    } else {
                        value.to_string()
                    }
                }
                _ => value.to_string(),
            }
        }

        Some(normalize(contents))
    }

    fn extract_symbol_info(&self, hover: &Value, definition: &Value) -> SymbolInfo {
        let qualified_name = self.extract_qualified_name(hover);

        let kind = hover
            .get("contents")
            .and_then(|c| c.get("value"))
            .and_then(|v| v.as_str())
            .and_then(|s| self.extract_kind_from_hover(s));

        let (definition_uri, definition_line) = if let Some(arr) = definition.as_array() {
            if let Some(first) = arr.first() {
                // Handle both Location (uri) and LocationLink (targetUri) formats
                let uri = first
                    .get("uri")
                    .or_else(|| first.get("targetUri"))
                    .and_then(|u| u.as_str())
                    .map(String::from);
                // LSP line numbers are 0-indexed, convert to 1-indexed
                let line = first
                    .get("range")
                    .or_else(|| first.get("targetSelectionRange"))
                    .and_then(|r| r.get("start"))
                    .and_then(|s| s.get("line"))
                    .and_then(|l| l.as_u64())
                    .map(|l| (l + 1) as u32);
                (uri, line)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        SymbolInfo {
            qualified_name,
            kind,
            definition_uri,
            definition_line,
        }
    }
}

pub struct GenericSymbolExtractor {
    language: String,
}

impl GenericSymbolExtractor {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn contents_to_text(&self, contents: &Value) -> Option<String> {
        match contents {
            Value::String(s) => Some(s.clone()),
            Value::Object(obj) => {
                if let Some(Value::String(v)) = obj.get("value") {
                    Some(v.clone())
                } else if let Some(kind) = obj.get("kind") {
                    // MarkedString with language
                    if kind.as_str() == Some("markdown") || obj.contains_key("language") {
                        obj.get("value").and_then(|v| v.as_str()).map(String::from)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Value::Array(arr) => {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|v| self.contents_to_text(v))
                    .collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("\n"))
                }
            }
            _ => None,
        }
    }

    fn strip_markdown_code_block(&self, text: &str) -> String {
        let trimmed = text.trim();
        // Handle markdown code blocks: ```lang\ncode\n```
        if trimmed.starts_with("```") {
            let lines: Vec<&str> = trimmed.lines().collect();
            // Find the closing ```
            let end_idx = lines
                .iter()
                .skip(1)
                .position(|l| l.trim() == "```")
                .map(|i| i + 1);
            if let Some(end) = end_idx {
                if end > 1 {
                    return lines[1..end].join("\n");
                }
            }
        }
        trimmed.to_string()
    }

    fn extract_symbol_name_generic(&self, text: &str) -> Option<String> {
        let clean = self.strip_markdown_code_block(text);
        let first_line = clean.lines().find(|l| !l.trim().is_empty())?;
        let trimmed = first_line.trim();

        match self.language.as_str() {
            "typescript" | "javascript" => self.extract_ts_symbol(trimmed),
            "python" => self.extract_python_symbol(trimmed),
            "go" => self.extract_go_symbol(trimmed),
            _ => Some(trimmed.to_string()),
        }
    }

    fn extract_ts_symbol(&self, line: &str) -> Option<String> {
        // TypeScript patterns: (property) foo: Type, (method) bar(): void, const x: Type, etc.
        let stripped = line
            .strip_prefix("(property)")
            .or_else(|| line.strip_prefix("(method)"))
            .or_else(|| line.strip_prefix("(alias)"))
            .or_else(|| line.strip_prefix("(type alias)"))
            .or_else(|| line.strip_prefix("(parameter)"))
            .or_else(|| line.strip_prefix("(local var)"))
            .or_else(|| line.strip_prefix("(local const)"))
            .map(|s| s.trim())
            .unwrap_or(line);

        // Handle: const/let/var name: Type, function name(), class Name, interface Name
        let parts: Vec<&str> = stripped.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let (kind, rest) = match parts[0] {
            "const" | "let" | "var" => ("const", parts.get(1..)),
            "function" | "async" => ("function", parts.get(1..)),
            "class" => ("class", parts.get(1..)),
            "interface" => ("interface", parts.get(1..)),
            "type" => ("type", parts.get(1..)),
            "enum" => ("enum", parts.get(1..)),
            "namespace" => ("namespace", parts.get(1..)),
            _ => ("", Some(parts.as_slice())),
        };

        let name = rest
            .and_then(|p| p.first())
            .map(|s| {
                s.split(':')
                    .next()
                    .unwrap_or(s)
                    .split('(')
                    .next()
                    .unwrap_or(s)
                    .split('<')
                    .next()
                    .unwrap_or(s)
            })
            .unwrap_or(stripped);

        if kind.is_empty() {
            Some(name.to_string())
        } else {
            Some(format!("{} {}", kind, name))
        }
    }

    fn extract_python_symbol(&self, line: &str) -> Option<String> {
        // Python patterns: def foo(...), class Bar, variable: type
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        match parts[0] {
            "def" | "async" => {
                let name_idx = if parts[0] == "async" { 2 } else { 1 };
                parts.get(name_idx).map(|s| {
                    let name = s.split('(').next().unwrap_or(s);
                    format!("def {}", name)
                })
            }
            "class" => parts.get(1).map(|s| {
                let name = s
                    .split('(')
                    .next()
                    .unwrap_or(s)
                    .split(':')
                    .next()
                    .unwrap_or(s);
                format!("class {}", name)
            }),
            _ => {
                // Handle: (variable) name: type or name: type
                let stripped = line.strip_prefix("(variable)").unwrap_or(line).trim();
                Some(
                    stripped
                        .split(':')
                        .next()
                        .unwrap_or(stripped)
                        .trim()
                        .to_string(),
                )
            }
        }
    }

    fn extract_go_symbol(&self, line: &str) -> Option<String> {
        // Go patterns: func Foo(...), type Bar struct, var x Type
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        match parts[0] {
            "func" => parts.get(1).map(|s| {
                let name = s.split('(').next().unwrap_or(s);
                format!("func {}", name)
            }),
            "type" => {
                if parts.len() >= 2 {
                    let kind = parts.get(2).unwrap_or(&"type");
                    Some(format!("{} {}", kind, parts[1]))
                } else {
                    parts.get(1).map(|s| format!("type {}", s))
                }
            }
            "var" | "const" => parts.get(1).map(|s| format!("{} {}", parts[0], s)),
            _ => Some(line.to_string()),
        }
    }
}

impl SymbolExtractor for GenericSymbolExtractor {
    fn extract_qualified_name(&self, hover_result: &Value) -> Option<String> {
        let contents = hover_result.get("contents")?;
        let text = self.contents_to_text(contents)?;
        self.extract_symbol_name_generic(&text)
    }

    fn extract_hover_text(&self, hover_result: &Value) -> Option<String> {
        let contents = hover_result.get("contents")?;
        self.contents_to_text(contents)
    }

    fn extract_symbol_info(&self, hover: &Value, definition: &Value) -> SymbolInfo {
        let qualified_name = self.extract_qualified_name(hover);
        let (definition_uri, definition_line) = extract_definition_location(definition);

        SymbolInfo {
            qualified_name,
            kind: None, // kind is embedded in qualified_name for generic extractors
            definition_uri,
            definition_line,
        }
    }
}

fn extract_definition_location(definition: &Value) -> (Option<String>, Option<u32>) {
    // Handle both array and single Location/LocationLink response
    let location = if let Some(arr) = definition.as_array() {
        arr.first()
    } else if definition.is_object() {
        Some(definition)
    } else {
        None
    };

    if let Some(loc) = location {
        // Try Location format first (uri + range)
        let uri = loc
            .get("uri")
            .or_else(|| loc.get("targetUri")) // LocationLink format
            .and_then(|u| u.as_str())
            .map(String::from);
        let line = loc
            .get("range")
            .or_else(|| loc.get("targetSelectionRange")) // LocationLink format
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .map(|l| (l + 1) as u32);
        (uri, line)
    } else {
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_symbol_name_removes_generics() {
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("TmuxSessionManager<E>"),
            "TmuxSessionManager"
        );
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("TmuxSessionManager<E"),
            "TmuxSessionManager"
        );
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("HashMap<K, V>"),
            "HashMap"
        );
    }

    #[test]
    fn test_clean_symbol_name_removes_trailing_chars() {
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("DEFAULT_TIMEOUT:"),
            "DEFAULT_TIMEOUT"
        );
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("MyStruct{"),
            "MyStruct"
        );
        assert_eq!(RustSymbolExtractor::clean_symbol_name("Value;"), "Value");
        assert_eq!(RustSymbolExtractor::clean_symbol_name("func("), "func");
        assert_eq!(RustSymbolExtractor::clean_symbol_name("item,"), "item");
    }

    #[test]
    fn test_clean_symbol_name_no_change_needed() {
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("SimpleType"),
            "SimpleType"
        );
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("my_function"),
            "my_function"
        );
    }

    #[test]
    fn test_clean_symbol_name_combined() {
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("Generic<T>:"),
            "Generic"
        );
        assert_eq!(
            RustSymbolExtractor::clean_symbol_name("Type<A, B>{"),
            "Type"
        );
    }
}
