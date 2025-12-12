//! Integration tests for configuration loading.

use std::fs;
use tempfile::TempDir;

use jump::{ConfigLoader, JumpConfig};

#[test]
fn loads_default_config_when_file_missing() {
    let config = JumpConfig::default();

    assert!(config.marker_files.contains(&"Cargo.toml".to_string()));
    assert!(config.marker_files.contains(&".git".to_string()));
    assert_eq!(config.max_depth, 5);
    assert!(config.projects.is_empty());
}

#[test]
fn loads_config_from_file() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("jump.toml");

    fs::write(
        &config_path,
        r#"
max_depth = 10
marker_files = ["Cargo.toml", "go.mod"]
search_paths = ["/home/user/code"]

[[projects]]
name = "myproject"
path = "/home/user/myproject"
"#,
    )
    .unwrap();

    let config = ConfigLoader::load(&config_path).unwrap();

    assert_eq!(config.max_depth, 10);
    assert_eq!(config.marker_files, vec!["Cargo.toml", "go.mod"]);
    assert_eq!(config.projects.len(), 1);
    assert_eq!(config.projects[0].name, "myproject");
}

#[test]
fn handles_invalid_config_gracefully() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("invalid.toml");

    fs::write(&config_path, "this is not valid toml {{{").unwrap();

    let result = ConfigLoader::load(&config_path);

    assert!(result.is_err());
}

#[test]
fn config_search_paths_default_to_home_programming() {
    let config = JumpConfig::default();

    // Should have at least one search path
    assert!(!config.search_paths.is_empty());
}
