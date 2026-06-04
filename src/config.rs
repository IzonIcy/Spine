use anyhow::{Context, Result};
use dirs::config_dir;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub managers: BTreeMap<String, ManagerConfig>,

    #[serde(skip)]
    active_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagerConfig {
    pub name: String,
    pub check_command: String,
    pub refresh: Option<String>,
    pub upgrade_all: String,
    pub cleanup: Option<String>,
    pub requires_sudo: Option<bool>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let search_paths = config_search_paths();
        for path in search_paths {
            if path.exists() {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config at {}", path.display()))?;
                let mut parsed: Config = toml::from_str(&content)
                    .with_context(|| format!("Invalid TOML in {}", path.display()))?;
                parsed.active_path = Some(path);
                return Ok(parsed);
            }
        }

        let mut parsed: Config = toml::from_str(DEFAULT_CONFIG)?;
        parsed.active_path = None;
        Ok(parsed)
    }

    pub fn active_path(&self) -> Option<&Path> {
        self.active_path.as_deref()
    }
}

fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = config_dir() {
        paths.push(dir.join("spine").join("backbone.toml"));
    }
    paths.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        .join("backbone.toml"));
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".spine").join("backbone.toml"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("backbone.toml"));
        }
    }
    paths.push(PathBuf::from("/etc/spine/backbone.toml"));
    paths.push(PathBuf::from("/usr/local/etc/spine/backbone.toml"));
    paths
}

const DEFAULT_CONFIG: &str = r#"
[managers.brew]
name = "Homebrew"
check_command = "brew --version"
refresh = "brew update"
upgrade_all = "brew upgrade"
cleanup = "brew cleanup"
requires_sudo = false

[managers.apt]
name = "APT"
check_command = "apt --version"
refresh = "sudo apt update"
upgrade_all = "sudo apt upgrade -y"
cleanup = "sudo apt autoremove -y"
requires_sudo = true

[managers.dnf]
name = "DNF"
check_command = "dnf --version"
refresh = "sudo dnf makecache"
upgrade_all = "sudo dnf upgrade -y"
cleanup = "sudo dnf autoremove -y"
requires_sudo = true

[managers.pacman]
name = "Pacman"
check_command = "pacman --version"
refresh = "sudo pacman -Sy"
upgrade_all = "sudo pacman -Syu --noconfirm"
cleanup = "sudo pacman -Rns $(pacman -Qdtq)"
requires_sudo = true

[managers.nix]
name = "Nix"
check_command = "nix --version"
refresh = "nix flake update"
upgrade_all = "nix profile upgrade --all"
cleanup = "nix store gc"
requires_sudo = false

[managers.snap]
name = "Snap"
check_command = "snap --version"
refresh = "sudo snap refresh"
upgrade_all = "sudo snap refresh"
cleanup = "sudo snap remove --purge $(snap list | awk 'NR>1 {print $1}')"
requires_sudo = true

[managers.flatpak]
name = "Flatpak"
check_command = "flatpak --version"
refresh = "flatpak update -y"
upgrade_all = "flatpak update -y"
cleanup = "flatpak uninstall --unused -y"
requires_sudo = false

[managers.zypper]
name = "Zypper"
check_command = "zypper --version"
refresh = "sudo zypper refresh"
upgrade_all = "sudo zypper update -y"
cleanup = "sudo zypper clean -a"
requires_sudo = true

[managers.apk]
name = "APK"
check_command = "apk --version"
refresh = "sudo apk update"
upgrade_all = "sudo apk upgrade"
cleanup = "sudo apk cache clean"
requires_sudo = true

[managers.pkg]
name = "pkg"
check_command = "pkg --version"
refresh = "sudo pkg update"
upgrade_all = "sudo pkg upgrade -y"
cleanup = "sudo pkg autoremove -y"
requires_sudo = true

[managers.emerge]
name = "Portage"
check_command = "emerge --version"
refresh = "sudo emerge --sync"
upgrade_all = "sudo emerge -avuDN @world"
cleanup = "sudo emerge --depclean"
requires_sudo = true

[managers.yarn]
name = "Yarn"
check_command = "yarn --version"
refresh = "yarn cache clean"
upgrade_all = "yarn global upgrade"
cleanup = "yarn cache clean"
requires_sudo = false

[managers.pnpm]
name = "pnpm"
check_command = "pnpm --version"
refresh = "pnpm store prune"
upgrade_all = "pnpm update -g"
cleanup = "pnpm store prune"
requires_sudo = false
"#;

pub fn write_default() -> Result<()> {
    let dir = config_dir().unwrap_or_else(|| PathBuf::from("."));
    let target_dir = dir.join("spine");
    let target_path = target_dir.join("backbone.toml");
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("Failed to create config directory {}", target_dir.display()))?;
    fs::write(&target_path, DEFAULT_CONFIG)
        .with_context(|| format!("Failed to write config to {}", target_path.display()))?;
    Ok(())
}
