use captain_node::{node_shutdown_channel, NodeShutdown};
use std::{ffi::OsString, path::PathBuf, sync::OnceLock, time::Duration};
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

static SERVICE_HOME: OnceLock<PathBuf> = OnceLock::new();
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;
const TRANSITION_WAIT: Duration = Duration::from_secs(15);

define_windows_service!(ffi_service_main, service_main);

pub(crate) fn dispatch(home: PathBuf) -> Result<(), String> {
    SERVICE_HOME
        .set(home)
        .map_err(|_| "The Windows service runtime was initialized twice".to_string())?;
    service_dispatcher::start(captain_node::NODE_WINDOWS_SERVICE, ffi_service_main)
        .map_err(|_| "The Windows service dispatcher could not start".to_string())
}

fn service_main(_arguments: Vec<OsString>) {
    if run_service().is_err() {
        tracing::error!("Captain Node Windows service terminated with an internal error");
    }
}

fn run_service() -> windows_service::Result<()> {
    let Some(home) = SERVICE_HOME.get().cloned() else {
        tracing::error!("Captain Node Windows service home is unavailable");
        return Ok(());
    };
    let (shutdown_handle, shutdown) = node_shutdown_channel();
    let event_handler = move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            shutdown_handle.cancel();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status =
        service_control_handler::register(captain_node::NODE_WINDOWS_SERVICE, event_handler)?;
    set_status(
        &status,
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        1,
        TRANSITION_WAIT,
    )?;
    set_status(
        &status,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
        0,
        Duration::ZERO,
    )?;

    let succeeded = run_worker(&home, shutdown);
    set_status(
        &status,
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        1,
        TRANSITION_WAIT,
    )?;
    set_status(
        &status,
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        if succeeded {
            ServiceExitCode::Win32(0)
        } else {
            ServiceExitCode::Win32(1)
        },
        0,
        Duration::ZERO,
    )
}

fn run_worker(home: &std::path::Path, shutdown: NodeShutdown) -> bool {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return false,
    };
    match runtime.block_on(crate::runtime::run_node_service(home, shutdown)) {
        Ok(()) => true,
        Err(_) => {
            tracing::error!("Captain Node Windows service worker failed");
            false
        }
    }
}

fn set_status(
    handle: &ServiceStatusHandle,
    current_state: ServiceState,
    controls_accepted: ServiceControlAccept,
    exit_code: ServiceExitCode,
    checkpoint: u32,
    wait_hint: Duration,
) -> windows_service::Result<()> {
    handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state,
        controls_accepted,
        exit_code,
        checkpoint,
        wait_hint,
        process_id: None,
    })
}
