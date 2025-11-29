use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub qualified_name: Option<String>,
    pub kind: Option<String>,
    pub definition_uri: Option<String>,
    pub definition_line: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HoverOutput {
    pub symbol_info: SymbolInfo,
    pub hover_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CursorPosition {
    pub file: String,
    pub line: u32,
    pub character: u32,
}
