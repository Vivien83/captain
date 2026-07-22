use captain_memory::workflow_learning_control::{
    WorkflowArtifactKind, WorkflowProposalRecord, WorkflowProposalState,
};

use crate::workflow_learning_isolated_test::WorkflowIsolatedTestRunner;
use crate::workflow_learning_proposer::{
    ActiveModelIdentity, AutomationScheduleDraft, RefinementTargetKind, WorkflowDraft,
    WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_staging::{StageWorkflowDraftRequest, WorkflowStagingRoot};

fn model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn proposal_for(
    home: &std::path::Path,
    job_id: &str,
    draft: &WorkflowDraft,
) -> (WorkflowStagingRoot, WorkflowProposalRecord) {
    let staging = WorkflowStagingRoot::new(home).unwrap();
    let signature = "a".repeat(64);
    let receipt = staging
        .stage(StageWorkflowDraftRequest {
            job_id,
            workflow_signature: &signature,
            draft,
            active_model: &model(),
        })
        .unwrap();
    (
        staging,
        WorkflowProposalRecord {
            id: format!("proposal-{job_id}"),
            idempotency_key: format!("proposal-{job_id}:observed"),
            workflow_signature: signature,
            state: WorkflowProposalState::ApprovedPendingInstall,
            state_version: 5,
            revision_sha256: Some(receipt.revision_sha256.clone()),
            operator_token: Some(receipt.revision_sha256[..20].to_string()),
            artifact_sha256: Some(receipt.artifact_sha256),
            staging_job_id: Some(job_id.to_string()),
            kind: Some(match draft.kind {
                WorkflowDraftKind::Skill => WorkflowArtifactKind::Skill,
                WorkflowDraftKind::Capspec => WorkflowArtifactKind::Capspec,
                WorkflowDraftKind::Automation => WorkflowArtifactKind::Automation,
                WorkflowDraftKind::Refinement => WorkflowArtifactKind::Refinement,
            }),
            name: Some(draft.name.clone()),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: "{}".to_string(),
            validation_json: Some("{}".to_string()),
            isolated_test: None,
            snoozed_until_unix_ms: None,
            last_error_code: None,
            last_error_message: None,
            created_at_unix_ms: 1_000,
            updated_at_unix_ms: 2_000,
        },
    )
}

fn draft(kind: WorkflowDraftKind, name: &str, artifact: WorkflowDraftArtifact) -> WorkflowDraft {
    WorkflowDraft {
        schema_version: 1,
        kind,
        name: name.to_string(),
        purpose: "Exercise one repeatable workflow safely.".to_string(),
        trigger: "Use when the matching task is requested.".to_string(),
        artifact,
        required_capabilities: vec!["file_read".to_string()],
        expected_benefit: "Makes the workflow deterministic and auditable.".to_string(),
        limitations: Vec::new(),
    }
}

fn capspec_source(name: &str) -> String {
    format!(
        r#"format = 1
name = "{name}"
description = "Read one project summary."
version = "1.0.0"

[permissions]
tools = ["file_read"]
read_paths = ["{{{{input.path}}}}"]

[inputs.path]
type = "string"
description = "Summary path"

[[steps]]
id = "read"
tool = "file_read"
needs = []
with = {{ path = "{{{{input.path}}}}" }}
"#
    )
}

#[test]
fn isolated_skill_test_uses_private_native_registry_and_keeps_active_file_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let active = home.join("skills/learned/private-research.md");
    captain_types::durable_fs::create_dir_all(active.parent().unwrap()).unwrap();
    captain_types::durable_fs::atomic_write(&active, b"active-sentinel").unwrap();
    let draft = draft(
        WorkflowDraftKind::Skill,
        "private-research",
        WorkflowDraftArtifact::SkillMarkdown {
            source: "---\nname: private-research\ndescription: Private research\n---\n# Workflow\nRead and summarize sources.".to_string(),
        },
    );
    let (staging, proposal) = proposal_for(&home, "skill-test-stage", &draft);

    let report = WorkflowIsolatedTestRunner::new(staging)
        .run(&proposal, 3_000)
        .unwrap();

    assert!(report.passed, "{:#?}", report.checks);
    assert!(report
        .checks
        .iter()
        .any(|check| check.code == "native_skill_registry" && check.passed));
    assert_eq!(std::fs::read(&active).unwrap(), b"active-sentinel");
}

#[test]
fn isolated_capspec_test_compiles_in_a_private_registry() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let name = "read-project-summary";
    let draft = draft(
        WorkflowDraftKind::Capspec,
        name,
        WorkflowDraftArtifact::CapspecToml {
            source: capspec_source(name),
        },
    );
    let (staging, proposal) = proposal_for(&home, "capspec-test-stage", &draft);

    let report = WorkflowIsolatedTestRunner::new(staging)
        .run(&proposal, 3_000)
        .unwrap();

    assert!(report.passed, "{:#?}", report.checks);
    assert!(report
        .checks
        .iter()
        .any(|check| check.code == "native_capspec_registry" && check.passed));
    assert!(!home.join(format!("capabilities/{name}.captain")).exists());
}

#[test]
fn isolated_automation_test_validates_without_registering_a_cron() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = draft(
        WorkflowDraftKind::Automation,
        "daily-project-summary",
        WorkflowDraftArtifact::Automation {
            schedule: AutomationScheduleDraft::Cron {
                expression: "0 9 * * 1-5".to_string(),
                timezone: Some("Europe/Paris".to_string()),
            },
            instruction: "Summarize the current project status.".to_string(),
        },
    );
    let (staging, proposal) = proposal_for(&home, "automation-test-stage", &draft);

    let report = WorkflowIsolatedTestRunner::new(staging)
        .run(&proposal, 3_000)
        .unwrap();

    assert!(report.passed, "{:#?}", report.checks);
    assert!(report
        .checks
        .iter()
        .any(|check| check.code == "native_scheduler_contract" && check.passed));
    assert!(!home.join("cron/jobs.json").exists());
}

#[test]
fn isolated_refinement_test_uses_the_target_native_parser() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let target = "refined-research";
    let draft = draft(
        WorkflowDraftKind::Refinement,
        "improve-refined-research",
        WorkflowDraftArtifact::Refinement {
            target_kind: RefinementTargetKind::Skill,
            target_name: target.to_string(),
            source: "---\nname: refined-research\ndescription: Refined research\n---\n# Workflow\nCross-check every source.".to_string(),
        },
    );
    let (staging, proposal) = proposal_for(&home, "refinement-test-stage", &draft);

    let report = WorkflowIsolatedTestRunner::new(staging)
        .run(&proposal, 3_000)
        .unwrap();

    assert!(report.passed, "{:#?}", report.checks);
    assert!(report
        .checks
        .iter()
        .any(|check| check.code == "native_skill_registry" && check.passed));
}
