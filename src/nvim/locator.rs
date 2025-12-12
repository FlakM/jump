use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use crate::project::ProjectRoot;
use crate::tmux::SessionInfo;

use super::types::NvimInstance;

const DEFAULT_SOCKET_DIR: &str = "/tmp";

pub trait NeovimInstanceLocator {
    fn locate(&self, session: &SessionInfo, root: &ProjectRoot) -> Result<Option<NvimInstance>>;
}

pub struct EnvAndSocketLocator {
    search_dirs: Vec<PathBuf>,
}

impl EnvAndSocketLocator {
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self { search_dirs }
    }

    pub fn with_default_tmp() -> Self {
        Self::new(vec![PathBuf::from(DEFAULT_SOCKET_DIR)])
    }

    fn from_env(session: &SessionInfo) -> Option<NvimInstance> {
        let addr = env::var("NVIM_LISTEN_ADDRESS").ok()?;
        let path = PathBuf::from(addr.clone());
        if path.exists() {
            return Some(NvimInstance {
                address: path,
                session_name: Some(session.name.clone()),
                cwd: None,
            });
        }
        None
    }

    fn collect_socket_dirs(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        for dir in &self.search_dirs {
            if !dir.is_dir() {
                continue;
            }

            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with("nvim") {
                                candidates.push(path);
                            }
                        }
                    }
                }
            }
        }
        candidates
    }

    fn select_socket_path(&self, session: &SessionInfo, root: &ProjectRoot) -> Option<PathBuf> {
        for dir in self.collect_socket_dirs() {
            let socket = dir.join("0");
            if !socket.exists() {
                continue;
            }

            let dir_str = dir.to_string_lossy();
            if dir_str.contains(&session.name) || dir_str.contains(&root.name) {
                return Some(socket);
            }
        }

        // Fallback to first socket if nothing matched heuristics
        self.collect_socket_dirs()
            .into_iter()
            .map(|d| d.join("0"))
            .find(|p| p.exists())
    }
}

impl NeovimInstanceLocator for EnvAndSocketLocator {
    fn locate(&self, session: &SessionInfo, root: &ProjectRoot) -> Result<Option<NvimInstance>> {
        if let Some(inst) = Self::from_env(session) {
            return Ok(Some(inst));
        }

        let socket = self.select_socket_path(session, root);
        Ok(socket.map(|address| NvimInstance {
            address,
            session_name: Some(session.name.clone()),
            cwd: Some(root.path.clone()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Mutex to ensure tests that touch NVIM_LISTEN_ADDRESS run serially
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn project(temp: &TempDir) -> ProjectRoot {
        ProjectRoot::new(temp.path().to_path_buf(), ".git".to_string())
    }

    #[test]
    fn prefers_env_var_socket() {
        let _lock = ENV_LOCK.lock().unwrap();

        let temp = TempDir::new().unwrap();
        let env_socket = temp.path().join("env.sock");
        fs::write(&env_socket, "").unwrap();

        std::env::set_var("NVIM_LISTEN_ADDRESS", &env_socket);

        let locator = EnvAndSocketLocator::with_default_tmp();
        let session = SessionInfo {
            name: "dev".to_string(),
            path: temp.path().to_path_buf(),
        };

        let inst = locator
            .locate(&session, &project(&temp))
            .expect("should locate")
            .unwrap();

        assert_eq!(inst.address, env_socket);
        std::env::remove_var("NVIM_LISTEN_ADDRESS");
    }

    #[test]
    fn finds_socket_in_custom_dir() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NVIM_LISTEN_ADDRESS");

        let temp = TempDir::new().unwrap();
        let socket_dir = temp.path().join("nvim1234");
        fs::create_dir_all(&socket_dir).unwrap();
        let socket = socket_dir.join("0");
        fs::write(&socket, "").unwrap();

        let locator = EnvAndSocketLocator::new(vec![temp.path().to_path_buf()]);
        let session = SessionInfo {
            name: "dev".to_string(),
            path: temp.path().to_path_buf(),
        };

        let inst = locator
            .locate(&session, &project(&temp))
            .expect("should locate")
            .unwrap();

        assert_eq!(inst.address, socket);
        assert_eq!(inst.session_name.unwrap(), "dev");
    }
}
