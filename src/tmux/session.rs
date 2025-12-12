use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::materializer::MaterializedPath;
use crate::project::ProjectRoot;

const PROJECT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    ".git",
    ".obsidian",
];

fn has_project_marker(path: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| path.join(m).exists())
}

const NVIM_SOCKET_PATH_FMT: &str = "/run/user/{}/nvim.{}.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub name: String,
    pub path: PathBuf,
}

// Cached regex patterns for performance
static NVIM_PID_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"nvim\((\d+)\)").expect("invalid nvim pid regex"));

static TMUX_CLIENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"tmux: client\((\d+)\)").expect("invalid tmux client regex"));

/// Information about a Neovim pane within a tmux session.
#[derive(Debug, Clone)]
pub struct NvimPaneInfo {
    pub session: String,
    pub window: String,
    pub pane: String,
    pub nvim_pid: u32,
}

impl NvimPaneInfo {
    /// Returns the path to the Neovim RPC socket for this instance.
    pub fn socket_path(&self) -> PathBuf {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(
            NVIM_SOCKET_PATH_FMT
                .replacen("{}", &uid.to_string(), 1)
                .replacen("{}", &self.nvim_pid.to_string(), 1),
        )
    }

    /// Returns the tmux pane target string (session:window.pane).
    pub fn pane_target(&self) -> String {
        format!("{}:{}.{}", self.session, self.window, self.pane)
    }

    /// Returns the tmux window target string (session:window).
    pub fn window_target(&self) -> String {
        format!("{}:{}", self.session, self.window)
    }
}

/// Runs `pstree` and returns its output if successful.
fn run_pstree(pid: u32) -> Option<String> {
    let output = Command::new("pstree")
        .args(["-p", &pid.to_string()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Extracts the tmux client PID from pstree output.
fn extract_tmux_client_pid(pstree_output: &str) -> Option<u32> {
    TMUX_CLIENT_RE
        .captures(pstree_output)
        .and_then(|caps| caps.get(1)?.as_str().parse().ok())
}

/// Finds the actual nvim process PID from a pane's shell PID.
/// Nvim often spawns itself (nvim→nvim), so we return the last (innermost) PID.
fn find_nvim_child_pid(parent_pid: u32) -> Option<u32> {
    let pstree_output = run_pstree(parent_pid)?;

    // Return the last nvim PID (innermost process has the socket)
    NVIM_PID_RE
        .captures_iter(&pstree_output)
        .filter_map(|caps| caps.get(1)?.as_str().parse().ok())
        .last()
}
use tmux_interface::{
    ListPanes, ListSessions, NewSession, NewWindow, SelectPane, SelectWindow, SwitchClient, Tmux,
};

/// Executes tmux commands and returns their output.
pub trait TmuxCommandExecutor {
    fn run(&self, args: &[&str]) -> Result<String>;
}

#[derive(Clone, Default)]
pub struct DefaultTmuxExecutor;

impl DefaultTmuxExecutor {
    fn run_tmux(cmd: Tmux) -> Result<String> {
        let output = cmd.output().context("Failed to execute tmux command")?;
        if !output.success() {
            bail!("tmux command failed: {}", output);
        }
        Ok(output.to_string())
    }
}

impl TmuxCommandExecutor for DefaultTmuxExecutor {
    fn run(&self, args: &[&str]) -> Result<String> {
        // Map the few tmux commands we use into tmux_interface builders.
        match args {
            ["list-sessions", "-F", fmt] => {
                let cmd = ListSessions::new().format(*fmt);
                Self::run_tmux(Tmux::with_command(cmd))
            }
            ["list-panes", "-a", "-F", fmt] => {
                let cmd = ListPanes::new().all().format(*fmt);
                Self::run_tmux(Tmux::with_command(cmd))
            }
            ["new-session", "-d", "-s", name, "-c", dir, command] => {
                let cmd = NewSession::new()
                    .detached()
                    .session_name(*name)
                    .start_directory(*dir)
                    .shell_command(*command);
                Self::run_tmux(Tmux::with_command(cmd)).map(|_| String::new())
            }
            ["new-window", "-t", target, "-c", dir, command] => {
                let cmd = NewWindow::new()
                    .target_window(*target)
                    .start_directory(*dir)
                    .shell_command(*command);
                Self::run_tmux(Tmux::with_command(cmd)).map(|_| String::new())
            }
            ["switch-client", "-t", target] => {
                let cmd = SwitchClient::new().target_session(*target);
                Self::run_tmux(Tmux::with_command(cmd)).map(|_| String::new())
            }
            ["select-window", "-t", target] => {
                let cmd = SelectWindow::new().target_window(*target);
                Self::run_tmux(Tmux::with_command(cmd)).map(|_| String::new())
            }
            ["select-pane", "-t", target] => {
                let cmd = SelectPane::new().target_pane(*target);
                Self::run_tmux(Tmux::with_command(cmd)).map(|_| String::new())
            }
            ["list-panes", "-a", "-F", fmt, ..] => {
                let cmd = ListPanes::new().all().format(*fmt);
                Self::run_tmux(Tmux::with_command(cmd))
            }
            _ => {
                // Fallback to CLI for any unhandled call
                let output = Command::new("tmux")
                    .args(args)
                    .output()
                    .with_context(|| format!("Failed to run tmux {:?}", args))?;

                if !output.status.success() {
                    bail!(
                        "tmux {:?} failed: {}",
                        args,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }

                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
        }
    }
}

pub trait SessionInventory {
    fn list(&self) -> Result<Vec<SessionInfo>>;
    fn find_repo_session(&self, root: &ProjectRoot) -> Result<Option<SessionInfo>>;
    fn find_session_by_name(&self, name: &str) -> Result<Option<SessionInfo>>;
}

pub trait SessionProvisioner {
    fn spawn(&self, name: &str, root: &Path, target: &MaterializedPath) -> Result<SessionInfo>;
}

pub struct TmuxSessionManager<E: TmuxCommandExecutor = DefaultTmuxExecutor> {
    executor: E,
}

impl TmuxSessionManager<DefaultTmuxExecutor> {
    pub fn new() -> Self {
        Self {
            executor: DefaultTmuxExecutor,
        }
    }
}

impl Default for TmuxSessionManager<DefaultTmuxExecutor> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: TmuxCommandExecutor> TmuxSessionManager<E> {
    pub fn with_executor(executor: E) -> Self {
        Self { executor }
    }

    /// Finds the tmux session associated with a terminal window by its PID.
    pub fn find_session_by_parent_pid(&self, kitty_pid: u32) -> Result<Option<String>> {
        let pstree_output = match run_pstree(kitty_pid) {
            Some(output) => output,
            None => return Ok(None),
        };

        let client_pid = match extract_tmux_client_pid(&pstree_output) {
            Some(pid) => pid,
            None => return Ok(None),
        };

        let clients_output =
            self.executor
                .run(&["list-clients", "-F", "#{client_pid} #{session_name}"])?;

        Ok(clients_output
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let pid: u32 = parts.next()?.parse().ok()?;
                let session = parts.next()?;
                (pid == client_pid).then(|| session.to_string())
            })
            .next())
    }

    /// Finds the kitty PID that has a tmux client attached to the given session.
    pub fn find_kitty_for_session(
        &self,
        session_name: &str,
        kitty_pids: &[u32],
    ) -> Result<Option<u32>> {
        for &kitty_pid in kitty_pids {
            if let Ok(Some(attached_session)) = self.find_session_by_parent_pid(kitty_pid) {
                if attached_session.eq_ignore_ascii_case(session_name) {
                    return Ok(Some(kitty_pid));
                }
            }
        }
        Ok(None)
    }

    pub fn find_nvim_pane(&self, session_name: &str) -> Result<Option<NvimPaneInfo>> {
        let panes_output = self.executor.run(&[
            "list-panes",
            "-s",
            "-t",
            session_name,
            "-F",
            "#{window_index} #{pane_index} #{pane_current_command} #{pane_pid}",
        ])?;

        for line in panes_output.lines() {
            let mut parts = line.split_whitespace();
            let window = parts.next();
            let pane = parts.next();
            let cmd = parts.next();
            let pid_str = parts.next();

            if let (Some(w), Some(p), Some(c), Some(pid_s)) = (window, pane, cmd, pid_str) {
                if c == "nvim" {
                    if let Ok(pane_pid) = pid_s.parse::<u32>() {
                        if let Some(nvim_pid) = find_nvim_child_pid(pane_pid) {
                            return Ok(Some(NvimPaneInfo {
                                session: session_name.to_string(),
                                window: w.to_string(),
                                pane: p.to_string(),
                                nvim_pid,
                            }));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Finds an empty shell pane (bash/zsh/fish) in window 1 of the session.
    fn find_empty_shell_pane(&self, session_name: &str) -> Result<Option<String>> {
        let panes_output = self.executor.run(&[
            "list-panes",
            "-t",
            &format!("{}:1", session_name),
            "-F",
            "#{pane_index} #{pane_current_command}",
        ])?;

        for line in panes_output.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(pane_idx), Some(cmd)) = (parts.next(), parts.next()) {
                if cmd == "zsh" || cmd == "bash" || cmd == "fish" || cmd == "sh" {
                    return Ok(Some(format!("{}:1.{}", session_name, pane_idx)));
                }
            }
        }
        Ok(None)
    }

    pub fn open_nvim_in_session(
        &self,
        session_name: &str,
        root: &Path,
        target: &MaterializedPath,
    ) -> Result<()> {
        let root_path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let line = target.line.unwrap_or(1);
        let file_path = target.absolute.to_string_lossy();
        let nvim_cmd = format!("nvim +{} {}", line, file_path);

        // Try to reuse an empty shell pane in window 1
        if let Ok(Some(pane_target)) = self.find_empty_shell_pane(session_name) {
            // Send the nvim command to the empty pane
            let _ = self.executor.run(&[
                "send-keys",
                "-t",
                &pane_target,
                &format!("cd {} && {}", root_path.to_string_lossy(), nvim_cmd),
                "Enter",
            ]);
            return Ok(());
        }

        // Fallback: create new window
        self.executor.run(&[
            "new-window",
            "-t",
            session_name,
            "-c",
            root_path.to_string_lossy().as_ref(),
            &nvim_cmd,
        ])?;

        Ok(())
    }

    pub fn select_pane(&self, nvim_pane: &NvimPaneInfo) -> Result<()> {
        let window_target = nvim_pane.window_target();
        let pane_target = nvim_pane.pane_target();
        let _ = self.executor.run(&["select-window", "-t", &window_target]);
        let _ = self.executor.run(&["select-pane", "-t", &pane_target]);
        Ok(())
    }

    /// Switches a specific tmux client (identified by kitty PID) to the target session.
    pub fn switch_client_in_kitty(&self, kitty_pid: u32, session_name: &str) -> Result<()> {
        let pstree_output = match run_pstree(kitty_pid) {
            Some(output) => output,
            None => return Ok(()),
        };

        let client_pid = match extract_tmux_client_pid(&pstree_output) {
            Some(pid) => pid,
            None => return Ok(()),
        };

        let clients_output =
            self.executor
                .run(&["list-clients", "-F", "#{client_tty} #{client_pid}"])?;

        for line in clients_output.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(tty), Some(pid_str)) = (parts.next(), parts.next()) {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    if pid == client_pid {
                        let _ =
                            self.executor
                                .run(&["switch-client", "-c", tty, "-t", session_name]);
                        return Ok(());
                    }
                }
            }
        }

        Ok(())
    }

    pub fn activate_session(&self, session_name: &str) -> Result<()> {
        // Gather panes for the session, preferring one running nvim.
        let pane_output = self.executor.run(&[
            "list-panes",
            "-a",
            "-F",
            "#{session_name} #{window_index} #{pane_index} #{pane_current_command}",
        ]);

        if let Ok(output) = pane_output {
            let panes_for_session = output
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let session = parts.next()?;
                    if session != session_name {
                        return None;
                    }
                    let win = parts.next()?;
                    let pane = parts.next()?;
                    let cmd = parts.next().unwrap_or("");
                    Some((win.to_string(), pane.to_string(), cmd.to_string()))
                })
                .collect::<Vec<_>>();

            // Prefer a pane running nvim, else fallback to last pane of session.
            let target = panes_for_session
                .iter()
                .find(|(_, _, cmd)| cmd.contains("nvim"))
                .or_else(|| panes_for_session.last());

            if let Some((win, pane, _)) = target {
                let window_target = format!("{}:{}", session_name, win);
                let pane_target = format!("{}:{}.{}", session_name, win, pane);
                let _ = self.executor.run(&["select-window", "-t", &window_target]);
                let _ = self.executor.run(&["select-pane", "-t", &pane_target]);
                let _ = self.executor.run(&["switch-client", "-t", session_name]);
                return Ok(());
            }
        }

        // Fallback: just switch client to session.
        let _ = self.executor.run(&["switch-client", "-t", session_name]);

        Ok(())
    }

    fn parse_pane_paths(output: &str) -> HashMap<String, PathBuf> {
        let mut map = HashMap::new();

        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut parts = trimmed.splitn(2, ' ');
            if let (Some(name), Some(path)) = (parts.next(), parts.next()) {
                map.entry(name.to_string())
                    .or_insert_with(|| PathBuf::from(path));
            }
        }

        map
    }

    fn collect_pane_paths(&self) -> Result<HashMap<String, PathBuf>> {
        let output = self.executor.run(&[
            "list-panes",
            "-a",
            "-F",
            "#{session_name} #{pane_current_path}",
        ])?;

        Ok(Self::parse_pane_paths(&output))
    }

    fn parse_sessions(output: &str) -> Vec<SessionInfo> {
        output
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }

                let mut parts = trimmed.splitn(2, ' ');
                let name = parts.next()?.to_string();
                let path_part = parts.next().unwrap_or(".");
                Some(SessionInfo {
                    name,
                    path: PathBuf::from(path_part),
                })
            })
            .collect()
    }
}

impl<E: TmuxCommandExecutor> SessionInventory for TmuxSessionManager<E> {
    fn list(&self) -> Result<Vec<SessionInfo>> {
        let session_output =
            self.executor
                .run(&["list-sessions", "-F", "#{session_name} #{session_path}"])?;

        let mut sessions = Self::parse_sessions(&session_output);
        if let Ok(pane_paths) = self.collect_pane_paths() {
            for session in sessions.iter_mut() {
                if let Some(path) = pane_paths.get(&session.name) {
                    session.path = path.clone();
                }
            }
        }

        Ok(sessions)
    }

    fn find_repo_session(&self, root: &ProjectRoot) -> Result<Option<SessionInfo>> {
        let root_path = root
            .path
            .canonicalize()
            .unwrap_or_else(|_| root.path.clone());

        let sessions = self.list()?;

        // First try exact match
        for session in &sessions {
            if let Ok(session_path) = session.path.canonicalize() {
                if session_path == root_path {
                    return Ok(Some(session.clone()));
                }
            }
        }

        // Then try ancestor match, but only if the ancestor is also a project root.
        // This handles workspace members (e.g., Cargo workspace with multiple crates).
        for session in &sessions {
            if let Ok(session_path) = session.path.canonicalize() {
                if root_path.starts_with(&session_path)
                    && session_path != root_path
                    && has_project_marker(&session_path)
                {
                    return Ok(Some(session.clone()));
                }
            }
        }

        Ok(None)
    }

    fn find_session_by_name(&self, name: &str) -> Result<Option<SessionInfo>> {
        for session in self.list()? {
            if session.name.eq_ignore_ascii_case(name) {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }
}

impl<E: TmuxCommandExecutor> SessionProvisioner for TmuxSessionManager<E> {
    fn spawn(&self, name: &str, root: &Path, target: &MaterializedPath) -> Result<SessionInfo> {
        let root_path = root
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize root {:?}", root))?;

        let line = target.line.unwrap_or(1);
        let file_path = target.absolute.to_string_lossy();
        let nvim_cmd = format!("nvim +{} {}", line, file_path);

        self.executor.run(&[
            "new-session",
            "-d",
            "-s",
            name,
            "-c",
            root_path.to_string_lossy().as_ref(),
            &nvim_cmd,
        ])?;

        Ok(SessionInfo {
            name: name.to_string(),
            path: root_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[derive(Clone, Default)]
    struct TestExecutor {
        outputs: Arc<Mutex<VecDeque<String>>>,
        fail: bool,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl TmuxCommandExecutor for TestExecutor {
        fn run(&self, args: &[&str]) -> Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            if self.fail {
                bail!("forced failure");
            }
            Ok(self.outputs.lock().unwrap().pop_front().unwrap_or_default())
        }
    }

    #[test]
    fn lists_sessions_from_output() {
        let exec = TestExecutor {
            outputs: Arc::new(Mutex::new(VecDeque::from(vec![
                "dev /home/user/dev\nwork /tmp/work\n".to_string(),
                "dev /home/user/dev\nwork /tmp/work\n".to_string(),
            ]))),
            ..Default::default()
        };
        let manager = TmuxSessionManager::with_executor(exec.clone());

        let sessions = manager.list().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "dev");
        assert_eq!(sessions[1].path, PathBuf::from("/tmp/work"));

        let calls = exec.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            vec!["list-sessions", "-F", "#{session_name} #{session_path}"]
        );
        assert_eq!(
            calls[1],
            vec![
                "list-panes",
                "-a",
                "-F",
                "#{session_name} #{pane_current_path}"
            ]
        );
    }

    #[test]
    fn finds_matching_session_by_root() {
        let temp_dir = TempDir::new().unwrap();
        let project_root = ProjectRoot::new(temp_dir.path().to_path_buf(), ".git".to_string());

        let exec = TestExecutor {
            outputs: Arc::new(Mutex::new(VecDeque::from(vec![
                format!(
                    "dev {}\nother /tmp/other",
                    temp_dir.path().canonicalize().unwrap().display()
                ),
                format!(
                    "dev {}\nother /tmp/other",
                    temp_dir.path().canonicalize().unwrap().display()
                ),
            ]))),
            ..Default::default()
        };
        let manager = TmuxSessionManager::with_executor(exec);

        let found = manager.find_repo_session(&project_root).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "dev");
    }

    #[test]
    fn spawns_session_with_nvim_command() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();

        let target_file = root.join("src/main.rs");
        std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
        std::fs::write(&target_file, "// test").unwrap();

        let target = MaterializedPath {
            absolute: target_file.clone(),
            relative: Some(PathBuf::from("src/main.rs")),
            line: Some(12),
            end_line: None,
            kind: crate::parser::JumpLinkKind::Relative,
            revision: None,
        };

        let exec = TestExecutor::default();
        let manager = TmuxSessionManager::with_executor(exec.clone());

        let session = manager
            .spawn("dev", &root, &target)
            .expect("spawn should succeed");

        assert_eq!(session.name, "dev");
        assert_eq!(session.path, root.canonicalize().unwrap());

        let calls = exec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let args = &calls[0];
        assert_eq!(args[0], "new-session");
        assert_eq!(args[3], "dev");
        assert_eq!(args[4], "-c");
        assert_eq!(args[5], root.canonicalize().unwrap().to_string_lossy());
        assert!(args[6].contains("nvim +12"));
        assert!(args[6].contains(target_file.to_string_lossy().as_ref()));
    }

    #[test]
    fn opens_new_window_in_existing_session() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();

        let target_file = root.join("src/main.rs");
        std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
        std::fs::write(&target_file, "// test").unwrap();

        let target = MaterializedPath {
            absolute: target_file.clone(),
            relative: Some(PathBuf::from("src/main.rs")),
            line: Some(8),
            end_line: None,
            kind: crate::parser::JumpLinkKind::Relative,
            revision: None,
        };

        let exec = TestExecutor::default();
        let manager = TmuxSessionManager::with_executor(exec.clone());

        manager
            .open_nvim_in_session("dev", &root, &target)
            .expect("open should succeed");

        let calls = exec.calls.lock().unwrap();
        // First call: list-panes to check for empty shell
        // Second call: new-window (since no empty shell found)
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0][0], "list-panes");
        let args = &calls[1];
        assert_eq!(args[0], "new-window");
        assert_eq!(args[2], "dev");
        assert_eq!(args[3], "-c");
        assert_eq!(args[4], root.canonicalize().unwrap().to_string_lossy());
        assert!(args[5].contains("nvim +8"));
        assert!(args[5].contains(target_file.to_string_lossy().as_ref()));
    }

    #[test]
    fn activates_session() {
        let exec = TestExecutor::default();
        let manager = TmuxSessionManager::with_executor(exec.clone());

        // first call: no panes listed, we attempt switch-client fallback
        manager
            .activate_session("work")
            .expect("activate should succeed");

        {
            let calls = exec.calls.lock().unwrap();
            assert!(!calls.is_empty());
            assert_eq!(
                calls[0],
                vec![
                    "list-panes",
                    "-a",
                    "-F",
                    "#{session_name} #{window_index} #{pane_index} #{pane_current_command}"
                ]
            );
            assert!(calls
                .iter()
                .any(|c| c == &vec!["switch-client", "-t", "work"]));
        }

        // When list-panes returns data, we should select window and pane too (prefer nvim pane).
        exec.outputs.lock().unwrap().clear();
        exec.outputs.lock().unwrap().extend([
            String::from("work 1 0 bash\nwork 1 1 nvim\n"), // list-panes output
        ]);

        manager
            .activate_session("work")
            .expect("activate should succeed with pane focus");

        let calls = exec.calls.lock().unwrap();
        assert!(calls
            .iter()
            .any(|c| c.starts_with(&["select-window".to_string()])));
        assert!(calls
            .iter()
            .any(|c| c.starts_with(&["select-pane".to_string()])));
        assert!(calls
            .iter()
            .any(|c| c == &vec!["switch-client", "-t", "work"]));
    }
}
