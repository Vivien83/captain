use super::{NodeNativeServiceError, NodeNativeServiceState};
use crate::{NODE_WINDOWS_DISPLAY_NAME, NODE_WINDOWS_SERVICE};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    thread,
    time::{Duration, Instant},
};
use windows_service::{
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
        ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
        ServiceState, ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const SERVICE_TRANSITION_TIMEOUT: Duration = Duration::from_secs(15);
const SERVICE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

pub(super) fn status() -> Result<NodeNativeServiceState, NodeNativeServiceError> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let Some(service) = open_service(&manager, ServiceAccess::QUERY_STATUS)? else {
        return Ok(NodeNativeServiceState::NotInstalled);
    };
    service_state(&service)
}

pub(super) fn install(
    binary: &Path,
    home: &Path,
    force: bool,
    account: &str,
    password: &str,
) -> Result<(), NodeNativeServiceError> {
    validate_account(account, password)?;
    let manager =
        service_manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::DELETE;
    let info = service_info(binary, home, account, password);
    let existing = open_service(&manager, access)?;
    let created = existing.is_none();
    let service = match existing {
        Some(_) if !force => return Err(NodeNativeServiceError::AlreadyInstalled),
        Some(service) => {
            service
                .change_config(&info)
                .map_err(|_| NodeNativeServiceError::ActionFailed)?;
            service
        }
        None => manager
            .create_service(&info, access)
            .map_err(|_| NodeNativeServiceError::ActionFailed)?,
    };

    let configured = service
        .set_description("Outbound-only local execution node for Captain")
        .and_then(|_| {
            service.update_failure_actions(ServiceFailureActions {
                reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
                reboot_msg: None,
                command: None,
                actions: Some(vec![
                    ServiceAction {
                        action_type: ServiceActionType::Restart,
                        delay: Duration::from_secs(5),
                    },
                    ServiceAction {
                        action_type: ServiceActionType::Restart,
                        delay: Duration::from_secs(15),
                    },
                    ServiceAction {
                        action_type: ServiceActionType::Restart,
                        delay: Duration::from_secs(60),
                    },
                ]),
            })
        });
    if configured.is_err() {
        if created {
            let _ = service.delete();
        }
        return Err(NodeNativeServiceError::ActionFailed);
    }
    if service_state(&service)? != NodeNativeServiceState::Running {
        if service.start::<&OsStr>(&[]).is_err()
            || wait_for_state(&service, ServiceState::Running).is_err()
        {
            if created {
                let _ = service.delete();
            }
            return Err(NodeNativeServiceError::ActionFailed);
        }
    }
    Ok(())
}

pub(super) fn uninstall() -> Result<(), NodeNativeServiceError> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let Some(service) = open_service(&manager, access)? else {
        return Ok(());
    };
    if service_state(&service)? == NodeNativeServiceState::Running {
        service
            .stop()
            .map_err(|_| NodeNativeServiceError::ActionFailed)?;
        wait_for_state(&service, ServiceState::Stopped)?;
    }
    service
        .delete()
        .map_err(|_| NodeNativeServiceError::ActionFailed)
}

pub(super) fn start() -> Result<(), NodeNativeServiceError> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = open_service(&manager, ServiceAccess::QUERY_STATUS | ServiceAccess::START)?
        .ok_or(NodeNativeServiceError::ActionFailed)?;
    if service_state(&service)? != NodeNativeServiceState::Running {
        service
            .start::<&OsStr>(&[])
            .map_err(|_| NodeNativeServiceError::ActionFailed)?;
        wait_for_state(&service, ServiceState::Running)?;
    }
    Ok(())
}

pub(super) fn stop() -> Result<(), NodeNativeServiceError> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = open_service(&manager, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)?
        .ok_or(NodeNativeServiceError::ActionFailed)?;
    if service_state(&service)? == NodeNativeServiceState::Running {
        service
            .stop()
            .map_err(|_| NodeNativeServiceError::ActionFailed)?;
        wait_for_state(&service, ServiceState::Stopped)?;
    }
    Ok(())
}

fn service_manager(access: ServiceManagerAccess) -> Result<ServiceManager, NodeNativeServiceError> {
    ServiceManager::local_computer(None::<&str>, access)
        .map_err(|_| NodeNativeServiceError::ManagerUnavailable)
}

fn open_service(
    manager: &ServiceManager,
    access: ServiceAccess,
) -> Result<Option<windows_service::service::Service>, NodeNativeServiceError> {
    match manager.open_service(NODE_WINDOWS_SERVICE, access) {
        Ok(service) => Ok(Some(service)),
        Err(error) if is_missing_service(&error) => Ok(None),
        Err(_) => Err(NodeNativeServiceError::ActionFailed),
    }
}

fn is_missing_service(error: &windows_service::Error) -> bool {
    matches!(
        error,
        windows_service::Error::Winapi(error)
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST)
    )
}

fn service_info(binary: &Path, home: &Path, account: &str, password: &str) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(NODE_WINDOWS_SERVICE),
        display_name: OsString::from(NODE_WINDOWS_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: binary.to_path_buf(),
        launch_arguments: vec![
            OsString::from("--home"),
            home.as_os_str().to_owned(),
            OsString::from("service-runtime"),
        ],
        dependencies: Vec::new(),
        account_name: Some(OsString::from(account)),
        account_password: Some(OsString::from(password)),
    }
}

fn service_state(
    service: &windows_service::service::Service,
) -> Result<NodeNativeServiceState, NodeNativeServiceError> {
    let state = service
        .query_status()
        .map_err(|_| NodeNativeServiceError::ActionFailed)?
        .current_state;
    Ok(match state {
        ServiceState::Running | ServiceState::StartPending | ServiceState::ContinuePending => {
            NodeNativeServiceState::Running
        }
        _ => NodeNativeServiceState::Stopped,
    })
}

fn wait_for_state(
    service: &windows_service::service::Service,
    expected: ServiceState,
) -> Result<(), NodeNativeServiceError> {
    let deadline = Instant::now() + SERVICE_TRANSITION_TIMEOUT;
    loop {
        let current = service
            .query_status()
            .map_err(|_| NodeNativeServiceError::ActionFailed)?
            .current_state;
        if current == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(NodeNativeServiceError::ActionFailed);
        }
        thread::sleep(SERVICE_POLL_INTERVAL);
    }
}

fn validate_account(account: &str, password: &str) -> Result<(), NodeNativeServiceError> {
    if account.trim().is_empty()
        || password.is_empty()
        || account.contains('\0')
        || password.contains('\0')
        || account.chars().any(char::is_control)
    {
        return Err(NodeNativeServiceError::WindowsCredentialsRequired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_service_requires_a_nonempty_user_credential() {
        assert!(validate_account("DOMAIN\\operator", "password").is_ok());
        assert!(validate_account("", "password").is_err());
        assert!(validate_account("DOMAIN\\operator", "").is_err());
        assert!(validate_account("DOMAIN\\bad\naccount", "password").is_err());
    }

    #[test]
    fn windows_service_keeps_runtime_arguments_structured() {
        let info = service_info(
            Path::new(r"C:\\Captain\\captain-node.exe"),
            Path::new(r"C:\\Captain Home"),
            r"DOMAIN\operator",
            "password",
        );
        assert_eq!(
            info.launch_arguments,
            vec![
                OsString::from("--home"),
                OsString::from(r"C:\\Captain Home"),
                OsString::from("service-runtime"),
            ]
        );
        assert_eq!(info.account_name, Some(OsString::from(r"DOMAIN\operator")));
    }

    #[test]
    fn only_the_windows_missing_service_code_maps_to_absence() {
        assert!(is_missing_service(&windows_service::Error::Winapi(
            std::io::Error::from_raw_os_error(ERROR_SERVICE_DOES_NOT_EXIST)
        )));
        assert!(!is_missing_service(&windows_service::Error::Winapi(
            std::io::Error::from_raw_os_error(5)
        )));
    }
}
