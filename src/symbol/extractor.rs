use serde_json::Value;

use super::types::SymbolInfo;

pub trait SymbolExtractor {
    fn extract_qualified_name(&self, hover_result: &Value) -> Option<String>;
    fn extract_hover_text(&self, hover_result: &Value) -> Option<String>;
    fn extract_symbol_info(&self, hover: &Value, definition: &Value) -> SymbolInfo;
}

#[derive(Default)]
pub struct RustSymbolExtractor;

impl RustSymbolExtractor {
    fn extract_kind_and_name<'a>(&self, line: &'a str) -> Option<(&'a str, &'a str)> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }

        let kind = parts[0];
        match kind {
            "struct" | "enum" | "trait" | "const" | "static" => {
                let name = parts[1].trim_end_matches(['{', ';', '<']);
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
            let field_name = trimmed.split(':').next()?.trim();
            if !field_name.is_empty() && !field_name.contains(' ') {
                return Some(field_name);
            }
        }
        None
    }
}

impl SymbolExtractor for RustSymbolExtractor {
    fn extract_qualified_name(&self, hover_result: &Value) -> Option<String> {
        let contents = hover_result.get("contents")?;

        let value_str = match contents {
            Value::Object(obj) => obj.get("value")?.as_str()?,
            Value::String(s) => s.as_str(),
            _ => return None,
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
        if !module_path.contains("::") && lines.len() < 3 {
            return None;
        }

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if let Some(rest) = trimmed.strip_prefix("pub ") {
                if let Some((kind, name)) = self.extract_kind_and_name(rest) {
                    return Some(format!("{} {}::{}", kind, module_path, name));
                }
            } else if let Some((kind, name)) = self.extract_kind_and_name(trimmed) {
                if idx > 0 {
                    return Some(format!("{} {}::{}", kind, module_path, name));
                }
            } else if idx > 0 {
                if let Some(field_name) = self.extract_field(trimmed) {
                    return Some(format!("field {}::{}", module_path, field_name));
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
            .and_then(|s| {
                s.lines()
                    .nth(1)
                    .and_then(|line| line.split_whitespace().nth(1))
                    .map(String::from)
            });

        let (definition_uri, definition_line) = if let Some(arr) = definition.as_array() {
            if let Some(first) = arr.first() {
                let uri = first.get("uri").and_then(|u| u.as_str()).map(String::from);
                // LSP line numbers are 0-indexed, convert to 1-indexed
                let line = first
                    .get("range")
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
