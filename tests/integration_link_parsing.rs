//! Integration tests for link parsing and resolution.

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use jump::{
    FastProjectScanner, FilesystemMaterializer, JumpLinkKind, JumpLinkParser, LinkParser,
    PathMaterializer, ProjectRoot, ProjectRootLocator,
};

fn setup_project(name: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join(name);
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(project.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
    (temp, project)
}

#[test]
fn parses_github_url_and_extracts_components() {
    let parser = LinkParser;
    let url = "https://github.com/FlakM/jump/blob/main/src/main.rs#L10-L20";

    let request = parser.parse(url).expect("should parse");

    assert_eq!(request.kind, JumpLinkKind::Github);
    assert_eq!(request.path, PathBuf::from("src/main.rs"));
    assert_eq!(request.line, Some(10));
    assert_eq!(request.end_line, Some(20));
    assert_eq!(request.revision.as_deref(), Some("main"));
    assert_eq!(request.repo_name.as_deref(), Some("jump"));
}

#[test]
fn parses_markdown_link_with_github_url() {
    let parser = LinkParser;
    let markdown = "[some symbol](https://github.com/user/repo/blob/abc123/path/to/file.rs#L42)";

    let request = parser.parse(markdown).expect("should parse");

    assert_eq!(request.kind, JumpLinkKind::Github);
    assert_eq!(request.path, PathBuf::from("path/to/file.rs"));
    assert_eq!(request.line, Some(42));
    assert_eq!(request.revision.as_deref(), Some("abc123"));
    assert_eq!(request.repo_name.as_deref(), Some("repo"));
}

#[test]
fn parses_relative_path_with_line_number() {
    let parser = LinkParser;

    let request = parser.parse("src/lib.rs:42").expect("should parse");

    assert_eq!(request.kind, JumpLinkKind::Relative);
    assert_eq!(request.path, PathBuf::from("src/lib.rs"));
    assert_eq!(request.line, Some(42));
    assert!(request.repo_name.is_none());
}

#[test]
fn parses_absolute_path_with_fragment() {
    let parser = LinkParser;

    let request = parser.parse("/tmp/test/file.rs#L5").expect("should parse");

    assert_eq!(request.kind, JumpLinkKind::Absolute);
    assert_eq!(request.path, PathBuf::from("/tmp/test/file.rs"));
    assert_eq!(request.line, Some(5));
}

#[test]
fn materializes_relative_path_within_project() {
    let (_temp, project) = setup_project("myproject");
    let root = ProjectRoot::new(project.clone(), "Cargo.toml".into());
    let parser = LinkParser;
    let materializer = FilesystemMaterializer;

    let request = parser.parse("src/lib.rs:1").unwrap();
    let result = materializer.materialize(&root, &request).unwrap();

    assert_eq!(
        result.absolute,
        project.join("src/lib.rs").canonicalize().unwrap()
    );
    assert_eq!(result.relative, Some(PathBuf::from("src/lib.rs")));
    assert_eq!(result.line, Some(1));
}

#[test]
fn rejects_path_escaping_project_root() {
    let (_temp, project) = setup_project("myproject");
    let root = ProjectRoot::new(project, "Cargo.toml".into());
    let parser = LinkParser;
    let materializer = FilesystemMaterializer;

    let request = parser.parse("../../../etc/passwd").unwrap();
    let result = materializer.materialize(&root, &request);

    assert!(result.is_err());
}

#[test]
fn finds_project_root_from_nested_directory() {
    let (_temp, project) = setup_project("myproject");
    let nested = project.join("src");
    let scanner = FastProjectScanner::with_defaults();

    let root = scanner
        .find_root_from(&nested)
        .unwrap()
        .expect("should find root");

    assert_eq!(root.path, project.canonicalize().unwrap());
    assert_eq!(root.marker_file, "Cargo.toml");
}

#[test]
fn finds_all_projects_in_directory_tree() {
    let temp = TempDir::new().unwrap();

    // Create multiple projects
    for name in ["proj1", "proj2", "proj3"] {
        let proj = temp.path().join(name);
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("Cargo.toml"), "").unwrap();
    }

    let scanner = FastProjectScanner::with_defaults();
    let projects = scanner.find_all_projects(temp.path(), 3).unwrap();

    assert_eq!(projects.len(), 3);
}

#[test]
fn respects_max_depth_when_scanning() {
    let temp = TempDir::new().unwrap();

    // Create deeply nested project
    let deep = temp.path().join("a/b/c/d/e/project");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("Cargo.toml"), "").unwrap();

    let scanner = FastProjectScanner::with_defaults();

    // Should not find with depth 2
    let shallow = scanner.find_all_projects(temp.path(), 2).unwrap();
    assert!(shallow.is_empty());

    // Should find with depth 10
    let deep_scan = scanner.find_all_projects(temp.path(), 10).unwrap();
    assert_eq!(deep_scan.len(), 1);
}
