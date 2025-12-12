pub mod client;
pub mod types;

pub use client::{
    find_largest_kitty, find_largest_kitty_any_workspace, focus_window, list_clients,
    list_kitty_windows,
};
pub use types::{HyprlandWindow, Workspace};
