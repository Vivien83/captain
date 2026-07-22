use captain_memory::workflow_learning::{
    WorkflowEpisodeEvidence, WorkflowEpisodeRecord, WorkflowEpisodeStepRecord,
};

use crate::workflow_learning_analysis::{
    analyze_workflow_evidence, ExistingCapabilityKind, WorkflowAnalysisCatalog,
    WorkflowClassification, WorkflowRejectionReason,
};

fn episode(
    id: &str,
    session_id: &str,
    explicit_reuse_request: bool,
    steps: Vec<WorkflowEpisodeStepRecord>,
) -> WorkflowEpisodeEvidence {
    WorkflowEpisodeEvidence {
        episode: WorkflowEpisodeRecord {
            id: id.to_string(),
            session_id: session_id.to_string(),
            turn_id: format!("turn-{id}"),
            agent_id: "captain".to_string(),
            origin_channel: Some("cli".to_string()),
            project_id: None,
            workspace_scope: None,
            intent_redacted: format!("workflow {id}"),
            intent_fingerprint: format!("intent-{id}"),
            status: "succeeded".to_string(),
            explicit_reuse_request,
            tool_attempt_count: steps.len() as u32,
            success_count: steps.len() as u32,
            failure_count: 0,
            has_secret_input: false,
            has_unverified_mutation: false,
            failure_reason: None,
            started_at_unix_ms: 100,
            completed_at_unix_ms: Some(200),
            analysis_status: "pending".to_string(),
            analysis_result_json: None,
            analysis_proposal_id: None,
            analysis_updated_at_unix_ms: None,
        },
        steps,
    }
}

#[allow(clippy::too_many_arguments)]
fn step(
    episode_id: &str,
    id: &str,
    ordinal: u32,
    tool_name: &str,
    dependencies: &[&str],
    input_shape: serde_json::Value,
    effect_class: &str,
    verification_marker: Option<&str>,
) -> WorkflowEpisodeStepRecord {
    WorkflowEpisodeStepRecord {
        episode_id: episode_id.to_string(),
        tool_use_id: id.to_string(),
        ordinal,
        tool_name: tool_name.to_string(),
        dependency_ids_json: serde_json::to_string(dependencies).unwrap(),
        input_shape_json: serde_json::to_string(&input_shape).unwrap(),
        input_fingerprint: format!("fingerprint-{id}"),
        effect_class: effect_class.to_string(),
        status: "succeeded".to_string(),
        retry_count: 0,
        output_class: Some("tool_success".to_string()),
        verification_marker: verification_marker.map(str::to_string),
        secret_detected: false,
        started_at_unix_ms: 110 + i64::from(ordinal),
        completed_at_unix_ms: Some(120 + i64::from(ordinal)),
        duration_ms: Some(10),
    }
}

fn document_workflow(id: &str, session: &str) -> WorkflowEpisodeEvidence {
    episode(
        id,
        session,
        false,
        vec![
            step(
                id,
                &format!("{id}-read"),
                0,
                "file_read",
                &[],
                serde_json::json!({"path": "<path>"}),
                "read",
                Some("result_received"),
            ),
            step(
                id,
                &format!("{id}-extract"),
                1,
                "document_extract",
                &[&format!("{id}-read")],
                serde_json::json!({"format": "enum:markdown", "path": "<path>"}),
                "read",
                Some("result_received"),
            ),
        ],
    )
}

#[test]
fn parallel_order_and_tool_ids_do_not_change_the_signature() {
    let first = episode(
        "parallel-a",
        "session-a",
        false,
        vec![
            step(
                "parallel-a",
                "search-a",
                0,
                "web_search",
                &[],
                serde_json::json!({"query": "<text>"}),
                "read",
                Some("result_received"),
            ),
            step(
                "parallel-a",
                "fetch-a",
                1,
                "web_fetch",
                &[],
                serde_json::json!({"url": "<url>"}),
                "read",
                Some("result_received"),
            ),
        ],
    );
    let second = episode(
        "parallel-b",
        "session-b",
        false,
        vec![
            step(
                "parallel-b",
                "fetch-b",
                0,
                "web_fetch",
                &[],
                serde_json::json!({"url": "<url>"}),
                "read",
                Some("result_received"),
            ),
            step(
                "parallel-b",
                "search-b",
                1,
                "web_search",
                &[],
                serde_json::json!({"query": "<text>"}),
                "read",
                Some("result_received"),
            ),
        ],
    );

    let batch = analyze_workflow_evidence(vec![first, second], &Default::default());
    assert_eq!(batch.groups.len(), 1);
    assert_eq!(batch.groups[0].occurrence_count, 2);
    assert_eq!(
        batch.groups[0].classification,
        WorkflowClassification::Skill
    );
    assert_eq!(batch.groups[0].signature.len(), 64);
    assert!(batch.groups[0]
        .signature
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn identical_parallel_roots_can_swap_without_splitting_the_group() {
    let branch = |episode_id: &str, session: &str, swapped: bool| {
        let first_root = format!("{episode_id}-root-a");
        let second_root = format!("{episode_id}-root-b");
        let roots = if swapped {
            vec![second_root.clone(), first_root.clone()]
        } else {
            vec![first_root.clone(), second_root.clone()]
        };
        episode(
            episode_id,
            session,
            false,
            vec![
                step(
                    episode_id,
                    &roots[0],
                    0,
                    "web_fetch",
                    &[],
                    serde_json::json!({"url": "<url>"}),
                    "read",
                    Some("result_received"),
                ),
                step(
                    episode_id,
                    &roots[1],
                    1,
                    "web_fetch",
                    &[],
                    serde_json::json!({"url": "<url>"}),
                    "read",
                    Some("result_received"),
                ),
                step(
                    episode_id,
                    &format!("{episode_id}-document"),
                    2,
                    "document_extract",
                    &[&first_root],
                    serde_json::json!({"path": "<path>"}),
                    "read",
                    Some("result_received"),
                ),
                step(
                    episode_id,
                    &format!("{episode_id}-file"),
                    3,
                    "file_read",
                    &[&second_root],
                    serde_json::json!({"path": "<path>"}),
                    "read",
                    Some("result_received"),
                ),
            ],
        )
    };

    let batch = analyze_workflow_evidence(
        vec![
            branch("branch-a", "session-a", false),
            branch("branch-b", "session-b", true),
        ],
        &Default::default(),
    );
    assert_eq!(batch.groups.len(), 1);
    assert_eq!(batch.groups[0].occurrence_count, 2);
}

#[test]
fn three_turns_across_two_sessions_make_a_deterministic_capspec_candidate() {
    let batch = analyze_workflow_evidence(
        vec![
            document_workflow("doc-a", "session-a"),
            document_workflow("doc-b", "session-a"),
            document_workflow("doc-c", "session-b"),
        ],
        &Default::default(),
    );
    let group = &batch.groups[0];
    assert_eq!(group.classification, WorkflowClassification::Capspec);
    assert!(group.eligible, "{:?}", group.reasons);
    assert_eq!(group.distinct_turn_count, 3);
    assert_eq!(group.distinct_session_count, 2);
}

#[test]
fn explicit_reuse_fast_tracks_only_a_verified_high_value_step() {
    let health = episode(
        "health",
        "session-a",
        true,
        vec![step(
            "health",
            "health-check",
            0,
            "ssh_health_check",
            &[],
            serde_json::json!({"host": "<host>"}),
            "external",
            Some("operation_confirmed"),
        )],
    );
    let ordinary = episode(
        "ordinary",
        "session-b",
        true,
        vec![step(
            "ordinary",
            "read",
            0,
            "file_read",
            &[],
            serde_json::json!({"path": "<path>"}),
            "read",
            Some("result_received"),
        )],
    );

    let batch = analyze_workflow_evidence(vec![health, ordinary], &Default::default());
    let health = batch
        .groups
        .iter()
        .find(|group| group.canonical.nodes[0].tool_name == "ssh_health_check")
        .unwrap();
    let ordinary = batch
        .groups
        .iter()
        .find(|group| group.canonical.nodes[0].tool_name == "file_read")
        .unwrap();
    assert!(health.eligible);
    assert!(!ordinary.eligible);
    assert_eq!(ordinary.classification, WorkflowClassification::None);
    assert!(ordinary
        .reasons
        .contains(&WorkflowRejectionReason::OrdinarySingleStep));
}

#[test]
fn research_automation_memory_and_refinement_route_deterministically() {
    let research = (0..3)
        .map(|index| {
            let id = format!("research-{index}");
            episode(
                &id,
                if index < 2 { "session-a" } else { "session-b" },
                false,
                vec![
                    step(
                        &id,
                        &format!("{id}-search"),
                        0,
                        "web_search",
                        &[],
                        serde_json::json!({"query": "<text>"}),
                        "read",
                        Some("result_received"),
                    ),
                    step(
                        &id,
                        &format!("{id}-fetch"),
                        1,
                        "web_fetch",
                        &[&format!("{id}-search")],
                        serde_json::json!({"url": "<url>"}),
                        "read",
                        Some("result_received"),
                    ),
                ],
            )
        })
        .collect::<Vec<_>>();
    let research_batch = analyze_workflow_evidence(research, &Default::default());
    assert_eq!(
        research_batch.groups[0].classification,
        WorkflowClassification::Skill
    );
    assert!(research_batch.groups[0].eligible);

    let automation = (0..3)
        .map(|index| {
            let id = format!("automation-{index}");
            episode(
                &id,
                if index < 2 { "session-a" } else { "session-b" },
                false,
                vec![
                    step(
                        &id,
                        &format!("{id}-cron"),
                        0,
                        "cron_create",
                        &[],
                        serde_json::json!({"schedule": "<string>"}),
                        "external",
                        Some("operation_confirmed"),
                    ),
                    step(
                        &id,
                        &format!("{id}-send"),
                        1,
                        "channel_send",
                        &[&format!("{id}-cron")],
                        serde_json::json!({"channel": "<string>", "message": "<text>"}),
                        "external",
                        Some("operation_confirmed"),
                    ),
                ],
            )
        })
        .collect::<Vec<_>>();
    let automation_batch = analyze_workflow_evidence(automation, &Default::default());
    assert_eq!(
        automation_batch.groups[0].classification,
        WorkflowClassification::Automation
    );
    assert!(automation_batch.groups[0].eligible);

    let memory = (0..3)
        .map(|index| {
            let id = format!("memory-{index}");
            episode(
                &id,
                if index < 2 { "session-a" } else { "session-b" },
                false,
                vec![step(
                    &id,
                    &format!("{id}-recall"),
                    0,
                    "memory_recall",
                    &[],
                    serde_json::json!({"query": "<text>"}),
                    "read",
                    Some("result_received"),
                )],
            )
        })
        .collect::<Vec<_>>();
    let memory_batch = analyze_workflow_evidence(memory, &Default::default());
    assert_eq!(
        memory_batch.groups[0].classification,
        WorkflowClassification::Memory
    );
    assert!(!memory_batch.groups[0].eligible);
    assert!(memory_batch.groups[0]
        .reasons
        .contains(&WorkflowRejectionReason::MemoryOnly));

    let docs = vec![
        document_workflow("refine-a", "session-a"),
        document_workflow("refine-b", "session-a"),
        document_workflow("refine-c", "session-b"),
    ];
    let initial = analyze_workflow_evidence(docs.clone(), &Default::default());
    let mut catalog = WorkflowAnalysisCatalog::default();
    catalog.existing_signatures.insert(
        initial.groups[0].signature.clone(),
        ExistingCapabilityKind::Skill,
    );
    let refined = analyze_workflow_evidence(docs, &catalog);
    assert_eq!(
        refined.groups[0].classification,
        WorkflowClassification::Refinement
    );
    assert!(refined.groups[0].eligible);
}

#[test]
fn discovery_steps_are_transparent_and_dependencies_are_reconnected() {
    let workflow = episode(
        "transparent",
        "session-a",
        true,
        vec![
            step(
                "transparent",
                "read",
                0,
                "file_read",
                &[],
                serde_json::json!({"path": "<path>"}),
                "read",
                Some("result_received"),
            ),
            step(
                "transparent",
                "discover",
                1,
                "tool_search",
                &["read"],
                serde_json::json!({"query": "<text>"}),
                "external",
                Some("operation_confirmed"),
            ),
            step(
                "transparent",
                "extract",
                2,
                "document_extract",
                &["discover"],
                serde_json::json!({"path": "<path>"}),
                "read",
                Some("result_received"),
            ),
        ],
    );

    let batch = analyze_workflow_evidence(vec![workflow], &Default::default());
    let canonical = &batch.groups[0].canonical;
    assert_eq!(canonical.nodes.len(), 2);
    assert_eq!(canonical.nodes[0].tool_name, "file_read");
    assert_eq!(canonical.nodes[1].tool_name, "document_extract");
    assert_eq!(canonical.nodes[1].dependencies, vec![0]);
}

#[test]
fn unsafe_background_and_malformed_episodes_are_rejected_before_grouping() {
    let mut secret = document_workflow("secret", "session-a");
    secret.episode.has_secret_input = true;

    let mut failed = document_workflow("failed", "session-a");
    failed.episode.status = "failed".to_string();
    failed.episode.failure_count = 1;
    failed.steps[0].status = "failed".to_string();

    let mut background = document_workflow("background", "session-a");
    background.episode.origin_channel = Some("heartbeat".to_string());

    let mut malformed = document_workflow("malformed", "session-a");
    malformed.steps[1].dependency_ids_json = r#"["missing"]"#.to_string();

    let batch = analyze_workflow_evidence(
        vec![secret, failed, background, malformed],
        &Default::default(),
    );
    assert!(batch.groups.is_empty());
    assert_eq!(batch.rejected_episodes.len(), 4);
    assert!(batch.rejected_episodes.iter().any(|rejected| {
        rejected
            .reasons
            .contains(&WorkflowRejectionReason::SecretBearingInput)
    }));
    assert!(batch.rejected_episodes.iter().any(|rejected| {
        rejected
            .reasons
            .contains(&WorkflowRejectionReason::BackgroundNoise)
    }));
    assert!(batch.rejected_episodes.iter().any(|rejected| {
        rejected
            .reasons
            .contains(&WorkflowRejectionReason::MalformedEvidence)
    }));
}

#[test]
fn pending_signature_is_suppressed_without_losing_its_reason() {
    let docs = vec![
        document_workflow("pending-a", "session-a"),
        document_workflow("pending-b", "session-a"),
        document_workflow("pending-c", "session-b"),
    ];
    let initial = analyze_workflow_evidence(docs.clone(), &Default::default());
    let mut catalog = WorkflowAnalysisCatalog::default();
    catalog
        .pending_signatures
        .insert(initial.groups[0].signature.clone());

    let suppressed = analyze_workflow_evidence(docs, &catalog);
    assert_eq!(
        suppressed.groups[0].classification,
        WorkflowClassification::None
    );
    assert!(!suppressed.groups[0].eligible);
    assert!(suppressed.groups[0]
        .reasons
        .contains(&WorkflowRejectionReason::DuplicatePending));
}
