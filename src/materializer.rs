use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    parser::{JumpLinkKind, JumpRequest},
    project::ProjectRoot,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedPath {
    pub absolute: PathBuf,
    pub relative: Option<PathBuf>,
    pub line: Option<u32>,
    pub end_line: Option<u32>,
    pub kind: JumpLinkKind,
    pub revision: Option<String>,
}

/// Resolves a parsed [`JumpRequest`] into a verified filesystem path.
///
/// The materializer bridges the gap between a user's input (which may be a relative path,
/// GitHub URL, or absolute path) and an actual file on disk. It validates that the file
/// exists and canonicalizes the path.
///
/// # Behavior by Link Kind
///
/// - **Relative/GitHub**: Path is joined with the project root and canonicalized.
///   Returns an error if the resolved path escapes the project root (e.g., `../../../etc/passwd`).
/// - **Absolute**: Path is canonicalized directly, bypassing the project root constraint.
///
/// # Example
///
/// ```ignore
/// let materializer = FilesystemMaterializer;
/// let root = ProjectRoot::new("/home/user/myproject".into(), ".git".into());
/// let request = JumpRequest {
///     kind: JumpLinkKind::Relative,
///     path: "src/main.rs".into(),
///     line: Some(42),
///     ..Default::default()
/// };
///
/// let result = materializer.materialize(&root, &request)?;
/// // result.absolute == "/home/user/myproject/src/main.rs"
/// // result.relative == Some("src/main.rs")
/// // result.line == Some(42)
/// ```
pub trait PathMaterializer {
    /// Resolves the request to a [`MaterializedPath`].
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file does not exist
    /// - A relative path resolves outside the project root
    /// - Path canonicalization fails
    fn materialize(&self, root: &ProjectRoot, req: &JumpRequest) -> Result<MaterializedPath>;
}

#[derive(Default)]
pub struct FilesystemMaterializer;

impl FilesystemMaterializer {
    fn canonical_root(root: &ProjectRoot) -> PathBuf {
        root.path
            .canonicalize()
            .unwrap_or_else(|_| root.path.clone())
    }

    fn resolve_under_root(root: &ProjectRoot, path: &Path) -> Result<PathBuf> {
        let root_path = Self::canonical_root(root);
        let candidate = root_path.join(path);

        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize {:?}", candidate))?;

        if !canonical.starts_with(&root_path) {
            bail!(
                "Resolved path {:?} escapes project root {:?}",
                canonical,
                root_path
            );
        }

        Ok(canonical)
    }
}

impl FilesystemMaterializer {
    fn git_show_to_tempfile(root: &ProjectRoot, revision: &str, path: &Path) -> Result<PathBuf> {
        let repo =
            git2::Repository::discover(&root.path).context("Failed to discover git repository")?;

        let obj = repo
            .revparse_single(revision)
            .with_context(|| format!("Revision '{}' not found in local git history", revision))?;

        let commit = obj
            .peel_to_commit()
            .with_context(|| format!("Failed to peel '{}' to commit", revision))?;

        let tree = commit.tree().context("Failed to get commit tree")?;

        let entry = tree
            .get_path(path)
            .with_context(|| format!("File {:?} not found at revision '{}'", path, revision))?;

        let blob = repo
            .find_blob(entry.id())
            .context("Failed to read blob from git object store")?;

        let short_rev = revision.get(..8).unwrap_or(revision);
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let temp_path = std::env::temp_dir().join(format!("jump_{}_{}", short_rev, file_name));

        debug!(
            "Writing git blob for {:?}@{} to {:?}",
            path, short_rev, temp_path
        );

        std::fs::write(&temp_path, blob.content())
            .with_context(|| format!("Failed to write temp file {:?}", temp_path))?;

        Ok(temp_path)
    }
}

impl PathMaterializer for FilesystemMaterializer {
    fn materialize(&self, root: &ProjectRoot, req: &JumpRequest) -> Result<MaterializedPath> {
        let absolute = match req.kind {
            JumpLinkKind::Absolute => req
                .path
                .canonicalize()
                .with_context(|| format!("Failed to canonicalize {:?}", req.path)),
            JumpLinkKind::Relative => Self::resolve_under_root(root, &req.path),
            JumpLinkKind::Github => Self::resolve_under_root(root, &req.path).or_else(|_| {
                if let Some(revision) = &req.revision {
                    debug!(
                        "File {:?} not found locally, trying git show at {}",
                        req.path, revision
                    );
                    Self::git_show_to_tempfile(root, revision, &req.path)
                } else {
                    Self::resolve_under_root(root, &req.path)
                }
            }),
        }?;

        let root_path = Self::canonical_root(root);
        let relative = absolute
            .strip_prefix(&root_path)
            .ok()
            .map(|p| p.to_path_buf());

        Ok(MaterializedPath {
            absolute,
            relative,
            line: req.line,
            end_line: req.end_line,
            kind: req.kind.clone(),
            revision: req.revision.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn build_root(temp_dir: &TempDir) -> ProjectRoot {
        ProjectRoot::new(temp_dir.path().to_path_buf(), ".git".to_string())
    }

    #[test]
    fn materializes_relative_path_with_line() {
        let temp_dir = TempDir::new().unwrap();
        let root = build_root(&temp_dir);
        let file_path = temp_dir.path().join("src/lib.rs");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "// test").unwrap();

        let request = JumpRequest {
            kind: JumpLinkKind::Relative,
            path: PathBuf::from("src/lib.rs"),
            line: Some(8),
            end_line: None,
            revision: None,
            repo_name: None,
        };

        let materializer = FilesystemMaterializer;
        let result = materializer.materialize(&root, &request).unwrap();

        assert_eq!(result.absolute, file_path.canonicalize().unwrap());
        assert_eq!(result.relative, Some(PathBuf::from("src/lib.rs")));
        assert_eq!(result.line, Some(8));
        assert_eq!(result.end_line, None);
    }

    #[test]
    fn rejects_paths_outside_root() {
        let temp_dir = TempDir::new().unwrap();
        let root = build_root(&temp_dir);

        let outside_file = temp_dir
            .path()
            .parent()
            .unwrap()
            .join("outside.rs")
            .canonicalize();
        // Create the file and canonicalize again to ensure it exists
        let outside_file = match outside_file {
            Ok(path) => path,
            Err(_) => {
                let path = temp_dir.path().parent().unwrap().join("outside.rs");
                std::fs::write(&path, "// outside").unwrap();
                path.canonicalize().unwrap()
            }
        };

        let request = JumpRequest {
            kind: JumpLinkKind::Relative,
            path: PathBuf::from("../outside.rs"),
            line: None,
            end_line: None,
            revision: None,
            repo_name: None,
        };

        let materializer = FilesystemMaterializer;
        let result = materializer.materialize(&root, &request);
        assert!(result.is_err());

        // Absolute paths bypass the root restriction
        let absolute_request = JumpRequest {
            kind: JumpLinkKind::Absolute,
            path: outside_file.clone(),
            line: None,
            end_line: None,
            revision: None,
            repo_name: None,
        };

        let absolute = materializer
            .materialize(&root, &absolute_request)
            .expect("absolute path should resolve");
        assert_eq!(absolute.absolute, outside_file);
        assert!(absolute.relative.is_none());
    }
}
