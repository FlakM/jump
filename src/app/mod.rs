use anyhow::{Context, Result};
use lsp_types::{
    request::{Initialize, Request},
    ClientCapabilities, DidOpenTextDocumentParams, InitializeParams, InitializedParams, Position,
    TextDocumentItem, WorkspaceFolder,
};
use std::path::PathBuf;
use tracing::info;

use crate::{
    DefinitionProvider, HoverOutput, HoverProvider, LspClient, LspConnection, RustSymbolExtractor,
    SymbolExtractor,
};

pub struct HoverRequest {
    pub root: PathBuf,
    pub file: PathBuf,
    pub line: u32,
    pub character: u32,
    pub server_path: String,
}

async fn initialize_lsp_client(
    client: &mut LspClient,
    root_uri: String,
    workspace_name: String,
) -> Result<()> {
    info!("Initializing LSP client");

    let init_params = InitializeParams {
        process_id: None,
        root_uri: None,
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
        line: request.line,
        character: request.character,
    };

    info!(
        "Requesting hover at line {}, character {}",
        request.line, request.character
    );
    let hover_result = client.hover(&file_uri, position).await?;

    info!("Requesting definition");
    let definition_result = client.definition(&file_uri, position).await?;

    let extractor = RustSymbolExtractor::new();
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
