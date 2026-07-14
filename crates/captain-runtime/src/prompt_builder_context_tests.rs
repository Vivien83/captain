use super::*;

fn basic_ctx() -> PromptContext {
    PromptContext {
        agent_name: "researcher".to_string(),
        agent_description: "Research agent".to_string(),
        base_system_prompt: "You are Researcher, a research agent.".to_string(),
        granted_tools: vec![
            "web_search".to_string(),
            "web_fetch".to_string(),
            "file_read".to_string(),
            "file_write".to_string(),
            "memory_save".to_string(),
            "memory_recall".to_string(),
        ],
        ..Default::default()
    }
}

fn assert_contains_in_order(haystack: &str, needles: &[&str]) {
    let mut cursor = 0;
    for needle in needles {
        let relative = haystack[cursor..]
            .find(needle)
            .unwrap_or_else(|| panic!("missing ordered marker: {needle}"));
        cursor += relative + needle.len();
    }
}

#[test]
fn active_project_prompt_includes_context_capsule() {
    let mut ctx = basic_ctx();
    ctx.active_project = Some(ActiveProjectSummary {
        id: "project-1".to_string(),
        slug: "calc-python".to_string(),
        name: "Calculatrice Python".to_string(),
        goal: "Créer une calculatrice CLI fiable.".to_string(),
        status: "active".to_string(),
        source_type: Some("local".to_string()),
        workspace_path: Some("/tmp/calc-python".to_string()),
        repository: None,
        latest_checkpoint: Some("Plan validé, reste à écrire les tests.".to_string()),
        active_tasks: vec!["BUILD: implémenter main.py".to_string()],
        blocked_tasks: vec!["VERIFY: attente pytest".to_string()],
        next_actions: vec!["Lancer pytest puis checkpoint_save".to_string()],
        milestone_status: Some("1/3 complete, 0 missed".to_string()),
        project_goals: vec!["goal-smoke: garder le smoke test vert".to_string()],
        project_rules: Some("Run pytest before checkpoint_save.".to_string()),
    });

    let prompt = build_system_prompt(&ctx);

    assert!(prompt.contains("## Active Project"));
    assert!(prompt.contains("Latest checkpoint"));
    assert!(prompt.contains("BUILD: implémenter main.py"));
    assert!(prompt.contains("goal-smoke"));
    assert!(prompt.contains("Run pytest before checkpoint_save"));
    assert!(prompt.contains("OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN"));
    assert!(prompt.contains("checkpoint_save"));
}

#[test]
fn active_project_compact_prompt_uses_compact_labels() {
    let mut ctx = basic_ctx();
    ctx.prompt_profile = PromptProfile::CodexEconomy;
    ctx.active_project = Some(ActiveProjectSummary {
        id: "project-1".to_string(),
        slug: "calc-python".to_string(),
        name: "Calculatrice Python".to_string(),
        goal: "Créer une calculatrice CLI fiable.".to_string(),
        status: "active".to_string(),
        source_type: Some("local".to_string()),
        workspace_path: Some("/tmp/calc-python".to_string()),
        repository: Some("git@example.com:calc.git".to_string()),
        latest_checkpoint: Some("Plan validé.".to_string()),
        active_tasks: vec!["BUILD: implémenter main.py".to_string()],
        blocked_tasks: vec![],
        next_actions: vec!["Lancer pytest".to_string()],
        milestone_status: Some("1/3 complete".to_string()),
        project_goals: vec!["goal-smoke: garder le smoke test vert".to_string()],
        project_rules: Some("Run pytest before checkpoint_save.".to_string()),
    });

    let prompt = build_system_prompt(&ctx);

    assert!(prompt.contains("## Active Project"));
    assert!(prompt.contains("goal: Créer une calculatrice CLI fiable."));
    assert!(prompt.contains("latest_checkpoint: Plan validé."));
    assert!(prompt.contains("active_tasks:"));
    assert!(prompt.contains("loop: OBSERVE -> THINK -> PLAN"));
    assert!(!prompt.contains("Latest checkpoint"));
}

#[test]
fn recent_projects_prompt_keeps_project_aliases_visible() {
    let mut ctx = basic_ctx();
    ctx.recent_projects = vec![RecentProjectSummary {
        slug: "projet1-documents-couple".to_string(),
        name: "Projet1 — Gestion documents couple".to_string(),
        goal: "Développer une app locale de gestion documentaire du couple.".to_string(),
        status: "planning".to_string(),
        runtime_status: "ready".to_string(),
        runtime_phase: "observe".to_string(),
        progress: 10,
        next_actions: vec![
            "project_list {\"query\":\"projet1-documents-couple\"}".to_string(),
            "project_get {\"slug\":\"projet1-documents-couple\"}".to_string(),
        ],
    }];

    let prompt = build_system_prompt(&ctx);

    assert!(prompt.contains("## Recent Projects"));
    assert!(prompt.contains("projet1-documents-couple"));
    assert!(prompt.contains("runtime ready/observe, 10%"));
    assert!(prompt.contains("before any filesystem/workspace search"));
    assert!(prompt.contains("before interpreting a number as a menu option"));
    assert!(prompt.contains("Only inspect files after the durable project state"));
    assert!(prompt.contains("project_get"));
}

#[test]
fn recent_projects_compact_prompt_stays_short_but_actionable() {
    let mut ctx = basic_ctx();
    ctx.prompt_profile = PromptProfile::CodexEconomy;
    ctx.recent_projects = vec![RecentProjectSummary {
        slug: "projet1-documents-couple".to_string(),
        name: "Projet1 — Gestion documents couple".to_string(),
        goal: "Développer une app locale de gestion documentaire du couple.".to_string(),
        status: "planning".to_string(),
        runtime_status: "ready".to_string(),
        runtime_phase: "observe".to_string(),
        progress: 10,
        next_actions: vec!["project_get {\"slug\":\"projet1-documents-couple\"}".to_string()],
    }];

    let prompt = build_system_prompt(&ctx);

    assert!(prompt.contains("## Recent Projects"));
    assert!(prompt.contains("projet1-documents-couple"));
    assert!(prompt.contains("Resolve user refs like `projet1`"));
    assert!(prompt.contains("Projects store first; do not search files first"));
    assert!(!prompt.contains("project_get"));
}

#[test]
fn dynamic_prompt_suffix_keeps_turn_context_order() {
    let mut ctx = basic_ctx();
    ctx.active_project = Some(ActiveProjectSummary {
        id: "project-1".to_string(),
        slug: "ops-check".to_string(),
        name: "Ops Check".to_string(),
        goal: "Keep runtime checks visible.".to_string(),
        status: "active".to_string(),
        source_type: None,
        workspace_path: None,
        repository: None,
        latest_checkpoint: None,
        active_tasks: vec![],
        blocked_tasks: vec![],
        next_actions: vec![],
        milestone_status: None,
        project_goals: vec![],
        project_rules: None,
    });
    ctx.current_date = Some("Saturday, June 20, 2026 (2026-06-20)".into());
    ctx.configured_language = Some("fr".into());
    ctx.deployment_profile = Some("vps".into());
    ctx.persistent_memory_capsule = Some("- [facts] durable fact".into());
    ctx.recalled_memories = vec![("project_hint".into(), "dynamic memory".into())];
    ctx.recent_journal = Some("operator restarted the daemon".into());
    ctx.feedback_rules = Some("prefer short summaries".into());
    ctx.user_name = Some("Alex".into());
    ctx.channel_type = Some("telegram".into());
    ctx.sender_name = Some("Alex".into());
    ctx.sender_id = Some("42".into());
    ctx.peer_agents = vec![("worker".into(), "running".into(), "flash".into())];

    let built = build_system_prompt_with_cache(&ctx);
    let split = built.cacheable_prefix_bytes.unwrap();
    let prefix = &built.system_prompt[..split];
    let suffix = &built.system_prompt[split..];

    assert!(!prefix.contains("## Current Turn Context"));
    assert_contains_in_order(
        suffix,
        &[
            "## Current Turn Context",
            "## Active Project",
            "## Current Date",
            "## Language Contract",
            "## Deployment Context",
            "### Persistent memory capsule",
            "### Recalled memories",
            "## Recent Activity Journal",
            "## Learned Rules",
            "## User Profile",
            "## Channel",
            "## Sender",
            "## Peer Agents",
        ],
    );
}

#[test]
fn cacheable_prefix_excludes_turn_dynamic_sections() {
    let mut ctx = basic_ctx();
    ctx.current_date = Some("Monday, May 4, 2026 (2026-05-04)".into());
    ctx.recalled_memories = vec![("project_hint".into(), "dynamic memory".into())];
    ctx.persistent_memory_capsule =
        Some("- [learnings/user_preferences] user prefers Telegram approvals".into());
    ctx.sender_name = Some("Alex".into());

    let built = build_system_prompt_with_cache(&ctx);
    let split = built.cacheable_prefix_bytes.unwrap();
    let prefix = &built.system_prompt[..split];

    assert!(prefix.contains("## Memory"));
    assert!(prefix.contains("Use it proactively"));
    assert!(!prefix.contains("## Current Turn Context"));
    assert!(!prefix.contains("## Current Date"));
    assert!(!prefix.contains("dynamic memory"));
    assert!(!prefix.contains("Telegram approvals"));
    assert!(!prefix.contains("Alex"));
    assert!(built.system_prompt[split..].contains("## Current Turn Context"));
    assert!(built.system_prompt[split..].contains("## Current Date"));
    assert!(built.system_prompt[split..].contains("dynamic memory"));
    assert!(built.system_prompt[split..].contains("Telegram approvals"));
    assert!(built.system_prompt[split..].contains("Alex"));
}

#[test]
fn cacheable_prefix_is_stable_across_date_changes() {
    let mut first = basic_ctx();
    let mut second = basic_ctx();
    first.current_date = Some("Monday, May 4, 2026 (2026-05-04)".into());
    second.current_date = Some("Tuesday, May 5, 2026 (2026-05-05)".into());

    let first = build_system_prompt_with_cache(&first);
    let second = build_system_prompt_with_cache(&second);
    let first_prefix = &first.system_prompt[..first.cacheable_prefix_bytes.unwrap()];
    let second_prefix = &second.system_prompt[..second.cacheable_prefix_bytes.unwrap()];

    assert_eq!(first_prefix, second_prefix);
    assert_ne!(first.system_prompt, second.system_prompt);
}

#[test]
fn test_canonical_context_not_in_system_prompt() {
    let mut ctx = basic_ctx();
    ctx.canonical_context = Some("User was discussing Rust async patterns last time.".to_string());
    let prompt = build_system_prompt(&ctx);
    // Canonical context should NOT be in system prompt (moved to user message)
    assert!(!prompt.contains("## Previous Conversation Context"));
    assert!(!prompt.contains("Rust async patterns"));
    // But should be available via build_canonical_context_message
    let msg = build_canonical_context_message(&ctx);
    assert!(msg.is_some());
    let msg = msg.unwrap();
    assert!(msg.contains("Rust async patterns"));
    assert!(msg.contains("reference de compaction"));
    assert!(msg.contains("pas une nouvelle demande utilisateur"));
    assert!(msg.contains("dernier message utilisateur"));
}

#[test]
fn test_canonical_context_keeps_compaction_summary_depth() {
    let mut ctx = basic_ctx();
    let long_summary = "important project memory. ".repeat(200);
    ctx.canonical_context = Some(long_summary.clone());

    let msg = build_canonical_context_message(&ctx).expect("context message");

    assert!(msg.contains("important project memory"));
    assert!(
        msg.len() > 4_000,
        "compacted memory must not be crushed to a tiny teaser"
    );
    assert!(msg.contains(long_summary.trim_end()));
}

#[test]
fn test_canonical_context_omitted_for_subagent() {
    let mut ctx = basic_ctx();
    ctx.is_subagent = true;
    ctx.canonical_context = Some("Previous context here.".to_string());
    let prompt = build_system_prompt(&ctx);
    assert!(!prompt.contains("Previous Conversation Context"));
    // Should also be None from build_canonical_context_message
    assert!(build_canonical_context_message(&ctx).is_none());
}
