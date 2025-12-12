use super::locator::ProjectRootLocator;
use super::types::ProjectRoot;
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::debug;

pub struct FastProjectScanner {
    marker_files: Vec<String>,
}

impl FastProjectScanner {
    pub fn new(marker_files: Vec<String>) -> Self {
        Self { marker_files }
    }

    pub fn with_defaults() -> Self {
        Self::new(vec![
            "Cargo.toml".to_string(),
            "package.json".to_string(),
            "go.mod".to_string(),
            "pyproject.toml".to_string(),
            ".git".to_string(),
            ".obsidian".to_string(),
        ])
    }

    fn find_marker_in_directory(&self, dir: &Path) -> Option<String> {
        for marker in &self.marker_files {
            let marker_path = dir.join(marker);
            if marker_path.exists() {
                debug!("Found marker {} in {:?}", marker, dir);
                return Some(marker.clone());
            }
        }
        None
    }

    fn walk_up_to_root(&self, start_path: &Path) -> Option<ProjectRoot> {
        let mut current = start_path.to_path_buf();

        loop {
            if let Some(marker) = self.find_marker_in_directory(&current) {
                return Some(ProjectRoot::new(current.clone(), marker));
            }

            if !current.pop() {
                break;
            }
        }

        None
    }
}

impl ProjectRootLocator for FastProjectScanner {
    fn find_root_from(&self, start_path: &Path) -> Result<Option<ProjectRoot>> {
        let abs_path = start_path
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize {:?}", start_path))?;

        debug!("Looking for project root from {:?}", abs_path);

        Ok(self.walk_up_to_root(&abs_path))
    }

    fn find_all_projects(&self, search_path: &Path, max_depth: usize) -> Result<Vec<ProjectRoot>> {
        let abs_path = search_path
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize {:?}", search_path))?;

        debug!(
            "Scanning for all projects in {:?} with max_depth {}",
            abs_path, max_depth
        );

        let mut found_projects = Vec::new();
        let mut seen_dirs: HashSet<PathBuf> = HashSet::new();

        let walker = WalkBuilder::new(&abs_path)
            .max_depth(Some(max_depth))
            .hidden(true)
            .git_ignore(true)
            .build();

        for entry in walker {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            if seen_dirs.contains(path) {
                continue;
            }

            if let Some(marker) = self.find_marker_in_directory(path) {
                let project = ProjectRoot::new(path.to_path_buf(), marker);
                debug!("Found project: {:?}", project);
                found_projects.push(project);

                seen_dirs.insert(path.to_path_buf());
            }
        }

        debug!("Found {} projects total", found_projects.len());
        Ok(found_projects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_find_root_from_current_dir() {
        let temp_dir = TempDir::new().unwrap();
        let marker_file = temp_dir.path().join("Cargo.toml");
        fs::write(&marker_file, "").unwrap();

        let scanner = FastProjectScanner::with_defaults();
        let result = scanner.find_root_from(temp_dir.path()).unwrap();

        assert!(result.is_some());
        let project = result.unwrap();
        assert_eq!(project.marker_file, "Cargo.toml");
    }

    #[test]
    fn test_find_root_from_subdirectory() {
        let temp_dir = TempDir::new().unwrap();
        let marker_file = temp_dir.path().join("Cargo.toml");
        fs::write(&marker_file, "").unwrap();

        let subdir = temp_dir.path().join("src");
        fs::create_dir(&subdir).unwrap();

        let scanner = FastProjectScanner::with_defaults();
        let result = scanner.find_root_from(&subdir).unwrap();

        assert!(result.is_some());
        let project = result.unwrap();
        assert_eq!(project.path, temp_dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_find_all_projects() {
        let temp_dir = TempDir::new().unwrap();

        let proj1 = temp_dir.path().join("project1");
        fs::create_dir(&proj1).unwrap();
        fs::write(proj1.join("Cargo.toml"), "").unwrap();

        let proj2 = temp_dir.path().join("project2");
        fs::create_dir(&proj2).unwrap();
        fs::write(proj2.join("package.json"), "").unwrap();

        let scanner = FastProjectScanner::with_defaults();
        let projects = scanner.find_all_projects(temp_dir.path(), 3).unwrap();

        assert_eq!(projects.len(), 2);
        assert!(projects.iter().any(|p| p.marker_file == "Cargo.toml"));
        assert!(projects.iter().any(|p| p.marker_file == "package.json"));
    }

    #[test]
    fn test_no_project_root_found() {
        let temp_dir = TempDir::new().unwrap();

        let scanner = FastProjectScanner::with_defaults();
        let result = scanner.find_root_from(temp_dir.path()).unwrap();

        assert!(result.is_none());
    }
}
