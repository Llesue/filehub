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
    /// Install or remove the service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// First-run setup wizard (Windows)
    Setup {
        /// Accept the defaults without prompting
        #[arg(long)]
        yes: bool,
        /// Internal: elevated child process request file
        #[arg(long, hide = true)]
        elevated: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceAction {
    Install {
        /// Start the service after installing it
        #[arg(long)]
        start: bool,
    },
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

    #[cfg(windows)]
    match first_run_wizard() {
        FirstRun::Continue => {}
        FirstRun::Exit => return Ok(()),
        FirstRun::Foreground(path) => {
            let state = AppState::new(Some(&path)).await?;
            return app::run(state).await;
        }
    }

    let cli = Cli::parse();
    if let Some(Command::Service { action }) = &cli.command {
        #[cfg(windows)]
        {
            return match action {
                ServiceAction::Install { start } => {
                    filehub::service::install(cli.config.as_deref(), *start)
                }
                ServiceAction::Uninstall => filehub::service::uninstall(),
            };
        }
        #[cfg(not(windows))]
        {
            let _ = action;
            anyhow::bail!("Service management is only available on Windows");
        }
    }
    if let Some(Command::Setup { yes, elevated }) = &cli.command {
        #[cfg(windows)]
        {
            match filehub::setup::run_wizard(*yes, elevated.as_deref()) {
                Ok(filehub::setup::Outcome::Exit) => return Ok(()),
                Ok(filehub::setup::Outcome::Foreground(path)) => {
                    let state = AppState::new(Some(&path)).await?;
                    return app::run(state).await;
                }
                Err(error) => {
                    eprintln!("{error:#}");
                    filehub::setup::pause();
                    std::process::exit(1);
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (yes, elevated);
            anyhow::bail!("The setup wizard is only available on Windows");
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

#[cfg(windows)]
enum FirstRun {
    Continue,
    Exit,
    Foreground(PathBuf),
}

/// Double-clicking an unconfigured `filehub.exe` starts the wizard. With an
/// existing config, an already running service opens the browser; otherwise
/// the previous foreground behaviour is preserved.
#[cfg(windows)]
fn first_run_wizard() -> FirstRun {
    use std::io::IsTerminal;

    if std::env::args_os().len() != 1
        || std::env::var_os("FILEHUB_CONFIG").is_some()
        || !std::io::stdin().is_terminal()
    {
        return FirstRun::Continue;
    }

    if filehub::platform::default_config_path().exists() {
        if filehub::setup::open_existing_if_running() {
            return FirstRun::Exit;
        }
        return FirstRun::Continue;
    }

    match filehub::setup::run_wizard(false, None) {
        Ok(filehub::setup::Outcome::Exit) => FirstRun::Exit,
        Ok(filehub::setup::Outcome::Foreground(path)) => FirstRun::Foreground(path),
        Err(error) => {
            eprintln!("{error:#}");
            filehub::setup::pause();
            std::process::exit(1);
        }
    }
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
