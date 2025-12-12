//! Integration tests for git operations and GitHub link generation.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

use jump::{GitHubPermalinkGenerator, PermalinkGenerator};

fn setup_git_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .unwrap();

    // Configure git user for commits
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .unwrap();

    // Add a remote
    Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "git@github.com:testuser/testrepo.git",
        ])
        .current_dir(repo)
        .output()
        .unwrap();

    // Create and commit a file
    fs::write(repo.join("test.rs"), "fn main() {}").unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(repo)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(repo)
        .output()
        .unwrap();

    temp
}

#[test]
fn generates_github_permalink_for_single_line() {
    let temp = setup_git_repo();
    let file = temp.path().join("test.rs");

    let generator = GitHubPermalinkGenerator::new(&file, None).unwrap();
    let link = generator.generate(&file, 1, None).unwrap();

    assert!(link.url.contains("github.com"));
    assert!(link.url.contains("testuser/testrepo"));
    assert!(link.url.contains("#L1"));
    assert!(!link.url.contains("-L")); // No range for single line
}

#[test]
fn generates_github_permalink_with_line_range() {
    let temp = setup_git_repo();
    let file = temp.path().join("test.rs");

    let generator = GitHubPermalinkGenerator::new(&file, None).unwrap();
    let link = generator.generate(&file, 1, Some(10)).unwrap();

    assert!(link.url.contains("#L1-L10"));
}

#[test]
fn uses_commit_sha_in_permalink() {
    let temp = setup_git_repo();
    let file = temp.path().join("test.rs");

    let generator = GitHubPermalinkGenerator::new(&file, None).unwrap();
    let link = generator.generate(&file, 1, None).unwrap();

    // Should contain a commit SHA (40 hex chars)
    let sha_pattern = regex::Regex::new(r"/blob/[a-f0-9]{40}/").unwrap();
    assert!(sha_pattern.is_match(&link.url));
}

#[test]
fn handles_ssh_remote_url() {
    let temp = setup_git_repo();
    let file = temp.path().join("test.rs");

    let generator = GitHubPermalinkGenerator::new(&file, Some("origin".into())).unwrap();
    let link = generator.generate(&file, 1, None).unwrap();

    // Should normalize SSH URL to HTTPS
    assert!(link.url.starts_with("https://github.com/"));
}

#[test]
fn fails_for_file_outside_repo() {
    let temp = setup_git_repo();
    let outside_file = temp.path().parent().unwrap().join("outside.rs");
    fs::write(&outside_file, "").unwrap();

    let result = GitHubPermalinkGenerator::new(&outside_file, None);

    // Should fail because file is not in a git repo
    assert!(result.is_err());
}
