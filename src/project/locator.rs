use super::types::ProjectRoot;
use anyhow::Result;
use std::path::Path;

pub trait ProjectRootLocator {
    fn find_root_from(&self, start_path: &Path) -> Result<Option<ProjectRoot>>;
    fn find_all_projects(&self, search_path: &Path, max_depth: usize) -> Result<Vec<ProjectRoot>>;
}
