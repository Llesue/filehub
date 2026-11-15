#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use filehub::app;
use filehub::config;
use filehub::state::AppState;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "filehub", version, about = "Self-hosted file manager")]
struct Cli {
    /// Path to config.json
    #[arg(long, env = "FILEHUB_CONFIG")]
    config: Option<PathBuf>,

    /// Validate config and exit
    #[arg(long)]
    check_config: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Install or remove the Windows service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceAction {
    Install,
    Uninstall,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if cfg!(windows) && std::env::args().any(|arg| arg == "--service") {
        #[cfg(windows)]
        return filehub::service::run_dispatcher();
        #[cfg(not(windows))]
        anyhow::bail!("Windows service mode is only available on Windows");
    }
    let cli = Cli::parse();
    if let Some(Command::Service { action }) = &cli.command {
        #[cfg(windows)]
        {
            return match action {
                ServiceAction::Install => filehub::service::install(cli.config.as_deref()),
                ServiceAction::Uninstall => filehub::service::uninstall(),
            };
        }
        #[cfg(not(windows))]
        {
            let _ = action;
            anyhow::bail!("Windows service management is only available on Windows");
        }
    }
    let config_file = config::load_config_file(cli.config.as_deref())?;
    let bootstrap = config::build_snapshot(&config_file, &[])?;

    init_tracing(&bootstrap.settings.log.level)?;

    if cli.check_config {
        println!("config ok: listen={}", bootstrap.settings.listen);
        return Ok(());
    }

    let state = AppState::new(cli.config.as_deref()).await?;
    app::run(state).await
}

fn init_tracing(level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .context("invalid log filter")?;

    if std::env::var("RUST_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .try_init()
            .map_err(|err| anyhow::anyhow!(err))?;
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .try_init()
            .map_err(|err| anyhow::anyhow!(err))?;
    }

    Ok(())
}
