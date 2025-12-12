use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubLink {
    pub url: String,
    pub markdown: String,
    pub relative_path: String,
    pub revision: String,
    pub lines: LineRange,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub remote_url: String,
    pub revision: String,
}
