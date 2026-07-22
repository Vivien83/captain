use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::reflection_job::ReflectionCompleter;
use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowClassification, WorkflowGroupAnalysis,
    WorkflowScope,
};
use crate::workflow_learning_proposer::{
    parse_workflow_draft, ActiveModelIdentity, WorkflowDraft, WorkflowDraftArtifact,
    WorkflowDraftKind, WorkflowDraftProposer, WorkflowProposerError, WorkflowProposerOutcome,
};

#[derive(Default)]
struct RecordingCompleter {
    response: String,
    calls: Mutex<Vec<(String, String, String)>>,
}

#[async_trait]
impl ReflectionCompleter for RecordingCompleter {
    async fn complete(&self, model: &str, system: &str, user: &str) -> Result<String, String> {
        self.calls
            .lock()
            .unwrap()
            .push((model.to_string(), system.to_string(), user.to_string()));
        Ok(self.response.clone())
    }
}

fn group(classification: WorkflowClassification) -> WorkflowGroupAnalysis {
    WorkflowGroupAnalysis {
        signature: "a".repeat(64),
        classification,
        eligible: true,
        reasons: Vec::new(),
        occurrence_count: 3,
        distinct_turn_count: 3,
        distinct_session_count: 2,
        explicit_reuse_request: false,
        scope: WorkflowScope::Global,
        episode_ids: vec!["private-episode-id".to_string()],
        intent_samples: vec!["check sources then write a verified report".to_string()],
        canonical: CanonicalWorkflow {
            version: 1,
            nodes: vec![CanonicalWorkflowNode {
                index: 0,
                tool_name: "web_search".to_string(),
                role: "research".to_string(),
                input_shape: json!({"query": "text"}),
                effect_class: "read".to_string(),
                verification_shape: "result_received".to_string(),
                dependencies: Vec::new(),
            }],
        },
    }
}

fn skill_response() -> String {
    json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "skill",
        "name": "sourced-research",
        "purpose": "Research a subject and keep source-backed conclusions.",
        "trigger": "Use when a question requires current sourced research.",
        "artifact": {
            "format": "skill_markdown",
            "source": "---\nname: sourced-research\ndescription: Produce source-backed research\n---\n# Workflow\nSearch authoritative sources, compare them, and cite the evidence."
        },
        "required_capabilities": ["web_search"],
        "expected_benefit": "Produces repeatable research with explicit evidence.",
        "limitations": ["A human should review high-stakes conclusions."]
    })
    .to_string()
}

fn skill_draft() -> WorkflowDraft {
    match parse_workflow_draft(&skill_response(), WorkflowDraftKind::Skill).unwrap() {
        WorkflowProposerOutcome::Draft(draft) => draft,
        WorkflowProposerOutcome::Declined { .. } => panic!("fixture unexpectedly declined"),
    }
}

#[tokio::test]
async fn proposer_uses_exact_active_model_once_without_fallback() {
    let completer = Arc::new(RecordingCompleter {
        response: skill_response(),
        ..Default::default()
    });
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(2),
        "French",
    );

    let outcome = proposer
        .draft(&group(WorkflowClassification::Skill))
        .await
        .unwrap();

    assert!(matches!(outcome, WorkflowProposerOutcome::Draft(_)));
    let calls = completer.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "gpt-5.6-sol");
    assert!(calls[0]
        .1
        .contains("User-facing fields must be written in French"));
    assert!(!calls[0].2.contains("private-episode-id"));
}

#[tokio::test]
async fn initial_draft_cannot_expand_authority_beyond_observed_tools() {
    let completer = Arc::new(RecordingCompleter {
        response: skill_response().replace(
            r#""required_capabilities":["web_search"]"#,
            r#""required_capabilities":["shell_exec","web_search"]"#,
        ),
        ..Default::default()
    });
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(2),
        "French",
    );

    let error = proposer
        .draft(&group(WorkflowClassification::Skill))
        .await
        .unwrap_err();

    assert!(matches!(error, WorkflowProposerError::InvalidDraft(_)));
    assert!(error.to_string().contains("shell_exec"));
    assert_eq!(completer.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn refinement_prompt_only_offers_valid_target_kinds() {
    let completer = Arc::new(RecordingCompleter {
        response: json!({
            "decision": "decline",
            "schema_version": 1,
            "reason": "No installed target is present in the bounded evidence."
        })
        .to_string(),
        ..Default::default()
    });
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(2),
        "French",
    );

    let outcome = proposer
        .draft(&group(WorkflowClassification::Refinement))
        .await
        .unwrap();

    assert!(matches!(outcome, WorkflowProposerOutcome::Declined { .. }));
    let calls = completer.calls.lock().unwrap();
    assert!(calls[0].1.contains(r#""target_kind":"skill""#));
    assert!(calls[0].1.contains(r#""target_kind":"capspec""#));
    assert!(!calls[0].1.contains(r#""target_kind":"skill|capspec""#));
}

#[tokio::test]
async fn operator_refinement_uses_active_model_once_and_preserves_full_context() {
    let completer = Arc::new(RecordingCompleter {
        response: skill_response().replace(
            "Search authoritative sources, compare them, and cite the evidence.",
            "Search authoritative sources, compare them, cite the evidence, and finish with a concise table.",
        ),
        ..Default::default()
    });
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(2),
        "French",
    );

    let outcome = proposer
        .refine(
            &skill_draft(),
            "Ajoute un tableau final concis sans retirer les sources.",
            "fr",
        )
        .await
        .unwrap();

    assert!(matches!(outcome, WorkflowProposerOutcome::Draft(_)));
    let calls = completer.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "gpt-5.6-sol");
    assert!(calls[0].1.contains("Preserve kind \"skill\""));
    assert!(calls[0].1.contains("written in fr"));
    assert!(calls[0]
        .2
        .contains("Ajoute un tableau final concis sans retirer les sources."));
    assert!(calls[0].2.contains("Produce source-backed research"));
}

#[tokio::test]
async fn operator_refinement_rejects_identity_and_authority_changes() {
    for response in [
        skill_response().replace("sourced-research", "renamed-research"),
        skill_response().replace(r#"["web_search"]"#, r#"["web_search","shell_exec"]"#),
    ] {
        let proposer = WorkflowDraftProposer::new(
            Arc::new(RecordingCompleter {
                response,
                ..Default::default()
            }),
            ActiveModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            Duration::from_secs(2),
            "French",
        );
        assert!(matches!(
            proposer
                .refine(&skill_draft(), "Clarifie le résultat produit.", "fr")
                .await,
            Err(WorkflowProposerError::InvalidDraft(_))
        ));
    }
}

#[tokio::test]
async fn unsafe_refinement_instruction_never_reaches_the_provider() {
    let completer = Arc::new(RecordingCompleter {
        response: skill_response(),
        ..Default::default()
    });
    let proposer = WorkflowDraftProposer::new(
        completer.clone(),
        ActiveModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        },
        Duration::from_secs(2),
        "French",
    );

    let result = proposer
        .refine(
            &skill_draft(),
            "Utilise token: sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "fr",
        )
        .await;

    assert!(matches!(
        result,
        Err(WorkflowProposerError::UnsafeRefinementInstruction(_))
    ));
    assert!(completer.calls.lock().unwrap().is_empty());
}

#[test]
fn parser_rejects_prose_code_fences_unknown_fields_and_wrong_version() {
    let valid = skill_response();
    for raw in [
        format!("Here is the draft: {valid}"),
        format!("```json\n{valid}\n```"),
        valid.replacen(
            "\"decision\":\"draft\"",
            "\"extra\":true,\"decision\":\"draft\"",
            1,
        ),
        valid.replacen("\"schema_version\":1", "\"schema_version\":2", 1),
    ] {
        assert!(parse_workflow_draft(&raw, WorkflowDraftKind::Skill).is_err());
    }
}

#[test]
fn deterministic_kind_cannot_be_overridden_by_model() {
    let error = parse_workflow_draft(&skill_response(), WorkflowDraftKind::Capspec).unwrap_err();
    assert!(matches!(error, WorkflowProposerError::InvalidDraft(_)));
    assert!(error.to_string().contains("deterministic classification"));
}

#[test]
fn parser_accepts_strict_decline_and_rejects_short_or_extra_decline() {
    let outcome = parse_workflow_draft(
        r#"{"decision":"decline","schema_version":1,"reason":"The evidence is not portable across environments."}"#,
        WorkflowDraftKind::Skill,
    )
    .unwrap();
    assert!(matches!(outcome, WorkflowProposerOutcome::Declined { .. }));
    assert!(parse_workflow_draft(
        r#"{"decision":"decline","schema_version":1,"reason":"no","extra":true}"#,
        WorkflowDraftKind::Skill,
    )
    .is_err());
}

#[test]
fn parser_compiles_capspec_and_validates_automation_schedule() {
    let capspec_source = r#"format = 1
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
    let capspec = json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "capspec",
        "name": "read-project-summary",
        "purpose": "Read a project summary through a typed path input.",
        "trigger": "Use when a project summary must be loaded.",
        "artifact": {"format": "capspec_toml", "source": capspec_source},
        "required_capabilities": ["file_read"],
        "expected_benefit": "Makes the read deterministic and auditable.",
        "limitations": []
    })
    .to_string();
    assert!(parse_workflow_draft(&capspec, WorkflowDraftKind::Capspec).is_ok());

    let invalid_automation = json!({
        "decision": "draft",
        "schema_version": 1,
        "kind": "automation",
        "name": "daily-report",
        "purpose": "Produce a recurring operational report.",
        "trigger": "Run on the configured recurring schedule.",
        "artifact": {
            "format": "automation",
            "schedule": {"kind": "every", "every_secs": 12},
            "instruction": "Generate the verified report."
        },
        "required_capabilities": [],
        "expected_benefit": "Keeps the report current.",
        "limitations": []
    })
    .to_string();
    assert!(parse_workflow_draft(&invalid_automation, WorkflowDraftKind::Automation).is_err());
}

#[test]
fn parser_rejects_secret_material_before_it_can_be_staged() {
    let raw = skill_response().replace(
        "A human should review high-stakes conclusions.",
        "Use token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
    );
    let error = parse_workflow_draft(&raw, WorkflowDraftKind::Skill).unwrap_err();
    assert!(error.to_string().contains("secret-like material"));
}

#[test]
fn parsed_skill_keeps_the_exact_source_for_revision_hashing() {
    let outcome = parse_workflow_draft(&skill_response(), WorkflowDraftKind::Skill).unwrap();
    let WorkflowProposerOutcome::Draft(draft) = outcome else {
        panic!("expected draft");
    };
    assert!(matches!(
        draft.artifact,
        WorkflowDraftArtifact::SkillMarkdown { ref source }
            if source.contains("# Workflow")
    ));
}
