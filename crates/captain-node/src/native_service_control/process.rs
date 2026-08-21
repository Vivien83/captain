use super::{
    platform::CommandSpec, storage::safe_definition_exists, NodeNativeServiceError,
    NodeNativeServiceState,
};
use std::{path::Path, process::Command};

pub(super) struct ServiceCommandOutput {
    pub(super) success: bool,
}

pub(super) trait ServiceCommandRunner: Send + Sync {
    fn run(&self, command: &CommandSpec) -> Result<ServiceCommandOutput, ()>;
}

pub(super) struct ProcessServiceCommandRunner;

impl ServiceCommandRunner for ProcessServiceCommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<ServiceCommandOutput, ()> {
        let status = Command::new(command.program)
            .args(&command.args)
            .status()
            .map_err(|_| ())?;
        Ok(ServiceCommandOutput {
            success: status.success(),
        })
    }
}

pub(super) fn run_required(
    runner: &dyn ServiceCommandRunner,
    command: &CommandSpec,
) -> Result<(), NodeNativeServiceError> {
    let output = runner
        .run(command)
        .map_err(|_| NodeNativeServiceError::ManagerUnavailable)?;
    if output.success {
        Ok(())
    } else {
        Err(NodeNativeServiceError::ActionFailed)
    }
}

pub(super) fn definition_state<F>(
    definition: &Path,
    query: F,
) -> Result<NodeNativeServiceState, NodeNativeServiceError>
where
    F: FnOnce() -> Result<ServiceCommandOutput, ()>,
{
    if !safe_definition_exists(definition)? {
        return Ok(NodeNativeServiceState::NotInstalled);
    }
    let output = query().map_err(|_| NodeNativeServiceError::ManagerUnavailable)?;
    Ok(if output.success {
        NodeNativeServiceState::Running
    } else {
        NodeNativeServiceState::Stopped
    })
}
