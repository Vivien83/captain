use super::*;
use crate::NODE_LAUNCHD_LABEL;
use process::ServiceCommandOutput;
use std::{collections::VecDeque, fs, sync::Mutex};

#[derive(Default)]
struct MockRunner {
    outputs: Mutex<VecDeque<Result<ServiceCommandOutput, ()>>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl MockRunner {
    fn with_outputs(outputs: Vec<ServiceCommandOutput>) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into_iter().map(Ok).collect()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ServiceCommandRunner for MockRunner {
    fn run(&self, command: &CommandSpec) -> Result<ServiceCommandOutput, ()> {
        self.calls.lock().unwrap().push((
            command.program.to_string(),
            command
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        ));
        self.outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(output(true)))
    }
}

fn output(success: bool) -> ServiceCommandOutput {
    ServiceCommandOutput { success }
}

#[test]
fn launchd_install_uses_structured_commands_and_private_atomic_definition() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("captain-node");
    fs::write(&binary, b"binary").unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let definition = temp.path().join("captain-node.plist");
    let runner = MockRunner::with_outputs(vec![
        output(false),
        output(true),
        output(true),
        output(true),
        output(true),
    ]);
    let controller = NativeNodeServiceController::test_controller(
        NativeServicePlatform::Launchd {
            domain: "gui/501".to_string(),
            target: format!("gui/501/{NODE_LAUNCHD_LABEL}"),
            definition: definition.clone(),
        },
        binary,
        home,
        runner.clone(),
    );

    let status = controller.install(false).unwrap();
    assert_eq!(status.state, NodeNativeServiceState::Running);
    let content = fs::read_to_string(&definition).unwrap();
    assert!(content.contains("service-runtime"));
    assert!(controller.home.join("node").join("logs").is_dir());
    let calls = runner.calls();
    assert!(calls.iter().any(|(program, args)| {
        program == "launchctl" && args.first().map(String::as_str) == Some("bootstrap")
    }));
    assert!(calls
        .iter()
        .all(|(_, args)| !args.join(" ").contains("sh -c")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&definition).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn unsafe_definition_symlink_is_rejected_without_touching_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    fs::write(&target, b"do-not-touch").unwrap();
    let definition = temp.path().join("captain-node.service");
    symlink(&target, &definition).unwrap();
    let error = install_definition(&definition, b"new", true).unwrap_err();
    assert_eq!(error, NodeNativeServiceError::UnsafeDefinition);
    assert_eq!(fs::read(&target).unwrap(), b"do-not-touch");
}

#[test]
fn failed_install_restores_previous_definition() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("captain-node");
    fs::write(&binary, b"binary").unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let definition = temp.path().join("captain-node.service");
    fs::write(&definition, b"previous").unwrap();
    let runner = MockRunner::with_outputs(vec![output(true), output(false), output(true)]);
    let controller = NativeNodeServiceController::test_controller(
        NativeServicePlatform::SystemdUser {
            definition: definition.clone(),
        },
        binary,
        home,
        runner,
    );

    assert_eq!(
        controller.install(true).unwrap_err(),
        NodeNativeServiceError::ActionFailed
    );
    assert_eq!(fs::read(&definition).unwrap(), b"previous");
}

#[test]
fn uninstall_requires_confirmation_before_any_command() {
    let runner = MockRunner::with_outputs(Vec::new());
    let controller = NativeNodeServiceController::test_controller(
        NativeServicePlatform::Windows,
        PathBuf::from(r"C:\Captain\captain-node.exe"),
        PathBuf::from(r"C:\CaptainHome"),
        runner.clone(),
    );
    assert_eq!(
        controller.uninstall(false).unwrap_err(),
        NodeNativeServiceError::ConfirmationRequired
    );
    assert!(runner.calls().is_empty());
}

#[test]
fn failed_active_service_stop_preserves_definition() {
    let temp = tempfile::tempdir().unwrap();
    let definition = temp.path().join("captain-node.plist");
    fs::write(&definition, b"installed").unwrap();
    let runner = MockRunner::with_outputs(vec![output(true), output(false)]);
    let controller = NativeNodeServiceController::test_controller(
        NativeServicePlatform::Launchd {
            domain: "gui/501".to_string(),
            target: format!("gui/501/{NODE_LAUNCHD_LABEL}"),
            definition: definition.clone(),
        },
        temp.path().join("captain-node"),
        temp.path().join("home"),
        runner,
    );

    assert_eq!(
        controller.uninstall(true).unwrap_err(),
        NodeNativeServiceError::ActionFailed
    );
    assert_eq!(fs::read(&definition).unwrap(), b"installed");
}
