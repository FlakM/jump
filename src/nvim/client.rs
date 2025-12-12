use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::materializer::MaterializedPath;

use super::types::NvimInstance;

/// Executes nvr (neovim-remote) commands.
pub trait NvrCommandExecutor {
    fn run(&self, args: &[&str]) -> Result<()>;
}

/// Default executor that shells out to the nvr binary.
#[derive(Clone, Default)]
pub struct DefaultNvrExecutor;

impl NvrCommandExecutor for DefaultNvrExecutor {
    fn run(&self, args: &[&str]) -> Result<()> {
        let status = Command::new("nvr")
            .args(args)
            .status()
            .context("Failed to execute nvr command")?;

        if !status.success() {
            bail!("nvr command failed with exit code: {}", status);
        }

        Ok(())
    }
}

/// Client for interacting with Neovim instances.
pub trait NeovimClient {
    fn open(&self, instance: &NvimInstance, target: &MaterializedPath) -> Result<()>;
}

/// Neovim client using nvr (neovim-remote) for communication.
pub struct NvrClient<E = DefaultNvrExecutor> {
    executor: E,
}

impl NvrClient {
    pub fn new() -> Self {
        Self {
            executor: DefaultNvrExecutor,
        }
    }
}

impl Default for NvrClient {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: NvrCommandExecutor> NvrClient<E> {
    pub fn with_executor(executor: E) -> Self {
        Self { executor }
    }
}

impl<E: NvrCommandExecutor> NeovimClient for NvrClient<E> {
    fn open(&self, instance: &NvimInstance, target: &MaterializedPath) -> Result<()> {
        let address = instance.address.to_string_lossy();
        let line = target.line.unwrap_or(1);
        let path = target.absolute.to_string_lossy();
        let cmd = format!("<Esc>:edit +{} {}<CR>", line, path);

        self.executor
            .run(&["--servername", &address, "--remote-send", &cmd])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct TestExecutor {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        fail: bool,
    }

    impl NvrCommandExecutor for TestExecutor {
        fn run(&self, args: &[&str]) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());

            if self.fail {
                bail!("forced fail");
            }

            Ok(())
        }
    }

    fn materialized(tmp: &Path) -> MaterializedPath {
        MaterializedPath {
            absolute: tmp.to_path_buf(),
            relative: None,
            line: Some(5),
            end_line: None,
            kind: crate::parser::JumpLinkKind::Absolute,
            revision: None,
        }
    }

    #[test]
    fn sends_edit_command_to_nvr() {
        let exec = TestExecutor::default();
        let client = NvrClient::with_executor(exec.clone());

        let temp = tempfile::NamedTempFile::new().unwrap();
        let instance = NvimInstance {
            address: "/tmp/nvim1234/0".into(),
            session_name: None,
            cwd: None,
        };

        client
            .open(&instance, &materialized(temp.path()))
            .expect("should succeed");

        let calls = exec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "--servername");
        assert_eq!(calls[0][1], "/tmp/nvim1234/0");
        assert_eq!(calls[0][2], "--remote-send");
        assert!(calls[0][3].contains(":edit +5"));
    }
}
