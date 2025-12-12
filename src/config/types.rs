use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpConfig {
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,

    #[serde(default = "default_marker_files")]
    pub marker_files: Vec<String>,

    #[serde(default = "default_max_depth")]
    pub max_depth: usize,

    #[serde(default = "default_search_paths")]
    pub search_paths: Vec<PathBuf>,
}

fn default_search_paths() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|h| vec![h.join("programming")])
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub path: PathBuf,

    #[serde(default)]
    pub lsp_server: Option<String>,
}

fn default_marker_files() -> Vec<String> {
    vec![
        "Cargo.toml".to_string(),
        "package.json".to_string(),
        "go.mod".to_string(),
        "pyproject.toml".to_string(),
        ".git".to_string(),
    ]
}

fn default_max_depth() -> usize {
    5
}

impl Default for JumpConfig {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            marker_files: default_marker_files(),
            max_depth: default_max_depth(),
            search_paths: default_search_paths(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = JumpConfig::default();
        assert_eq!(config.max_depth, 5);
        assert!(config.marker_files.contains(&"Cargo.toml".to_string()));
        assert!(config.projects.is_empty());
    }

    #[test]
    fn test_deserialize_config() {
        let toml_str = r#"
            max_depth = 3
            marker_files = ["Cargo.toml", ".git"]

            [[projects]]
            name = "jump"
            path = "/home/user/projects/jump"
            lsp_server = "rust-analyzer"
        "#;

        let config: JumpConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_depth, 3);
        assert_eq!(config.marker_files.len(), 2);
        assert_eq!(config.projects.len(), 1);
        assert_eq!(config.projects[0].name, "jump");
    }
}
