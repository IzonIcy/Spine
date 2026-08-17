# Spine

[![CI](https://github.com/IzonIcy/Spine/actions/workflows/ci.yml/badge.svg)](https://github.com/IzonIcy/Spine/actions/workflows/ci.yml)

Spine is a meta package manager for most *nix systems. It discovers installed package managers and runs update-oriented workflows in parallel with either a lightweight TUI or script-friendly CLI output.

## Features

- Auto-detects common system and developer package managers
- Parallel `check`, `upgrade`, and `cleanup` workflows
- TUI with manager selection, live stdout/stderr output, status colors, and elapsed time
- CLI mode for scripts or headless environments
- Safer cleanup: cleanup is explicit via `spn cleanup`, `spn --cleanup`, or config
- Configurable via TOML, including profiles, timeouts, shell mode, and enabled managers
- Shell-aware command execution for pipes, variables, and command substitution
- Dry-run/plan output for previewing commands before they run
- Run history stored as JSON under `~/.local/state/spine/history`

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
spn upgrade

# Run without the TUI
spn cli
spn --no-tui

# Check for available updates without upgrading
spn check

# Run only cleanup commands
spn cleanup

# Upgrade, then run cleanup commands
spn --cleanup
spn upgrade --cleanup

# Preview detected managers and commands
spn --dry-run
spn check --dry-run
spn cleanup --dry-run

# Check configuration, detection, profiles, and search paths
spn doctor

# List detected managers
spn list

# Run only specific managers
spn --only brew,nix

# Skip specific managers
spn --skip snap

# Use a configured profile
spn --profile dev
spn check --profile system

# Keep going even if one manager fails
spn --continue-on-error

# Config helpers
spn config init
spn config path
spn config edit

# History
spn history
spn history last
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
[settings]
timeout_seconds = 3600
continue_on_error = false
cleanup_after_upgrade = false
shell = true

[profiles.dev]
only = ["brew", "cargo", "npm", "pnpm"]

[managers.brew]
name = "Homebrew"
check_command = "brew --version"
check_updates = "brew outdated"
refresh = "brew update"
upgrade_all = "brew upgrade"
cleanup = "brew cleanup"
requires_sudo = false
enabled = true
```

### Manager fields

| Field | Required | Description |
| --- | --- | --- |
| `name` | yes | Human-readable manager name |
| `check_command` | yes | Command used to detect whether the manager is installed |
| `enabled` | no | Set to `false` to disable a manager permanently |
| `check_updates` | no | Command used by `spn check` |
| `refresh` | no | Command run before upgrade |
| `upgrade_all` | no | Command used by upgrade workflows |
| `cleanup` | no | Command used by cleanup workflows |
| `requires_sudo` | no | Whether Spine should prime sudo before running privileged workflows |
| `timeout_seconds` | no | Per-manager timeout override |
| `shell` | no | Per-manager shell execution override |

## Development

```bash
cargo build
cargo test
```

## License

MIT