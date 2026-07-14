pub(crate) async fn tool_captain_docs(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"]
        .as_str()
        .ok_or("Missing 'query' parameter")?
        .trim();
    if query.is_empty() {
        return Err("'query' cannot be empty".into());
    }
    let requested_family = input["family"].as_str().filter(|s| !s.is_empty());
    let family = requested_family.map(|family| {
        crate::captain_docs::normalize_family_filter(family, query).unwrap_or(family)
    });
    let max_results = input["max_results"]
        .as_u64()
        .map(|n| n.clamp(1, 14) as usize)
        .unwrap_or(5);

    let hits = crate::captain_docs::search_family_docs(query, family, max_results);
    if hits.is_empty() {
        let known: Vec<&str> = crate::captain_docs::FAMILIES
            .iter()
            .map(|(s, _)| *s)
            .collect();
        return Ok(serde_json::json!({
            "hits": [],
            "total": 0,
            "query": query,
            "family": family,
            "requested_family": requested_family,
            "hint": format!(
                "no audit prose matched '{query}'. Available families: {}. Refine the query or set `family` to one of these.",
                known.join(", ")
            ),
        })
        .to_string());
    }
    let payload: Vec<serde_json::Value> = hits
        .into_iter()
        .map(|(slug, snippet)| {
            serde_json::json!({
                "family": slug,
                "snippet": snippet,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "hits": payload,
        "total": payload.len(),
        "query": query,
        "family": family,
    })
    .to_string())
}
