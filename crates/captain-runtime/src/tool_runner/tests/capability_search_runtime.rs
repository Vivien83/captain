use super::*;

fn parse_capability_response(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("capability_search must return valid JSON")
}

fn parse_capability_results(raw: &str) -> Vec<serde_json::Value> {
    let v = parse_capability_response(raw);
    v.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .expect("capability_search response must have a 'results' array")
}

#[test]
fn core_tools_constant_exposes_capability_search() {
    assert!(crate::core_tools::CORE_TOOLS.contains(&"capability_search"));
}

#[tokio::test]
async fn capability_search_finds_deferred_builtin_and_next_step() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "select:browser_navigate",
            "sources": ["builtin"],
            "max_results": 5
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    assert_eq!(results.len(), 1, "select: must isolate the builtin");
    let candidate = &results[0];
    assert_eq!(candidate["source"], "builtin");
    assert_eq!(candidate["name"], "browser_navigate");
    assert_eq!(candidate["status"], "deferred_builtin");
    assert!(
        candidate["input_schema"].is_object(),
        "deferred builtin candidates must carry schemas by default"
    );
    assert!(
        candidate["usage"]
            .as_str()
            .is_some_and(|usage| usage.contains("tool_search")),
        "deferred builtin usage must teach the exact-schema handoff"
    );
}

#[tokio::test]
async fn capability_search_hides_frozen_builtin_surfaces_by_default() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "select:hand_activate,a2a_send,peer_list,fleet_metrics",
            "sources": ["builtin"],
            "max_results": 10
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    assert!(
        results.is_empty(),
        "frozen builtin tools must not be surfaced by capability_search: {results:?}"
    );
}

#[tokio::test]
async fn capability_search_finds_document_create_for_pdf_requests() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "créer générer pdf rapport document docx synthèse",
            "sources": ["builtin"],
            "max_results": 5
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    assert!(
        results
            .iter()
            .any(|candidate| candidate["name"] == "document_create"),
        "PDF/document requests must discover document_create, got {results:?}"
    );
}

#[tokio::test]
async fn capability_search_finds_pdf_source_intake_tools() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "telecharger analyser extraire pdf rapport source citations",
            "sources": ["builtin"],
            "max_results": 10
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    let names: std::collections::HashSet<&str> = results
        .iter()
        .filter_map(|candidate| candidate["name"].as_str())
        .collect();
    assert!(
        names.contains("web_download"),
        "PDF source intake must discover web_download, got {results:?}"
    );
    assert!(
        names.contains("document_extract"),
        "PDF source intake must discover document_extract, got {results:?}"
    );
}

#[tokio::test]
async fn capability_search_finds_grouped_p0_p1_rails() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "select:web_research_batch,file_inspect_batch,ssh_health_check,document_pipeline,memory_context_batch,media_pipeline,channel_delivery_batch",
            "sources": ["builtin"],
            "max_results": 10
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    let names: std::collections::HashSet<&str> = results
        .iter()
        .filter_map(|candidate| candidate["name"].as_str())
        .collect();
    for expected in [
        "web_research_batch",
        "file_inspect_batch",
        "ssh_health_check",
        "document_pipeline",
        "memory_context_batch",
        "media_pipeline",
        "channel_delivery_batch",
    ] {
        assert!(
            names.contains(expected),
            "grouped rail {expected} must be discoverable, got {results:?}"
        );
    }
}

#[tokio::test]
async fn capability_search_finds_docs_family_for_recovery() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "select:ssh",
            "sources": ["docs"],
            "max_results": 3
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    assert_eq!(results.len(), 1, "select:ssh must return the docs family");
    let candidate = &results[0];
    assert_eq!(candidate["source"], "docs_family");
    assert_eq!(candidate["name"], "ssh");
    assert!(
        candidate["usage"].as_str().is_some_and(
            |usage| usage.contains("captain_docs") && usage.contains("Live Tool Schemas")
        ),
        "docs candidates must route to captain_docs and mention live schemas"
    );
}

#[tokio::test]
async fn capability_search_finds_installed_skill_tool() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("deploy-helper");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        r#"
[skill]
name = "deploy-helper"
version = "0.1.0"
description = "Reusable deployment diagnostics and rollout checks"

[[tools.provided]]
name = "deploy_check"
description = "Check deployment health for a named service"
input_schema = { type = "object", properties = { service = { type = "string" } }, required = ["service"] }
"#,
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_skill(&skill_dir).unwrap();

    let res = tool_capability_search(
        &serde_json::json!({
            "query": "select:deploy_check",
            "sources": ["skill"],
            "max_results": 5
        }),
        Some(&registry),
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let results = parse_capability_results(&res);
    assert_eq!(
        results.len(),
        1,
        "exact skill tool lookup must return one result"
    );
    let candidate = &results[0];
    assert_eq!(candidate["source"], "skill_tool");
    assert_eq!(candidate["name"], "deploy_check");
    assert_eq!(candidate["metadata"]["skill"], "deploy-helper");
    assert!(candidate["input_schema"].is_object());
}

#[test]
fn skill_search_filters_by_family_and_can_include_context() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("generated");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("debug-helper.md"),
        r#"
---
name: debug-helper
description: Systematic debugging workflow for failing tests
family: software-development
tags:
  - generated
---
Reproduce the failure, isolate the minimal case, patch the root cause, then rerun the focused test.
"#,
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();
    let res = tool_skill_search(
        &serde_json::json!({
            "family": "software-development",
            "query": "debug failing test",
            "include_context": true
        }),
        Some(&registry),
    )
    .expect("skill_search must succeed");
    let response: serde_json::Value = serde_json::from_str(&res).unwrap();
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "debug-helper");
    assert_eq!(results[0]["family"]["id"], "software-development");
    assert_eq!(results[0]["file_backed"], true);
    assert!(
        results[0].get("path").is_none(),
        "skill_search must not expose local skill paths"
    );
    assert!(
        !res.contains(dir.path().to_string_lossy().as_ref()),
        "skill_search output must not contain the temp skill root"
    );
    assert!(results[0]["context_excerpt"]
        .as_str()
        .is_some_and(|excerpt| excerpt.contains("failure")));
}

#[test]
fn skill_search_empty_query_returns_minimal_index() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("generated");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("plan-helper.md"),
        r#"
---
name: plan-helper
description: Compact project planning workflow
family: project-management
---
Plan the work, identify risks, and verify the result.
"#,
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();
    let res = tool_skill_search(&serde_json::json!({}), Some(&registry))
        .expect("skill_search must return the minimal index");
    let response: serde_json::Value = serde_json::from_str(&res).unwrap();
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "plan-helper");
    assert_eq!(results[0]["file_backed"], true);
    assert!(
        results[0].get("path").is_none(),
        "minimal skill index must not expose local skill paths"
    );
    assert!(response["hint"]
        .as_str()
        .is_some_and(|hint| hint.contains("skill_view")));
}

#[test]
fn skill_search_select_exact_name_isolated_result() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("generated");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("debug-helper.md"),
        r#"
---
name: debug-helper
description: Systematic debugging workflow for failing tests
family: software-development
---
Reproduce the failure and rerun the focused test.
"#,
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("plan-helper.md"),
        r#"
---
name: plan-helper
description: Compact project planning workflow
family: project-management
---
Plan the work, identify risks, and verify the result.
"#,
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.path().to_path_buf());
    registry.load_all().unwrap();
    let res = tool_skill_search(
        &serde_json::json!({
            "query": "select:plan-helper",
            "include_families": false
        }),
        Some(&registry),
    )
    .expect("skill_search select must succeed");
    let response: serde_json::Value = serde_json::from_str(&res).unwrap();
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "plan-helper");
    assert_eq!(results[0]["score"], 1000);
    assert!(response.get("families").is_none());
}

#[tokio::test]
async fn capability_search_unmatched_query_guides_autonomy() {
    let res = tool_capability_search(
        &serde_json::json!({
            "query": "xyzzy_no_such_capability_anywhere",
            "sources": ["builtin"],
            "max_results": 3
        }),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("capability_search must succeed");
    let response = parse_capability_response(&res);
    assert!(
        response["results"].as_array().is_some_and(|r| r.is_empty()),
        "unmatched query must keep an empty results array"
    );
    assert_eq!(
        response["searched_sources"],
        serde_json::json!(["builtin"]),
        "searched_sources must reflect the source filter"
    );
    let hint = response["hint"]
        .as_str()
        .expect("unmatched query must include a recovery hint");
    assert!(hint.contains("captain_docs"));
    assert!(hint.contains("MCP"));
    assert!(hint.contains("scaffold_skill"));
}

#[test]
fn lexical_score_weighs_name_double() {
    let tool = ToolDefinition {
        name: "browser_navigate".into(),
        description: "Open a URL".into(),
        input_schema: serde_json::json!({}),
    };
    let toks = vec!["browser".to_string()];
    assert_eq!(crate::tools::lexical_tool_score(&toks, &tool), 2);

    let tool2 = ToolDefinition {
        name: "foo".into(),
        description: "Manage browser windows".into(),
        input_schema: serde_json::json!({}),
    };
    assert_eq!(crate::tools::lexical_tool_score(&toks, &tool2), 1);
}
