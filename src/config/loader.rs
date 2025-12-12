use super::types::JumpConfig;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::debug;

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn load(path: &Path) -> Result<JumpConfig> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {:?}", path))?;

        let config: JumpConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse TOML config from {:?}", path))?;

        debug!("Loaded config from {:?}: {:?}", path, config);
        Ok(config)
    }

    pub fn load_or_default(path: Option<&Path>) -> JumpConfig {
        if let Some(path) = path {
            Self::load(path).unwrap_or_else(|e| {
                debug!("Failed to load config: {}, using default", e);
                JumpConfig::default()
            })
        } else {
            Self::find_and_load().unwrap_or_default()
        }
    }

    pub fn find_and_load() -> Option<JumpConfig> {
        let config_path = Self::find_config_file()?;
        Self::load(&config_path).ok()
    }

    pub fn find_config_file() -> Option<PathBuf> {
        let config_locations = vec![
            PathBuf::from(".jump.toml"),
            dirs::config_dir()?.join("jump/config.toml"),
            dirs::home_dir()?.join(".config/jump/config.toml"),
        ];

        for path in config_locations {
            if path.exists() {
                debug!("Found config file at {:?}", path);
                return Some(path);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_valid_config() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
            max_depth = 3
            marker_files = ["Cargo.toml"]
        "#
        )
        .unwrap();

        let config = ConfigLoader::load(temp_file.path()).unwrap();
        assert_eq!(config.max_depth, 3);
    }

    #[test]
    fn test_load_invalid_config() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "invalid toml content {{").unwrap();

        let result = ConfigLoader::load(temp_file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let config = ConfigLoader::load_or_default(Some(Path::new("/nonexistent/path.toml")));
        assert_eq!(config.max_depth, 5);
    }
}
