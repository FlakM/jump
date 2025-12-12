use anyhow::{Context, Result};
use lsp_types::{
    request::{Initialize, Request},
    ClientCapabilities, DidOpenTextDocumentParams, InitializeParams, InitializedParams, Position,
    TextDocumentItem, WorkspaceFolder,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::{
    DefinitionProvider, EnvAndSocketLocator, FastProjectScanner, FilesystemMaterializer,
    GitHubPermalinkGenerator, HoverOutput, HoverProvider, JumpConfig, JumpLinkKind, JumpLinkParser,
    JumpRequest, LinkParser, LspClient, LspConnection, MarkdownFormatter, MaterializedPath,
    NeovimClient, NeovimInstanceLocator, PathMaterializer, PermalinkGenerator, ProjectRoot,
    ProjectRootLocator, ReferenceFormatter, RustSymbolExtractor, SessionInfo, SessionInventory,
    SessionProvisioner, SymbolExtractor, TmuxSessionManager,
};

pub struct HoverRequest {
    pub root: PathBuf,
    pub file: PathBuf,
    pub line: u32,
    pub character: u32,
    pub server_path: String,
}

pub struct CopyMarkdownRequest {
    pub root: PathBuf,
    pub file: PathBuf,
    pub line: u32,
    pub character: u32,
    pub server_path: String,
    pub use_github_link: bool,
    pub remote: String,
}

impl From<crate::cli::CopyMarkdownArgs> for CopyMarkdownRequest {
    fn from(args: crate::cli::CopyMarkdownArgs) -> Self {
        Self {
            root: args.root,
            file: args.file,
            line: args.line,
            character: args.character,
            server_path: args.server_path,
            use_github_link: args.github,
            remote: args.remote,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CopyMarkdownOutput {
    pub markdown: String,
}

pub struct ResolveLinkRequest {
    pub input: String,
    pub root: Option<PathBuf>,
    pub markers: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolveLinkOutput {
    pub request: JumpRequest,
    pub materialized: MaterializedPath,
    pub root: ProjectRoot,
}

pub struct JumpInput {
    pub link: String,
    pub markers: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JumpOutcome {
    pub status: String,
    pub session: String,
    pub file: String,
    pub line: Option<u32>,
}

async fn initialize_lsp_client(
    client: &mut LspClient,
    root_uri: String,
    workspace_name: String,
) -> Result<()> {
    info!("Initializing LSP client");

    let init_params = InitializeParams {
        process_id: None,
        capabilities: ClientCapabilities::default(),
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri.parse()?,
            name: workspace_name,
        }]),
        ..Default::default()
    };

    client
        .send_request(Initialize::METHOD, serde_json::to_value(init_params)?)
        .await?;

    client
        .send_notification("initialized", serde_json::to_value(InitializedParams {})?)
        .await?;

    Ok(())
}

async fn open_document(client: &mut LspClient, file_uri: String, text: String) -> Result<()> {
    info!("Opening document: {}", file_uri);

    let did_open_params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: file_uri.parse()?,
            language_id: "rust".to_string(),
            version: 1,
            text,
        },
    };

    client
        .send_notification(
            "textDocument/didOpen",
            serde_json::to_value(did_open_params)?,
        )
        .await?;

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    Ok(())
}

pub async fn run(request: HoverRequest) -> Result<HoverOutput> {
    let root = request.root.canonicalize()?;
    let file_path = if request.file.is_absolute() {
        request.file
    } else {
        root.join(&request.file)
    }
    .canonicalize()?;

    let text = tokio::fs::read_to_string(&file_path)
        .await
        .context("Failed to read file")?;

    let mut client = LspClient::new(&request.server_path).await?;

    let root_uri = format!("file://{}", root.display());
    let file_uri = format!("file://{}", file_path.display());
    let workspace_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    initialize_lsp_client(&mut client, root_uri, workspace_name).await?;
    open_document(&mut client, file_uri.clone(), text).await?;

    let position = Position {
        line: request.line.saturating_sub(1),
        character: request.character.saturating_sub(1),
    };

    info!(
        "Requesting hover at line {}, character {} (1-indexed)",
        request.line, request.character
    );
    let hover_result = client.hover(&file_uri, position).await?;

    info!("Requesting definition");
    let definition_result = client.definition(&file_uri, position).await?;

    let extractor = RustSymbolExtractor;
    let symbol_info = extractor.extract_symbol_info(&hover_result, &definition_result);
    let hover_text = extractor.extract_hover_text(&hover_result);

    let output = HoverOutput {
        symbol_info,
        hover_text,
    };

    info!("Shutting down LSP client");
    client.shutdown().await?;

    Ok(output)
}

pub async fn copy_markdown(request: CopyMarkdownRequest) -> Result<CopyMarkdownOutput> {
    let root = request.root.canonicalize()?;
    let file_path = if request.file.is_absolute() {
        request.file.clone()
    } else {
        root.join(&request.file)
    }
    .canonicalize()?;

    let text = tokio::fs::read_to_string(&file_path)
        .await
        .context("Failed to read file")?;

    let mut client = LspClient::new(&request.server_path).await?;

    let root_uri = format!("file://{}", root.display());
    let file_uri = format!("file://{}", file_path.display());
    let workspace_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    initialize_lsp_client(&mut client, root_uri, workspace_name).await?;
    open_document(&mut client, file_uri.clone(), text).await?;

    let position = Position {
        line: request.line.saturating_sub(1),
        character: request.character.saturating_sub(1),
    };

    info!(
        "Requesting hover and definition at line {}, character {} (1-indexed)",
        request.line, request.character
    );
    let hover_result = client.hover(&file_uri, position).await?;
    let definition_result = client.definition(&file_uri, position).await?;

    tracing::debug!("Hover result: {:?}", hover_result);
    tracing::debug!("Definition result: {:?}", definition_result);

    let extractor = RustSymbolExtractor;
    let symbol_info = extractor.extract_symbol_info(&hover_result, &definition_result);
    tracing::debug!("Symbol info: {:?}", symbol_info);

    if symbol_info.qualified_name.is_none() && symbol_info.definition_uri.is_none() {
        anyhow::bail!("No symbol found at cursor position");
    }

    let github_link = if request.use_github_link {
        if let Some(def_uri) = &symbol_info.definition_uri {
            let def_path = def_uri
                .strip_prefix("file://")
                .unwrap_or(def_uri)
                .parse::<PathBuf>()?;

            let generator = GitHubPermalinkGenerator::new(&def_path, Some(request.remote))?;
            let line = symbol_info.definition_line.unwrap_or(1);
            Some(generator.generate(&def_path, line, None)?)
        } else {
            None
        }
    } else {
        None
    };

    let formatter = MarkdownFormatter;
    let markdown = formatter.format_markdown(&symbol_info, github_link.as_ref())?;

    info!("Shutting down LSP client");
    client.shutdown().await?;

    Ok(CopyMarkdownOutput { markdown })
}

fn build_scanner(markers: &Option<Vec<String>>) -> FastProjectScanner {
    if let Some(list) = markers {
        FastProjectScanner::new(list.clone())
    } else {
        FastProjectScanner::with_defaults()
    }
}

fn find_project_by_name(
    repo_name: &str,
    search_paths: &[PathBuf],
    scanner: &FastProjectScanner,
    max_depth: usize,
) -> Option<ProjectRoot> {
    for search_path in search_paths {
        if !search_path.exists() {
            continue;
        }
        if let Ok(projects) = scanner.find_all_projects(search_path, max_depth) {
            for project in projects {
                if project.name.eq_ignore_ascii_case(repo_name) {
                    return Some(project);
                }
            }
        }
    }
    None
}

/// Determines the project root directory for a jump request.
///
/// Uses a priority-based search strategy to find the most appropriate project root:
///
/// 1. **Explicit root** (`requested_root`): If provided, scan upward for project markers.
///    Falls back to the path itself if no markers found.
///
/// 2. **GitHub repo name match**: For GitHub links, search configured paths
///    (`~/programming`, etc.) for a project whose name matches the repository.
///
/// 3. **Upward scan from start path**: Walk up from a starting directory looking for
///    project markers (.git, Cargo.toml, package.json, etc.):
///    - Absolute paths: start from the file's parent directory
///    - Relative/GitHub: start from current working directory
///
/// 4. **Fallback for absolute paths**: Use the file's parent directory as root.
///
/// # Arguments
///
/// * `jump_req` - The parsed link request containing path and link kind
/// * `requested_root` - Optional explicit root directory override
/// * `markers` - Optional custom project marker files (defaults: .git, Cargo.toml, etc.)
///
/// # Errors
///
/// Returns an error if no project root can be determined for relative or GitHub links.
/// Absolute paths always succeed (worst case: use parent directory).
fn locate_root(
    jump_req: &JumpRequest,
    requested_root: &Option<PathBuf>,
    markers: &Option<Vec<String>>,
) -> Result<ProjectRoot> {
    let scanner = build_scanner(markers);
    let config = JumpConfig::default();

    if let Some(root) = requested_root {
        let root_path = root
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize root {:?}", root))?;

        if let Some(found) = scanner.find_root_from(&root_path)? {
            return Ok(found);
        }

        return Ok(ProjectRoot::new(root_path, "manual".to_string()));
    }

    // For GitHub links, search for project by repo name
    if jump_req.kind == JumpLinkKind::Github {
        if let Some(repo_name) = &jump_req.repo_name {
            if let Some(found) =
                find_project_by_name(repo_name, &config.search_paths, &scanner, config.max_depth)
            {
                return Ok(found);
            }
        }
    }

    let start_path: PathBuf = match jump_req.kind {
        JumpLinkKind::Absolute => jump_req
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/")),
        JumpLinkKind::Github => {
            // GitHub link but couldn't find by repo name, try cwd as last resort
            std::env::current_dir().context("Failed to get current directory")?
        }
        JumpLinkKind::Relative => {
            std::env::current_dir().context("Failed to get current directory")?
        }
    };

    if let Some(found) = scanner.find_root_from(&start_path)? {
        return Ok(found);
    }

    if jump_req.kind == JumpLinkKind::Absolute {
        let fallback = jump_req
            .path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf();
        return Ok(ProjectRoot::new(fallback, "manual".to_string()));
    }

    anyhow::bail!(
        "Unable to locate project root from {:?} with markers {:?}",
        start_path,
        markers
    );
}

pub fn resolve_link(request: ResolveLinkRequest) -> Result<ResolveLinkOutput> {
    let parser = LinkParser;
    let jump_req = parser
        .parse(&request.input)
        .context("Failed to parse link text")?;

    let root = locate_root(&jump_req, &request.root, &request.markers)?;

    let materializer = FilesystemMaterializer;
    let materialized = materializer.materialize(&root, &jump_req)?;

    Ok(ResolveLinkOutput {
        request: jump_req,
        materialized,
        root,
    })
}

fn ensure_session(
    inventory: &impl SessionInventory,
    provisioner: &impl SessionProvisioner,
    root: &ProjectRoot,
    target: &MaterializedPath,
) -> Result<SessionInfo> {
    if let Some(existing) = inventory.find_repo_session(root)? {
        return Ok(existing);
    }

    // Use project name as session name when creating
    provisioner.spawn(&root.name, &root.path, target)
}

fn build_outcome(session_name: String, target: &MaterializedPath) -> JumpOutcome {
    JumpOutcome {
        status: "success".to_string(),
        session: session_name,
        file: target.absolute.to_string_lossy().to_string(),
        line: target.line,
    }
}

fn open_in_nvim(
    session_name: &str,
    root: &ProjectRoot,
    target: &MaterializedPath,
    socket_path: PathBuf,
) -> Result<()> {
    let nvim_instance = crate::NvimInstance {
        address: socket_path,
        session_name: Some(session_name.to_string()),
        cwd: Some(root.path.clone()),
    };
    let client = crate::NvrClient::new();
    client.open(&nvim_instance, target)
}

fn focus_and_switch_kitty(tmux: &TmuxSessionManager, session_name: &str) -> Option<u32> {
    let largest_kitty = crate::hyprland::find_largest_kitty(1).ok()??;
    let _ = crate::hyprland::focus_window(largest_kitty.pid);
    let _ = tmux.switch_client_in_kitty(largest_kitty.pid, session_name);
    Some(largest_kitty.pid)
}

/// Attempts to open the file in an existing nvim pane within the session.
/// Returns `Some(JumpOutcome)` if successful, `None` if no usable nvim found.
fn try_open_via_nvim_pane(
    tmux: &TmuxSessionManager,
    session_name: &str,
    root: &ProjectRoot,
    target: &MaterializedPath,
) -> Result<Option<JumpOutcome>> {
    let nvim_pane = match tmux.find_nvim_pane(session_name)? {
        Some(pane) => pane,
        None => return Ok(None),
    };

    let socket_path = nvim_pane.socket_path();
    if !socket_path.exists() {
        return Ok(None);
    }

    info!(
        "Found nvim in pane: {:?}, socket: {:?}",
        nvim_pane, socket_path
    );

    open_in_nvim(session_name, root, target, socket_path)?;
    focus_and_switch_kitty(tmux, session_name);

    if let Err(e) = tmux.select_pane(&nvim_pane) {
        info!("Failed to select pane: {}", e);
    }

    Ok(Some(build_outcome(session_name.to_string(), target)))
}

/// Finds a session matching the project using multiple strategies.
fn find_matching_tmux_session(
    tmux: &TmuxSessionManager,
    jump_req: &JumpRequest,
    root: &ProjectRoot,
) -> Option<SessionInfo> {
    // For GitHub links, try repo name first
    if jump_req.kind == JumpLinkKind::Github {
        if let Some(repo_name) = &jump_req.repo_name {
            if let Ok(Some(session)) = tmux.find_session_by_name(repo_name) {
                info!(
                    "Found existing session '{}' matching repo name '{}'",
                    session.name, repo_name
                );
                return Some(session);
            }
        }
    }

    // Try path-based matching
    if let Ok(Some(session)) = tmux.find_repo_session(root) {
        return Some(session);
    }

    // Check if any parent directory name matches a session
    if let Some(session) = root.path.ancestors().skip(1).find_map(|ancestor| {
        ancestor
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|name| tmux.find_session_by_name(name).ok().flatten())
    }) {
        return Some(session);
    }

    // Try project name
    tmux.find_session_by_name(&root.name).ok().flatten()
}

/// Opens file in a session, creating nvim window if needed.
fn open_in_session_with_fallback(
    tmux: &TmuxSessionManager,
    session_name: &str,
    root: &ProjectRoot,
    target: &MaterializedPath,
) -> Result<JumpOutcome> {
    // Try existing nvim first
    if let Some(outcome) = try_open_via_nvim_pane(tmux, session_name, root, target)? {
        return Ok(outcome);
    }

    // No nvim found - open new window
    info!(
        "No nvim found in session '{}', opening new window",
        session_name
    );
    tmux.open_nvim_in_session(session_name, &root.path, target)?;
    focus_and_switch_kitty(tmux, session_name);
    let _ = tmux.activate_session(session_name);

    Ok(build_outcome(session_name.to_string(), target))
}

/// Opens a file reference in neovim via tmux session management.
///
/// Resolves the input link, finds or creates an appropriate tmux session,
/// and opens the file in neovim. Integrates with Hyprland for window focus.
///
/// # Session Selection Strategy
///
/// 1. Find existing session matching the project (by repo name, path, or parent directory)
/// 2. Create a new session for the project
pub async fn jump(input: JumpInput) -> Result<JumpOutcome> {
    let parser = LinkParser;
    let jump_req = parser
        .parse(&input.link)
        .context("Failed to parse link text")?;

    let root = locate_root(&jump_req, &None, &input.markers)?;

    let materializer = FilesystemMaterializer;
    let target = materializer.materialize(&root, &jump_req)?;

    let tmux = TmuxSessionManager::new();

    // Strategy 1: Find a session matching the project
    if let Some(session) = find_matching_tmux_session(&tmux, &jump_req, &root) {
        info!(
            "Found existing session '{}' for project '{}'",
            session.name,
            root.path.display()
        );
        return open_in_session_with_fallback(&tmux, &session.name, &root, &target);
    }

    // Strategy 2: Create or find session via ensure_session
    let session = ensure_session(&tmux, &tmux, &root, &target)?;

    let locator = EnvAndSocketLocator::with_default_tmp();
    if let Some(nvim) = locator.locate(&session, &root)? {
        let nvr_client = crate::NvrClient::new();
        nvr_client.open(&nvim, &target)?;
    } else {
        tmux.open_nvim_in_session(&session.name, &root.path, &target)?;
    }

    let _ = tmux.activate_session(&session.name);

    Ok(build_outcome(session.name, &target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_project() -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir_all(project_dir.join("src")).unwrap();
        fs::write(project_dir.join(".git"), "").unwrap();
        (temp, project_dir)
    }

    #[test]
    fn resolves_relative_link_with_detected_root() {
        let (_temp, project_dir) = create_project();
        let file = project_dir.join("src/lib.rs");
        fs::write(&file, "// test").unwrap();

        let prev_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project_dir).unwrap();

        let output = resolve_link(ResolveLinkRequest {
            input: "src/lib.rs:5".to_string(),
            root: None,
            markers: None,
        })
        .expect("link should resolve");

        std::env::set_current_dir(prev_dir).unwrap();

        assert_eq!(output.root.path, project_dir.canonicalize().unwrap());
        assert_eq!(output.request.kind, JumpLinkKind::Relative);
        assert_eq!(output.materialized.line, Some(5));
        assert_eq!(output.materialized.absolute, file.canonicalize().unwrap());
    }

    #[test]
    fn resolves_github_link_with_explicit_root() {
        let (_temp, project_dir) = create_project();
        let file = project_dir.join("src/main.rs");
        fs::write(&file, "// test").unwrap();

        let output = resolve_link(ResolveLinkRequest {
            input: "https://github.com/example/repo/blob/main/src/main.rs#L10".to_string(),
            root: Some(project_dir.clone()),
            markers: None,
        })
        .expect("link should resolve");

        assert_eq!(output.request.kind, JumpLinkKind::Github);
        assert_eq!(output.materialized.absolute, file.canonicalize().unwrap());
        assert_eq!(output.materialized.line, Some(10));
        assert_eq!(output.materialized.end_line, None);
        assert_eq!(output.materialized.kind, JumpLinkKind::Github);
    }

    // Integration with real tmux/nvr is intentionally skipped in unit tests.
}
