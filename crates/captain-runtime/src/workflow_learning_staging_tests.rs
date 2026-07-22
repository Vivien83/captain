use std::fs;

use serde_json::json;

use crate::workflow_learning_proposer::{
    parse_workflow_draft, ActiveModelIdentity, WorkflowDraft, WorkflowDraftKind,
    WorkflowProposerOutcome,
};
use crate::workflow_learning_staging::{
    StageWorkflowDraftRequest, StagedWorkflowDraft, WorkflowStagingError, WorkflowStagingRoot,
};

fn active_model() -> ActiveModelIdentity {
    ActiveModelIdentity {
        provider: "codex".to_string(),
        model: "gpt-5.6-sol".to_string(),
    }
}

fn strict_skill_draft() -> WorkflowDraft {
    let response = json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "skill",
        "name": "sourced-research",
        "purpose": "Research a subject and retain source-backed conclusions.",
        "trigger": "Use when a question requires sourced current research.",
        "artifact": {
            "format": "skill_markdown",
            "source": "---\nname: sourced-research\ndescription: Produce source-backed research\n---\n# Workflow\nSearch authoritative sources, compare them, and cite the evidence."
        },
        "required_capabilities": ["web_search"],
        "expected_benefit": "Produces repeatable research with explicit evidence.",
        "limitations": ["A human should review high-stakes conclusions."]
    })
    .to_string();
    let WorkflowProposerOutcome::Draft(draft) =
        parse_workflow_draft(&response, WorkflowDraftKind::Skill).unwrap()
    else {
        panic!("expected draft");
    };
    draft
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
    let response = json!({
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
    })
    .to_string();
    let WorkflowProposerOutcome::Draft(draft) =
        parse_workflow_draft(&response, WorkflowDraftKind::Capspec).unwrap()
    else {
        panic!("expected draft");
    };
    draft
}

#[test]
fn stage_is_immutable_idempotent_private_and_hash_verified() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let staging = WorkflowStagingRoot::new(&home).unwrap();
    let draft = strict_skill_draft();
    let model = active_model();
    let request = StageWorkflowDraftRequest {
        job_id: "job-001",
        workflow_signature: &"a".repeat(64),
        draft: &draft,
        active_model: &model,
    };

    let first = staging.stage(request.clone()).unwrap();
    let second = staging.stage(request).unwrap();
    let loaded = staging
        .load_exact("job-001", &first.revision_sha256)
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(loaded.manifest.draft, draft);
    assert_eq!(loaded.artifact_path, first.artifact_path);
    assert_eq!(
        loaded.artifact_bytes,
        fs::read(&first.artifact_path).unwrap()
    );
    assert!(first.artifact_path.ends_with("SKILL.md"));
    assert_eq!(first.revision_sha256.len(), 64);
    assert_eq!(first.artifact_sha256.len(), 64);
    let manifest: StagedWorkflowDraft =
        serde_json::from_slice(&fs::read(&first.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest.revision_sha256, first.revision_sha256);
    assert_eq!(manifest.artifact_sha256, first.artifact_sha256);
    assert_eq!(manifest.draft, draft);
    assert!(first
        .revision_dir
        .starts_with(home.join("learning/staging")));
    assert!(!first.revision_dir.starts_with(home.join("skills")));
    assert!(!first.revision_dir.starts_with(home.join("capabilities")));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&first.revision_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&first.artifact_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn staged_skill_and_capspec_are_invisible_to_active_registries() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    fs::create_dir_all(home.join("skills")).unwrap();
    fs::create_dir_all(home.join("capabilities")).unwrap();
    fs::create_dir_all(home.join("data")).unwrap();
    let staging = WorkflowStagingRoot::new(&home).unwrap();
    let model = active_model();

    for (job_id, signature, draft) in [
        ("job-skill", "b".repeat(64), strict_skill_draft()),
        ("job-capspec", "c".repeat(64), strict_capspec_draft()),
    ] {
        staging
            .stage(StageWorkflowDraftRequest {
                job_id,
                workflow_signature: &signature,
                draft: &draft,
                active_model: &model,
            })
            .unwrap();
    }

    let mut skill_registry = captain_skills::registry::SkillRegistry::new(home.join("skills"));
    assert_eq!(skill_registry.load_all().unwrap(), 0);
    assert!(skill_registry.list().is_empty());

    let capspec_registry = captain_capspec::CapabilityRegistry::open(
        &home.join("capabilities"),
        &home.join("data/capabilities.db"),
    )
    .unwrap();
    assert!(capspec_registry.list().unwrap().is_empty());
}

#[test]
fn tampered_revision_is_never_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let staging = WorkflowStagingRoot::new(&home).unwrap();
    let draft = strict_skill_draft();
    let model = active_model();
    let signature = "d".repeat(64);
    let request = StageWorkflowDraftRequest {
        job_id: "job-tampered",
        workflow_signature: &signature,
        draft: &draft,
        active_model: &model,
    };
    let receipt = staging.stage(request.clone()).unwrap();
    fs::write(&receipt.artifact_path, "tampered").unwrap();

    let error = staging.stage(request).unwrap_err();
    assert!(matches!(error, WorkflowStagingError::ImmutableConflict(_)));
    let load_error = staging
        .load_exact("job-tampered", &receipt.revision_sha256)
        .unwrap_err();
    assert!(matches!(
        load_error,
        WorkflowStagingError::ImmutableConflict(_)
    ));
    assert_eq!(
        fs::read_to_string(receipt.artifact_path).unwrap(),
        "tampered"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_staging_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(home.join("learning")).unwrap();
    symlink(&outside, home.join("learning/staging")).unwrap();
    let staging = WorkflowStagingRoot::new(&home).unwrap();
    let draft = strict_skill_draft();
    let signature = "e".repeat(64);

    let error = staging
        .stage(StageWorkflowDraftRequest {
            job_id: "job-symlink",
            workflow_signature: &signature,
            draft: &draft,
            active_model: &active_model(),
        })
        .unwrap_err();

    assert!(matches!(error, WorkflowStagingError::UnsafeFilesystem(_)));
    assert!(fs::read_dir(outside).unwrap().next().is_none());
}

#[test]
fn unsafe_identifiers_and_non_hash_signatures_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let draft = strict_skill_draft();
    for (job_id, signature) in [("../escape", "f".repeat(64)), ("job", "not-a-hash".into())] {
        let error = staging
            .stage(StageWorkflowDraftRequest {
                job_id,
                workflow_signature: &signature,
                draft: &draft,
                active_model: &active_model(),
            })
            .unwrap_err();
        assert!(matches!(error, WorkflowStagingError::InvalidRequest(_)));
    }
}

#[test]
fn recovery_accepts_one_complete_revision_and_ignores_manifestless_partial_work() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("captain-home");
    let staging = WorkflowStagingRoot::new(&home).unwrap();
    let receipt = staging
        .stage(StageWorkflowDraftRequest {
            job_id: "job-recover",
            workflow_signature: &"1".repeat(64),
            draft: &strict_skill_draft(),
            active_model: &active_model(),
        })
        .unwrap();
    fs::create_dir_all(staging.path().join("job-recover").join("2".repeat(64))).unwrap();

    let recovered = staging.recover_job("job-recover").unwrap().unwrap();
    assert_eq!(recovered.manifest.revision_sha256, receipt.revision_sha256);
    assert_eq!(
        recovered.artifact_bytes,
        fs::read(receipt.artifact_path).unwrap()
    );
}

#[test]
fn recovery_refuses_multiple_complete_model_revisions_for_one_job() {
    let temp = tempfile::tempdir().unwrap();
    let staging = WorkflowStagingRoot::new(temp.path().join("captain-home")).unwrap();
    let model = active_model();
    let signature = "3".repeat(64);
    let first = strict_skill_draft();
    let mut second = first.clone();
    second.expected_benefit = "A distinct model revision that must not win implicitly.".to_string();
    for draft in [&first, &second] {
        staging
            .stage(StageWorkflowDraftRequest {
                job_id: "job-ambiguous",
                workflow_signature: &signature,
                draft,
                active_model: &model,
            })
            .unwrap();
    }

    assert!(matches!(
        staging.recover_job("job-ambiguous").unwrap_err(),
        WorkflowStagingError::ImmutableConflict(_)
    ));
}
