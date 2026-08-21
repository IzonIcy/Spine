use anyhow::{Context, Result};
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,

    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,

    pub managers: BTreeMap<String, ManagerConfig>,

    #[serde(skip)]
    active_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Settings {
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,

    #[serde(default)]
    pub continue_on_error: bool,

    #[serde(default)]
    pub cleanup_after_upgrade: bool,

    #[serde(default = "default_shell")]
    pub shell: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout_seconds(),
            continue_on_error: false,
            cleanup_after_upgrade: false,
            shell: default_shell(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProfileConfig {
    #[serde(default)]
    pub only: Vec<String>,

    #[serde(default)]
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManagerConfig {
    pub name: String,
    pub check_command: String,

    #[serde(default = "default_enabled")]
    pub enabled: bool,

    pub refresh: Option<String>,
    pub check_updates: Option<String>,
    pub upgrade_all: Option<String>,
    pub cleanup: Option<String>,
    pub requires_sudo: Option<bool>,
    pub timeout_seconds: Option<u64>,
    pub shell: Option<bool>,
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

    pub fn default_user_path() -> PathBuf {
        config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("spine")
            .join("backbone.toml")
    }

    pub fn editable_path(&self) -> PathBuf {
        self.active_path
            .clone()
            .unwrap_or_else(Self::default_user_path)
    }

    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        if self.managers.is_empty() {
            warnings.push("no managers are configured".to_string());
        }

        for (key, manager) in &self.managers {
            if manager.name.trim().is_empty() {
                warnings.push(format!("manager `{key}` has an empty name"));
            }
            if manager.check_command.trim().is_empty() {
                warnings.push(format!("manager `{key}` has an empty check_command"));
            }
            if manager.upgrade_all.is_none()
                && manager.cleanup.is_none()
                && manager.check_updates.is_none()
            {
                warnings.push(format!(
                    "manager `{key}` has no check_updates, upgrade_all, or cleanup command"
                ));
            }
            if manager.timeout_seconds == Some(0) {
                warnings.push(format!("manager `{key}` has timeout_seconds = 0"));
            }
        }

        for (profile_name, profile) in &self.profiles {
            for key in profile.only.iter().chain(profile.skip.iter()) {
                if !self.managers.contains_key(key) {
                    warnings.push(format!(
                        "profile `{profile_name}` references unknown manager `{key}`"
                    ));
                }
            }
        }

        warnings
    }
}

pub fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(Config::default_user_path());
    paths.push(
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("backbone.toml"),
    );
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

pub fn write_default() -> Result<PathBuf> {
    let target_path = Config::default_user_path();
    let target_dir = target_path
        .parent()
        .context("Config path does not have a parent directory")?;
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create config directory {}", target_dir.display()))?;
    fs::write(&target_path, DEFAULT_CONFIG)
        .with_context(|| format!("Failed to write config to {}", target_path.display()))?;
    Ok(target_path)
}

pub fn edit_config(config: &Config) -> Result<PathBuf> {
    let path = if config.active_path().is_some() {
        config.editable_path()
    } else {
        write_default()?
    };

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to launch editor `{editor}`"))?;

    if !status.success() {
        return Err(anyhow::anyhow!("Editor exited unsuccessfully"));
    }

    Ok(path)
}

fn default_timeout_seconds() -> u64 {
    60 * 60
}

fn default_enabled() -> bool {
    true
}

fn default_shell() -> bool {
    true
}

pub const DEFAULT_CONFIG: &str = r#"
[settings]
timeout_seconds = 3600
continue_on_error = false
cleanup_after_upgrade = false
shell = true

[profiles.dev]
only = ["brew", "cargo", "npm", "pnpm", "yarn", "pipx", "uv", "gem", "composer", "asdf", "mise"]

[profiles.system]
only = ["brew", "apt", "dnf", "pacman", "nix", "snap", "flatpak", "zypper", "apk", "pkg", "emerge", "mas"]

[managers.brew]
name = "Homebrew"
check_command = "brew --version"
check_updates = "brew outdated"
refresh = "brew update"
upgrade_all = "brew upgrade"
cleanup = "brew cleanup"
requires_sudo = false

[managers.apt]
name = "APT"
check_command = "apt --version"
check_updates = "apt list --upgradable"
refresh = "sudo apt update"
upgrade_all = "sudo apt upgrade -y"
cleanup = "sudo apt autoremove -y"
requires_sudo = true

[managers.dnf]
name = "DNF"
check_command = "dnf --version"
check_updates = "dnf check-update"
refresh = "sudo dnf makecache"
upgrade_all = "sudo dnf upgrade -y"
cleanup = "sudo dnf autoremove -y"
requires_sudo = true

[managers.pacman]
name = "Pacman"
check_command = "pacman --version"
check_updates = "checkupdates"
refresh = "sudo pacman -Sy"
upgrade_all = "sudo pacman -Syu --noconfirm"
cleanup = "orphans=$(pacman -Qdtq); [ -z \"$orphans\" ] || sudo pacman -Rns --noconfirm $orphans"
requires_sudo = true

[managers.nix]
name = "Nix"
check_command = "nix --version"
check_updates = "nix profile list"
refresh = "nix flake update"
upgrade_all = "nix profile upgrade --all"
cleanup = "nix store gc"
requires_sudo = false

[managers.snap]
name = "Snap"
check_command = "snap --version"
check_updates = "snap refresh --list"
refresh = "sudo snap refresh"
upgrade_all = "sudo snap refresh"
cleanup = "sudo snap remove --purge $(snap list --all | awk '/disabled/{print $1, $3}')"
requires_sudo = true

[managers.flatpak]
name = "Flatpak"
check_command = "flatpak --version"
check_updates = "flatpak remote-ls --updates"
refresh = "flatpak update --appstream -y"
upgrade_all = "flatpak update -y"
cleanup = "flatpak uninstall --unused -y"
requires_sudo = false

[managers.zypper]
name = "Zypper"
check_command = "zypper --version"
check_updates = "zypper list-updates"
refresh = "sudo zypper refresh"
upgrade_all = "sudo zypper update -y"
cleanup = "sudo zypper clean -a"
requires_sudo = true

[managers.apk]
name = "APK"
check_command = "apk --version"
check_updates = "apk version -l '<'"
refresh = "sudo apk update"
upgrade_all = "sudo apk upgrade"
cleanup = "sudo apk cache clean"
requires_sudo = true

[managers.pkg]
name = "pkg"
check_command = "pkg --version"
check_updates = "pkg version -vIL="
refresh = "sudo pkg update"
upgrade_all = "sudo pkg upgrade -y"
cleanup = "sudo pkg autoremove -y"
requires_sudo = true

[managers.emerge]
name = "Portage"
check_command = "emerge --version"
check_updates = "emerge -puvDN @world"
refresh = "sudo emerge --sync"
upgrade_all = "sudo emerge -avuDN @world"
cleanup = "sudo emerge --depclean"
requires_sudo = true

[managers.yarn]
name = "Yarn"
check_command = "yarn --version"
check_updates = "yarn global outdated"
refresh = "yarn cache clean"
upgrade_all = "yarn global upgrade"
cleanup = "yarn cache clean"
requires_sudo = false

[managers.pnpm]
name = "pnpm"
check_command = "pnpm --version"
check_updates = "pnpm outdated -g"
refresh = "pnpm store prune"
upgrade_all = "pnpm update -g"
cleanup = "pnpm store prune"
requires_sudo = false

[managers.npm]
name = "npm"
check_command = "npm --version"
check_updates = "npm outdated -g --depth=0"
upgrade_all = "npm update -g"
cleanup = "npm cache verify"
requires_sudo = false

[managers.cargo]
name = "Cargo"
check_command = "cargo --version"
check_updates = "cargo install-update --list"
upgrade_all = "cargo install-update --all"
requires_sudo = false

[managers.pipx]
name = "pipx"
check_command = "pipx --version"
check_updates = "pipx list --short"
upgrade_all = "pipx upgrade-all"
requires_sudo = false

[managers.uv]
name = "uv"
check_command = "uv --version"
check_updates = "uv tool list"
upgrade_all = "uv tool upgrade --all"
requires_sudo = false

[managers.gem]
name = "RubyGems"
check_command = "gem --version"
check_updates = "gem outdated"
upgrade_all = "gem update"
cleanup = "gem cleanup"
requires_sudo = false

[managers.composer]
name = "Composer"
check_command = "composer --version"
check_updates = "composer global outdated"
upgrade_all = "composer global update"
requires_sudo = false

[managers.mas]
name = "Mac App Store"
check_command = "mas version"
check_updates = "mas outdated"
upgrade_all = "mas upgrade"
requires_sudo = false

[managers.conda]
name = "Conda"
check_command = "conda --version"
check_updates = "conda update --all --dry-run"
upgrade_all = "conda update --all -y"
cleanup = "conda clean --all -y"
requires_sudo = false

[managers.asdf]
name = "asdf"
check_command = "asdf --version"
check_updates = "asdf latest --all"
upgrade_all = "asdf plugin update --all"
requires_sudo = false

[managers.mise]
name = "mise"
check_command = "mise --version"
check_updates = "mise outdated"
upgrade_all = "mise upgrade"
requires_sudo = false
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> Config {
        toml::from_str(toml_str).expect("config should parse")
    }

    #[test]
    fn default_config_is_valid() {
        let config = parse(DEFAULT_CONFIG);
        assert!(
            config.validate().is_empty(),
            "defaults must be warning-free"
        );
        assert_eq!(config.settings.timeout_seconds, 3600);
        assert!(config.settings.shell);
        assert!(!config.managers.is_empty());
    }

    #[test]
    fn validate_flags_missing_managers() {
        let warnings = parse("[settings]\n[managers]").validate();
        assert!(warnings.iter().any(|w| w.contains("no managers")));
    }

    #[test]
    fn validate_flags_empty_check_command() {
        let config = parse(
            r#"
[managers.bad]
name = "Bad"
check_command = ""
"#,
        );
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("empty check_command")));
    }

    #[test]
    fn validate_flags_manager_without_commands() {
        let config = parse(
            r#"
[managers.idle]
name = "Idle"
check_command = "idle --version"
"#,
        );
        assert!(config
            .validate()
            .iter()
            .any(|w| w.contains("no check_updates, upgrade_all, or cleanup")));
    }

    #[test]
    fn validate_flags_zero_timeout() {
        let config = parse(
            r#"
[managers.zero]
name = "Zero"
check_command = "zero --version"
check_updates = "zero outdated"
timeout_seconds = 0
"#,
        );
        assert!(config
            .validate()
            .iter()
            .any(|w| w.contains("timeout_seconds = 0")));
    }

    #[test]
    fn validate_flags_unknown_manager_in_profile() {
        let config = parse(
            r#"
[managers.brew]
name = "Homebrew"
check_command = "brew --version"
check_updates = "brew outdated"

[profiles.dev]
only = ["brew", "ghost"]
"#,
        );
        let warnings = config.validate();
        assert!(warnings
            .iter()
            .any(|w| w.contains("unknown manager `ghost`")));
    }

    #[test]
    fn manager_defaults_apply() {
        let config = parse(
            r#"
[managers.brew]
name = "Homebrew"
check_command = "brew --version"
"#,
        );
        let manager = &config.managers["brew"];
        assert!(manager.enabled, "enabled defaults to true");
        assert!(
            manager.shell.is_none(),
            "shell default deferred to settings"
        );
        assert_eq!(manager.timeout_seconds, None);
    }
}
