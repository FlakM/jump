use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NvimInstance {
    pub address: PathBuf,
    pub session_name: Option<String>,
    pub cwd: Option<PathBuf>,
}
