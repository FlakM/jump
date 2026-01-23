#![allow(async_fn_in_trait)]

use anyhow::{Context, Result};
use lsp_types::Position;
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{debug, trace};

pub trait LspConnection {
    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value>;
    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
}

pub trait HoverProvider {
    async fn hover(&mut self, file_uri: &str, position: Position) -> Result<Value>;
}

pub trait DefinitionProvider {
    async fn definition(&mut self, file_uri: &str, position: Position) -> Result<Value>;
}

pub const DEFAULT_LSP_TIMEOUT_MS: u64 = 30000;

pub struct LspClient {
    child: Child,
    next_id: i32,
    timeout: Duration,
}

fn server_needs_stdio_flag(server_path: &str) -> bool {
    let server_name = std::path::Path::new(server_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(server_path);

    matches!(
        server_name,
        "typescript-language-server" | "pyright-langserver" | "pyright"
    )
}

impl LspClient {
    pub async fn new(server_path: &str) -> Result<Self> {
        Self::with_timeout(server_path, Duration::from_millis(DEFAULT_LSP_TIMEOUT_MS)).await
    }

    pub async fn with_timeout(server_path: &str, timeout: Duration) -> Result<Self> {
        let mut cmd = Command::new("lspmux");
        cmd.arg("client").arg("--server-path").arg(server_path);

        if server_needs_stdio_flag(server_path) {
            cmd.arg("--").arg("--stdio");
        }

        let child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn lspmux client")?;

        Ok(Self {
            child,
            next_id: 1,
            timeout,
        })
    }

    async fn send_message(&mut self, message: &Value) -> Result<()> {
        let body = serde_json::to_string(message)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        let stdin = self.child.stdin.as_mut().context("Failed to get stdin")?;

        stdin.write_all(header.as_bytes()).await?;
        stdin.write_all(body.as_bytes()).await?;
        stdin.flush().await?;

        Ok(())
    }

    async fn read_message(&mut self) -> Result<Value> {
        let stdout = self.child.stdout.as_mut().context("Failed to get stdout")?;
        let mut reader = BufReader::new(stdout);

        let mut content_length = 0;
        let mut line = String::new();

        loop {
            line.clear();
            reader.read_line(&mut line).await?;
            let trimmed = line.trim();

            if trimmed.is_empty() {
                break;
            }

            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = value.trim().parse()?;
            }
        }

        let mut buffer = vec![0u8; content_length];
        tokio::io::AsyncReadExt::read_exact(&mut reader, &mut buffer).await?;

        let message: Value = serde_json::from_slice(&buffer)?;
        Ok(message)
    }

    async fn read_message_with_timeout(&mut self) -> Result<Value> {
        tokio::time::timeout(self.timeout, self.read_message())
            .await
            .map_err(|_| anyhow::anyhow!("LSP read timeout after {:?}", self.timeout))?
    }

    async fn wait_for_response(&mut self, expected_id: i32) -> Result<Value> {
        loop {
            let message = self.read_message_with_timeout().await?;

            if message.get("method").is_some() && message.get("id").is_none() {
                debug!("Received notification: {}", message.get("method").unwrap());
                continue;
            }

            if let Some(id) = message.get("id") {
                if id.as_i64() == Some(expected_id as i64) {
                    if let Some(error) = message.get("error") {
                        anyhow::bail!("LSP error: {}", error);
                    }
                    return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                } else {
                    trace!("Received response for different request: id={}", id);
                }
            }
        }
    }
}

impl LspConnection for LspClient {
    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        self.send_message(&message).await?;
        self.wait_for_response(id).await
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        self.send_message(&message).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.send_request("shutdown", Value::Null).await?;
        self.send_notification("exit", Value::Null).await?;
        Ok(())
    }
}

impl HoverProvider for LspClient {
    async fn hover(&mut self, file_uri: &str, position: Position) -> Result<Value> {
        use lsp_types::{
            request::HoverRequest, request::Request, HoverParams, TextDocumentIdentifier,
            TextDocumentPositionParams,
        };

        let hover_params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: file_uri.parse()?,
                },
                position,
            },
            work_done_progress_params: Default::default(),
        };

        self.send_request(HoverRequest::METHOD, serde_json::to_value(hover_params)?)
            .await
    }
}

impl DefinitionProvider for LspClient {
    async fn definition(&mut self, file_uri: &str, position: Position) -> Result<Value> {
        use lsp_types::{GotoDefinitionParams, TextDocumentIdentifier, TextDocumentPositionParams};

        let definition_params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: file_uri.parse()?,
                },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        self.send_request(
            "textDocument/definition",
            serde_json::to_value(definition_params)?,
        )
        .await
    }
}
