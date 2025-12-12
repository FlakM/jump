use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRoot {
    pub path: PathBuf,
    pub name: String,
    pub marker_file: String,
}

impl ProjectRoot {
    pub fn new(path: PathBuf, marker_file: String) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Self {
            path,
            name,
            marker_file,
        }
    }
}
