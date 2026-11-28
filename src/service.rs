use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::platform;

const SERVICE_NAME: &str = "FileHub";
const SERVICE_DISPLAY_NAME: &str = "FileHub";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

pub fn run_dispatcher() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|err| anyhow::anyhow!("failed to start service dispatcher: {err}"))
}

#[allow(clippy::print_stdout)]
pub fn install(config_path: Option<&std::path::Path>, start: bool) -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("failed to locate FileHub executable")?;
    install_at(&executable, config_path, start)
}

/// Install the service for a specific executable path.
///
/// The wizard copies `filehub.exe` to a stable location first, so the service
/// must not point at the executable it happened to be launched from.
#[allow(clippy::print_stdout)]
pub fn install_at(
    executable: &std::path::Path,
    config_path: Option<&std::path::Path>,
    start: bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(platform::default_data_dir())
        .context("failed to create ProgramData directory")?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|err| anyhow::anyhow!("failed to connect service manager: {err}"))?;
    if let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        if service
            .query_status()
            .map_err(|err| anyhow::anyhow!("failed to query existing service: {err}"))?
            .current_state
            != ServiceState::Stopped
        {
            anyhow::bail!("FileHub service is running; stop it before reinstalling");
        }
        service
            .delete()
            .map_err(|err| anyhow::anyhow!("failed to replace existing service: {err}"))?;
        std::thread::sleep(Duration::from_millis(300));
    }
    let mut launch_arguments = vec![OsString::from("--service")];
    if let Some(path) = config_path {
        launch_arguments.push(OsString::from("--config"));
        launch_arguments.push(path.as_os_str().to_owned());
    }
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: executable.to_path_buf(),
        launch_arguments,
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS;
    let mut created = None;
    let mut last_error = None;
    for attempt in 0..5 {
        match manager.create_service(&info, access) {
            Ok(service) => {
                created = Some(service);
                break;
            }
            Err(error) => {
                last_error = Some(error);
                if attempt < 4 {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        }
    }
    let service = match created {
        Some(service) => service,
        None => {
            let detail = last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".to_owned());
            anyhow::bail!("failed to create service: {detail}");
        }
    };
    service
        .set_description("FileHub self-hosted file manager")
        .map_err(|err| anyhow::anyhow!("failed to set service description: {err}"))?;
    if start {
        service
            .start::<&str>(&[])
            .map_err(|err| anyhow::anyhow!("failed to start service: {err}"))?;
        wait_for_state(&service, ServiceState::Running)?;
        println!("FileHub service installed and started.");
    } else {
        println!("FileHub service installed. Start it with: sc.exe start FileHub");
    }
    Ok(())
}

/// Current service state, or `None` when the service is not installed.
pub fn query() -> anyhow::Result<Option<ServiceState>> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|err| anyhow::anyhow!("failed to connect service manager: {err}"))?;
    match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(service) => Ok(Some(
            service
                .query_status()
                .map_err(|err| anyhow::anyhow!("failed to query service: {err}"))?
                .current_state,
        )),
        Err(_) => Ok(None),
    }
}

/// Start an installed service and wait until it reports running.
pub fn start_existing() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|err| anyhow::anyhow!("failed to connect service manager: {err}"))?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::START,
        )
        .map_err(|err| anyhow::anyhow!("failed to open FileHub service: {err}"))?;
    if service
        .query_status()
        .map_err(|err| anyhow::anyhow!("failed to query FileHub service: {err}"))?
        .current_state
        != ServiceState::Running
    {
        service
            .start::<&str>(&[])
            .map_err(|err| anyhow::anyhow!("failed to start service: {err}"))?;
        wait_for_state(&service, ServiceState::Running)?;
    }
    Ok(())
}

fn wait_for_state(
    service: &windows_service::service::Service,
    target: ServiceState,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        if service
            .query_status()
            .map_err(|err| anyhow::anyhow!("failed to query service status: {err}"))?
            .current_state
            == target
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    anyhow::bail!("service did not reach {target:?}");
}

#[allow(clippy::print_stdout)]
pub fn uninstall() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|err| anyhow::anyhow!("failed to connect service manager: {err}"))?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .map_err(|err| anyhow::anyhow!("failed to open FileHub service: {err}"))?;
    let status = service
        .query_status()
        .map_err(|err| anyhow::anyhow!("failed to query FileHub service: {err}"))?;
    if status.current_state != ServiceState::Stopped {
        service
            .stop()
            .map_err(|err| anyhow::anyhow!("failed to stop FileHub service: {err}"))?;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(200));
            if service
                .query_status()
                .map(|current| current.current_state == ServiceState::Stopped)
                .unwrap_or(false)
            {
                break;
            }
        }
    }
    service
        .delete()
        .map_err(|err| anyhow::anyhow!("failed to delete FileHub service: {err}"))?;
    println!("FileHub service uninstalled.");
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(arguments: Vec<OsString>) {
    if let Err(error) = run_service(&arguments) {
        let _ = std::fs::write(
            platform::default_data_dir().join("service-error.log"),
            format!("{error:#}\n"),
        );
    }
}

fn run_service(arguments: &[OsString]) -> anyhow::Result<()> {
    init_service_logging();
    let (stop_tx, stop_rx) = mpsc::channel();
    let handler = move |control| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status = service_control_handler::register(SERVICE_NAME, handler)
        .map_err(|err| anyhow::anyhow!("failed to register service handler: {err}"))?;
    let status = Arc::new(status);
    let stopping = Arc::new(AtomicBool::new(false));
    status
        .set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        })
        .map_err(|err| anyhow::anyhow!("failed to report start pending: {err}"))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    let result = runtime.block_on(async {
        status
            .set_service_status(ServiceStatus {
                service_type: SERVICE_TYPE,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
            .map_err(|err| anyhow::anyhow!("failed to report running: {err}"))?;
        let config_path = arguments
            .windows(2)
            .find(|pair| pair[0] == "--config")
            .map(|pair| std::path::PathBuf::from(&pair[1]));
        let state = crate::state::AppState::new(config_path.as_deref()).await?;
        let status_for_shutdown = Arc::clone(&status);
        let stopping_for_shutdown = Arc::clone(&stopping);
        let shutdown = async move {
            let _ = tokio::task::spawn_blocking(move || stop_rx.recv()).await;
            stopping_for_shutdown.store(true, Ordering::Relaxed);
            let _ = status_for_shutdown.set_service_status(ServiceStatus {
                service_type: SERVICE_TYPE,
                current_state: ServiceState::StopPending,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 1,
                wait_hint: Duration::from_secs(10),
                process_id: None,
            });
        };
        let run = crate::app::run_until(state, shutdown);
        tokio::pin!(run);
        let mut checkpoint = 1_u32;
        loop {
            tokio::select! {
                result = &mut run => break result,
                _ = tokio::time::sleep(Duration::from_secs(8)) => {
                    if stopping.load(Ordering::Relaxed) {
                        checkpoint = checkpoint.saturating_add(1);
                        let _ = status.set_service_status(ServiceStatus {
                            service_type: SERVICE_TYPE,
                            current_state: ServiceState::StopPending,
                            controls_accepted: ServiceControlAccept::empty(),
                            exit_code: ServiceExitCode::Win32(0),
                            checkpoint,
                            wait_hint: Duration::from_secs(10),
                            process_id: None,
                        });
                    }
                }
            }
        }
    });
    let _ = status.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });
    result
}

fn init_service_logging() {
    let dir = platform::default_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let appender = tracing_appender::rolling::daily(dir, "filehub.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .try_init();
    std::mem::forget(guard);
}
