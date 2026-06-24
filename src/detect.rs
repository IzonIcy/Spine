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
