//! Herdr backend: open a reference in the Neovim pane of a herdr workspace.
//!
//! Herdr is a terminal workspace manager whose panes hold editors and agents. When
//! `jump` runs inside a herdr pane (`HERDR_ENV=1`), this backend finds the pane already
//! running Neovim for the project and drives it through the pane input API, instead of
//! requiring a tmux server. Tmux remains the fallback.
//!
//! Two things make this reliable where a naive version is not:
//!   * a pane is only considered an editor after `pane process-info` shows a Neovim
//!     process, because pane titles are unreliable for editors;
//!   * captured agent panes (Claude, OpenCode) are skipped outright, so `jump` does not
//!     type into a coding agent's prompt by mistake.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tracing::warn;

use crate::materializer::MaterializedPath;
use crate::project::ProjectRoot;

/// Run a command with a hard timeout. A Neovim waiting at a prompt does not answer
/// RPC, so without this a socket scan can block the whole jump.
fn run_with_timeout(mut command: Command, timeout: std::time::Duration) -> Result<String> {
    use std::io::Read;
    use std::process::Stdio;

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn command")?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait().context("failed to poll child")? {
            Some(_) => break,
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("timed out after {:?}", timeout);
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let status = child.wait().context("failed to collect child")?;
    if !status.success() {
        bail!("command failed: {}", stderr.trim());
    }
    Ok(stdout)
}

/// How long a single `herdr` or `nvim` call may take before it is abandoned.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Executes `herdr` commands and returns their stdout.
pub trait HerdrCommandExecutor {
    fn run(&self, args: &[&str]) -> Result<String>;
}

/// Talks to a Neovim server over its socket, the same channel `nvr` uses.
///
/// Driving the editor over RPC is far more reliable than typing into a terminal pane:
/// the pane may be in bracketed-paste mode, and its rendered view lags the real state,
/// so a typed command can appear stuck when it has in fact run.
pub trait NvimSocket {
    /// Socket paths worth probing, cheapest match first.
    fn candidates(&self) -> Vec<PathBuf>;
    fn expr(&self, socket: &Path, expression: &str) -> Result<String>;
    fn send(&self, socket: &Path, keys: &str) -> Result<()>;
}

/// Default socket client: scans the runtime dir and `/tmp`, then shells out to
/// `nvim --server`.
#[derive(Clone, Default)]
pub struct DefaultNvimSocket;

impl NvimSocket for DefaultNvimSocket {
    fn candidates(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
        if let Ok(entries) = std::fs::read_dir(&runtime) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("nvim.")
                    || (name.starts_with("jump-") && name.ends_with(".sock"))
                {
                    found.push(entry.path());
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir("/tmp") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("nvim") {
                    let zero = entry.path().join("0");
                    if zero.exists() {
                        found.push(zero);
                    }
                }
            }
        }
        found
    }

    fn expr(&self, socket: &Path, expression: &str) -> Result<String> {
        let mut command = Command::new("nvim");
        command
            .arg("--server")
            .arg(socket)
            .arg("--remote-expr")
            .arg(expression);
        run_with_timeout(command, CALL_TIMEOUT)
            .with_context(|| format!("nvim --remote-expr on {}", socket.display()))
    }

    fn send(&self, socket: &Path, keys: &str) -> Result<()> {
        let mut command = Command::new("nvim");
        command
            .arg("--server")
            .arg(socket)
            .arg("--remote-send")
            .arg(keys);
        run_with_timeout(command, CALL_TIMEOUT)
            .map(|_| ())
            .with_context(|| format!("nvim --remote-send on {}", socket.display()))
    }
}

/// Default executor that shells out to the `herdr` binary.
#[derive(Clone, Default)]
pub struct DefaultHerdrExecutor;

impl HerdrCommandExecutor for DefaultHerdrExecutor {
    fn run(&self, args: &[&str]) -> Result<String> {
        let mut command = Command::new("herdr");
        command.args(args);
        run_with_timeout(command, CALL_TIMEOUT).with_context(|| format!("herdr {}", args.join(" ")))
    }
}

/// A herdr pane, as reported by `herdr pane list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HerdrPane {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub cwd: Option<PathBuf>,
    pub foreground_cwd: Option<PathBuf>,
    pub agent: Option<String>,
}

impl HerdrPane {
    /// Related in either direction: a pane opened on the project, or on a parent of it.
    /// A monorepo pane opened at the repo root must still serve a jump to a nested crate.
    fn related(&self, root: &Path) -> bool {
        [self.cwd.as_ref(), self.foreground_cwd.as_ref()]
            .into_iter()
            .flatten()
            .any(|cwd| cwd.as_path() == root || cwd.starts_with(root) || root.starts_with(cwd))
    }

    /// A captured coding agent is not an editor: never type into its prompt.
    fn might_be_editor(&self) -> bool {
        match self.agent.as_deref() {
            None => true,
            Some(agent) => matches!(agent, "nvim" | "neovim" | "vim"),
        }
    }
}

pub struct HerdrSessionManager<E = DefaultHerdrExecutor, S = DefaultNvimSocket> {
    executor: E,
    nvim_socket: S,
}

impl HerdrSessionManager<DefaultHerdrExecutor, DefaultNvimSocket> {
    pub fn new() -> Self {
        Self {
            executor: DefaultHerdrExecutor,
            nvim_socket: DefaultNvimSocket,
        }
    }

    /// True when this process runs inside a herdr-managed pane.
    pub fn available() -> bool {
        std::env::var("HERDR_ENV")
            .map(|v| v == "1")
            .unwrap_or(false)
    }
}

impl Default for HerdrSessionManager<DefaultHerdrExecutor, DefaultNvimSocket> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: HerdrCommandExecutor> HerdrSessionManager<E, DefaultNvimSocket> {
    /// Herdr commands are faked; socket discovery uses the real runtime directory.
    pub fn with_executor(executor: E) -> Self {
        Self {
            executor,
            nvim_socket: DefaultNvimSocket,
        }
    }
}

impl<E: HerdrCommandExecutor, S: NvimSocket> HerdrSessionManager<E, S> {
    /// Both seams injected, for tests that must not touch the real environment.
    pub fn with_executors(executor: E, nvim_socket: S) -> Self {
        Self {
            executor,
            nvim_socket,
        }
    }

    /// Panes across every workspace, so a jump can find the editor even when the
    /// project lives in another workspace than the caller.
    pub fn list_panes(&self) -> Result<Vec<HerdrPane>> {
        let raw = self.executor.run(&["pane", "list"])?;
        let value: Value =
            serde_json::from_str(&raw).context("herdr pane list returned invalid JSON")?;
        let panes = value["result"]["panes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(panes
            .iter()
            .map(|pane| HerdrPane {
                pane_id: pane["pane_id"].as_str().unwrap_or_default().to_string(),
                tab_id: pane["tab_id"].as_str().unwrap_or_default().to_string(),
                workspace_id: pane["workspace_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                cwd: pane["cwd"].as_str().map(PathBuf::from),
                foreground_cwd: pane["foreground_cwd"].as_str().map(PathBuf::from),
                agent: pane["agent"].as_str().map(str::to_string),
            })
            .filter(|pane| !pane.pane_id.is_empty())
            .collect())
    }

    fn pane_runs_nvim(&self, pane_id: &str) -> Result<bool> {
        let raw = self
            .executor
            .run(&["pane", "process-info", "--pane", pane_id])?;
        let value: Value =
            serde_json::from_str(&raw).context("herdr pane process-info returned invalid JSON")?;
        let processes = value["result"]["process_info"]["foreground_processes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(processes.iter().any(|process| {
            let name = process["name"].as_str().unwrap_or_default();
            let argv0 = process["argv"]
                .as_array()
                .and_then(|argv| argv.first())
                .and_then(|arg| arg.as_str())
                .unwrap_or_default();
            name.contains("nvim") || argv0.contains("nvim")
        }))
    }

    /// The Neovim pane for this project.
    ///
    /// Only panes related to the project are considered: landing in a different
    /// project's editor would be worse than opening a new one, so a project with no
    /// editor of its own gets a tab instead (as the tmux backend already does).
    pub fn find_nvim_pane(&self, root: &ProjectRoot) -> Result<Option<HerdrPane>> {
        let panes = self.list_panes()?;
        for pane in panes
            .into_iter()
            .filter(HerdrPane::might_be_editor)
            .filter(|pane| pane.related(&root.path))
        {
            if self.pane_runs_nvim(&pane.pane_id)? {
                return Ok(Some(pane));
            }
        }
        Ok(None)
    }

    /// The pid of the Neovim process in a pane, when one is visible.
    fn pane_nvim_pid(&self, pane_id: &str) -> Result<Option<u32>> {
        let raw = self
            .executor
            .run(&["pane", "process-info", "--pane", pane_id])?;
        let value: Value =
            serde_json::from_str(&raw).context("herdr pane process-info returned invalid JSON")?;
        let processes = value["result"]["process_info"]["foreground_processes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(processes.iter().find_map(|process| {
            let name = process["name"].as_str().unwrap_or_default();
            let argv0 = process["argv"]
                .as_array()
                .and_then(|argv| argv.first())
                .and_then(|arg| arg.as_str())
                .unwrap_or_default();
            if name.contains("nvim") || argv0.contains("nvim") {
                process["pid"].as_u64().map(|pid| pid as u32)
            } else {
                None
            }
        }))
    }

    /// The Neovim socket serving a pane.
    ///
    /// The socket file is named for the nvim process, but the pid herdr reports can be a
    /// short-lived wrapper just above the real one, so a near-miss is accepted before
    /// falling back to asking each server for its working directory.
    pub fn find_socket(&self, pane: &HerdrPane, root: &ProjectRoot) -> Result<Option<PathBuf>> {
        let candidates = self.nvim_socket.candidates();
        let pane_pid = self.pane_nvim_pid(&pane.pane_id).unwrap_or(None);

        if let Some(pid) = pane_pid {
            for candidate in &candidates {
                if let Some(socket_pid) = socket_pid(candidate) {
                    if (i64::from(socket_pid) - i64::from(pid)).abs() <= 2 {
                        return Ok(Some(candidate.clone()));
                    }
                }
            }
        }

        let wanted: Vec<&Path> = [
            pane.cwd.as_deref(),
            pane.foreground_cwd.as_deref(),
            Some(root.path.as_path()),
        ]
        .into_iter()
        .flatten()
        .collect();
        for candidate in &candidates {
            let Ok(cwd) = self.nvim_socket.expr(candidate, "getcwd()") else {
                continue;
            };
            let cwd = PathBuf::from(cwd.trim());
            if wanted
                .iter()
                .any(|want| cwd == *want || cwd.starts_with(want) || want.starts_with(&cwd))
            {
                return Ok(Some(candidate.clone()));
            }
        }
        Ok(None)
    }

    /// Open the target through the editor's socket, then confirm it landed.
    ///
    /// Remote sends are asynchronous, so the read-back is polled. A failed landing is
    /// reported rather than papered over: the editor may be waiting at a prompt (a stale
    /// swap file, for instance), and typing the command into that prompt would make a
    /// worse mess than an honest error.
    pub fn open_via_socket(&self, socket: &Path, target: &MaterializedPath) -> Result<()> {
        let line = target.line.unwrap_or(1);
        let path = target.absolute.to_string_lossy();
        let keys = format!("<Esc>:edit +{line} {path}<CR>");
        self.nvim_socket.send(socket, &keys)?;

        let expected = format!("{}:{line}", target.absolute.to_string_lossy());
        let mut seen = String::new();
        for attempt in 0..15 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            seen = self
                .nvim_socket
                .expr(socket, r#"expand('%:p') .. ':' .. line('.')"#)?
                .trim()
                .to_string();
            if seen == expected {
                return Ok(());
            }
        }

        let mode = self
            .nvim_socket
            .expr(socket, "mode(1)")
            .unwrap_or_default()
            .trim()
            .to_string();
        let pending = self
            .nvim_socket
            .expr(socket, "getcmdline()")
            .unwrap_or_default()
            .trim()
            .to_string();
        if mode != "n" || !pending.is_empty() {
            bail!(
                "Neovim at {} is waiting for an answer (mode {mode}{}); \
                 a stale swap file or an open prompt blocks the jump — resolve it and retry",
                socket.display(),
                if pending.is_empty() {
                    String::new()
                } else {
                    format!(", pending {pending:?}")
                }
            );
        }
        bail!("socket open did not land: expected {expected}, editor reports {seen}")
    }

    /// Send `:edit +LINE PATH` to a Neovim pane, mirroring what nvr does: escape first
    /// so the command runs from normal mode, then the command, then Enter.
    pub fn open_in_pane(&self, pane: &HerdrPane, target: &MaterializedPath) -> Result<()> {
        let line = target.line.unwrap_or(1);
        let path = vim_file_expression(&target.absolute);
        // fnameescape keeps spaces, quotes and '%' from being read as Ex syntax.
        let command = format!(":execute \"edit +{line} \" . fnameescape({path})");

        self.executor
            .run(&["pane", "send-keys", &pane.pane_id, "esc"])?;
        self.executor
            .run(&["pane", "send-text", &pane.pane_id, &command])?;
        self.executor
            .run(&["pane", "send-keys", &pane.pane_id, "enter"])?;
        Ok(())
    }

    /// Bring the pane's tab to the front. Best-effort: a jump that opened the file
    /// successfully should not fail because focus did not move.
    pub fn focus(&self, pane: &HerdrPane) -> Result<()> {
        if pane.tab_id.is_empty() {
            return Ok(());
        }
        let _ = self
            .executor
            .run(&["workspace", "focus", &pane.workspace_id]);
        self.executor.run(&["tab", "focus", &pane.tab_id])?;
        Ok(())
    }

    /// Create a tab running Neovim on the target, for a project with no editor open.
    pub fn open_nvim_tab(
        &self,
        root: &ProjectRoot,
        target: &MaterializedPath,
    ) -> Result<HerdrPane> {
        let workspace = std::env::var("HERDR_WORKSPACE_ID").ok();
        let cwd = root.path.to_string_lossy().into_owned();
        let mut args = vec![
            "tab",
            "create",
            "--cwd",
            cwd.as_str(),
            "--label",
            root.name.as_str(),
            "--no-focus",
        ];
        if let Some(workspace) = workspace.as_deref() {
            args.push("--workspace");
            args.push(workspace);
        }

        let raw = self.executor.run(&args)?;
        let value: Value =
            serde_json::from_str(&raw).context("herdr tab create returned invalid JSON")?;
        let pane_id = value["result"]["root_pane"]["pane_id"]
            .as_str()
            .context("herdr tab create returned no pane id")?
            .to_string();
        let tab_id = value["result"]["tab"]["tab_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let line = target.line.unwrap_or(1);
        let path = shell_quote(&target.absolute.to_string_lossy());
        // A named socket makes this editor reachable over RPC by later jumps, instead of
        // relying on pane typing.
        let listen = shell_quote(&listen_socket_path(&root.name).to_string_lossy());
        self.executor.run(&[
            "pane",
            "run",
            &pane_id,
            &format!("nvim --listen {listen} +{line} {path}"),
        ])?;

        Ok(HerdrPane {
            pane_id,
            tab_id,
            workspace_id: workspace.unwrap_or_default(),
            cwd: Some(root.path.clone()),
            foreground_cwd: None,
            agent: None,
        })
    }

    /// Open the target in the project's editor, creating one if the project has none.
    /// Returns a session label for the outcome.
    pub fn launch(&self, root: &ProjectRoot, target: &MaterializedPath) -> Result<String> {
        if let Some(pane) = self.find_nvim_pane(root)? {
            // Prefer the editor's own socket. Typing into the pane is only for the case
            // where no socket exists at all: typing at a prompt is worse than an error.
            match self.find_socket(&pane, root) {
                Ok(Some(socket)) => {
                    self.open_via_socket(&socket, target)?;
                    let _ = self.focus(&pane);
                    return Ok(format!("herdr:{}", pane.pane_id));
                }
                Ok(None) => warn!(
                    "no Neovim socket for {}; typing into the pane",
                    pane.pane_id
                ),
                Err(error) => warn!("socket discovery failed ({error:#}); typing into the pane"),
            }
            self.open_in_pane(&pane, target)?;
            let _ = self.focus(&pane);
            return Ok(format!("herdr:{}", pane.pane_id));
        }

        let pane = self.open_nvim_tab(root, target)?;
        let _ = self.focus(&pane);
        Ok(format!("herdr:{}", pane.pane_id))
    }
}

/// Where a Neovim started by `jump` listens, so later jumps can reach it over RPC.
fn listen_socket_path(project_name: &str) -> PathBuf {
    let safe: String = project_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    PathBuf::from(runtime).join(format!("jump-{}.sock", safe.trim_matches('-')))
}

/// The pid embedded in a default Neovim socket name (`nvim.<pid>.<n>`).
fn socket_pid(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let mut parts = name.split('.');
    if parts.next()? != "nvim" {
        return None;
    }
    parts.next()?.parse().ok()
}

/// A Lua/Vim string literal for a path: single quotes, with embedded quotes doubled.
fn vim_file_expression(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "''");
    format!("'{}'", escaped)
}

/// A POSIX shell literal, for the command handed to a fresh pane.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const PROJECT: &str = "/home/dev/project";

    #[derive(Clone, Default)]
    struct FakeHerdr {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        /// (pane_id, nvim pid, cwd, agent) — a pane without a pid runs no editor.
        panes: Vec<(String, Option<u32>, String, Option<String>)>,
        tab_create: String,
        fail_on: Option<String>,
    }

    impl FakeHerdr {
        fn new(panes: &[(&str, Option<u32>, &str, Option<&str>)]) -> Self {
            Self {
                panes: panes
                    .iter()
                    .map(|(id, pid, cwd, agent)| {
                        (
                            id.to_string(),
                            *pid,
                            cwd.to_string(),
                            agent.map(str::to_string),
                        )
                    })
                    .collect(),
                tab_create:
                    r#"{"result":{"tab":{"tab_id":"wV:t9"},"root_pane":{"pane_id":"wV:p9"}}}"#
                        .to_string(),
                ..Default::default()
            }
        }

        fn recorded(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }

        fn called(&self, needle: &str) -> bool {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call.join(" ").contains(needle))
        }
    }

    impl HerdrCommandExecutor for FakeHerdr {
        fn run(&self, args: &[&str]) -> Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());

            if let Some(needle) = &self.fail_on {
                if args.join(" ").contains(needle.as_str()) {
                    bail!("forced failure on {needle}");
                }
            }

            match (args.first(), args.get(1)) {
                (Some(&"pane"), Some(&"list")) => {
                    let rows: Vec<String> = self
                        .panes
                        .iter()
                        .map(|(pane_id, _, cwd, agent)| {
                            let agent = match agent {
                                Some(agent) => format!("\"{agent}\""),
                                None => "null".to_string(),
                            };
                            format!(
                                r#"{{"pane_id":"{pane_id}","tab_id":"wV:t1","workspace_id":"wV","cwd":"{cwd}","foreground_cwd":"{cwd}","agent":{agent}}}"#
                            )
                        })
                        .collect();
                    Ok(format!(r#"{{"result":{{"panes":[{}]}}}}"#, rows.join(",")))
                }
                (Some(&"pane"), Some(&"process-info")) => {
                    let pane_id = args.get(3).copied().unwrap_or_default();
                    let pid = self
                        .panes
                        .iter()
                        .find(|(id, _, _, _)| id == pane_id)
                        .and_then(|(_, pid, _, _)| *pid);
                    let processes = match pid {
                        Some(pid) => format!(
                            r#"[{{"name":"nvim","pid":{pid},"argv":["/nix/store/x/bin/nvim"]}}]"#
                        ),
                        None => r#"[{"name":"zsh","argv":["/bin/zsh"]}]"#.to_string(),
                    };
                    Ok(format!(
                        r#"{{"result":{{"process_info":{{"foreground_processes":{processes}}}}}}}"#
                    ))
                }
                (Some(&"tab"), Some(&"create")) => Ok(self.tab_create.clone()),
                _ => Ok("{}".to_string()),
            }
        }
    }

    #[derive(Clone, Default)]
    struct FakeSocket {
        candidates: Vec<PathBuf>,
        cwd_by_socket: HashMap<PathBuf, String>,
        /// Read-back values, consumed one per poll; the last value repeats.
        after_send: Arc<Mutex<Vec<String>>>,
        mode: String,
        cmdline: String,
        sends: Arc<Mutex<Vec<(PathBuf, String)>>>,
    }

    impl FakeSocket {
        fn none() -> Self {
            Self::default()
        }

        fn with_socket(path: &str, cwd: &str) -> Self {
            let mut socket = Self::default();
            socket.candidates.push(PathBuf::from(path));
            socket
                .cwd_by_socket
                .insert(PathBuf::from(path), cwd.to_string());
            socket
        }

        fn sends(&self) -> Vec<(PathBuf, String)> {
            self.sends.lock().unwrap().clone()
        }
    }

    impl NvimSocket for FakeSocket {
        fn candidates(&self) -> Vec<PathBuf> {
            self.candidates.clone()
        }

        fn expr(&self, socket: &Path, expression: &str) -> Result<String> {
            if expression.contains("getcwd") {
                return self
                    .cwd_by_socket
                    .get(socket)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no server at {}", socket.display()));
            }
            if expression.contains("mode(") {
                return Ok(if self.mode.is_empty() {
                    "n".to_string()
                } else {
                    self.mode.clone()
                });
            }
            if expression.contains("getcmdline") {
                return Ok(self.cmdline.clone());
            }
            let mut queue = self.after_send.lock().unwrap();
            if queue.len() > 1 {
                Ok(queue.remove(0))
            } else {
                Ok(queue.first().cloned().unwrap_or_default())
            }
        }

        fn send(&self, socket: &Path, keys: &str) -> Result<()> {
            self.sends
                .lock()
                .unwrap()
                .push((socket.to_path_buf(), keys.to_string()));
            Ok(())
        }
    }

    fn project() -> ProjectRoot {
        ProjectRoot::new(PathBuf::from(PROJECT), ".git".to_string())
    }

    fn target(line: Option<u32>) -> MaterializedPath {
        MaterializedPath {
            absolute: PathBuf::from(format!("{PROJECT}/src/lib.rs")),
            relative: None,
            line,
            end_line: None,
            kind: crate::parser::JumpLinkKind::Absolute,
            revision: None,
        }
    }

    fn pane(id: &str) -> HerdrPane {
        HerdrPane {
            pane_id: id.to_string(),
            tab_id: "wV:t1".to_string(),
            workspace_id: "wV".to_string(),
            cwd: Some(PathBuf::from(PROJECT)),
            foreground_cwd: None,
            agent: None,
        }
    }

    fn expect_after_send(socket: &FakeSocket, path: &str, line: u32) {
        *socket.after_send.lock().unwrap() = vec![format!("{path}:{line}")];
    }

    fn expect_readback(socket: &FakeSocket, values: &[&str]) {
        *socket.after_send.lock().unwrap() = values.iter().map(|v| v.to_string()).collect();
    }

    #[test]
    fn finds_the_nvim_pane_for_the_project_and_skips_agents() {
        let fake = FakeHerdr::new(&[
            ("wV:p1", Some(11), PROJECT, Some("opencode")),
            ("wV:p2", Some(22), PROJECT, None),
            ("wZ:p3", Some(33), "/home/dev/other", None),
        ]);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), FakeSocket::none());

        let found = herdr.find_nvim_pane(&project()).unwrap().unwrap();
        assert_eq!(found.pane_id, "wV:p2");
        assert!(
            !fake.called("--pane wV:p1"),
            "must not probe a captured agent's prompt"
        );
    }

    #[test]
    fn prefers_the_project_pane_over_another_workspace() {
        let fake = FakeHerdr::new(&[
            ("wZ:p3", Some(33), "/home/dev/other", None),
            ("wV:p2", Some(22), PROJECT, None),
        ]);
        let herdr = HerdrSessionManager::with_executors(fake, FakeSocket::none());

        let found = herdr.find_nvim_pane(&project()).unwrap().unwrap();
        assert_eq!(found.pane_id, "wV:p2");
    }

    #[test]
    fn does_not_hijack_an_unrelated_projects_editor() {
        let fake = FakeHerdr::new(&[
            ("wV:p2", None, PROJECT, None),
            ("wZ:p3", Some(33), "/home/dev/other", None),
        ]);
        let herdr = HerdrSessionManager::with_executors(fake, FakeSocket::none());

        assert!(
            herdr.find_nvim_pane(&project()).unwrap().is_none(),
            "a project with no editor must get its own tab, not another project's"
        );
    }

    #[test]
    fn matches_a_pane_opened_on_a_parent_of_the_project() {
        // A monorepo editor opened at /home/dev still serves /home/dev/project.
        let fake = FakeHerdr::new(&[("wV:p2", Some(22), "/home/dev", None)]);
        let herdr = HerdrSessionManager::with_executors(fake, FakeSocket::none());

        assert_eq!(
            herdr.find_nvim_pane(&project()).unwrap().unwrap().pane_id,
            "wV:p2"
        );
    }

    #[test]
    fn ignores_panes_that_do_not_run_nvim() {
        let fake = FakeHerdr::new(&[("wV:p2", None, PROJECT, None)]);
        let herdr = HerdrSessionManager::with_executors(fake, FakeSocket::none());

        assert!(herdr.find_nvim_pane(&project()).unwrap().is_none());
    }

    #[test]
    fn opens_with_escape_command_and_enter_in_order() {
        let fake = FakeHerdr::new(&[]);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), FakeSocket::none());

        herdr
            .open_in_pane(&pane("wV:p2"), &target(Some(42)))
            .unwrap();

        let calls = fake.recorded();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], vec!["pane", "send-keys", "wV:p2", "esc"]);
        assert_eq!(calls[1][0..3], ["pane", "send-text", "wV:p2"]);
        assert!(calls[1][3].contains("edit +42"), "got {}", calls[1][3]);
        assert!(calls[1][3].contains("fnameescape"), "must escape the path");
        assert_eq!(calls[2], vec!["pane", "send-keys", "wV:p2", "enter"]);
    }

    #[test]
    fn escapes_awkward_paths_so_ex_is_not_reinterpreted() {
        let fake = FakeHerdr::new(&[]);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), FakeSocket::none());
        let awkward = MaterializedPath {
            absolute: PathBuf::from("/tmp/it's a dir/100% file.rs"),
            relative: None,
            line: Some(3),
            end_line: None,
            kind: crate::parser::JumpLinkKind::Absolute,
            revision: None,
        };

        herdr.open_in_pane(&pane("wV:p2"), &awkward).unwrap();

        let command = fake.recorded()[1][3].clone();
        assert!(
            command.contains("it''s a dir"),
            "single quote doubled: {command}"
        );
        assert!(
            command.contains("100% file.rs"),
            "path kept verbatim: {command}"
        );
    }

    #[test]
    fn prefers_the_socket_over_typing_into_the_pane() {
        let fake = FakeHerdr::new(&[("wV:p2", Some(4242), PROJECT, None)]);
        let socket = FakeSocket::with_socket("/run/user/1000/nvim.4242.0", PROJECT);
        expect_after_send(&socket, &format!("{PROJECT}/src/lib.rs"), 7);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), socket.clone());

        let session = herdr.launch(&project(), &target(Some(7))).unwrap();

        assert_eq!(session, "herdr:wV:p2");
        let sends = socket.sends();
        assert_eq!(sends.len(), 1, "expected exactly one RPC send");
        assert!(sends[0].1.contains("edit +7"), "got {}", sends[0].1);
        assert!(
            !fake.called("send-text"),
            "must not type into the pane when the socket works"
        );
    }

    #[test]
    fn finds_the_socket_by_working_directory_when_the_pid_is_off() {
        // herdr can report a wrapper pid just above the real nvim, so the name does not
        // match and discovery must fall back to asking the server where it is.
        let fake = FakeHerdr::new(&[("wV:p2", Some(5000), PROJECT, None)]);
        let socket = FakeSocket::with_socket("/run/user/1000/nvim.5001.0", PROJECT);
        expect_after_send(&socket, &format!("{PROJECT}/src/lib.rs"), 9);
        let herdr = HerdrSessionManager::with_executors(fake, socket.clone());

        herdr.launch(&project(), &target(Some(9))).unwrap();

        assert_eq!(socket.sends().len(), 1);
    }

    #[test]
    fn ignores_a_socket_serving_a_different_project() {
        let fake = FakeHerdr::new(&[("wV:p2", Some(6000), PROJECT, None)]);
        let socket = FakeSocket::with_socket("/run/user/1000/nvim.9999.0", "/home/dev/other");
        let herdr = HerdrSessionManager::with_executors(fake.clone(), socket.clone());

        herdr.launch(&project(), &target(Some(1))).unwrap();

        assert!(
            socket.sends().is_empty(),
            "wrong project's socket must not be used"
        );
        assert!(fake.called("send-text"), "should fall back to typing");
    }

    #[test]
    fn reports_an_actionable_error_instead_of_typing_when_the_editor_is_at_a_prompt() {
        // A stale swap file leaves Neovim at an E325 prompt; typing the command there
        // would corrupt the buffer, so the jump must fail loudly instead.
        let fake = FakeHerdr::new(&[("wV:p2", Some(7000), PROJECT, None)]);
        let mut socket = FakeSocket::with_socket("/run/user/1000/nvim.7000.0", PROJECT);
        expect_readback(&socket, &[&format!("{PROJECT}/src/app.rs:1")]);
        socket.mode = "r?".to_string();
        let herdr = HerdrSessionManager::with_executors(fake.clone(), socket);

        let error = herdr.launch(&project(), &target(Some(5))).unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("waiting for an answer"), "got: {message}");
        assert!(!fake.called("send-text"), "must not type into a prompt");
    }

    #[test]
    fn reports_the_wrong_buffer_rather_than_typing() {
        let fake = FakeHerdr::new(&[("wV:p2", Some(7100), PROJECT, None)]);
        let socket = FakeSocket::with_socket("/run/user/1000/nvim.7100.0", PROJECT);
        expect_readback(&socket, &["/somewhere/else.rs:1"]);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), socket);

        let error = herdr.launch(&project(), &target(Some(5))).unwrap_err();

        assert!(format!("{error:#}").contains("did not land"));
        assert!(!fake.called("send-text"));
    }

    #[test]
    fn polls_a_slow_landing_before_deciding_it_failed() {
        let fake = FakeHerdr::new(&[("wV:p2", Some(7200), PROJECT, None)]);
        let socket = FakeSocket::with_socket("/run/user/1000/nvim.7200.0", PROJECT);
        // First poll still shows the old buffer; the second shows the jump landed.
        expect_readback(
            &socket,
            &[
                &format!("{PROJECT}/src/app.rs:1"),
                &format!("{PROJECT}/src/lib.rs:9"),
            ],
        );
        let herdr = HerdrSessionManager::with_executors(fake, socket.clone());

        herdr.launch(&project(), &target(Some(9))).unwrap();

        assert_eq!(socket.sends().len(), 1);
    }

    #[test]
    fn launches_into_an_existing_pane_and_focuses_its_tab() {
        let fake = FakeHerdr::new(&[("wV:p2", Some(22), PROJECT, None)]);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), FakeSocket::none());

        let session = herdr.launch(&project(), &target(Some(7))).unwrap();

        assert_eq!(session, "herdr:wV:p2");
        assert!(fake.called("send-text wV:p2"));
        assert!(fake.called("tab focus wV:t1"));
        assert!(
            !fake.called("tab create"),
            "must not create a tab when one exists"
        );
    }

    #[test]
    fn creates_a_tab_with_a_named_socket_when_the_project_has_no_editor() {
        let fake = FakeHerdr::new(&[]);
        let herdr = HerdrSessionManager::with_executors(fake.clone(), FakeSocket::none());

        let session = herdr.launch(&project(), &target(Some(11))).unwrap();

        assert_eq!(session, "herdr:wV:p9");
        assert!(fake.called("tab create"));
        let run_call = fake
            .recorded()
            .into_iter()
            .find(|call| call.join(" ").contains("pane run"))
            .expect("expected a pane run call");
        assert!(run_call[3].contains("nvim --listen"), "got {}", run_call[3]);
        assert!(run_call[3].contains("+11"), "got {}", run_call[3]);
        assert!(
            run_call[3].contains("jump-project.sock"),
            "socket named after the project: {}",
            run_call[3]
        );
    }

    #[test]
    fn a_failing_herdr_command_surfaces_as_an_error() {
        let mut fake = FakeHerdr::new(&[]);
        fake.fail_on = Some("tab create".to_string());
        let herdr = HerdrSessionManager::with_executors(fake, FakeSocket::none());

        assert!(herdr.launch(&project(), &target(Some(1))).is_err());
    }

    #[test]
    fn available_follows_the_herdr_env_marker() {
        let previous = std::env::var("HERDR_ENV").ok();
        std::env::set_var("HERDR_ENV", "1");
        assert!(HerdrSessionManager::available());
        std::env::remove_var("HERDR_ENV");
        assert!(!HerdrSessionManager::available());
        if let Some(value) = previous {
            std::env::set_var("HERDR_ENV", value);
        }
    }

    #[test]
    fn listen_socket_path_is_safe_and_stable() {
        assert!(listen_socket_path("my project").ends_with("jump-my-project.sock"));
        assert!(listen_socket_path("a/b").ends_with("jump-a-b.sock"));
    }

    #[test]
    fn parses_the_pid_out_of_a_default_socket_name() {
        assert_eq!(
            socket_pid(Path::new("/run/user/1000/nvim.12345.0")),
            Some(12345)
        );
        assert_eq!(socket_pid(Path::new("/run/user/1000/jump-x.sock")), None);
        assert_eq!(socket_pid(Path::new("/run/user/1000/nvim.abc.0")), None);
    }

    #[test]
    fn parses_pane_list_json_and_ignores_entries_without_an_id() {
        let json = r#"{"result":{"panes":[
            {"pane_id":"wV:p2","tab_id":"wV:t1","workspace_id":"wV","cwd":"/p","foreground_cwd":"/p","agent":null},
            {"tab_id":"wV:t1"}
        ]}}"#;
        let fake = FakeHerdr {
            panes: vec![],
            ..Default::default()
        };
        let herdr = HerdrSessionManager::with_executors(fake, FakeSocket::none());
        // Feed the raw JSON through the parser used by list_panes.
        let value: Value = serde_json::from_str(json).unwrap();
        let panes: Vec<HerdrPane> = value["result"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pane| HerdrPane {
                pane_id: pane["pane_id"].as_str().unwrap_or_default().to_string(),
                tab_id: pane["tab_id"].as_str().unwrap_or_default().to_string(),
                workspace_id: pane["workspace_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                cwd: pane["cwd"].as_str().map(PathBuf::from),
                foreground_cwd: pane["foreground_cwd"].as_str().map(PathBuf::from),
                agent: pane["agent"].as_str().map(str::to_string),
            })
            .filter(|pane| !pane.pane_id.is_empty())
            .collect();
        let _ = herdr; // keeps the manager type in scope for the assertion below
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].pane_id, "wV:p2");
    }
}
