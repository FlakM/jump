use clap::builder::ValueHint;
use clap::{Args as ClapArgs, Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "jump")]
#[command(about = "Navigate code references - open files in neovim via tmux")]
pub struct Args {
    /// Link text or URL to resolve and open (default action)
    #[arg(value_name = "LINK", value_hint = ValueHint::Other)]
    pub link: Option<String>,

    /// Custom marker files (comma-separated)
    #[arg(
        long,
        value_delimiter = ',',
        num_args = 0..,
        value_hint = ValueHint::Other
    )]
    pub markers: Vec<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Generate a GitHub permalink for a file and line range
    GithubLink {
        /// File to generate link for
        #[arg(long, value_hint = ValueHint::FilePath)]
        file: PathBuf,

        /// Start line number (1-indexed)
        #[arg(long)]
        start_line: u32,

        /// End line number (1-indexed, optional)
        #[arg(long)]
        end_line: Option<u32>,

        /// Git remote name
        #[arg(long, default_value = "origin")]
        remote: String,
    },

    /// Generate markdown reference for symbol at cursor position
    CopyMarkdown(CopyMarkdownArgs),

    /// Format symbol from pre-computed LSP hover/definition JSON (fast path via neovim)
    FormatSymbol(FormatSymbolArgs),

    /// Verify system setup (check required tools are installed)
    Verify,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(ClapArgs, Debug)]
pub struct CopyMarkdownArgs {
    /// Workspace root directory
    #[arg(long, value_hint = ValueHint::DirPath)]
    pub root: PathBuf,

    /// Path to file (relative or absolute)
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub file: PathBuf,

    /// 1-based line number
    #[arg(long)]
    pub line: u32,

    /// 1-based column number
    #[arg(long)]
    pub character: u32,

    /// Language server executable passed to lspmux (auto-detected from file extension if not specified)
    #[arg(long, value_hint = ValueHint::ExecutablePath)]
    pub server_path: Option<String>,

    /// Use GitHub permalink instead of local file URI
    #[arg(long)]
    pub github: bool,

    /// Git remote name
    #[arg(long, default_value = "origin")]
    pub remote: String,

    /// Initial delay after opening document before LSP requests (ms)
    #[arg(long, default_value = "300")]
    pub lsp_init_delay_ms: u64,

    /// Maximum retries for LSP requests when result is empty
    #[arg(long, default_value = "5")]
    pub lsp_max_retries: u32,

    /// Timeout for LSP read operations (ms)
    #[arg(long, default_value = "30000")]
    pub lsp_timeout_ms: u64,
}

#[derive(ClapArgs, Debug)]
pub struct FormatSymbolArgs {
    /// Workspace root directory
    #[arg(long, value_hint = ValueHint::DirPath)]
    pub root: PathBuf,

    /// Path to file (relative or absolute)
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub file: PathBuf,

    /// 1-based line number (for context)
    #[arg(long)]
    pub line: u32,

    /// Pre-computed hover result JSON from neovim LSP (or @filepath to read from file)
    #[arg(long)]
    pub hover_json: Option<String>,

    /// Pre-computed definition result JSON from neovim LSP (or @filepath to read from file)
    #[arg(long)]
    pub definition_json: Option<String>,

    /// File containing hover JSON (alternative to --hover-json)
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub hover_file: Option<PathBuf>,

    /// File containing definition JSON (alternative to --definition-json)
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub definition_file: Option<PathBuf>,

    /// Use GitHub permalink instead of local file URI
    #[arg(long)]
    pub github: bool,

    /// Git remote name
    #[arg(long, default_value = "origin")]
    pub remote: String,
}
