use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use spine::config::Config;
use spine::execute::{RunOptions, Workflow};
use spine::{config, detect, execute, schedule, tui};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "spn",
    version,
    about = "Meta package manager for most *nix systems"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, global = true)]
    no_tui: bool,

    #[arg(
        long,
        global = true,
        help = "Preview the selected workflow without running commands"
    )]
    dry_run: bool,

    #[arg(long, global = true, value_delimiter = ',')]
    only: Vec<String>,

    #[arg(long, global = true, value_delimiter = ',')]
    skip: Vec<String>,

    #[arg(long, global = true, help = "Use a named profile from backbone.toml")]
    profile: Option<String>,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Load this exact config file instead of the search paths (cwd is never searched)"
    )]
    config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Run cleanup commands after an upgrade workflow"
    )]
    cleanup: bool,

    #[arg(
        long,
        global = true,
        help = "Do not return an error if one manager fails"
    )]
    continue_on_error: bool,

    #[arg(
        long,
        global = true,
        help = "Send a desktop notification when the workflow completes"
    )]
    notify: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Cli,
    Upgrade,
    Check,
    Cleanup,
    List,
    Doctor,
    Schedule {
        #[arg(long, help = "Run every day instead of every Monday")]
        daily: bool,

        #[arg(long, default_value = "09:00", help = "Time of day to run (HH:MM)")]
        at: String,

        #[arg(long, help = "Write the plist to this path instead of stdout")]
        out: Option<PathBuf>,
    },
    History {
        #[command(subcommand)]
        command: Option<HistoryCommands>,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
enum HistoryCommands {
    Last,
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    Init,
    Path,
    Edit,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::History { command }) = &cli.command {
        execute::print_history(matches!(command, Some(HistoryCommands::Last)))?;
        return Ok(());
    }

    if let Some(Commands::Schedule { daily, at, out }) = &cli.command {
        let written = schedule::emit(*daily, at, out.as_deref())?;
        if out.is_some() {
            println!("Wrote launchd plist to {}", written.display());
            println!(
                "Load it with: launchctl bootstrap gui/$(id -u) {}",
                written.display()
            );
        }
        return Ok(());
    }

    if matches!(
        cli.command,
        Some(Commands::Config {
            command: ConfigCommands::Init
        })
    ) {
        let path = config::write_default()?;
        println!("Wrote default config to {}", path.display());
        return Ok(());
    }

    let config = match &cli.config {
        Some(path) => Config::load_from(path)?,
        None => Config::load()?,
    };

    match &cli.command {
        Some(Commands::Config {
            command: ConfigCommands::Path,
        }) => {
            if let Some(path) = config.active_path() {
                println!("{}", path.display());
            } else {
                println!(
                    "default (built-in); user config path: {}",
                    Config::default_user_path().display()
                );
            }
            return Ok(());
        }
        Some(Commands::Config {
            command: ConfigCommands::Edit,
        }) => {
            let path = config::edit_config(&config)?;
            println!("Edited config at {}", path.display());
            return Ok(());
        }
        _ => {}
    }

    let (only, skip) = selected_filters(&cli, &config)?;
    let detected = detect::discover(&config).await?;
    let filtered = detect::filter_managers(detected, &only, &skip);

    match &cli.command {
        Some(Commands::List) => {
            execute::print_list(&filtered, config.active_path());
        }
        Some(Commands::Doctor) => {
            execute::print_doctor(&config, &filtered).await?;
        }
        Some(Commands::Config { .. })
        | Some(Commands::History { .. })
        | Some(Commands::Schedule { .. }) => unreachable!(),
        Some(Commands::Cli)
        | Some(Commands::Upgrade)
        | Some(Commands::Check)
        | Some(Commands::Cleanup)
        | None => {
            let workflow = workflow_for_command(&cli.command);
            let options = RunOptions {
                workflow,
                cleanup: cleanup_enabled(&cli, &config, workflow),
                continue_on_error: cli.continue_on_error || config.settings.continue_on_error,
                notify: cli.notify || config.settings.notify,
            };

            if cli.dry_run {
                execute::print_plan(&filtered, config.active_path(), options);
                return Ok(());
            }

            if execute::needs_sudo(&filtered, options.workflow, options.cleanup) {
                execute::prime_sudo().await?;
            }

            let force_cli = matches!(cli.command, Some(Commands::Cli) | Some(Commands::Check));
            let use_tui = !cli.no_tui && !force_cli;
            if use_tui {
                tui::run(filtered, options).await?;
            } else {
                execute::run_cli(filtered, options).await?;
            }
        }
    }

    Ok(())
}

fn workflow_for_command(command: &Option<Commands>) -> Workflow {
    match command {
        Some(Commands::Check) => Workflow::Check,
        Some(Commands::Cleanup) => Workflow::Cleanup,
        _ => Workflow::Upgrade,
    }
}

fn cleanup_enabled(cli: &Cli, config: &Config, workflow: Workflow) -> bool {
    match workflow {
        Workflow::Cleanup => true,
        Workflow::Upgrade => cli.cleanup || config.settings.cleanup_after_upgrade,
        Workflow::Check => false,
    }
}

fn selected_filters(cli: &Cli, config: &Config) -> Result<(Vec<String>, Vec<String>)> {
    let mut only = Vec::new();
    let mut skip = Vec::new();

    if let Some(profile_name) = &cli.profile {
        let profile = config
            .profiles
            .get(profile_name)
            .with_context(|| format!("Unknown profile `{profile_name}`"))?;
        only = profile.only.clone();
        skip = profile.skip.clone();
    }

    if !cli.only.is_empty() {
        only = cli.only.clone();
    }
    skip.extend(cli.skip.clone());

    Ok((only, skip))
}
