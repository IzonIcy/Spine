//! Integration tests for Spine's config discovery.
//!
//! `Config::load()` walks a fixed list of search paths. These tests pin
//! `HOME` and the current directory to temp dirs so discovery is
//! deterministic and never touches the real user's config.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use spine::config::{Config, DEFAULT_CONFIG};

/// `set_current_dir`/`set_var` are process-global, so these tests must not
/// run in parallel with each other.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Restores process-global state even when a test panics.
struct EnvGuard {
    old_dir: PathBuf,
    old_home: Option<String>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.old_dir).ok();
        match &self.old_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn isolate_in(tempdir: &Path) -> EnvGuard {
    let old_dir = std::env::current_dir().expect("current dir");
    let old_home = std::env::var("HOME").ok();
    std::env::set_current_dir(tempdir).expect("enter tempdir");
    std::env::set_var("HOME", tempdir.join("fake-home"));
    EnvGuard {
        old_dir,
        old_home,
    }
}

fn env_guard_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn load_prefers_backbone_toml_in_current_dir() {
    let _lock = env_guard_lock();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let temp_root = fs::canonicalize(tempdir.path()).expect("canonicalize tempdir");
    let config_path = temp_root.join("backbone.toml");
    fs::write(
        &config_path,
        r#"
[settings]
timeout_seconds = 42

[managers.brew]
name = "Homebrew"
check_command = "brew --version"
check_updates = "brew outdated"
upgrade_all = "brew upgrade"
"#,
    )
    .expect("write config");
    let _guard = isolate_in(tempdir.path());

    let config = Config::load().expect("load config");

    assert_eq!(
        config.active_path(),
        Some(config_path.as_path()),
        "cwd backbone.toml should be picked up"
    );
    assert_eq!(config.settings.timeout_seconds, 42);
    assert!(config.managers.contains_key("brew"));
    assert!(config.validate().is_empty());
}

#[test]
fn load_falls_back_to_defaults_when_nothing_found() {
    let _lock = env_guard_lock();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let _guard = isolate_in(tempdir.path());

    let config = Config::load().expect("load config");

    assert!(config.active_path().is_none());
    assert_eq!(config.settings.timeout_seconds, 3600);
    assert!(
        config.managers.contains_key("brew"),
        "fallback is the full built-in DEFAULT_CONFIG"
    );
}

#[test]
fn default_config_serializes_back_to_valid_toml() {
    let config: Config = toml::from_str(DEFAULT_CONFIG).expect("default parses");
    let serialized = toml::to_string(&config).expect("serializes");
    let reparsed: Config = toml::from_str(&serialized).expect("roundtrips");
    assert!(reparsed.validate().is_empty());
}