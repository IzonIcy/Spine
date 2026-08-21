use crate::config::{Config, ManagerConfig};
use crate::execute;
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct Manager {
    pub key: String,
    pub config: ManagerConfig,
    pub timeout_seconds: u64,
    pub shell: bool,
}

pub async fn discover(config: &Config) -> Result<Vec<Manager>> {
    let mut detected = Vec::new();
    for (key, manager) in &config.managers {
        if !manager.enabled {
            continue;
        }

        let timeout_seconds = manager
            .timeout_seconds
            .unwrap_or(config.settings.timeout_seconds);
        let shell = manager.shell.unwrap_or(config.settings.shell);
        let ok = execute::check_command(&manager.check_command, shell, timeout_seconds).await?;
        if ok {
            detected.push(Manager {
                key: key.to_string(),
                config: manager.clone(),
                timeout_seconds,
                shell,
            });
        }
    }
    Ok(detected)
}

pub fn filter_managers(
    mut managers: Vec<Manager>,
    only: &[String],
    skip: &[String],
) -> Vec<Manager> {
    if !only.is_empty() {
        managers.retain(|manager| only.iter().any(|key| key == &manager.key));
    }
    if !skip.is_empty() {
        managers.retain(|manager| !skip.iter().any(|key| key == &manager.key));
    }
    managers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(key: &str) -> Manager {
        Manager {
            key: key.to_string(),
            config: ManagerConfig {
                name: key.to_string(),
                check_command: format!("{key} --version"),
                enabled: true,
                refresh: None,
                check_updates: None,
                upgrade_all: None,
                cleanup: None,
                requires_sudo: None,
                timeout_seconds: None,
                shell: None,
            },
            timeout_seconds: 3600,
            shell: true,
        }
    }

    #[test]
    fn no_filters_keeps_everything() {
        let managers = vec![manager("brew"), manager("cargo"), manager("npm")];
        assert_eq!(filter_managers(managers, &[], &[]).len(), 3);
    }

    #[test]
    fn only_keeps_listed_in_order() {
        let managers = vec![manager("brew"), manager("cargo"), manager("npm")];
        let filtered = filter_managers(managers, &["npm".into(), "brew".into()], &[]);
        let keys: Vec<_> = filtered.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["brew", "npm"], "original order preserved");
    }

    #[test]
    fn skip_removes_listed() {
        let managers = vec![manager("brew"), manager("cargo")];
        let filtered = filter_managers(managers, &[], &["cargo".into()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].key, "brew");
    }

    #[test]
    fn skip_wins_over_only() {
        let managers = vec![manager("brew"), manager("cargo")];
        let filtered =
            filter_managers(managers, &["brew".into(), "cargo".into()], &["brew".into()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].key, "cargo");
    }
}
