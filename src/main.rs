use anyhow::Result;
use clap::Parser;
use jump::PermalinkGenerator;
use notify_rust::{Notification, Timeout, Urgency};
use std::path::PathBuf;
use tracing_subscriber::prelude::*;

use jump::cli::{Args, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging();

    let args = Args::parse();

    // Handle subcommands first
    if let Some(command) = args.command {
        match command {
            Commands::GithubLink {
                file,
                start_line,
                end_line,
                remote,
            } => {
                let file = file.canonicalize()?;
                let generator = jump::GitHubPermalinkGenerator::new(&file, Some(remote))?;
                let link = generator.generate(&file, start_line, end_line)?;
                println!("{}", serde_json::to_string_pretty(&link)?);
            }

            Commands::CopyMarkdown(args) => {
                use std::io::IsTerminal;
                let output = jump::copy_markdown(args.into()).await?;
                if std::io::stdout().is_terminal() {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    println!("{}", output.markdown);
                }
            }

            Commands::FormatSymbol(args) => {
                use std::io::IsTerminal;
                let output = jump::format_symbol(args.into())?;
                if std::io::stdout().is_terminal() {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    println!("{}", output.markdown);
                }
            }

            Commands::Verify => {
                return verify_system();
            }

            Commands::Completions { shell } => {
                use clap::CommandFactory;
                clap_complete::generate(
                    shell,
                    &mut Args::command(),
                    "jump",
                    &mut std::io::stdout(),
                );
                return Ok(());
            }
        }
        return Ok(());
    }

    // Default action: jump to link
    let link = args
        .link
        .ok_or_else(|| anyhow::anyhow!("No link provided. Usage: jump <link> or jump --help"))?;

    log_diagnostic_context();
    tracing::info!("Jump command: {}", link);

    let markers_opt = if args.markers.is_empty() {
        None
    } else {
        Some(args.markers)
    };

    match jump::jump(jump::JumpInput {
        link: link.clone(),
        markers: markers_opt,
    })
    .await
    {
        Ok(outcome) => {
            let file_name = PathBuf::from(&outcome.file)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| outcome.file.clone());
            let line_info = outcome.line.map(|l| format!(":{}", l)).unwrap_or_default();
            notify("Jump", &format!("{}{}", file_name, line_info), true);
            tracing::info!("Jump success: {:?}", outcome);
            println!("{}", serde_json::to_string_pretty(&outcome)?);
        }
        Err(e) => {
            notify("Jump failed", &format!("{}", e), false);
            tracing::error!("Jump failed for '{}': {}", link, e);
            return Err(e);
        }
    }

    Ok(())
}

fn log_diagnostic_context() {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));

    // Parse hyprland active window - extract only class, pid, workspace
    let active_window = std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .map(|v| {
            format!(
                "{}(pid={}, ws={})",
                v["class"].as_str().unwrap_or("?"),
                v["pid"].as_u64().unwrap_or(0),
                v["workspace"]["id"].as_i64().unwrap_or(0)
            )
        });

    let kitty_pids: Vec<_> = jump::list_clients()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.class == "kitty")
        .map(|w| format!("{}@ws{}", w.pid, w.workspace.id))
        .collect();

    let tmux_sessions: Vec<_> = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let nvim_panes: Vec<_> = std::process::Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Get pane commands to filter nvim
            let commands = std::process::Command::new("tmux")
                .args(["list-panes", "-a", "-F", "#{pane_current_command}"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();

            stdout
                .lines()
                .zip(commands.lines())
                .filter(|(_, cmd)| cmd.contains("nvim"))
                .map(|(pane, _)| pane.to_string())
                .collect()
        })
        .unwrap_or_default();

    tracing::info!(
        cwd = ?cwd,
        active = ?active_window,
        kitty = ?kitty_pids,
        sessions = ?tmux_sessions,
        nvim = ?nvim_panes,
        "context"
    );
}

fn verify_system() -> Result<()> {
    use std::process::Command;

    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const BOLD: &str = "\x1b[1m";
    const DIM: &str = "\x1b[2m";
    const RESET: &str = "\x1b[0m";

    struct Check {
        name: &'static str,
        cmd: &'static str,
        args: &'static [&'static str],
        required: bool,
        check_exists_only: bool,
    }

    let checks = [
        Check {
            name: "tmux",
            cmd: "tmux",
            args: &["-V"],
            required: true,
            check_exists_only: false,
        },
        Check {
            name: "nvim",
            cmd: "nvim",
            args: &["--version"],
            required: true,
            check_exists_only: false,
        },
        Check {
            name: "nvr",
            cmd: "nvr",
            args: &["--help"],
            required: true,
            check_exists_only: true,
        },
        Check {
            name: "git",
            cmd: "git",
            args: &["--version"],
            required: true,
            check_exists_only: false,
        },
        Check {
            name: "hyprctl",
            cmd: "hyprctl",
            args: &["version"],
            required: false,
            check_exists_only: false,
        },
        Check {
            name: "pstree",
            cmd: "pstree",
            args: &["-p", "1"],
            required: true,
            check_exists_only: true,
        },
    ];

    let mut all_ok = true;
    let mut results = Vec::new();

    for check in &checks {
        let result = Command::new(check.cmd).args(check.args).output();

        let (status, version, ok) = match result {
            Ok(output) if output.status.success() => {
                let ver = if check.check_exists_only {
                    "ok".to_string()
                } else {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                (format!("{}✓{}", GREEN, RESET), ver, true)
            }
            Ok(output) => {
                // Some tools output to stderr or exit non-zero but still work
                if check.check_exists_only {
                    (format!("{}✓{}", GREEN, RESET), "ok".to_string(), true)
                } else {
                    // Try stderr if stdout is empty
                    let ver = String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !ver.is_empty() {
                        (format!("{}✓{}", GREEN, RESET), ver, true)
                    } else {
                        if check.required {
                            all_ok = false;
                        }
                        (
                            format!("{}✗{}", RED, RESET),
                            "installed but failed".to_string(),
                            false,
                        )
                    }
                }
            }
            Err(_) => {
                if check.required {
                    all_ok = false;
                }
                (format!("{}✗{}", RED, RESET), "not found".to_string(), false)
            }
        };
        let _ = ok;

        let req = if check.required {
            "required"
        } else {
            "optional"
        };
        results.push((check.name, status, version, req));
    }

    // Check tmux server
    let tmux_server = Command::new("tmux").args(["list-sessions"]).output();
    let tmux_status = match tmux_server {
        Ok(o) if o.status.success() => {
            let count = String::from_utf8_lossy(&o.stdout).lines().count();
            format!("{}✓{} {} sessions", GREEN, RESET, count)
        }
        _ => format!("{}✗{} not running", RED, RESET),
    };

    // Check hyprland
    let hyprland_status = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| format!("{}✓{} active", GREEN, RESET))
        .unwrap_or_else(|| format!("{}-{} not running", DIM, RESET));

    println!("{}Jump System Verification{}", BOLD, RESET);
    println!("========================\n");

    println!("{}Tools:{}", BOLD, RESET);
    for (name, status, version, req) in &results {
        let version_display = if version.len() > 50 {
            format!("{}...", &version[..47])
        } else {
            version.clone()
        };
        let req_display = if *req == "optional" {
            format!("{}({}){}", DIM, req, RESET)
        } else {
            format!("({})", req)
        };
        println!(
            "  {} {:10} {} {}{}{}",
            status, name, req_display, DIM, version_display, RESET
        );
    }

    println!("\n{}Services:{}", BOLD, RESET);
    println!("  tmux server:  {}", tmux_status);
    println!("  hyprland:     {}", hyprland_status);

    // Log file location
    let log_path = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("jump/jump.log");
    let log_exists = log_path.exists();
    println!("\n{}Log file:{}", BOLD, RESET);
    println!(
        "  {} {}{}{}",
        if log_exists {
            format!("{}✓{}", GREEN, RESET)
        } else {
            format!("{}-{}", DIM, RESET)
        },
        DIM,
        log_path.display(),
        RESET
    );

    println!();
    if all_ok {
        println!(
            "{}{}All required tools are available.{}",
            BOLD, GREEN, RESET
        );
        Ok(())
    } else {
        println!("{}{}Some required tools are missing.{}", BOLD, RED, RESET);
        anyhow::bail!("Some required tools are missing.")
    }
}

fn notify(title: &str, message: &str, success: bool) {
    let urgency = if success {
        Urgency::Normal
    } else {
        Urgency::Critical
    };
    let icon = if success {
        "dialog-information"
    } else {
        "dialog-error"
    };
    let _ = Notification::new()
        .summary(title)
        .body(message)
        .icon(icon)
        .urgency(urgency)
        .timeout(Timeout::Milliseconds(3000))
        .show();
}

fn setup_logging() {
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("jump");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join("jump.log");

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if let Ok(file) = file {
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(std::io::stderr)
            .init();
    }
}
