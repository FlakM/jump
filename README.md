# jump

A CLI tool for navigating code references. Parse file links, GitHub URLs, or relative paths and open them directly in your editor.

## Why

When working across multiple terminal windows, you often encounter file references in logs, error messages, or documentation. `jump` lets you quickly open these references in your existing neovim instance without manually switching windows or sessions.

With Hyprland integration, `jump` automatically finds your main editor window and opens files there, regardless of which terminal you invoke it from.

## Install

```bash
cargo install --path .
```

## Usage

### Jump to a file

```bash
# Relative path with line number
jump src/main.rs:11

# Absolute path
jump /home/user/project/src/lib.rs:100

# GitHub permalink
jump "https://github.com/FlakM/jump/blob/8fd8d1b77395f68ef9ca9ce85f070e2376923eae/src/main.rs#L11"
```

### Other commands

```bash
# Generate GitHub permalink
jump github-link --file src/main.rs --start-line 10 --end-line 20
{
  "url": "https://github.com/FlakM/jump/blob/8fd8d1b77395f68ef9ca9ce85f070e2376923eae/src/main.rs#L10-L20",
  "relative_path": "src/main.rs",
  "revision": "8fd8d1b77395f68ef9ca9ce85f070e2376923eae",
  "lines": {
    "start": 10,
    "end": 20
  },
  "provider": "github"
}

# Copy markdown link to clipboard
jump copy-markdown --root . --file src/main.rs --line 11 --character 10
{
  "markdown": "[fn jump::main](file:///home/flakm/programming/flakm/jump/src/main.rs#L11)"
}
```

## How it works

1. **Parse the link** - supports relative paths, absolute paths, and GitHub URLs
2. **Find project root** - looks for `.git`, `Cargo.toml`, `package.json`, etc.
3. **Locate editor** - finds your neovim instance via Hyprland window detection
4. **Open file** - sends the file to neovim via its RPC socket

### Hyprland integration

On Hyprland, `jump` automatically:
- Finds the largest kitty window on workspace 1 (your main editor)
- Maps it to the tmux session running in that window
- Finds the first nvim pane in that session
- Opens files directly in that nvim instance

This means you can run `jump` from any terminal and have files open in your editor window.

## Requirements

- neovim (with RPC socket enabled - default since nvim 0.9)
- tmux
- neovim-remote (`nvr`) for sending commands to nvim
- Hyprland (optional, for automatic window detection)

## Environment

- `RUST_LOG=jump=info` - enable logging
