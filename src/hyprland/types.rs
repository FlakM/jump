use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyprlandWindow {
    pub address: String,
    pub pid: u32,
    pub class: String,
    pub title: String,
    pub workspace: Workspace,
    pub at: [i32; 2],
    pub size: [u32; 2],
}

impl HyprlandWindow {
    pub fn area(&self) -> u64 {
        self.size[0] as u64 * self.size[1] as u64
    }
}
