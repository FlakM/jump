use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JumpLinkKind {
    Github,
    Relative,
    Absolute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JumpRequest {
    pub kind: JumpLinkKind,
    pub path: PathBuf,
    pub line: Option<u32>,
    pub end_line: Option<u32>,
    pub revision: Option<String>,
    pub repo_name: Option<String>,
}
