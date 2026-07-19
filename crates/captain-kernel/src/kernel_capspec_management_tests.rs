use super::*;
use async_trait::async_trait;
use captain_capspec::{
    CapabilityExecutionAuthority, CapabilityExecutionContext, CapabilityInvocation,
    CapabilityInvocationResult, CapabilityNodeStatus, CapabilityRunStatus, CapabilityToolInvoker,
};
use captain_types::agent::AgentMode;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::oneshot;

fn read_source(name: &str, version: &str) -> String {
    format!(
        r#"format = 1
name = "{name}"
description = "Read one project file."
version = "{version}"

[permissions]
tools = ["file_read"]
read_paths = ["/tmp/**"]

[[steps]]
id = "read"
tool = "file_read"
with = {{ path = "/tmp/input.txt" }}
"#
    )
}

fn test_kernel(temp: &TempDir) -> CaptainKernel {
    CaptainKernel::boot_with_config(KernelConfig {
        home_dir: temp.path().join("home"),
        data_dir: temp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    })
    .unwrap()
}

struct BlockingInvoker {
    started: Mutex<Option<oneshot::Sender<CapabilityInvocation>>>,
}

#[async_trait]
impl CapabilityToolInvoker for BlockingInvoker {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(invocation);
        }
        std::future::pending().await
    }
}

fn write_source(path: &Path) -> String {
    format!(
        r#"format = 1
name = "resume-writer"
description = "Write one file after an explicit uncertain decision."
version = "1.0.0"

[permissions]
tools = ["file_write"]
write_paths = ["{}"]

[[steps]]
id = "write"
tool = "file_write"
with = {{ path = "{}", content = "resumed safely" }}
"#,
        path.display(),
        path.display(),
    )
}

fn principal_id(kernel: &CaptainKernel) -> AgentId {
    kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name == PRINCIPAL_AGENT_NAME)
        .unwrap()
        .id
}

fn install_write_capability(kernel: &CaptainKernel, path: &Path) {
    let installed = kernel
        .capspec_management_install(
            &write_source(path),
            Some("resume-writer"),
            CapSpecForgeScope::Global,
            None,
            "test-operator",
        )
        .unwrap();
    let hash = installed["pending_hash"].as_str().unwrap();
    kernel
        .capspec_management_decide(
            "resume-writer",
            CapSpecForgeScope::Global,
            None,
            hash,
            true,
            "test-operator",
        )
        .unwrap();
}

#[tokio::test]
async fn telegram_approval_is_exact_audited_and_single_use() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let output = workspace.join("approved.txt");
    let kernel = Arc::new(test_kernel(&temp));
    let installed = kernel
        .capspec_management_install(
            &write_source(&output),
            Some("resume-writer"),
            CapSpecForgeScope::Global,
            None,
            "captain-agent",
        )
        .unwrap();
    assert_eq!(installed["status"], "pending_approval");

    let prompt = kernel
        .capspec_telegram_prompts()
        .unwrap()
        .into_iter()
        .find(|prompt| prompt.name == "resume-writer")
        .unwrap();
    assert_eq!(
        prompt.kind,
        crate::kernel::CapSpecTelegramPromptKind::Approval
    );
    assert_eq!(prompt.source_hash, installed["pending_hash"]);
    assert_eq!(prompt.token.len(), 20);
    assert!(prompt
        .authority
        .iter()
        .any(|line| line.contains("write paths")));

    kernel.shutdown();
    drop(kernel);
    let kernel = Arc::new(test_kernel(&temp));
    let restored = kernel
        .capspec_telegram_prompts()
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.name == "resume-writer")
        .unwrap();
    assert_eq!(restored.token, prompt.token);
    assert_eq!(restored.source_hash, prompt.source_hash);

    let callback = format!("capspec:approve:{}", prompt.token);
    let response = kernel
        .capspec_resolve_telegram_callback(&callback, "telegram:42")
        .await
        .unwrap();
    assert!(response.contains("approuvé"));
    assert_eq!(
        kernel
            .capspec_registry
            .active_by_tool("cap_resume_writer", None)
            .unwrap()
            .unwrap()
            .source_hash,
        prompt.source_hash
    );
    let stale = kernel
        .capspec_resolve_telegram_callback(&callback, "telegram:42")
        .await
        .unwrap_err();
    assert!(
        stale.contains("stale") || stale.contains("resolved"),
        "{stale}"
    );
    assert!(kernel.audit_log.recent(20).iter().any(|entry| {
        entry.agent_id == "telegram:42"
            && entry.detail.contains("Captain Forge Telegram approved")
            && entry.detail.contains(&prompt.source_hash)
    }));
    kernel.shutdown();
}

#[tokio::test]
async fn telegram_uncertain_failure_uses_exact_attempt_and_is_single_use() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let output = workspace.join("uncertain.txt");
    let kernel = Arc::new(test_kernel(&temp));
    kernel.set_self_handle();
    install_write_capability(&kernel, &output);
    let invocation = create_uncertain_write_run(&kernel, &workspace).await;
    let prompt = kernel
        .capspec_telegram_prompts()
        .unwrap()
        .into_iter()
        .find(|prompt| prompt.run_id.as_deref() == Some(&invocation.run_id))
        .unwrap();
    assert_eq!(
        prompt.kind,
        crate::kernel::CapSpecTelegramPromptKind::Uncertain
    );
    assert_eq!(prompt.attempt, Some(invocation.attempt));
    assert_eq!(
        prompt.tool_use_id.as_deref(),
        Some(invocation.tool_use_id.as_str())
    );

    let callback = format!("capspec:fail:{}", prompt.token);
    let response = kernel
        .capspec_resolve_telegram_callback(&callback, "telegram:42")
        .await
        .unwrap();
    assert!(response.contains("`échec` acceptée"));
    assert_eq!(
        kernel
            .capspec_executor
            .run(&invocation.run_id)
            .unwrap()
            .status,
        CapabilityRunStatus::Failed
    );
    assert!(!output.exists());
    assert!(kernel
        .capspec_resolve_telegram_callback(&callback, "telegram:42")
        .await
        .unwrap_err()
        .contains("resolved"));
    kernel.shutdown();
}

async fn create_uncertain_write_run(
    kernel: &Arc<CaptainKernel>,
    workspace: &Path,
) -> CapabilityInvocation {
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let executor = Arc::clone(&kernel.capspec_executor);
    let caller = principal_id(kernel).to_string();
    let workspace = workspace.to_string_lossy().into_owned();
    let task_invoker = Arc::clone(&invoker);
    let task = tokio::spawn(async move {
        executor
            .execute_tool(
                "cap_resume_writer",
                json!({}),
                CapabilityExecutionContext {
                    caller_agent_id: Some(caller),
                    workspace: Some(workspace),
                    origin: "test".to_string(),
                    authority: Some(CapabilityExecutionAuthority {
                        allowed_tools: vec!["file_write".to_string()],
                        ..CapabilityExecutionAuthority::default()
                    }),
                },
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = tokio::time::timeout(Duration::from_secs(3), started_rx)
        .await
        .expect("initial write dispatch timed out")
        .unwrap();
    task.abort();
    let _ = task.await;
    invocation
}

#[test]
fn only_the_principal_agent_can_persist_a_forge_proposal() {
    let temp = TempDir::new().unwrap();
    let kernel = test_kernel(&temp);
    let request = CapSpecForgeRequest {
        action: CapSpecForgeAction::Propose,
        scope: Some(CapSpecForgeScope::Global),
        name: Some("reader".to_string()),
        source: Some(read_source("reader", "1.0.0")),
        include_source: false,
    };

    let outsider = AgentId::new().to_string();
    let error = kernel
        .handle_capspec_forge(&request, None, Some(&outsider))
        .unwrap_err();
    assert!(error.contains("principal"), "{error}");

    let captain_id = kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name == PRINCIPAL_AGENT_NAME)
        .unwrap()
        .id
        .to_string();
    let installed = kernel
        .handle_capspec_forge(&request, None, Some(&captain_id))
        .unwrap();
    assert_eq!(installed["status"], "operational");
    assert_eq!(installed["ready"], true);
    kernel.shutdown();
}

#[test]
fn project_override_and_disable_match_the_effective_runtime_catalog() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let kernel = test_kernel(&temp);

    kernel
        .capspec_management_install(
            &read_source("reader", "1.0.0"),
            Some("reader"),
            CapSpecForgeScope::Global,
            None,
            "test-operator",
        )
        .unwrap();
    kernel
        .capspec_management_install(
            &read_source("reader", "2.0.0"),
            Some("reader"),
            CapSpecForgeScope::Project,
            Some(&workspace),
            "test-operator",
        )
        .unwrap();

    let project = kernel
        .capspec_management_inspect(
            "reader",
            CapSpecForgeScope::Effective,
            Some(&workspace),
            true,
        )
        .unwrap();
    assert_eq!(project["scope"], "project");
    assert_eq!(project["version"], "2.0.0");
    assert!(project["source"].as_str().unwrap().contains("2.0.0"));

    kernel
        .capspec_management_disable(
            "reader",
            CapSpecForgeScope::Project,
            Some(&workspace),
            "test-operator",
        )
        .unwrap();
    let global = kernel
        .capspec_management_inspect(
            "reader",
            CapSpecForgeScope::Effective,
            Some(&workspace),
            false,
        )
        .unwrap();
    assert_eq!(global["scope"], "global");
    assert_eq!(global["version"], "1.0.0");
    assert_eq!(
        kernel
            .capspec_registry
            .active_by_tool("cap_reader", Some(&workspace))
            .unwrap()
            .unwrap()
            .version,
        "1.0.0"
    );
    kernel.shutdown();
}

#[tokio::test]
async fn exact_operator_retry_resumes_through_the_real_toolrunner() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let output = workspace.join("resumed.txt");
    std::fs::create_dir_all(&workspace).unwrap();
    let kernel = Arc::new(test_kernel(&temp));
    kernel.set_self_handle();
    install_write_capability(&kernel, &output);
    let invocation = create_uncertain_write_run(&kernel, &workspace).await;
    let view = kernel.capspec_executor.run(&invocation.run_id).unwrap();
    assert_eq!(view.status, CapabilityRunStatus::WaitingDecision);
    assert_eq!(view.nodes[0].status, CapabilityNodeStatus::Uncertain);

    let response = kernel
        .capspec_management_resolve_run(
            &invocation.run_id,
            "write",
            UncertainNodeExpectation {
                tool_use_id: invocation.tool_use_id,
                attempt: invocation.attempt,
            },
            UncertainResolution::Retry,
            "test-operator",
        )
        .await
        .unwrap();
    assert_eq!(response["accepted"], true);
    assert_eq!(response["resume_scheduled"], true);

    let finished = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if output.is_file()
                && kernel
                    .capspec_executor
                    .run(&invocation.run_id)
                    .is_ok_and(|run| run.status == CapabilityRunStatus::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    assert!(
        finished.is_ok(),
        "resumed ToolRunner write did not finish: {:?}",
        kernel.capspec_executor.run(&invocation.run_id)
    );
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "resumed safely");
    assert!(kernel
        .audit_log
        .recent(20)
        .iter()
        .any(|entry| entry.detail.contains("uncertain-node-decision")));
    kernel.shutdown();
}

#[tokio::test]
async fn authority_revocation_blocks_resume_without_consuming_the_decision() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let output = workspace.join("revoked.txt");
    std::fs::create_dir_all(&workspace).unwrap();
    let kernel = Arc::new(test_kernel(&temp));
    kernel.set_self_handle();
    install_write_capability(&kernel, &output);
    let invocation = create_uncertain_write_run(&kernel, &workspace).await;
    kernel
        .registry
        .set_mode(principal_id(&kernel), AgentMode::Observe)
        .unwrap();

    let expectation = UncertainNodeExpectation {
        tool_use_id: invocation.tool_use_id,
        attempt: invocation.attempt,
    };
    let error = kernel
        .capspec_management_resolve_run(
            &invocation.run_id,
            "write",
            expectation.clone(),
            UncertainResolution::Retry,
            "test-operator",
        )
        .await
        .unwrap_err();
    assert!(error.contains("revoked"), "{error}");
    assert!(!output.exists());
    assert_eq!(
        kernel
            .capspec_executor
            .run(&invocation.run_id)
            .unwrap()
            .status,
        CapabilityRunStatus::WaitingDecision
    );

    let failed = kernel
        .capspec_management_resolve_run(
            &invocation.run_id,
            "write",
            expectation,
            UncertainResolution::MarkFailed {
                reason: "caller authority was revoked".to_string(),
            },
            "test-operator",
        )
        .await
        .unwrap();
    assert_eq!(failed["run"]["status"], "failed");
    assert_eq!(failed["resume_scheduled"], false);
    kernel.shutdown();
}
