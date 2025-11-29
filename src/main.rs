use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "jump")]
#[command(about = "LSP-based symbol information tool")]
struct Args {
    #[arg(long, help = "Workspace root directory")]
    root: PathBuf,

    #[arg(long, help = "Path to file (relative or absolute)")]
    file: PathBuf,

    #[arg(long, help = "0-based line for hover request")]
    line: u32,

    #[arg(long, help = "0-based character for hover request")]
    character: u32,

    #[arg(
        long,
        default_value = "rust-analyzer",
        help = "Language server executable passed to lspmux"
    )]
    server_path: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    let request = jump::HoverRequest {
        root: args.root,
        file: args.file,
        line: args.line,
        character: args.character,
        server_path: args.server_path,
    };

    let output = jump::run(request).await?;

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}
