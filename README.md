# Spine

[![CI](https://github.com/IzonIcy/Spine/actions/workflows/ci.yml/badge.svg)](https://github.com/IzonIcy/Spine/actions/workflows/ci.yml)

Spine is a meta package manager for most *nix systems. It discovers the package managers you already have installed and runs update-oriented workflows across all of them in parallel, through either a lightweight TUI or script-friendly CLI output.

## Features

- Auto-detects common system and developer package managers
- Parallel `check`, `upgrade`, and `cleanup` workflows
- TUI with manager selection, live stdout/stderr output, status colors, and elapsed time
- CLI mode for scripts and headless environments
- Cleanup never runs unless you ask for it (`spn cleanup`, `spn --cleanup`, or config)
- Configurable via TOML: profiles, timeouts, shell mode, enabled managers
- Shell-aware command execution, so pipes, variables, and command substitution work
- Dry-run output for previewing commands before anything runs
- Optional desktop notification when a workflow finishes
- Run history stored as JSON under `~/.local/state/spine/history`

## Install

```bash
git clone https://github.com/IzonIcy/Spine.git
cd Spine
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

# Preview detected managers and commands without running anything
spn --dry-run
spn check --dry-run
spn cleanup --dry-run

# Inspect configuration, detection, profiles, and search paths
spn doctor

# List detected managers
spn list

# Run only specific managers, or skip some
spn --only brew,nix
spn --skip snap

# Use a configured profile
spn --profile dev
spn check --profile system

# Keep going even if one manager fails
spn --continue-on-error

# Send a desktop notification when the workflow completes
spn --notify

# Print a launchd plist for scheduled runs (Mondays at 09:00 by default)
spn schedule
spn schedule --daily --at 09:00 --out ~/Library/LaunchAgents/dev.spine.spn.plist

# Config helpers
spn config init
spn config path
spn config edit

# History
spn history
spn history last
```

## Configuration

Spine reads `backbone.toml` from, in order:

- platform config dir (`~/.config/spine/backbone.toml` on most Linux, `~/Library/Application Support/spine/backbone.toml` on macOS)
- `~/.spine/backbone.toml`
- the directory containing the `spn` binary
- `/etc/spine/backbone.toml`
- `/usr/local/etc/spine/backbone.toml`

The current directory is deliberately not searched. Since `backbone.toml` defines shell commands (including sudo-requiring ones), honoring whatever config file happens to sit in your current checkout would let any repository you clone execute arbitrary code the next time you run `spn` there. To use a specific non-standard config file, pass it explicitly:

```sh
spn --config ./path/to/backbone.toml upgrade
```

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
cargo clippy
```

## License

MIT
