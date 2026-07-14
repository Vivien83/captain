use super::kernel_agent_runtime::{
    apply_subagent_lineage_metadata, is_lean_direct_turn, normalize_subagent_tool_scope,
};
use super::*;
use std::collections::HashMap;

fn test_manifest(name: &str, description: &str, tags: Vec<String>) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        description: description.to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        schedule: ScheduleMode::default(),
        model: ModelConfig::default(),
        fallback_models: vec![],
        resources: ResourceQuota::default(),
        priority: Priority::default(),
        capabilities: ManifestCapabilities::default(),
        profile: None,
        tools: HashMap::new(),
        skills: vec![],
        mcp_servers: vec![],
        metadata: HashMap::new(),
        tags,
        routing: None,
        autonomous: None,
        pinned_model: None,
        workspace: None,
        generate_identity_files: true,
        exec_policy: None,
        tool_allowlist: vec![],
        tool_blocklist: vec![],
        orchestration_mode: captain_types::agent::OrchestrationMode::default(),
    }
}

fn test_agent_entry(id: AgentId, manifest: AgentManifest) -> AgentEntry {
    AgentEntry {
        id,
        name: manifest.name.clone(),
        tags: manifest.tags.clone(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent: None,
        children: vec![],
        session_id: SessionId::new(),
        identity: Default::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        mission: None,
        mission_set_at: None,
        autoscale: None,
        last_scale_event: None,
    }
}

#[test]
fn spawned_child_manifest_gets_subagent_lineage_metadata() {
    let parent_id = AgentId::new();
    let parent_manifest = test_manifest("parent", "parent", vec![]);
    let parent_entry = test_agent_entry(parent_id, parent_manifest);
    let mut child = test_manifest("child", "child", vec![]);
    let parent_id_string = parent_id.to_string();

    apply_subagent_lineage_metadata(&mut child, Some(parent_id), Some(&parent_entry));

    assert_eq!(
        child.metadata.get("is_subagent").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        child
            .metadata
            .get("parent_agent_id")
            .and_then(|v| v.as_str()),
        Some(parent_id_string.as_str())
    );
    assert_eq!(
        child.metadata.get("root_agent_id").and_then(|v| v.as_str()),
        Some(parent_id_string.as_str())
    );
    assert_eq!(
        child
            .metadata
            .get("subagent_depth")
            .and_then(|v| v.as_u64()),
        Some(1)
    );
}

#[test]
fn spawned_grandchild_inherits_root_and_increments_depth() {
    let root_id = AgentId::new();
    let parent_id = AgentId::new();
    let mut parent_manifest = test_manifest("parent", "parent", vec![]);
    parent_manifest.metadata.insert(
        "root_agent_id".to_string(),
        serde_json::json!(root_id.to_string()),
    );
    parent_manifest
        .metadata
        .insert("subagent_depth".to_string(), serde_json::json!(2));
    let parent_entry = test_agent_entry(parent_id, parent_manifest);
    let mut child = test_manifest("child", "child", vec![]);
    let root_id_string = root_id.to_string();

    apply_subagent_lineage_metadata(&mut child, Some(parent_id), Some(&parent_entry));

    assert_eq!(
        child.metadata.get("root_agent_id").and_then(|v| v.as_str()),
        Some(root_id_string.as_str())
    );
    assert_eq!(
        child
            .metadata
            .get("subagent_depth")
            .and_then(|v| v.as_u64()),
        Some(3)
    );
}

#[test]
fn test_send_to_agent_by_name_resolution() {
    let registry = AgentRegistry::new();
    let manifest = test_manifest("coder", "A coder agent", vec!["coding".to_string()]);
    let agent_id = AgentId::new();
    let entry = AgentEntry {
        id: agent_id,
        name: "coder".to_string(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent: None,
        children: vec![],
        session_id: SessionId::new(),
        tags: vec!["coding".to_string()],
        identity: Default::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        mission: None,
        mission_set_at: None,
        autoscale: None,
        last_scale_event: None,
    };
    registry.register(entry).unwrap();

    let found = registry.find_by_name("coder");
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, agent_id);

    let found_by_id = registry.get(agent_id);
    assert!(found_by_id.is_some());
}

#[test]
fn test_find_agents_by_tag() {
    let registry = AgentRegistry::new();

    let m1 = test_manifest(
        "coder",
        "Expert coder",
        vec!["coding".to_string(), "rust".to_string()],
    );
    let e1 = AgentEntry {
        id: AgentId::new(),
        name: "coder".to_string(),
        manifest: m1,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent: None,
        children: vec![],
        session_id: SessionId::new(),
        tags: vec!["coding".to_string(), "rust".to_string()],
        identity: Default::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        mission: None,
        mission_set_at: None,
        autoscale: None,
        last_scale_event: None,
    };
    registry.register(e1).unwrap();

    let m2 = test_manifest(
        "auditor",
        "Security auditor",
        vec!["security".to_string(), "audit".to_string()],
    );
    let e2 = AgentEntry {
        id: AgentId::new(),
        name: "auditor".to_string(),
        manifest: m2,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        parent: None,
        children: vec![],
        session_id: SessionId::new(),
        tags: vec!["security".to_string(), "audit".to_string()],
        identity: Default::default(),
        onboarding_completed: false,
        onboarding_completed_at: None,
        mission: None,
        mission_set_at: None,
        autoscale: None,
        last_scale_event: None,
    };
    registry.register(e2).unwrap();

    let agents = registry.list();
    let security_agents: Vec<_> = agents
        .iter()
        .filter(|a| a.tags.iter().any(|t| t.to_lowercase().contains("security")))
        .collect();
    assert_eq!(security_agents.len(), 1);
    assert_eq!(security_agents[0].name, "auditor");

    let code_agents: Vec<_> = agents
        .iter()
        .filter(|a| a.name.to_lowercase().contains("coder"))
        .collect();
    assert_eq!(code_agents.len(), 1);
    assert_eq!(code_agents[0].name, "coder");
}

#[test]
fn subagent_scope_normalization_expands_profile_to_explicit_allowlist() {
    let mut manifest = AgentManifest {
        profile: Some(ToolProfile::Coding),
        ..Default::default()
    };

    normalize_subagent_tool_scope(&mut manifest);

    assert_eq!(manifest.tool_allowlist, manifest.capabilities.tools);
    assert!(manifest.tool_allowlist.contains(&"file_read".to_string()));
    assert!(manifest.tool_allowlist.contains(&"shell_exec".to_string()));
    assert!(manifest
        .tool_allowlist
        .contains(&"capability_search".to_string()));
    assert!(manifest.tool_allowlist.contains(&"tool_search".to_string()));
    assert!(manifest.capabilities.shell.contains(&"*".to_string()));
    assert!(manifest.capabilities.network.contains(&"*".to_string()));
}

#[test]
fn subagent_scope_normalization_keeps_explicit_tools_and_adds_defaults() {
    let mut manifest = AgentManifest {
        tool_allowlist: vec!["file_read".to_string()],
        ..Default::default()
    };

    normalize_subagent_tool_scope(&mut manifest);

    assert!(manifest.tool_allowlist.contains(&"file_read".to_string()));
    assert!(manifest
        .tool_allowlist
        .contains(&"capability_search".to_string()));
    assert!(manifest.tool_allowlist.contains(&"tool_search".to_string()));
    assert!(!manifest.tool_allowlist.contains(&"shell_exec".to_string()));
    assert_eq!(manifest.tool_allowlist, manifest.capabilities.tools);
}

#[test]
fn codex_background_model_sanitizer_rejects_claude_and_incompatible_codex_names() {
    let catalog = captain_runtime::model_catalog::ModelCatalog::new();
    let expected_primary = kernel_agent_runtime::default_codex_background_model(&catalog);
    assert_eq!(expected_primary, "gpt-5.5");
    assert_eq!(
        normalize_background_model_for_provider(&catalog, "codex", "claude-haiku-4-5"),
        expected_primary
    );
    assert_eq!(
        normalize_background_model_for_provider(&catalog, "codex", "codex/gpt-5.3-codex"),
        expected_primary
    );
    assert_eq!(
        normalize_background_model_for_provider(&catalog, "codex", "gpt-5.3-codex-spark"),
        expected_primary
    );
    assert_eq!(
        normalize_background_model_for_provider(&catalog, "codex", "codex/gpt-5.4"),
        "gpt-5.4"
    );
    let fallbacks = vec![
        "claude-sonnet-4-6".to_string(),
        "codex/gpt-5.3-codex".to_string(),
        "codex/gpt-5.4".to_string(),
        "codex/gpt-5.5".to_string(),
    ];
    assert_eq!(
        normalize_background_fallbacks_for_provider(&catalog, "codex", "gpt-5.5", &fallbacks,),
        vec!["gpt-5.4".to_string()]
    );
}

#[test]
fn lean_direct_classifier_is_conservative() {
    assert!(is_lean_direct_turn("hey"));
    assert!(is_lean_direct_turn(
        "Réponds exactement API_OK. N'utilise aucun outil."
    ));
    assert!(is_lean_direct_turn(
        "Réponds en deux mots: STREAM OK. Aucun outil."
    ));
    assert!(is_lean_direct_turn("just say OK"));
    assert!(!is_lean_direct_turn("check ton changelog"));
    assert!(!is_lean_direct_turn("tu peux check si mon vps va bien ?"));
    assert!(!is_lean_direct_turn(
        "analyse le contexte et réponds en trois points"
    ));
}
