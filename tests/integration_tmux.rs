//! Integration tests for tmux session management.
//!
//! These tests require tmux to be installed and available in PATH.
//! They use an isolated tmux server (separate socket) to avoid affecting user sessions.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

use jump::{MaterializedPath, ProjectRoot, SessionInventory, SessionProvisioner};

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A test harness that runs tmux with an isolated socket.
struct IsolatedTmux {
    socket: PathBuf,
    _temp: TempDir,
}

impl IsolatedTmux {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let socket = temp.path().join("tmux.sock");
        Self {
            socket,
            _temp: temp,
        }
    }

    fn run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .args(args)
            .output()
    }

    fn session_exists(&self, name: &str) -> bool {
        self.run(&["has-session", "-t", name])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn list_windows(&self, session: &str) -> usize {
        self.run(&["list-windows", "-t", session, "-F", "#{window_index}"])
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0)
    }

    fn create_session(&self, name: &str, dir: &PathBuf) {
        let dir_str = dir.to_string_lossy();
        let _ = self.run(&["new-session", "-d", "-s", name, "-c", &dir_str]);
        thread::sleep(Duration::from_millis(100));
    }

    fn kill_server(&self) {
        let _ = self.run(&["kill-server"]);
    }
}

impl Drop for IsolatedTmux {
    fn drop(&mut self) {
        self.kill_server();
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
fn isolated_tmux_creates_session() {
    if !tmux_available() {
        eprintln!("Skipping: tmux not available");
        return;
    }

    let tmux = IsolatedTmux::new();
    let (_temp, project) = setup_project();

    tmux.create_session("test_session", &project.path);

    assert!(
        tmux.session_exists("test_session"),
        "session should exist in isolated tmux"
    );
}

#[test]
fn isolated_tmux_lists_sessions() {
    if !tmux_available() {
        eprintln!("Skipping: tmux not available");
        return;
    }

    let tmux = IsolatedTmux::new();
    let (_temp, project) = setup_project();

    tmux.create_session("sess1", &project.path);
    tmux.create_session("sess2", &project.path);

    let output = tmux
        .run(&["list-sessions", "-F", "#{session_name}"])
        .expect("list should work");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let sessions: Vec<&str> = stdout.lines().collect();

    assert!(sessions.contains(&"sess1"));
    assert!(sessions.contains(&"sess2"));
}

#[test]
fn isolated_tmux_opens_new_window() {
    if !tmux_available() {
        eprintln!("Skipping: tmux not available");
        return;
    }

    let tmux = IsolatedTmux::new();
    let (_temp, project) = setup_project();

    tmux.create_session("wintest", &project.path);
    let before = tmux.list_windows("wintest");

    let dir_str = project.path.to_string_lossy();
    let _ = tmux.run(&["new-window", "-t", "wintest", "-c", &dir_str]);
    thread::sleep(Duration::from_millis(100));

    let after = tmux.list_windows("wintest");
    assert_eq!(after, before + 1, "should have one more window");
}

#[test]
fn isolated_tmux_selects_pane() {
    if !tmux_available() {
        eprintln!("Skipping: tmux not available");
        return;
    }

    let tmux = IsolatedTmux::new();
    let (_temp, project) = setup_project();

    tmux.create_session("panetest", &project.path);

    // select-window and select-pane should succeed
    let result = tmux.run(&["select-window", "-t", "panetest:0"]);
    assert!(result.is_ok());

    let result = tmux.run(&["select-pane", "-t", "panetest:0.0"]);
    assert!(result.is_ok());
}

// Tests using the actual TmuxSessionManager but verifying behavior with mocks
// These don't spawn real tmux sessions to avoid side effects

#[test]
fn session_manager_uses_correct_format_strings() {
    use jump::TmuxCommandExecutor;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingExecutor {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        outputs: Arc<Mutex<VecDeque<String>>>,
    }

    impl TmuxCommandExecutor for RecordingExecutor {
        fn run(&self, args: &[&str]) -> anyhow::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            Ok(self.outputs.lock().unwrap().pop_front().unwrap_or_default())
        }
    }

    let exec = RecordingExecutor {
        outputs: Arc::new(Mutex::new(VecDeque::from([
            "dev /home/user/dev\n".to_string(),
            "dev /home/user/dev\n".to_string(),
        ]))),
        ..Default::default()
    };

    let manager = jump::TmuxSessionManager::with_executor(exec.clone());
    let _ = manager.list();

    let calls = exec.calls.lock().unwrap();
    assert!(!calls.is_empty());
    // Verify correct format string is used
    assert!(calls[0].contains(&"#{session_name} #{session_path}".to_string()));
}

#[test]
fn session_manager_spawn_builds_correct_command() {
    use jump::TmuxCommandExecutor;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingExecutor {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl TmuxCommandExecutor for RecordingExecutor {
        fn run(&self, args: &[&str]) -> anyhow::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            Ok(String::new())
        }
    }

    let exec = RecordingExecutor::default();
    let manager = jump::TmuxSessionManager::with_executor(exec.clone());

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();

    let target_file = root.join("main.rs");
    fs::write(&target_file, "fn main() {}").unwrap();

    let target = MaterializedPath {
        absolute: target_file.clone(),
        relative: Some("main.rs".into()),
        line: Some(42),
        end_line: None,
        kind: jump::JumpLinkKind::Relative,
        revision: None,
    };

    let _ = manager.spawn("mydev", &root, &target);

    let calls = exec.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0], "new-session");
    assert_eq!(calls[0][1], "-d");
    assert_eq!(calls[0][2], "-s");
    assert_eq!(calls[0][3], "mydev");
    assert!(calls[0][6].contains("nvim +42"));
}

#[test]
fn nvim_pane_info_generates_correct_targets() {
    let pane = jump::NvimPaneInfo {
        session: "dev".to_string(),
        window: "2".to_string(),
        pane: "1".to_string(),
        nvim_pid: 12345,
    };

    assert_eq!(pane.pane_target(), "dev:2.1");
    assert_eq!(pane.window_target(), "dev:2");
}
