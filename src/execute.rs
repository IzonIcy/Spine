use crate::config::{config_search_paths, Config};
use crate::detect::Manager;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Workflow {
    Check,
    Upgrade,
    Cleanup,
}

impl Workflow {
    pub fn label(self) -> &'static str {
        match self {
            Workflow::Check => "check",
            Workflow::Upgrade => "upgrade",
            Workflow::Cleanup => "cleanup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    pub workflow: Workflow,
    pub cleanup: bool,
    pub continue_on_error: bool,
    pub notify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Pending,
    Checking,
    Refreshing,
    Upgrading,
    Cleaning,
    Complete,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct ManagerStatus {
    pub manager: Manager,
    pub stage: Stage,
    pub message: Option<String>,
    pub output: Vec<String>,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
}

impl ManagerStatus {
    pub fn pending(manager: Manager) -> Self {
        Self {
            manager,
            stage: Stage::Pending,
            message: None,
            output: Vec::new(),
            started_at: None,
            finished_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ManagerEvent {
    Status(ManagerStatus),
    Output { key: String, line: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerRun {
    pub key: String,
    pub name: String,
    pub success: bool,
    pub duration_ms: u128,
    pub error: Option<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub started_at: u64,
    pub finished_at: u64,
    pub workflow: Workflow,
    pub cleanup: bool,
    pub success: bool,
    pub managers: Vec<ManagerRun>,
}

#[derive(Debug, Clone)]
struct CommandRun {
    status_code: Option<i32>,
    output: Vec<String>,
}

/// Best-effort desktop notification when a workflow completes.
/// Never fails the run; notification problems are silently ignored.
pub fn notify_completion(summary: &RunSummary) {
    let ok = summary.managers.iter().filter(|m| m.success).count();
    let failed = summary.managers.len() - ok;
    let message = format!(
        "{} complete: {} ok, {} failed",
        summary.workflow.label(),
        ok,
        failed
    );

    #[cfg(target_os = "macos")]
    {
        let script = format!("display notification \"{message}\" with title \"spine\"");
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .status();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .arg("spine")
            .arg(&message)
            .status();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = message;
    }
}

pub async fn check_command(cmd: &str, shell: bool, timeout_seconds: u64) -> Result<bool> {
    let result = run_command(cmd, shell, timeout_seconds, None, None, false, true).await;

    match result {
        Ok(run) => Ok(run.status_code == Some(0)),
        Err(_) => Ok(false),
    }
}

pub async fn run_cli(managers: Vec<Manager>, options: RunOptions) -> Result<RunSummary> {
    let summary = run_many(managers, options, None, true).await;
    write_history(&summary)?;

    if options.notify {
        notify_completion(&summary);
    }

    if !summary.success && !options.continue_on_error {
        return Err(anyhow::anyhow!("One or more managers failed"));
    }

    Ok(summary)
}

pub async fn run_with_updates(
    managers: Vec<Manager>,
    options: RunOptions,
    tx: mpsc::UnboundedSender<ManagerEvent>,
) -> Result<RunSummary> {
    let summary = run_many(managers, options, Some(tx), false).await;
    write_history(&summary)?;

    if options.notify {
        notify_completion(&summary);
    }

    if !summary.success && !options.continue_on_error {
        return Err(anyhow::anyhow!("One or more managers failed"));
    }

    Ok(summary)
}

async fn run_many(
    managers: Vec<Manager>,
    options: RunOptions,
    tx: Option<mpsc::UnboundedSender<ManagerEvent>>,
    print_output: bool,
) -> RunSummary {
    let started_at = unix_timestamp();
    let mut handles = Vec::new();

    for manager in managers {
        let tx = tx.clone();
        handles.push(tokio::spawn(async move {
            run_manager_workflow(manager, options, tx, print_output).await
        }));
    }

    let mut runs = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(run) => runs.push(run),
            Err(err) => runs.push(ManagerRun {
                key: "unknown".to_string(),
                name: "unknown".to_string(),
                success: false,
                duration_ms: 0,
                error: Some(err.to_string()),
                output: Vec::new(),
            }),
        }
    }

    let success = runs.iter().all(|run| run.success);
    RunSummary {
        started_at,
        finished_at: unix_timestamp(),
        workflow: options.workflow,
        cleanup: options.cleanup,
        success,
        managers: runs,
    }
}

async fn run_manager_workflow(
    manager: Manager,
    options: RunOptions,
    tx: Option<mpsc::UnboundedSender<ManagerEvent>>,
    print_output: bool,
) -> ManagerRun {
    let start = Instant::now();
    let mut status = ManagerStatus::pending(manager.clone());
    status.started_at = Some(start);
    send_status(&tx, &status);

    let mut all_output = Vec::new();
    let mut error = None;

    let commands = workflow_commands(&manager, options);
    if commands.is_empty() {
        status.stage = Stage::Skipped;
        status.message = Some(format!(
            "No {} command configured",
            options.workflow.label()
        ));
        status.finished_at = Some(Instant::now());
        send_status(&tx, &status);
        return ManagerRun {
            key: manager.key,
            name: manager.config.name,
            success: true,
            duration_ms: start.elapsed().as_millis(),
            error: None,
            output: all_output,
        };
    }

    for (stage, command, allow_nonzero) in commands {
        status.stage = stage;
        status.message = Some(command.clone());
        send_status(&tx, &status);

        match run_command(
            &command,
            manager.shell,
            manager.timeout_seconds,
            tx.clone(),
            Some((&manager.key, &manager.config.name)),
            print_output,
            allow_nonzero,
        )
        .await
        {
            Ok(run) => {
                all_output.extend(run.output);
                if allow_nonzero && run.status_code != Some(0) {
                    let note = format!(
                        "Command exited with status {}; output may still contain useful results",
                        status_label(run.status_code)
                    );
                    all_output.push(note.clone());
                    if let Some(tx) = &tx {
                        let _ = tx.send(ManagerEvent::Output {
                            key: manager.key.clone(),
                            line: note,
                        });
                    }
                }
            }
            Err(err) => {
                error = Some(err.to_string());
                break;
            }
        }
    }

    status.stage = if error.is_some() {
        Stage::Failed
    } else {
        Stage::Complete
    };
    status.message = error.clone();
    status.finished_at = Some(Instant::now());
    send_status(&tx, &status);

    ManagerRun {
        key: manager.key,
        name: manager.config.name,
        success: error.is_none(),
        duration_ms: start.elapsed().as_millis(),
        error,
        output: all_output,
    }
}

fn workflow_commands(manager: &Manager, options: RunOptions) -> Vec<(Stage, String, bool)> {
    match options.workflow {
        Workflow::Check => manager
            .config
            .check_updates
            .clone()
            .map(|cmd| vec![(Stage::Checking, cmd, true)])
            .unwrap_or_default(),
        Workflow::Upgrade => {
            let mut commands = Vec::new();
            if let Some(refresh) = &manager.config.refresh {
                commands.push((Stage::Refreshing, refresh.clone(), false));
            }
            if let Some(upgrade) = &manager.config.upgrade_all {
                commands.push((Stage::Upgrading, upgrade.clone(), false));
            }
            if options.cleanup {
                if let Some(cleanup) = &manager.config.cleanup {
                    commands.push((Stage::Cleaning, cleanup.clone(), false));
                }
            }
            commands
        }
        Workflow::Cleanup => manager
            .config
            .cleanup
            .clone()
            .map(|cmd| vec![(Stage::Cleaning, cmd, false)])
            .unwrap_or_default(),
    }
}

async fn run_command(
    cmd: &str,
    shell: bool,
    timeout_seconds: u64,
    tx: Option<mpsc::UnboundedSender<ManagerEvent>>,
    manager: Option<(&str, &str)>,
    print_output: bool,
    allow_nonzero: bool,
) -> Result<CommandRun> {
    let mut command = build_command(cmd, shell)?;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to run command: {cmd}"))?;

    let key = manager.map(|(key, _)| key.to_string());
    let name = manager.map(|(_, name)| name.to_string());

    let stdout_task = child.stdout.take().map(|stdout| {
        tokio::spawn(read_stream(
            stdout,
            key.clone(),
            None,
            tx.clone(),
            name.clone(),
            print_output,
        ))
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(read_stream(
            stderr,
            key.clone(),
            Some("stderr"),
            tx.clone(),
            name.clone(),
            print_output,
        ))
    });

    let timeout = Duration::from_secs(timeout_seconds.max(1));
    let status = match time::timeout(timeout, child.wait()).await {
        Ok(status) => status.with_context(|| format!("Failed waiting for command: {cmd}"))?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(anyhow::anyhow!(
                "Command timed out after {} seconds: {}",
                timeout.as_secs(),
                cmd
            ));
        }
    };

    let mut output = Vec::new();
    if let Some(task) = stdout_task {
        if let Ok(lines) = task.await {
            output.extend(lines);
        }
    }
    if let Some(task) = stderr_task {
        if let Ok(lines) = task.await {
            output.extend(lines);
        }
    }

    if !status.success() && !allow_nonzero {
        return Err(anyhow::anyhow!(
            "Command failed with status {}: {}",
            status_label(status.code()),
            cmd
        ));
    }

    Ok(CommandRun {
        status_code: status.code(),
        output,
    })
}

fn build_command(cmd: &str, shell: bool) -> Result<Command> {
    let mut command = if shell {
        let mut command = Command::new("sh");
        command.arg("-c").arg(cmd);
        command
    } else {
        let mut parts = shell_words::split(cmd)?.into_iter();
        let Some(program) = parts.next() else {
            return Err(anyhow::anyhow!("Empty command"));
        };
        let mut command = Command::new(program);
        command.args(parts);
        command
    };
    // Aborting the runner (e.g. quitting the TUI mid-upgrade) drops the
    // child handles; without kill_on_drop the spawned managers would keep
    // running as orphans.
    command.kill_on_drop(true);
    Ok(command)
}

async fn read_stream<R>(
    reader: R,
    key: Option<String>,
    stream_label: Option<&'static str>,
    tx: Option<mpsc::UnboundedSender<ManagerEvent>>,
    manager_name: Option<String>,
    print_output: bool,
) -> Vec<String>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut lines = BufReader::new(reader).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let rendered = if let Some(label) = stream_label {
                    format!("{label}: {line}")
                } else {
                    line
                };

                if print_output {
                    if let Some(name) = &manager_name {
                        println!("[{name}] {rendered}");
                    } else {
                        println!("{rendered}");
                    }
                }

                if let (Some(tx), Some(key)) = (&tx, &key) {
                    let _ = tx.send(ManagerEvent::Output {
                        key: key.clone(),
                        line: rendered.clone(),
                    });
                }

                output.push(rendered);
            }
            Ok(None) => break,
            Err(err) => {
                let rendered = format!("failed to read command output: {err}");
                output.push(rendered.clone());
                if let (Some(tx), Some(key)) = (&tx, &key) {
                    let _ = tx.send(ManagerEvent::Output {
                        key: key.clone(),
                        line: rendered,
                    });
                }
                break;
            }
        }
    }

    output
}

fn send_status(tx: &Option<mpsc::UnboundedSender<ManagerEvent>>, status: &ManagerStatus) {
    if let Some(tx) = tx {
        let _ = tx.send(ManagerEvent::Status(status.clone()));
    }
}

pub fn needs_sudo(managers: &[Manager], workflow: Workflow, cleanup: bool) -> bool {
    let needs_privileged_command = match workflow {
        Workflow::Check => false,
        Workflow::Upgrade => true,
        Workflow::Cleanup => true,
    };

    needs_privileged_command
        && managers.iter().any(|manager| {
            manager.config.requires_sudo.unwrap_or(false)
                && (cleanup
                    || workflow != Workflow::Upgrade
                    || manager.config.upgrade_all.is_some())
        })
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
    print_config_line(active_config);
    for manager in managers {
        println!(
            "{} ({}) timeout={}s shell={}",
            manager.config.name, manager.key, manager.timeout_seconds, manager.shell
        );
    }
}

pub fn print_plan(
    managers: &[Manager],
    active_config: Option<&std::path::Path>,
    options: RunOptions,
) {
    print_config_line(active_config);
    println!("Workflow: {}", options.workflow.label());
    println!("Cleanup after upgrade: {}", options.cleanup);
    println!("Continue on error: {}", options.continue_on_error);
    println!(
        "Sudo required: {}",
        needs_sudo(managers, options.workflow, options.cleanup)
    );
    println!();

    for manager in managers {
        println!("{} ({})", manager.config.name, manager.key);
        println!("  check command: {}", manager.config.check_command);
        println!("  timeout: {}s", manager.timeout_seconds);
        println!("  shell: {}", manager.shell);
        println!(
            "  requires sudo: {}",
            manager.config.requires_sudo.unwrap_or(false)
        );
        for (stage, command, _) in workflow_commands(manager, options) {
            println!("  {}: {}", stage_label(stage), command);
        }
        if workflow_commands(manager, options).is_empty() {
            println!("  no {} command configured", options.workflow.label());
        }
        println!();
    }
}

pub async fn print_doctor(config: &Config, managers: &[Manager]) -> Result<()> {
    println!("Active config:");
    if let Some(path) = config.active_path() {
        println!("  {}", path.display());
    } else {
        println!("  default (built-in)");
    }

    println!("\nConfig search paths:");
    for path in config_search_paths() {
        println!(
            "  {}{}",
            path.display(),
            if path.exists() { " ✓" } else { "" }
        );
    }

    println!("\nSettings:");
    println!("  timeout_seconds = {}", config.settings.timeout_seconds);
    println!(
        "  continue_on_error = {}",
        config.settings.continue_on_error
    );
    println!(
        "  cleanup_after_upgrade = {}",
        config.settings.cleanup_after_upgrade
    );
    println!("  shell = {}", config.settings.shell);

    let warnings = config.validate();
    println!("\nConfig validation:");
    if warnings.is_empty() {
        println!("  ok");
    } else {
        for warning in warnings {
            println!("  warning: {warning}");
        }
    }

    println!("\nPrivileges:");
    let sudo_status = std::process::Command::new("sudo")
        .arg("-n")
        .arg("-v")
        .status();
    match sudo_status {
        Ok(status) if status.success() => {
            println!("  sudo: available (cached credentials or passwordless)");
        }
        Ok(_) => println!("  sudo: installed (password will be required for upgrades)"),
        Err(_) => println!("  sudo: NOT FOUND — privileged workflows will fail"),
    }

    println!("\nManagers:");
    println!("  configured: {}", config.managers.len());
    println!("  detected: {}", managers.len());
    for (key, manager) in &config.managers {
        let detected = managers.iter().any(|detected| detected.key == *key);
        let enabled = if manager.enabled {
            "enabled"
        } else {
            "disabled"
        };
        let status = if detected { "detected" } else { "not detected" };
        println!("  {key}: {} ({enabled}, {status})", manager.name);
    }

    println!("\nProfiles:");
    if config.profiles.is_empty() {
        println!("  none");
    } else {
        for (name, profile) in &config.profiles {
            println!("  {name}: only={:?} skip={:?}", profile.only, profile.skip);
        }
    }

    Ok(())
}

pub fn print_history(last: bool) -> Result<()> {
    let dir = history_dir();
    if !dir.exists() {
        println!("No history found at {}", dir.display());
        return Ok(());
    }

    let mut entries = fs::read_dir(&dir)
        .with_context(|| format!("Failed to read history directory {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    entries.sort();

    if entries.is_empty() {
        println!("No history found at {}", dir.display());
        return Ok(());
    }

    let selected: Vec<PathBuf> = if last {
        entries.into_iter().rev().take(1).collect()
    } else {
        entries.into_iter().rev().take(20).collect()
    };

    for path in selected {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read history file {}", path.display()))?;
        let summary: RunSummary = serde_json::from_str(&content)
            .with_context(|| format!("Invalid history JSON in {}", path.display()))?;
        println!(
            "{} workflow={} cleanup={} success={} managers={}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("history"),
            summary.workflow.label(),
            summary.cleanup,
            summary.success,
            summary.managers.len()
        );
        for manager in summary.managers {
            println!(
                "  {} ({}) {} {}ms{}",
                manager.name,
                manager.key,
                if manager.success { "ok" } else { "failed" },
                manager.duration_ms,
                manager
                    .error
                    .map(|error| format!(" - {error}"))
                    .unwrap_or_default()
            );
        }
    }

    Ok(())
}

fn write_history(summary: &RunSummary) -> Result<PathBuf> {
    let dir = history_dir();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create history directory {}", dir.display()))?;
    let path = dir.join(format!("run-{}.json", unix_timestamp_millis()));
    let content = serde_json::to_string_pretty(summary)?;
    fs::write(&path, content)
        .with_context(|| format!("Failed to write history to {}", path.display()))?;
    Ok(path)
}

fn history_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".local")
            .join("state")
            .join("spine")
            .join("history")
    } else {
        PathBuf::from(".spine").join("history")
    }
}

fn print_config_line(active_config: Option<&std::path::Path>) {
    if let Some(path) = active_config {
        println!("Config: {}", path.display());
    } else {
        println!("Config: default (built-in)");
    }
}

pub fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Pending => "pending",
        Stage::Checking => "checking",
        Stage::Refreshing => "refresh",
        Stage::Upgrading => "upgrade",
        Stage::Cleaning => "cleanup",
        Stage::Complete => "complete",
        Stage::Failed => "failed",
        Stage::Skipped => "skipped",
    }
}

fn status_label(code: Option<i32>) -> String {
    code.map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ManagerConfig;

    fn manager(requires_sudo: Option<bool>, has_upgrade: bool) -> Manager {
        Manager {
            key: "m".into(),
            config: ManagerConfig {
                name: "M".into(),
                check_command: "m --version".into(),
                enabled: true,
                refresh: None,
                check_updates: Some("m outdated".into()),
                upgrade_all: has_upgrade.then(|| "m upgrade".into()),
                cleanup: None,
                requires_sudo,
                timeout_seconds: None,
                shell: None,
            },
            timeout_seconds: 3600,
            shell: true,
        }
    }

    #[test]
    fn check_workflow_never_needs_sudo() {
        let managers = vec![manager(Some(true), true)];
        assert!(!needs_sudo(&managers, Workflow::Check, false));
    }

    #[test]
    fn upgrade_needs_sudo_only_when_required_and_possible() {
        assert!(needs_sudo(
            &[manager(Some(true), true)],
            Workflow::Upgrade,
            false
        ));
        assert!(!needs_sudo(
            &[manager(Some(false), true)],
            Workflow::Upgrade,
            false
        ));
        assert!(
            !needs_sudo(&[manager(Some(true), false)], Workflow::Upgrade, false),
            "no upgrade command means nothing privileged to run"
        );
    }

    #[test]
    fn cleanup_needs_sudo_for_privileged_managers() {
        assert!(needs_sudo(
            &[manager(Some(true), false)],
            Workflow::Cleanup,
            false
        ));
        assert!(!needs_sudo(
            &[manager(Some(false), false)],
            Workflow::Cleanup,
            false
        ));
    }

    #[test]
    fn workflow_labels_are_stable() {
        assert_eq!(Workflow::Check.label(), "check");
        assert_eq!(Workflow::Upgrade.label(), "upgrade");
        assert_eq!(Workflow::Cleanup.label(), "cleanup");
    }

    #[test]
    fn stage_labels_are_stable() {
        assert_eq!(stage_label(Stage::Pending), "pending");
        assert_eq!(stage_label(Stage::Refreshing), "refresh");
        assert_eq!(stage_label(Stage::Complete), "complete");
        assert_eq!(stage_label(Stage::Failed), "failed");
    }

    #[tokio::test]
    async fn check_command_detects_success_and_failure() {
        assert!(check_command("true", true, 5).await.unwrap());
        assert!(!check_command("false", true, 5).await.unwrap());
    }

    #[tokio::test]
    async fn check_command_timeout_reports_not_installed() {
        assert!(!check_command("sleep 3", true, 1).await.unwrap());
    }

    #[test]
    fn build_command_splits_unquoted_commands() {
        let command = build_command("brew upgrade --greedy", false).unwrap();
        assert_eq!(command.as_std().get_program(), "brew");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            ["upgrade", "--greedy"]
        );
    }

    #[test]
    fn build_command_shell_mode_uses_sh_c() {
        let command = build_command("echo hi", true).unwrap();
        assert_eq!(command.as_std().get_program(), "sh");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            ["-c", "echo hi"]
        );
    }

    #[test]
    fn build_command_rejects_empty_command() {
        assert!(build_command("   ", false).is_err());
    }
}
