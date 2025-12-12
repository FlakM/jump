use anyhow::{Context, Result};
use std::process::Command;

use super::types::HyprlandWindow;

pub fn list_clients() -> Result<Vec<HyprlandWindow>> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .context("Failed to execute hyprctl clients")?;

    if !output.status.success() {
        anyhow::bail!(
            "hyprctl clients failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let windows: Vec<HyprlandWindow> =
        serde_json::from_slice(&output.stdout).context("Failed to parse hyprctl output")?;

    Ok(windows)
}

pub fn find_largest_kitty(workspace: i32) -> Result<Option<HyprlandWindow>> {
    let windows = list_clients()?;

    let largest = windows
        .into_iter()
        .filter(|w| w.class == "kitty" && w.workspace.id == workspace)
        .max_by_key(|w| w.area());

    Ok(largest)
}

pub fn focus_window(pid: u32) -> Result<()> {
    let output = Command::new("hyprctl")
        .args(["dispatch", "focuswindow", &format!("pid:{}", pid)])
        .output()
        .context("Failed to execute hyprctl dispatch")?;

    if !output.status.success() {
        anyhow::bail!(
            "hyprctl dispatch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub fn list_kitty_windows() -> Result<Vec<HyprlandWindow>> {
    let windows = list_clients()?;
    Ok(windows.into_iter().filter(|w| w.class == "kitty").collect())
}

pub fn find_largest_kitty_any_workspace() -> Result<Option<HyprlandWindow>> {
    let windows = list_clients()?;

    let largest = windows
        .into_iter()
        .filter(|w| w.class == "kitty")
        .max_by_key(|w| w.area());

    Ok(largest)
}
