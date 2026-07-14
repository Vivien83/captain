use super::*;

fn parse_tool_search_results(raw: &str) -> Vec<serde_json::Value> {
    let v = parse_tool_search_response(raw);
    v.get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .expect("tool_search response must have a 'results' array")
}

fn parse_tool_search_response(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("tool_search must return valid JSON")
}

#[test]
fn core_tools_constant_exposes_tool_search() {
    assert!(crate::core_tools::CORE_TOOLS.contains(&"tool_search"));
}

#[test]
fn every_core_tool_has_a_builtin_definition() {
    // Cross-check: each name in CORE_TOOLS must be registered in
    // builtin_tool_definitions, otherwise CORE points at vapor.
    let defs = builtin_tool_definitions();
    let registered: std::collections::HashSet<&str> =
        defs.iter().map(|t| t.name.as_str()).collect();
    for core in crate::core_tools::CORE_TOOLS {
        assert!(
            registered.contains(core),
            "CORE_TOOLS lists '{core}' but builtin_tool_definitions() does not register it"
        );
    }
}

#[tokio::test]
async fn tool_search_returns_browser_tools_for_browser_query() {
    let res = tool_search(&serde_json::json!({ "query": "browser" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(
        !results.is_empty(),
        "browser query must return at least one result"
    );
    let names: Vec<&str> = results
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("browser_")),
        "expected at least one browser_* tool, got {names:?}"
    );
}

#[tokio::test]
async fn tool_search_excludes_core_tools_even_on_strong_match() {
    // capability_search is CORE — must never appear in tool_search output,
    // even though the exact name matches.
    let res = tool_search(&serde_json::json!({ "query": "capability_search" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    let names: Vec<&str> = results
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.contains(&"capability_search"),
        "tool_search MUST NOT return CORE tools (capability_search leaked: {names:?})"
    );
}

#[tokio::test]
async fn tool_search_select_syntax_returns_exact_match() {
    let res = tool_search(&serde_json::json!({ "query": "select:browser_click" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert_eq!(results.len(), 1, "select: must return exactly one match");
    assert_eq!(results[0]["name"].as_str().unwrap(), "browser_click");
    assert!(
        results[0]["input_schema"].is_object(),
        "select: must include the input_schema for the matched tool"
    );
}

#[tokio::test]
async fn tool_search_can_discover_skill_check() {
    let res = tool_search(&serde_json::json!({ "query": "select:skill_check" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert_eq!(results.len(), 1, "select:skill_check must resolve");
    assert_eq!(results[0]["name"].as_str().unwrap(), "skill_check");
    assert!(results[0]["description"]
        .as_str()
        .is_some_and(|desc| desc.contains("Préflight statique")));
}

#[tokio::test]
async fn tool_search_select_can_request_multiple_names() {
    let res = tool_search(&serde_json::json!({ "query": "select:browser_click,browser_type" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    let names: Vec<&str> = results
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"browser_click"));
    assert!(names.contains(&"browser_type"));
}

#[tokio::test]
async fn tool_search_select_skips_core_and_unknown_names() {
    // capability_search is core → should be skipped (not surfaced via select:).
    // 'nonexistent_tool' doesn't exist → skipped silently.
    // browser_click is deferred → kept.
    let res = tool_search(
        &serde_json::json!({ "query": "select:capability_search,nonexistent_tool,browser_click" }),
    )
    .await
    .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    let names: Vec<&str> = results
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["browser_click"]);
}

#[tokio::test]
async fn tool_search_hides_frozen_surfaces_by_default() {
    let res = tool_search(
        &serde_json::json!({ "query": "select:hand_activate,a2a_send,peer_list,fleet_metrics" }),
    )
    .await
    .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(
        results.is_empty(),
        "frozen surfaces must not enter active tool discovery: {results:?}"
    );
}

#[tokio::test]
async fn tool_search_respects_max_results() {
    let res = tool_search(&serde_json::json!({ "query": "browser", "max_results": 2 }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(
        results.len() <= 2,
        "must not exceed max_results: got {}",
        results.len()
    );
}

#[tokio::test]
async fn tool_search_clamps_max_results_to_20() {
    let res = tool_search(&serde_json::json!({ "query": "the", "max_results": 9999 }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(
        results.len() <= 20,
        "max_results must be clamped to 20, got {}",
        results.len()
    );
}

#[tokio::test]
async fn tool_search_empty_query_returns_empty_results() {
    let res = tool_search(&serde_json::json!({ "query": "" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(results.is_empty(), "empty query must yield no results");
    let response = parse_tool_search_response(&res);
    assert!(
        response["hint"]
            .as_str()
            .is_some_and(|h| h.contains("capability keywords")),
        "empty query must tell the agent how to search next"
    );
}

#[tokio::test]
async fn tool_search_missing_query_errors() {
    let res = tool_search(&serde_json::json!({})).await;
    assert!(res.is_err(), "missing 'query' parameter must error");
}

#[tokio::test]
async fn tool_search_unknown_query_returns_empty_results() {
    let res = tool_search(&serde_json::json!({ "query": "xyzzy_no_such_thing_anywhere" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(results.is_empty(), "unmatched query must yield no results");
    let response = parse_tool_search_response(&res);
    let hint = response["hint"]
        .as_str()
        .expect("unmatched query must include an autonomy hint");
    assert!(hint.contains("deferred builtin Captain tools"));
    assert!(hint.contains("capability_search"));
    assert!(hint.contains("MCP server"));
    assert!(hint.contains("captain_docs"));
}

#[tokio::test]
async fn tool_search_results_carry_input_schemas() {
    let res = tool_search(&serde_json::json!({ "query": "browser navigate" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(!results.is_empty());
    for r in &results {
        assert!(r["name"].is_string());
        assert!(r["description"].is_string());
        assert!(
            r["input_schema"].is_object(),
            "every result must carry an input_schema object so the LLM can call it next turn"
        );
    }
}

#[tokio::test]
async fn tool_search_finds_browser_batch_for_grouped_browser_actions() {
    let res = tool_search(&serde_json::json!({ "query": "browser grouped batch actions" }))
        .await
        .expect("tool_search must succeed");
    let results = parse_tool_search_results(&res);
    assert!(
        results.iter().any(|r| r["name"] == "browser_batch"),
        "grouped browser work must discover browser_batch, got {results:?}"
    );
}
