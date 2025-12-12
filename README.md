# jump

A CLI tool for navigating code references. Parse file links, GitHub URLs, or relative paths and open them directly in neovim.

## Why

When working across multiple terminal windows, you constantly encounter file references—in logs, error messages, stack traces, or documentation. `jump` lets you open these references in your existing neovim instance without manually switching windows or copying paths.

With Hyprland integration, `jump` automatically finds your main editor window and opens files there, regardless of which terminal you invoke it from.

## Installation

```bash
cargo install --path .
```

### Requirements

- **neovim** with RPC socket enabled (default since 0.9)
- **tmux**
- **nvr** (neovim-remote) for sending commands to nvim
- **Hyprland** (optional) for automatic window detection

## Usage

### Open a file reference

```bash
# Check environment
jump verify

# Relative path with line number
jump src/main.rs:42

# Absolute path
jump /home/user/project/src/lib.rs:100

# GitHub permalink
jump "https://github.com/user/repo/blob/abc123/src/main.rs#L42"

# Markdown link (strips markdown syntax)
jump "[link](https://github.com/user/repo/blob/main/src/lib.rs#L10)"

# File URI
jump "file:///home/user/project/src/main.rs#L20"
```

### Generate GitHub permalink

```bash
jump github-link --file src/main.rs --start-line 10 --end-line 20
```

Output:
```json
{
  "url": "https://github.com/user/repo/blob/abc123/src/main.rs#L10-L20",
  "relative_path": "src/main.rs",
  "revision": "abc123...",
  "lines": { "start": 10, "end": 20 },
  "provider": "github"
}
```

### Generate markdown reference with LSP

```bash
jump copy-markdown --root . --file src/main.rs --line 42 --character 10
```

Uses your language server to generate a markdown link with the symbol name:

```json
{
  "markdown": "[fn main](file:///path/to/src/main.rs#L42)"
}
```

With `--github` flag, generates a GitHub permalink instead of a local file URI.

### Other commands

```bash
jump verify              # Check that required tools are installed
jump completions bash    # Generate shell completions (bash, zsh, fish, etc.)
```

## How it works

```
┌─────────────┐    ┌──────────────┐    ┌─────────────┐    ┌─────────┐
│ Parse link  │───▶│ Find project │───▶│ Locate nvim │───▶│ Open    │
│             │    │ root         │    │ instance    │    │ file    │
└─────────────┘    └──────────────┘    └─────────────┘    └─────────┘
```

1. **Parse** - Handles relative paths, absolute paths, GitHub URLs, file URIs, markdown links
2. **Locate** - Finds project root by walking up looking for `.git`, `Cargo.toml`, `package.json`, etc.
3. **Resolve** - Validates the file exists and canonicalizes the path
4. **Open** - Sends the file to neovim via tmux/nvr and hyprland

### Hyprland integration

On Hyprland, `jump` automatically:
- Finds the largest kitty window on workspace 1 (your main editor)
- Maps it to the tmux session running in that window
- Locates the nvim pane in that session
- Opens files directly in that nvim instance

This means you can run `jump` from any terminal and have files open in your primary editor window.

## Configuration

Enable debug logging:

```bash
RUST_LOG=jump=debug jump src/main.rs:10
```

Custom project markers:

```bash
jump --markers=".project,.workspace" src/main.rs:10
```
