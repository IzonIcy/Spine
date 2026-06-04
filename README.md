# Spine

Spine is a meta package manager for any *nix system. It discovers installed package managers and runs their update workflows in parallel with a lightweight TUI.

## Features

- Auto-detects common package managers (Homebrew, APT, DNF, Pacman, Nix, Snap, Flatpak)
- Parallel refresh/upgrade/cleanup workflows
- TUI with real-time status and details panel
- CLI mode for scripts or headless environments
- Configurable via TOML

## Install

```bash
git clone https://github.com/plyght/spine.git
cd spine
cargo build --release
sudo cp target/release/spn /usr/local/bin/
```

## Usage

```bash
# Open the TUI and upgrade all detected managers
spn

# Run without the TUI
spn cli
spn --no-tui

# Preview detected managers and commands
spn --dry-run

# Check configuration and detection
spn doctor

# List detected managers
spn list

# Run only specific managers
spn --only brew,nix

# Skip specific managers
spn --skip snap

# Write a default config to user config dir
spn config init
```

## Configuration

Spine reads `backbone.toml` from:

- platform config dir (`~/.config/spine/backbone.toml` on most Linux, `~/Library/Application Support/spine/backbone.toml` on macOS)
- current directory
- `~/.spine/backbone.toml`
- binary directory
- `/etc/spine/backbone.toml`
- `/usr/local/etc/spine/backbone.toml`

Example:

```toml
[managers.brew]
name = "Homebrew"
check_command = "brew --version"
refresh = "brew update"
upgrade_all = "brew upgrade"
cleanup = "brew cleanup"
requires_sudo = false
```

## Development

```bash
cargo build
cargo test
```

## License

MIT
