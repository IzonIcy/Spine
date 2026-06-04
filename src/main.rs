mod config;
mod detect;
mod execute;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;

#[derive(Parser, Debug)]
#[command(name = "spn", version, about = "Meta package manager for *nix systems")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long)]
    no_tui: bool,

    #[arg(long)]
    dry_run: bool,

    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    #[arg(long, value_delimiter = ',')]
    skip: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Cli,
    Upgrade,
    List,
    Doctor,
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    Init,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    let detected = detect::discover(&config).await?;
    let filtered = detect::filter_managers(detected, &cli.only, &cli.skip);
    let needs_sudo = execute::needs_sudo(&filtered);
    if needs_sudo && !cli.dry_run {
        execute::prime_sudo().await?;
    }

    match cli.command {
        Some(Commands::List) => {
            execute::print_list(&filtered, config.active_path());
        }
        Some(Commands::Doctor) => {
            execute::print_doctor(&config, &filtered)?;
        }
        Some(Commands::Config { command: ConfigCommands::Init }) => {
            config::write_default()?;
            println!("Wrote default config to user config directory");
        }
        Some(Commands::Cli) | Some(Commands::Upgrade) | None => {
            let use_tui = !cli.no_tui && !matches!(cli.command, Some(Commands::Cli)) && !cli.dry_run;
            if cli.dry_run {
                execute::print_dry_run(&filtered, config.active_path());
            } else if use_tui {
                tui::run(filtered).await?;
            } else {
                execute::run_cli(filtered).await?;
            }
        }
    }

    Ok(())
}
