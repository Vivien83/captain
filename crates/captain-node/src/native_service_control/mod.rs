//! Native service lifecycle for the standalone Captain Node.

mod platform;
mod process;
mod storage;
#[cfg(target_os = "windows")]
mod windows;

use crate::{launchd_plist_content, systemd_user_unit_content, NODE_SYSTEMD_SERVICE};
use platform::{CommandSpec, NativeServicePlatform};
use process::{definition_state, run_required, ServiceCommandRunner};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use storage::{
    ensure_runtime_directories, install_definition, path_arg, remove_definition,
    require_definition, restore_definition, safe_definition_exists, validate_runtime_path,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeNativeServiceState {
    NotInstalled,
    Stopped,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeNativeServiceStatus {
    pub manager: &'static str,
    pub state: NodeNativeServiceState,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NodeNativeServiceError {
    #[error("Native Captain Node services are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("The Captain Node service path is invalid")]
    InvalidPath,
    #[error("The Captain Node service definition already exists; use --force to replace it")]
    AlreadyInstalled,
    #[error("The existing Captain Node service definition is not a safe regular file")]
    UnsafeDefinition,
    #[error("The Captain Node service definition could not be persisted safely")]
    DefinitionUnavailable,
    #[error("The native service manager is unavailable")]
    ManagerUnavailable,
    #[error("The native service manager rejected the requested action")]
    ActionFailed,
    #[error("Captain Node service removal requires explicit confirmation")]
    ConfirmationRequired,
    #[error("Windows service installation requires the interactive user account credential")]
    WindowsCredentialsRequired,
}

#[derive(Clone)]
pub struct NativeNodeServiceController {
    platform: NativeServicePlatform,
    binary: PathBuf,
    home: PathBuf,
    runner: Arc<dyn ServiceCommandRunner>,
}

impl std::fmt::Debug for NativeNodeServiceController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeNodeServiceController")
            .field("manager", &self.platform.manager_name())
            .finish()
    }
}

impl NativeNodeServiceController {
    pub fn detect(binary: PathBuf, home: PathBuf) -> Result<Self, NodeNativeServiceError> {
        validate_runtime_path(&binary, true)?;
        validate_runtime_path(&home, false)?;
        let user_home = dirs::home_dir().ok_or(NodeNativeServiceError::InvalidPath)?;
        Ok(Self {
            platform: NativeServicePlatform::detect(&user_home)?,
            binary,
            home,
            runner: Arc::new(process::ProcessServiceCommandRunner),
        })
    }

    pub fn status(&self) -> Result<NodeNativeServiceStatus, NodeNativeServiceError> {
        let state = match &self.platform {
            NativeServicePlatform::Launchd {
                target, definition, ..
            } => definition_state(definition, || {
                self.runner
                    .run(&CommandSpec::new("launchctl").args(["print", target]))
            })?,
            NativeServicePlatform::SystemdUser { definition } => {
                definition_state(definition, || {
                    self.runner.run(&CommandSpec::new("systemctl").args([
                        "--user",
                        "is-active",
                        "--quiet",
                        NODE_SYSTEMD_SERVICE,
                    ]))
                })?
            }
            NativeServicePlatform::Windows => {
                #[cfg(target_os = "windows")]
                {
                    windows::status()?
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err(NodeNativeServiceError::UnsupportedPlatform);
                }
            }
        };
        Ok(NodeNativeServiceStatus {
            manager: self.platform.manager_name(),
            state,
        })
    }

    pub fn install(&self, force: bool) -> Result<NodeNativeServiceStatus, NodeNativeServiceError> {
        ensure_runtime_directories(&self.home)?;
        match &self.platform {
            NativeServicePlatform::Launchd {
                domain,
                target,
                definition,
            } => {
                let content = launchd_plist_content(&self.binary, &self.home)
                    .map_err(|_| NodeNativeServiceError::InvalidPath)?;
                let previous = install_definition(definition, content.as_bytes(), force)?;
                let result = self.install_launchd(domain, target, definition);
                if result.is_err() {
                    let _ = self
                        .runner
                        .run(&CommandSpec::new("launchctl").args(["bootout", target]));
                    restore_definition(definition, previous.as_deref())?;
                    if previous.is_some() {
                        let _ = self.install_launchd(domain, target, definition);
                    }
                }
                result?;
            }
            NativeServicePlatform::SystemdUser { definition } => {
                let content = systemd_user_unit_content(&self.binary, &self.home)
                    .map_err(|_| NodeNativeServiceError::InvalidPath)?;
                let previous = install_definition(definition, content.as_bytes(), force)?;
                let result = self.install_systemd();
                if result.is_err() {
                    let _ = self.runner.run(&CommandSpec::new("systemctl").args([
                        "--user",
                        "disable",
                        "--now",
                        NODE_SYSTEMD_SERVICE,
                    ]));
                    restore_definition(definition, previous.as_deref())?;
                    let _ = self
                        .runner
                        .run(&CommandSpec::new("systemctl").args(["--user", "daemon-reload"]));
                    if previous.is_some() {
                        let _ = self.install_systemd();
                    }
                }
                result?;
            }
            NativeServicePlatform::Windows => {
                let _ = force;
                return Err(NodeNativeServiceError::WindowsCredentialsRequired);
            }
        }
        self.status()
    }

    pub fn uninstall(
        &self,
        confirmed: bool,
    ) -> Result<NodeNativeServiceStatus, NodeNativeServiceError> {
        if !confirmed {
            return Err(NodeNativeServiceError::ConfirmationRequired);
        }
        match &self.platform {
            NativeServicePlatform::Launchd {
                target, definition, ..
            } => {
                if safe_definition_exists(definition)? {
                    if self.status()?.state == NodeNativeServiceState::Running {
                        run_required(
                            self.runner.as_ref(),
                            &CommandSpec::new("launchctl").args(["bootout", target]),
                        )?;
                    }
                    remove_definition(definition)?;
                }
            }
            NativeServicePlatform::SystemdUser { definition } => {
                if safe_definition_exists(definition)? {
                    run_required(
                        self.runner.as_ref(),
                        &CommandSpec::new("systemctl").args([
                            "--user",
                            "disable",
                            "--now",
                            NODE_SYSTEMD_SERVICE,
                        ]),
                    )?;
                    remove_definition(definition)?;
                    run_required(
                        self.runner.as_ref(),
                        &CommandSpec::new("systemctl").args(["--user", "daemon-reload"]),
                    )?;
                }
            }
            NativeServicePlatform::Windows => {
                #[cfg(target_os = "windows")]
                {
                    windows::uninstall()?;
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err(NodeNativeServiceError::UnsupportedPlatform);
                }
            }
        }
        Ok(NodeNativeServiceStatus {
            manager: self.platform.manager_name(),
            state: NodeNativeServiceState::NotInstalled,
        })
    }

    pub fn start(&self) -> Result<NodeNativeServiceStatus, NodeNativeServiceError> {
        match &self.platform {
            NativeServicePlatform::Launchd {
                domain,
                target,
                definition,
            } => {
                require_definition(definition)?;
                run_required(
                    self.runner.as_ref(),
                    &CommandSpec::new("launchctl").args([
                        "bootstrap",
                        domain,
                        &path_arg(definition)?,
                    ]),
                )?;
                run_required(
                    self.runner.as_ref(),
                    &CommandSpec::new("launchctl").args(["kickstart", "-k", target]),
                )?;
            }
            NativeServicePlatform::SystemdUser { definition } => {
                require_definition(definition)?;
                run_required(
                    self.runner.as_ref(),
                    &CommandSpec::new("systemctl").args(["--user", "start", NODE_SYSTEMD_SERVICE]),
                )?;
            }
            NativeServicePlatform::Windows => {
                #[cfg(target_os = "windows")]
                {
                    windows::start()?;
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err(NodeNativeServiceError::UnsupportedPlatform);
                }
            }
        }
        self.status()
    }

    pub fn stop(&self) -> Result<NodeNativeServiceStatus, NodeNativeServiceError> {
        match &self.platform {
            NativeServicePlatform::Launchd {
                target, definition, ..
            } => {
                require_definition(definition)?;
                run_required(
                    self.runner.as_ref(),
                    &CommandSpec::new("launchctl").args(["bootout", target]),
                )?;
            }
            NativeServicePlatform::SystemdUser { definition } => {
                require_definition(definition)?;
                run_required(
                    self.runner.as_ref(),
                    &CommandSpec::new("systemctl").args(["--user", "stop", NODE_SYSTEMD_SERVICE]),
                )?;
            }
            NativeServicePlatform::Windows => {
                #[cfg(target_os = "windows")]
                {
                    windows::stop()?;
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err(NodeNativeServiceError::UnsupportedPlatform);
                }
            }
        }
        self.status()
    }

    fn install_launchd(
        &self,
        domain: &str,
        target: &str,
        definition: &std::path::Path,
    ) -> Result<(), NodeNativeServiceError> {
        let _ = self
            .runner
            .run(&CommandSpec::new("launchctl").args(["bootout", target]));
        run_required(
            self.runner.as_ref(),
            &CommandSpec::new("launchctl").args(["bootstrap", domain, &path_arg(definition)?]),
        )?;
        run_required(
            self.runner.as_ref(),
            &CommandSpec::new("launchctl").args(["enable", target]),
        )?;
        run_required(
            self.runner.as_ref(),
            &CommandSpec::new("launchctl").args(["kickstart", "-k", target]),
        )
    }

    fn install_systemd(&self) -> Result<(), NodeNativeServiceError> {
        run_required(
            self.runner.as_ref(),
            &CommandSpec::new("systemctl").args(["--user", "daemon-reload"]),
        )?;
        run_required(
            self.runner.as_ref(),
            &CommandSpec::new("systemctl").args([
                "--user",
                "enable",
                "--now",
                NODE_SYSTEMD_SERVICE,
            ]),
        )
    }

    #[cfg(target_os = "windows")]
    pub fn install_windows_user(
        &self,
        force: bool,
        account: &str,
        password: &str,
    ) -> Result<NodeNativeServiceStatus, NodeNativeServiceError> {
        ensure_runtime_directories(&self.home)?;
        windows::install(&self.binary, &self.home, force, account, password)?;
        self.status()
    }

    #[cfg(test)]
    fn test_controller(
        platform: NativeServicePlatform,
        binary: PathBuf,
        home: PathBuf,
        runner: Arc<dyn ServiceCommandRunner>,
    ) -> Self {
        Self {
            platform,
            binary,
            home,
            runner,
        }
    }
}

#[cfg(test)]
mod tests;
