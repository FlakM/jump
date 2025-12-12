//! Integration tests for neovim client and locator.
//!
//! These tests require neovim (nvim) to be installed and available in PATH.
//! They spawn headless neovim instances for testing RPC communication.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

use jump::{EnvAndSocketLocator, NeovimInstanceLocator, NvimInstance, ProjectRoot, SessionInfo};

fn nvim_available() -> bool {
    Command::new("nvim")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct NvimProcess {
    child: Child,
    socket: PathBuf,
}

impl NvimProcess {
    fn spawn(socket_path: &PathBuf) -> Option<Self> {
        if let Some(parent) = socket_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let child = Command::new("nvim")
            .args([
                "--headless",
                "--listen",
                socket_path.to_str().unwrap(),
                "-n", // no swap file
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        // Give nvim time to create the socket
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(100));
            if socket_path.exists() {
                return Some(Self {
                    child,
                    socket: socket_path.clone(),
                });
            }
        }

        None
    }
}

impl Drop for NvimProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.socket);
    }
}

fn setup_project() -> (TempDir, ProjectRoot) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let project = ProjectRoot::new(root.clone(), "Cargo.toml".to_string());
    (temp, project)
}

#[test]
fn locates_nvim_socket_in_search_dir() {
    if !nvim_available() {
        eprintln!("Skipping: nvim not available");
        return;
    }

    let temp = TempDir::new().unwrap();
    let socket_dir = temp.path().join("nvim_test_socket");
    let socket_path = socket_dir.join("0");

    let _nvim = match NvimProcess::spawn(&socket_path) {
        Some(p) => p,
        None => {
            eprintln!("Skipping: failed to spawn nvim");
            return;
        }
    };

    let (_proj_temp, project) = setup_project();
    let session = SessionInfo {
        name: "dev".to_string(),
        path: project.path.clone(),
    };

    let locator = EnvAndSocketLocator::new(vec![temp.path().to_path_buf()]);
    let result = locator.locate(&session, &project);

    assert!(result.is_ok());
    let instance = result.unwrap();
    assert!(instance.is_some(), "should find nvim instance");
    assert!(
        instance.as_ref().unwrap().address.exists(),
        "socket should exist"
    );
}

#[test]
fn locates_nvim_from_env_var() {
    if !nvim_available() {
        eprintln!("Skipping: nvim not available");
        return;
    }

    let temp = TempDir::new().unwrap();
    let socket_path = temp.path().join("nvim_env_socket");

    let _nvim = match NvimProcess::spawn(&socket_path) {
        Some(p) => p,
        None => {
            eprintln!("Skipping: failed to spawn nvim");
            return;
        }
    };

    std::env::set_var("NVIM_LISTEN_ADDRESS", &socket_path);

    let (_proj_temp, project) = setup_project();
    let session = SessionInfo {
        name: "dev".to_string(),
        path: project.path.clone(),
    };

    let locator = EnvAndSocketLocator::with_default_tmp();
    let result = locator.locate(&session, &project);

    std::env::remove_var("NVIM_LISTEN_ADDRESS");

    assert!(result.is_ok());
    let instance = result.unwrap();
    assert!(instance.is_some(), "should find nvim from env");
    assert_eq!(instance.unwrap().address, socket_path);
}

#[test]
fn prefers_socket_matching_session_name() {
    if !nvim_available() {
        eprintln!("Skipping: nvim not available");
        return;
    }

    let temp = TempDir::new().unwrap();

    // Create two socket directories
    let socket1_dir = temp.path().join("nvim_other");
    let socket1 = socket1_dir.join("0");
    let socket2_dir = temp.path().join("nvim_myproject");
    let socket2 = socket2_dir.join("0");

    let _nvim1 = match NvimProcess::spawn(&socket1) {
        Some(p) => p,
        None => {
            eprintln!("Skipping: failed to spawn nvim 1");
            return;
        }
    };

    let _nvim2 = match NvimProcess::spawn(&socket2) {
        Some(p) => p,
        None => {
            eprintln!("Skipping: failed to spawn nvim 2");
            return;
        }
    };

    let (_proj_temp, project) = setup_project();
    let session = SessionInfo {
        name: "myproject".to_string(),
        path: project.path.clone(),
    };

    let locator = EnvAndSocketLocator::new(vec![temp.path().to_path_buf()]);
    let result = locator.locate(&session, &project);

    assert!(result.is_ok());
    let instance = result.unwrap();
    assert!(instance.is_some());
    // Should prefer the socket that matches session name
    assert!(
        instance
            .unwrap()
            .address
            .to_string_lossy()
            .contains("myproject"),
        "should prefer socket matching session name"
    );
}

#[test]
fn returns_none_when_no_nvim_running() {
    let temp = TempDir::new().unwrap();
    let (_proj_temp, project) = setup_project();
    let session = SessionInfo {
        name: "dev".to_string(),
        path: project.path.clone(),
    };

    // Clear env var to ensure we don't find anything
    std::env::remove_var("NVIM_LISTEN_ADDRESS");

    let locator = EnvAndSocketLocator::new(vec![temp.path().to_path_buf()]);
    let result = locator.locate(&session, &project);

    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "should not find any instance");
}

#[test]
fn nvim_instance_fields() {
    let instance = NvimInstance {
        address: PathBuf::from("/tmp/nvim.sock"),
        session_name: Some("dev".to_string()),
        cwd: Some(PathBuf::from("/home/user/project")),
    };

    assert_eq!(instance.address, PathBuf::from("/tmp/nvim.sock"));
    assert_eq!(instance.session_name, Some("dev".to_string()));
    assert_eq!(instance.cwd, Some(PathBuf::from("/home/user/project")));
}
