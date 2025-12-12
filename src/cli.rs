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

    /// Language server executable passed to lspmux
    #[arg(long, default_value = "rust-analyzer", value_hint = ValueHint::ExecutablePath)]
    pub server_path: String,

    /// Use GitHub permalink instead of local file URI
    #[arg(long)]
    pub github: bool,

    /// Git remote name
    #[arg(long, default_value = "origin")]
    pub remote: String,
}
