use crate::config::Config;
use crate::detect::Manager;
use anyhow::{Context, Result};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Pending,
    Refreshing,
    Upgrading,
    Cleaning,
    Complete,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ManagerStatus {
    pub manager: Manager,
    pub stage: Stage,
    pub message: Option<String>,
}

pub async fn check_command(cmd: &str) -> Result<bool> {
    let mut parts = shell_words::split(cmd)?.into_iter();
    let Some(program) = parts.next() else {
        return Ok(false);
    };
    let status = Command::new(program)
        .args(parts)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .with_context(|| format!("Failed to run check command: {cmd}"))?;
    Ok(status.success())
}

pub async fn run_cli(managers: Vec<Manager>) -> Result<()> {
    let mut handles = Vec::new();
    for manager in managers {
        handles.push(tokio::spawn(run_manager(manager)));
    }
    for handle in handles {
        handle.await??;
    }
    Ok(())
}

pub async fn run_manager(manager: Manager) -> Result<()> {
    if let Some(refresh) = &manager.config.refresh {
        run_command(refresh).await?;
    }
    run_command(&manager.config.upgrade_all).await?;
    if let Some(clean) = &manager.config.cleanup {
        run_command(clean).await?;
    }
    Ok(())
}

pub async fn run_with_updates(
    managers: Vec<Manager>,
    tx: mpsc::UnboundedSender<ManagerStatus>,
) -> Result<()> {
    let mut handles = Vec::new();
    for manager in managers {
        let tx_clone = tx.clone();
        handles.push(tokio::spawn(async move {
            let _ = tx_clone.send(ManagerStatus {
                manager: manager.clone(),
                stage: Stage::Pending,
                message: None,
            });
            if let Some(refresh) = &manager.config.refresh {
                let _ = tx_clone.send(ManagerStatus {
                    manager: manager.clone(),
                    stage: Stage::Refreshing,
                    message: Some(refresh.to_string()),
                });
                if let Err(err) = run_command(refresh).await {
                    let _ = tx_clone.send(ManagerStatus {
                        manager: manager.clone(),
                        stage: Stage::Failed,
                        message: Some(err.to_string()),
                    });
                    return Err(err);
                }
            }
            let _ = tx_clone.send(ManagerStatus {
                manager: manager.clone(),
                stage: Stage::Upgrading,
                message: Some(manager.config.upgrade_all.clone()),
            });
            if let Err(err) = run_command(&manager.config.upgrade_all).await {
                let _ = tx_clone.send(ManagerStatus {
                    manager: manager.clone(),
                    stage: Stage::Failed,
                    message: Some(err.to_string()),
                });
                return Err(err);
            }
            if let Some(clean) = &manager.config.cleanup {
                let _ = tx_clone.send(ManagerStatus {
                    manager: manager.clone(),
                    stage: Stage::Cleaning,
                    message: Some(clean.to_string()),
                });
                if let Err(err) = run_command(clean).await {
                    let _ = tx_clone.send(ManagerStatus {
                        manager: manager.clone(),
                        stage: Stage::Failed,
                        message: Some(err.to_string()),
                    });
                    return Err(err);
                }
            }
            let _ = tx_clone.send(ManagerStatus {
                manager,
                stage: Stage::Complete,
                message: None,
            });
            Ok(())
        }));
    }

    for handle in handles {
        handle.await??;
    }
    Ok(())
}

pub fn needs_sudo(managers: &[Manager]) -> bool {
    managers
        .iter()
        .any(|manager| manager.config.requires_sudo.unwrap_or(false))
}

pub async fn prime_sudo() -> Result<()> {
    let status = Command::new("sudo")
        .arg("-v")
        .status()
        .await
        .context("Failed to run sudo -v")?;
    if !status.success() {
        return Err(anyhow::anyhow!("Sudo authentication failed"));
    }
    Ok(())
}

pub fn print_list(managers: &[Manager], active_config: Option<&std::path::Path>) {
    if let Some(path) = active_config {
        println!("Config: {}", path.display());
    } else {
        println!("Config: default (built-in)");
    }
    for manager in managers {
        println!("{} ({})", manager.config.name, manager.key);
    }
}

pub fn print_dry_run(managers: &[Manager], active_config: Option<&std::path::Path>) {
    if let Some(path) = active_config {
        println!("Config: {}", path.display());
    } else {
        println!("Config: default (built-in)");
    }
    for manager in managers {
        println!("{}:", manager.config.name);
        if let Some(refresh) = &manager.config.refresh {
            println!("  refresh: {}", refresh);
        }
        println!("  upgrade: {}", manager.config.upgrade_all);
        if let Some(clean) = &manager.config.cleanup {
            println!("  cleanup: {}", clean);
        }
    }
}

pub fn print_doctor(config: &Config, managers: &[Manager]) -> Result<()> {
    println!("Active config:");
    if let Some(path) = config.active_path() {
        println!("  {}", path.display());
    } else {
        println!("  default (built-in)");
    }
    println!("Detected managers: {}", managers.len());
    for manager in managers {
        println!("  {}", manager.config.name);
    }
    Ok(())
}

async fn run_command(cmd: &str) -> Result<()> {
    let mut parts = shell_words::split(cmd)?.into_iter();
    let Some(program) = parts.next() else {
        return Ok(());
    };
    let mut command = Command::new(program);
    command.args(parts);
    let status = time::timeout(Duration::from_secs(60 * 60), command.status())
        .await
        .context("Command timed out")??;
    if !status.success() {
        return Err(anyhow::anyhow!("Command failed: {cmd}"));
    }
    Ok(())
}
