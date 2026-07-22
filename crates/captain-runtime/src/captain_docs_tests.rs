use crate::captain_docs::{self, FAMILIES};
use std::path::PathBuf;

/// Resolve the workspace-root `docs/captain-tools/` directory from the
/// crate manifest path so the test runs identically on dev machines and
/// CI without depending on cwd.
fn docs_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at .../crates/captain-runtime
    // Workspace root is two levels up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable")
        .join("docs")
        .join("captain-tools")
}

#[test]
fn captain_docs_root_exists() {
    let root = docs_root();
    assert!(
        root.is_dir(),
        "docs/captain-tools/ scaffold missing at {}",
        root.display()
    );
    let index = root.join("README.md");
    assert!(
        index.is_file(),
        "docs/captain-tools/README.md index missing at {}",
        index.display()
    );
}

#[test]
fn every_family_has_a_stub_file() {
    let root = docs_root();
    let mut missing = Vec::new();
    for (slug, _phase) in FAMILIES {
        let path = root.join(format!("{slug}.md"));
        if !path.is_file() {
            missing.push(path.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "missing {} family stub(s):\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn family_aliases_normalize_only_when_safe() {
    assert_eq!(
        captain_docs::normalize_family_filter("changelog", "latest entry"),
        Some("runtime-changelog")
    );
    assert_eq!(
        captain_docs::normalize_family_filter("runtime_changelog", "latest entry"),
        Some("runtime-changelog")
    );
    assert_eq!(
        captain_docs::normalize_family_filter("docs", "runtime changelog latest entry"),
        Some("runtime-changelog")
    );
    assert_eq!(
        captain_docs::normalize_family_filter("docs", "memory recall"),
        None
    );
    assert_eq!(
        captain_docs::normalize_family_filter("unknown", "runtime"),
        None
    );
}

#[test]
fn runtime_changelog_alias_search_returns_runtime_family() {
    let family = captain_docs::normalize_family_filter("changelog", "latest entry");
    let hits = captain_docs::search_family_docs("latest entry", family, 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "runtime-changelog");
    assert!(hits[0].1.contains("Versioned Entries"));
}

#[test]
fn runtime_changelog_latest_query_returns_only_top_entry() {
    let hits = captain_docs::search_family_docs("latest entry", Some("runtime-changelog"), 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "runtime-changelog");
    let body = std::fs::read_to_string(docs_root().join("runtime-changelog.md"))
        .expect("runtime changelog doc should be readable");
    let headings: Vec<&str> = body
        .lines()
        .filter(|line| line.starts_with("### "))
        .collect();
    let latest = headings
        .first()
        .and_then(|line| line.trim_start_matches("### ").split(" — ").next())
        .expect("runtime changelog should have a latest entry");
    assert!(hits[0].1.contains(latest));
    if let Some(previous) = headings.get(1) {
        assert!(!hits[0].1.contains(previous));
    }
    assert!(
        hits[0].1.len() < 4_000,
        "latest entry payload should stay focused"
    );
}

/// Shared check used by every D.x family audit so the regression net
/// is identical across families: the markdown body mentions each
/// listed tool, the listed tools all exist in builtin_tool_definitions,
/// and the four canonical sections (Tools / Sandbox / Limites /
/// Exemples) stay in place so captain_docs (C.2) renders predictably.
fn assert_family_doc(slug: &str, family_tools: &[&str]) {
    let path = docs_root().join(format!("{slug}.md"));
    let body =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

    let mut doc_missing = Vec::new();
    for name in family_tools {
        if !body.contains(name) {
            doc_missing.push(*name);
        }
    }
    assert!(
        doc_missing.is_empty(),
        "{slug}.md is missing tool names: {doc_missing:?}\n→ append a section for each before merging."
    );

    let defs = crate::tool_runner::builtin_tool_definitions();
    let live: std::collections::HashSet<&str> = defs.iter().map(|t| t.name.as_str()).collect();
    let mut ghost = Vec::new();
    for name in family_tools {
        if !live.contains(*name) {
            ghost.push(*name);
        }
    }
    assert!(
        ghost.is_empty(),
        "{slug} family lists names not in builtin_tool_definitions(): {ghost:?}\n→ remove the entry or restore the tool."
    );

    for heading in ["## Tools", "## Sandbox", "## Limites", "## Exemples"] {
        assert!(
            body.contains(heading),
            "{slug}.md must keep section {heading}"
        );
    }
}

/// D.1 — file family audit (read/write/list/glob/grep/edit_file/...).
#[test]
fn d1_file_family_audit() {
    assert_family_doc("file", captain_docs::FILE_FAMILY_TOOLS);
}

/// D.2 — shell+process family audit (one-shot exec, persistent
/// processes, language wrappers, Docker sandbox).
#[test]
fn d2_shell_process_family_audit() {
    assert_family_doc("shell-process", captain_docs::SHELL_PROCESS_FAMILY_TOOLS);
}

/// D.3 — network family audit (outbound HTTP + multi-provider search).
#[test]
fn d3_network_family_audit() {
    assert_family_doc("network", captain_docs::NETWORK_FAMILY_TOOLS);
}

/// D.4 — browser family audit (headless Chrome remote-debug driver).
#[test]
fn d4_browser_family_audit() {
    assert_family_doc("browser", captain_docs::BROWSER_FAMILY_TOOLS);
}

/// D.5 — ssh family audit (russh exec + russh-sftp upload/download).
#[test]
fn d5_ssh_family_audit() {
    assert_family_doc("ssh", captain_docs::SSH_FAMILY_TOOLS);
}

/// D.6 — memory family audit (save/recall/forget +
/// legacy key-value store).
#[test]
fn d6_memory_family_audit() {
    assert_family_doc("memory", captain_docs::MEMORY_FAMILY_TOOLS);
}

/// D.7 — skill family audit (skill_execute + extensibility verbs).
#[test]
fn d7_skill_family_audit() {
    assert_family_doc("skill", captain_docs::SKILL_FAMILY_TOOLS);
}

#[test]
fn active_learning_docs_reject_retired_synthesizer_contracts() {
    let tool_docs_root = docs_root();
    let workspace_root = tool_docs_root
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root resolvable");
    let active_files = [
        tool_docs_root.join("skill.md"),
        tool_docs_root.join("config-secret.md"),
        workspace_root.join("docs/SKILL_LEARNING_V2.md"),
        workspace_root.join("captain.toml.example"),
    ];
    let retired_contracts = [
        "skill_proposal_list",
        "skill_proposal_decide",
        "/skill_approve",
        "/skill_reject",
        "/skill_proposals",
        "skills.pattern_threshold",
        "skills.proposer_model",
        "skills.fallback_models",
        "skills.min_confidence",
        "skills.reflection_provider",
        "skills.reflection_api_key_env",
    ];

    for path in active_files {
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for retired in retired_contracts {
            assert!(
                !body.contains(retired),
                "{} still advertises retired contract {retired}",
                path.display()
            );
        }
    }

    let contract = std::fs::read_to_string(workspace_root.join("docs/SKILL_LEARNING_V2.md"))
        .expect("Skill Learning V2 contract should be readable");
    assert!(contract.contains("exact active configured model"));
    assert!(contract.contains("canonical observed graph"));
}

/// D.8 — channel family audit (send + per-adapter hot-reload).
#[test]
fn d8_channel_family_audit() {
    assert_family_doc("channel", captain_docs::CHANNEL_FAMILY_TOOLS);
}

/// D.9 — agent-coordination family audit (spawn/list/kill, fleet,
/// task queue, ask_user).
#[test]
fn d9_agent_coordination_family_audit() {
    assert_family_doc(
        "agent-coordination",
        captain_docs::AGENT_COORDINATION_FAMILY_TOOLS,
    );
}

#[test]
fn agent_coordination_docs_include_agent_as_service_contract() {
    let body = std::fs::read_to_string(docs_root().join("agent-coordination.md"))
        .expect("agent-coordination docs should be readable");

    assert!(body.contains("agent-as-service"));
    assert!(body.contains("/api/agents/{id}/api/manifest"));
    assert!(body.contains("/api/agents/{id}/api/token/rotate"));
    assert!(body.contains("/hooks/agents/{id}/ingress"));
    assert!(body.contains("/api/agents/{id}/api/egress/configure"));
    assert!(body.contains("agent_api.completed"));
}

#[test]
fn live_tool_contracts_hide_frozen_surfaces() {
    let contracts = captain_docs::render_live_tool_contracts("agent-coordination")
        .expect("agent coordination contracts should render");
    assert!(!contracts.contains("### `hand_activate`"));
    assert!(!contracts.contains("### `a2a_send`"));
    assert!(!contracts.contains("### `peer_list`"));
    assert!(!contracts.contains("### `fleet_metrics`"));
    assert!(contracts.contains("### `agent_spawn`"));
    assert!(contracts.contains("Frozen surfaces are omitted"));
}

/// D.10 — scheduling family audit (cron, raw schedule, autopilot
/// Goal lifecycle + suggestions).
#[test]
fn d10_scheduling_family_audit() {
    assert_family_doc("scheduling", captain_docs::SCHEDULING_FAMILY_TOOLS);
}

/// D.11 — config-secret family audit (config.toml + secrets.env
/// CRUD with backup-and-roundtrip protection).
#[test]
fn d11_config_secret_family_audit() {
    assert_family_doc("config-secret", captain_docs::CONFIG_SECRET_FAMILY_TOOLS);
}

/// D.17 — MCP setup/recovery playbook. This family has no exclusive
/// builtin tools but must keep the canonical docs sections searchable.
#[test]
fn d17_mcp_family_audit() {
    assert_family_doc("mcp", captain_docs::MCP_FAMILY_TOOLS);
}

/// D.12 — knowledge family audit (KG entities + relations + query).
#[test]
fn d12_knowledge_family_audit() {
    assert_family_doc("knowledge", captain_docs::KNOWLEDGE_FAMILY_TOOLS);
}

/// D.13 — session-workspace family audit (cross-session recall +
/// multi-root sandbox grant).
#[test]
fn d13_session_workspace_family_audit() {
    assert_family_doc(
        "session-workspace",
        captain_docs::SESSION_WORKSPACE_FAMILY_TOOLS,
    );
}

/// D.14 — meta family audit (system_time + canvas_present +
/// captain_docs added by C.2).
#[test]
fn d14_meta_family_audit() {
    assert_family_doc("meta", captain_docs::META_FAMILY_TOOLS);
}

/// D.15 — project family audit (active project, tasks, milestones,
/// checkpoints).
#[test]
fn d15_project_family_audit() {
    assert_family_doc("project", captain_docs::PROJECT_FAMILY_TOOLS);
}

/// D.16 — multimedia family audit (image/audio/video — both input and
/// output verbs: analyze, describe, transcribe, generate, synthesize).
#[test]
fn d16_multimedia_family_audit() {
    assert_family_doc("multimedia", captain_docs::MULTIMEDIA_FAMILY_TOOLS);
}

/// D.19 — document family audit (native PDF/DOCX/HTML/Markdown creation).
#[test]
fn d19_document_family_audit() {
    assert_family_doc("document", captain_docs::DOCUMENT_FAMILY_TOOLS);
}

/// D.18 — runtime changelog family audit (public-safe versioned
/// update notes for agents after a real install/restart).
#[test]
fn d18_runtime_changelog_family_audit() {
    assert_family_doc(
        "runtime-changelog",
        captain_docs::RUNTIME_CHANGELOG_FAMILY_TOOLS,
    );
}

/// Every live builtin must belong to at least one docs family. The older
/// one-way check only caught ghost entries; this catches silent tools
/// that exist in `tool_runner` but are invisible to `captain_docs`.
#[test]
fn every_builtin_tool_has_a_docs_family() {
    let defs = crate::tool_runner::builtin_tool_definitions();
    let live: std::collections::HashSet<&str> = defs.iter().map(|t| t.name.as_str()).collect();
    let documented: std::collections::HashSet<&str> = [
        captain_docs::FILE_FAMILY_TOOLS,
        captain_docs::SHELL_PROCESS_FAMILY_TOOLS,
        captain_docs::NETWORK_FAMILY_TOOLS,
        captain_docs::BROWSER_FAMILY_TOOLS,
        captain_docs::SSH_FAMILY_TOOLS,
        captain_docs::MEMORY_FAMILY_TOOLS,
        captain_docs::SKILL_FAMILY_TOOLS,
        captain_docs::CHANNEL_FAMILY_TOOLS,
        captain_docs::AGENT_COORDINATION_FAMILY_TOOLS,
        captain_docs::SCHEDULING_FAMILY_TOOLS,
        captain_docs::CONFIG_SECRET_FAMILY_TOOLS,
        captain_docs::MCP_FAMILY_TOOLS,
        captain_docs::KNOWLEDGE_FAMILY_TOOLS,
        captain_docs::SESSION_WORKSPACE_FAMILY_TOOLS,
        captain_docs::META_FAMILY_TOOLS,
        captain_docs::PROJECT_FAMILY_TOOLS,
        captain_docs::MULTIMEDIA_FAMILY_TOOLS,
        captain_docs::DOCUMENT_FAMILY_TOOLS,
        captain_docs::RUNTIME_CHANGELOG_FAMILY_TOOLS,
    ]
    .into_iter()
    .flatten()
    .copied()
    .collect();

    let mut missing: Vec<&str> = live.difference(&documented).copied().collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "builtin tools missing from captain_docs families: {missing:?}"
    );
}

/// C.2 — `FAMILY_DOCS` entries must align 1-to-1 with `FAMILIES`.
/// A drift here means either a doc body wasn't bundled (search misses
/// it) or a slug was renamed in one place only.
#[test]
fn family_docs_match_families() {
    let docs: std::collections::HashSet<&str> =
        captain_docs::FAMILY_DOCS.iter().map(|(s, _)| *s).collect();
    let fams: std::collections::HashSet<&str> = FAMILIES.iter().map(|(s, _)| *s).collect();
    let only_in_docs: Vec<_> = docs.difference(&fams).collect();
    let only_in_fams: Vec<_> = fams.difference(&docs).collect();
    assert!(
        only_in_docs.is_empty() && only_in_fams.is_empty(),
        "FAMILY_DOCS / FAMILIES out of sync — only in docs: {only_in_docs:?}, only in families: {only_in_fams:?}"
    );
}

/// C.2 — keyword search with an explicit family returns the entire
/// family body so Captain can read it end-to-end when it knows where
/// to look.
#[test]
fn search_with_explicit_family_returns_full_body() {
    let hits = captain_docs::search_family_docs("anything", Some("file"), 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "file");
    // Body must contain the canonical sections.
    assert!(hits[0].1.contains("## Tools"));
    assert!(hits[0].1.contains("## Sandbox"));
    assert!(hits[0].1.contains("## Live Tool Schemas"));
    assert!(hits[0].1.contains("file_read"));
    assert!(hits[0].1.contains("\"required\""));
}

#[test]
fn live_tool_contracts_are_generated_from_runtime_defs() {
    let contracts = captain_docs::render_live_tool_contracts("config-secret")
        .expect("config-secret family must exist");
    assert!(contracts.contains("config_setup"));
    assert!(contracts.contains("\"integration\""));
    assert!(contracts.contains("\"credentials\""));
    assert!(!contracts.contains("\"name\""));
    assert!(!contracts.contains("\"values\""));
}

/// C.2 — multi-word query is ANDed across families. "edit_file"
/// should match the file family doc.
#[test]
fn search_multi_word_query_ands_terms() {
    let hits = captain_docs::search_family_docs("edit_file", None, 5);
    assert!(!hits.is_empty(), "edit_file should match the file family");
    assert!(hits.iter().any(|(s, _)| *s == "file"));
}

/// C.2 — a query that no family contains returns no hits, not a
/// fabricated answer.
#[test]
fn search_unrelated_query_returns_empty() {
    let hits = captain_docs::search_family_docs("xyzqwerty_nonexistent_token", None, 5);
    assert!(hits.is_empty());
}

/// C.2 — `captain_docs` is wired as a builtin tool with the expected
/// schema (query required, family optional, max_results optional).
#[test]
fn captain_docs_in_tool_registry() {
    let tools = crate::tool_runner::builtin_tool_definitions();
    let def = tools
        .iter()
        .find(|t| t.name == "captain_docs")
        .expect("captain_docs must exist in builtin_tool_definitions");
    let required = def.input_schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("query")));
    let props = def.input_schema["properties"].as_object().unwrap();
    assert!(props.contains_key("family"));
    assert!(props.contains_key("max_results"));
    assert!(
        def.description.contains("Live Tool Schemas"),
        "captain_docs description must advertise generated live schemas"
    );
}

#[test]
fn families_are_unique_and_kebab_case() {
    let mut seen = std::collections::HashSet::new();
    for (slug, _) in FAMILIES {
        assert!(seen.insert(*slug), "duplicate family slug: {slug}");
        assert!(
            slug.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "family slug must be kebab-case: {slug}"
        );
        assert!(
            !slug.starts_with('-') && !slug.ends_with('-'),
            "slug must not start or end with '-': {slug}"
        );
    }
}
