use std::fs;

use serde_json::json;

use crate::workflow_learning_promotion::WorkflowPromotionRoot;
use crate::workflow_learning_promotion_types::{
    PromoteWorkflowDraftRequest, WorkflowPromotionError, WorkflowPromotionPhase,
};
use crate::workflow_learning_proposer::{
    parse_workflow_draft, ActiveModelIdentity, WorkflowDraft, WorkflowDraftKind,
    WorkflowProposerOutcome,
};
use crate::workflow_learning_registry::{
    verify_capspec_rollback, verify_promoted_capspec, verify_promoted_skill, verify_skill_rollback,
};
use crate::workflow_learning_staging::{
    StageWorkflowDraftRequest, StagedWorkflowDraftReceipt, WorkflowStagingRoot,
};

fn active_model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn strict_skill_draft() -> WorkflowDraft {
    parse_draft(
        json!({
            "decision": "draft",
            "schema_version": 1,
            "kind": "skill",
            "name": "sourced-research",
            "purpose": "Research a subject and retain source-backed conclusions.",
            "trigger": "Use when a question requires sourced current research.",
            "artifact": {
                "format": "skill_markdown",
                "source": "---\nname: sourced-research\ndescription: Learned source-backed research\n---\n# Workflow\nSearch authoritative sources, compare them, and cite the evidence."
            },
            "required_capabilities": ["web_search"],
            "expected_benefit": "Produces repeatable research with explicit evidence.",
            "limitations": ["Review high-stakes conclusions."]
        }),
        WorkflowDraftKind::Skill,
    )
}

fn strict_capspec_draft() -> WorkflowDraft {
    let source = r#"format = 1
name = "read-project-summary"
description = "Read a project summary."
version = "1.0.0"

[permissions]
tools = ["file_read"]
read_paths = ["{{input.path}}"]

[inputs.path]
type = "string"
description = "Summary path"

[[steps]]
id = "read"
tool = "file_read"
needs = []
with = { path = "{{input.path}}" }
"#;
    parse_draft(
        json!({
            "decision": "draft",
            "schema_version": 1,
            "kind": "capspec",
            "name": "read-project-summary",
            "purpose": "Read a project summary through a typed path input.",
            "trigger": "Use when a project summary must be loaded.",
            "artifact": {"format": "capspec_toml", "source": source},
            "required_capabilities": ["file_read"],
            "expected_benefit": "Makes the read deterministic and auditable.",
            "limitations": []
        }),
        WorkflowDraftKind::Capspec,
    )
}

fn strict_automation_draft() -> WorkflowDraft {
    parse_draft(
        json!({
            "decision": "draft",
            "schema_version": 1,
            "kind": "automation",
            "name": "daily-status-report",
            "purpose": "Send a daily operational status report.",
            "trigger": "Use when daily status reporting is requested.",
            "artifact": {
                "format": "automation",
                "schedule": {"kind": "every", "every_secs": 86400},
                "instruction": "Inspect configured services and report verified health."
            },
            "required_capabilities": ["ssh_health_check"],
            "expected_benefit": "Reports service health on a stable cadence.",
            "limitations": []
        }),
        WorkflowDraftKind::Automation,
    )
}

fn parse_draft(value: serde_json::Value, kind: WorkflowDraftKind) -> WorkflowDraft {
    let WorkflowProposerOutcome::Draft(draft) =
        parse_workflow_draft(&value.to_string(), kind).unwrap()
    else {
        panic!("expected strict workflow draft");
    };
    draft
}

fn stage(
    home: &std::path::Path,
    job_id: &str,
    signature_byte: char,
    draft: &WorkflowDraft,
) -> StagedWorkflowDraftReceipt {
    WorkflowStagingRoot::new(home)
        .unwrap()
        .stage(StageWorkflowDraftRequest {
            job_id,
            workflow_signature: &signature_byte.to_string().repeat(64),
            draft,
            active_model: &active_model(),
        })
        .unwrap()
}

fn request<'a>(
    proposal_id: &'a str,
    job_id: &'a str,
    receipt: &'a StagedWorkflowDraftReceipt,
) -> PromoteWorkflowDraftRequest<'a> {
    PromoteWorkflowDraftRequest {
        proposal_id,
        staging_job_id: job_id,
        revision_sha256: &receipt.revision_sha256,
        artifact_sha256: &receipt.artifact_sha256,
    }
}

fn write_native_skill(home: &std::path::Path) -> std::path::PathBuf {
    let skill_dir = home.join("skills/sourced-research");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("skill.toml"),
        r#"[skill]
name = "sourced-research"
version = "1.0.0"
description = "Human-authored research workflow"
"#,
    )
    .unwrap();
    skill_dir
}

#[test]
fn skill_promotion_is_inactive_until_verified_and_rollback_restores_owner() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let native_path = write_native_skill(&home);
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-skill", 'a', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    let prepared = promotions
        .prepare(request("proposal-skill", "job-skill", &receipt))
        .unwrap();

    assert_eq!(prepared.manifest.phase, WorkflowPromotionPhase::Prepared);
    assert!(!prepared.target_path.exists());
    let mut registry = captain_skills::registry::SkillRegistry::new(home.join("skills"));
    registry.load_all().unwrap();
    registry.freeze();
    assert_eq!(registry.get("sourced-research").unwrap().path, native_path);

    let promoted = promotions
        .promote("proposal-skill", &receipt.revision_sha256)
        .unwrap();
    let verification = verify_promoted_skill(&promoted, &mut registry).unwrap();
    let verified = promotions.record_registry_verified(&verification).unwrap();
    let active = promotions
        .mark_active("proposal-skill", &receipt.revision_sha256)
        .unwrap();
    assert_eq!(
        verified.manifest.phase,
        WorkflowPromotionPhase::RegistryVerified
    );
    assert_eq!(active.manifest.phase, WorkflowPromotionPhase::Active);
    assert!(registry.is_frozen());
    let learned = registry.get("sourced-research").unwrap();
    assert_eq!(learned.path, promoted.target_path);
    assert!(learned.manifest.skill.tags.contains(&"learned".to_string()));

    let rolled_back = promotions
        .rollback("proposal-skill", &receipt.revision_sha256)
        .unwrap();
    verify_skill_rollback(&rolled_back, &mut registry).unwrap();
    assert_eq!(
        rolled_back.manifest.phase,
        WorkflowPromotionPhase::RolledBack
    );
    assert!(!rolled_back.target_path.exists());
    assert_eq!(registry.get("sourced-research").unwrap().path, native_path);
    assert!(registry.is_frozen());
}

#[test]
fn restart_reconciles_atomic_target_write_before_phase_commit() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-crash", 'b', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    let prepared = promotions
        .prepare(request("proposal-crash", "job-crash", &receipt))
        .unwrap();

    captain_types::durable_fs::atomic_copy(&receipt.artifact_path, &prepared.target_path).unwrap();
    let restarted = WorkflowPromotionRoot::new(&home).unwrap();
    let reconciled = restarted
        .reconcile("proposal-crash", &receipt.revision_sha256)
        .unwrap();

    assert_eq!(reconciled.manifest.phase, WorkflowPromotionPhase::Promoted);
    assert_eq!(
        restarted
            .load("proposal-crash", &receipt.revision_sha256)
            .unwrap()
            .manifest
            .phase,
        WorkflowPromotionPhase::Promoted
    );
}

#[test]
fn rollback_restores_the_previous_learned_revision_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let learned_path = home.join("skills/learned/sourced-research.md");
    let previous = b"---\nname: sourced-research\ndescription: Previous learned revision\n---\n# Previous\nKeep the prior verified workflow.";
    fs::create_dir_all(learned_path.parent().unwrap()).unwrap();
    fs::write(&learned_path, previous).unwrap();
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-refinement", '3', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    let prepared = promotions
        .prepare(request("proposal-refinement", "job-refinement", &receipt))
        .unwrap();

    assert!(prepared.manifest.previous_sha256.is_some());
    assert!(prepared
        .manifest
        .previous_backup_relative_path
        .as_ref()
        .is_some_and(|path| home.join(path).is_file()));
    promotions
        .promote("proposal-refinement", &receipt.revision_sha256)
        .unwrap();
    assert_ne!(fs::read(&learned_path).unwrap(), previous);

    let rolled_back = promotions
        .rollback("proposal-refinement", &receipt.revision_sha256)
        .unwrap();
    assert_eq!(
        rolled_back.manifest.phase,
        WorkflowPromotionPhase::RolledBack
    );
    assert_eq!(fs::read(learned_path).unwrap(), previous);
}

#[test]
fn restart_finishes_rollback_after_previous_bytes_are_restored() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-rollback-crash", '4', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    promotions
        .prepare(request(
            "proposal-rollback-crash",
            "job-rollback-crash",
            &receipt,
        ))
        .unwrap();
    let promoted = promotions
        .promote("proposal-rollback-crash", &receipt.revision_sha256)
        .unwrap();

    let mut interrupted = promoted.manifest.clone();
    interrupted.phase = WorkflowPromotionPhase::RollbackPending;
    captain_types::durable_fs::atomic_write(
        &promoted.journal_path,
        &serde_json::to_vec_pretty(&interrupted).unwrap(),
    )
    .unwrap();
    captain_types::durable_fs::remove_file(&promoted.target_path).unwrap();

    let reconciled = WorkflowPromotionRoot::new(&home)
        .unwrap()
        .reconcile("proposal-rollback-crash", &receipt.revision_sha256)
        .unwrap();
    assert_eq!(
        reconciled.manifest.phase,
        WorkflowPromotionPhase::RolledBack
    );
}

#[test]
fn rollback_refuses_to_clobber_an_external_edit() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-conflict", 'c', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    promotions
        .prepare(request("proposal-conflict", "job-conflict", &receipt))
        .unwrap();
    let promoted = promotions
        .promote("proposal-conflict", &receipt.revision_sha256)
        .unwrap();
    fs::write(&promoted.target_path, b"external operator edit").unwrap();

    let error = promotions
        .rollback("proposal-conflict", &receipt.revision_sha256)
        .unwrap_err();
    assert!(matches!(error, WorkflowPromotionError::Conflict(_)));
    assert_eq!(
        fs::read(promoted.target_path).unwrap(),
        b"external operator edit"
    );
}

#[test]
fn quarantine_removes_active_bytes_and_is_durable() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-quarantine", 'd', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    promotions
        .prepare(request("proposal-quarantine", "job-quarantine", &receipt))
        .unwrap();
    promotions
        .promote("proposal-quarantine", &receipt.revision_sha256)
        .unwrap();

    let quarantined = promotions
        .quarantine(
            "proposal-quarantine",
            &receipt.revision_sha256,
            "canary verification failed",
        )
        .unwrap();

    assert_eq!(
        quarantined.manifest.phase,
        WorkflowPromotionPhase::Quarantined
    );
    assert!(!quarantined.target_path.exists());
    assert!(home
        .join("learning/quarantine/proposal-quarantine")
        .join(&receipt.revision_sha256)
        .join("quarantine.json")
        .is_file());
    assert_eq!(
        WorkflowPromotionRoot::new(&home)
            .unwrap()
            .reconcile("proposal-quarantine", &receipt.revision_sha256)
            .unwrap()
            .manifest
            .phase,
        WorkflowPromotionPhase::Quarantined
    );
}

#[test]
fn capspec_promotion_requires_and_records_exact_registry_activation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_capspec_draft();
    let receipt = stage(&home, "job-capspec", 'e', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    promotions
        .prepare(request("proposal-capspec", "job-capspec", &receipt))
        .unwrap();
    let promoted = promotions
        .promote("proposal-capspec", &receipt.revision_sha256)
        .unwrap();
    let registry = captain_capspec::CapabilityRegistry::open(
        &home.join("capabilities"),
        &home.join("data/capabilities.db"),
    )
    .unwrap();

    let verification = verify_promoted_capspec(&promoted, &registry, "operator").unwrap();
    let verified = promotions.record_registry_verified(&verification).unwrap();
    let view = registry
        .capability(
            &captain_capspec::CapabilityScope::Global,
            "read-project-summary",
        )
        .unwrap();

    assert_eq!(
        verified.manifest.phase,
        WorkflowPromotionPhase::RegistryVerified
    );
    assert_eq!(
        view.source_path.canonicalize().unwrap(),
        promoted.target_path.canonicalize().unwrap()
    );
    assert!(view.active_hash.is_some());
    assert!(view.pending_hash.is_none());

    let rolled_back = promotions
        .rollback("proposal-capspec", &receipt.revision_sha256)
        .unwrap();
    verify_capspec_rollback(&rolled_back, &registry, "operator").unwrap();
    let disabled = registry
        .capability(
            &captain_capspec::CapabilityScope::Global,
            "read-project-summary",
        )
        .unwrap();
    assert_eq!(disabled.status, captain_capspec::CapabilityStatus::Disabled);
    assert!(disabled.active_hash.is_none());
    assert!(!rolled_back.target_path.exists());
}

#[test]
fn capspec_rollback_restores_the_previous_active_revision() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let capabilities = home.join("capabilities");
    fs::create_dir_all(&capabilities).unwrap();
    let target = capabilities.join("read-project-summary.captain");
    let previous = r#"format = 1
name = "read-project-summary"
description = "Read the previous project summary format."
version = "0.9.0"

[permissions]
tools = ["file_read"]
read_paths = ["{{input.path}}"]

[inputs.path]
type = "string"
description = "Previous summary path"

[[steps]]
id = "read"
tool = "file_read"
needs = []
with = { path = "{{input.path}}" }
"#;
    fs::write(&target, previous).unwrap();
    let registry = captain_capspec::CapabilityRegistry::open(
        &capabilities,
        &home.join("data/capabilities.db"),
    )
    .unwrap();
    let initial = registry
        .capability(
            &captain_capspec::CapabilityScope::Global,
            "read-project-summary",
        )
        .unwrap();
    let previous_hash = initial
        .active_hash
        .clone()
        .or_else(|| initial.pending_hash.clone())
        .unwrap();
    if initial.active_hash.is_none() {
        registry
            .approve(
                &captain_capspec::CapabilityScope::Global,
                "read-project-summary",
                &previous_hash,
                "operator",
            )
            .unwrap();
    }

    let draft = strict_capspec_draft();
    let receipt = stage(&home, "job-capspec-restore", '9', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();
    promotions
        .prepare(request(
            "proposal-capspec-restore",
            "job-capspec-restore",
            &receipt,
        ))
        .unwrap();
    let promoted = promotions
        .promote("proposal-capspec-restore", &receipt.revision_sha256)
        .unwrap();
    verify_promoted_capspec(&promoted, &registry, "operator").unwrap();

    let rolled_back = promotions
        .rollback("proposal-capspec-restore", &receipt.revision_sha256)
        .unwrap();
    verify_capspec_rollback(&rolled_back, &registry, "operator").unwrap();
    let restored = registry
        .capability(
            &captain_capspec::CapabilityScope::Global,
            "read-project-summary",
        )
        .unwrap();
    assert_eq!(
        restored.active_hash.as_deref(),
        Some(previous_hash.as_str())
    );
    assert_eq!(fs::read_to_string(target).unwrap(), previous);
}

#[test]
fn automation_cannot_be_faked_as_an_active_file() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_automation_draft();
    let receipt = stage(&home, "job-automation", 'f', &draft);
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();

    let error = promotions
        .prepare(request("proposal-automation", "job-automation", &receipt))
        .unwrap_err();

    assert!(matches!(
        error,
        WorkflowPromotionError::ExternalActivationRequired
    ));
    assert!(!home.join("automations/daily-status-report.json").exists());
}

#[test]
fn staged_tampering_is_rejected_before_active_write() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-tamper", '1', &draft);
    fs::write(&receipt.artifact_path, b"tampered staged bytes").unwrap();
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();

    let error = promotions
        .prepare(request("proposal-tamper", "job-tamper", &receipt))
        .unwrap_err();

    assert!(matches!(error, WorkflowPromotionError::InvalidStaging(_)));
    assert!(!home.join("skills/learned/sourced-research.md").exists());
}

#[cfg(unix)]
#[test]
fn symlinked_active_root_is_rejected_without_external_write() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-symlink", '2', &draft);
    let outside = temp.path().join("outside");
    fs::create_dir_all(home.join("skills")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, home.join("skills/learned")).unwrap();
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();

    let error = promotions
        .prepare(request("proposal-symlink", "job-symlink", &receipt))
        .unwrap_err();

    assert!(matches!(error, WorkflowPromotionError::UnsafeFilesystem(_)));
    assert!(fs::read_dir(outside).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn symlinked_installation_journal_is_rejected_without_external_write() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let draft = strict_skill_draft();
    let receipt = stage(&home, "job-journal-symlink", '5', &draft);
    let outside = temp.path().join("outside-journal");
    fs::create_dir_all(home.join("learning/installations")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(
        &outside,
        home.join("learning/installations/proposal-journal-symlink"),
    )
    .unwrap();
    let promotions = WorkflowPromotionRoot::new(&home).unwrap();

    let error = promotions
        .prepare(request(
            "proposal-journal-symlink",
            "job-journal-symlink",
            &receipt,
        ))
        .unwrap_err();

    assert!(matches!(error, WorkflowPromotionError::UnsafeFilesystem(_)));
    assert!(fs::read_dir(outside).unwrap().next().is_none());
}
