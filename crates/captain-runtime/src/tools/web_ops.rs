//! Web fetch/search batching handlers.

use crate::tools::{
    check_taint_net_fetch, collect_string_list, ensure_no_secret_literal, truncate_owned,
};
use crate::web_search::{parse_ddg_results, WebToolsContext};
use std::collections::HashSet;
use std::time::Duration;

const MAX_WEB_RESEARCH_QUERIES: usize = 5;
const MAX_WEB_RESEARCH_FETCHES: usize = 10;

/// Legacy web fetch used when WebToolsContext is unavailable.
pub(crate) async fn tool_web_fetch_legacy(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    if let Some(len) = resp.content_length() {
        if len > 10 * 1024 * 1024 {
            return Err(format!("Response too large: {len} bytes (max 10MB)"));
        }
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    let max_len = 50_000;
    let truncated = if body.len() > max_len {
        format!(
            "{}... [truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&body, max_len),
            body.len()
        )
    } else {
        body
    };
    Ok(format!("HTTP {status}\n\n{truncated}"))
}

/// Legacy DuckDuckGo HTML search used when WebToolsContext is unavailable.
pub(crate) async fn tool_web_search_legacy(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

    tracing::debug!(query, "Executing web search via DuckDuckGo HTML");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; CaptainAgent/0.1)")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read search response: {e}"))?;

    let results = parse_ddg_results(&body, max_results);
    if results.is_empty() {
        return Ok(format!("No results found for '{query}'."));
    }

    let mut output = format!("Search results for '{query}':\n\n");
    for (idx, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            idx + 1,
            title,
            url,
            snippet
        ));
    }
    Ok(output)
}

pub(crate) async fn tool_web_research_batch(
    input: &serde_json::Value,
    web_ctx: Option<&WebToolsContext>,
) -> Result<String, String> {
    let request = parse_web_research_batch_input(input)?;
    let mut urls = request.seed_urls.clone();
    let searches = run_web_research_searches(&request, web_ctx, &mut urls).await?;
    let urls = dedupe_research_urls(urls, request.max_fetches);
    let fetched = run_web_research_fetches(urls, web_ctx, request.preview_chars).await?;
    web_research_batch_response(&request, searches, fetched)
}

struct WebResearchBatchInput {
    queries: Vec<String>,
    results_per_query: usize,
    auto_fetch: bool,
    max_fetches: usize,
    preview_chars: usize,
    seed_urls: Vec<String>,
}

fn parse_web_research_batch_input(
    input: &serde_json::Value,
) -> Result<WebResearchBatchInput, String> {
    let queries = collect_string_list(input, "queries")
        .or_else(|| input["query"].as_str().map(|query| vec![query.to_string()]))
        .ok_or("Missing 'query' or 'queries' parameter")?;
    let queries: Vec<String> = queries
        .into_iter()
        .map(|query| query.trim().to_string())
        .filter(|query| !query.is_empty())
        .take(MAX_WEB_RESEARCH_QUERIES)
        .collect();
    if queries.is_empty() {
        return Err("web_research_batch requires at least one non-empty query".to_string());
    }

    Ok(WebResearchBatchInput {
        queries,
        results_per_query: input["max_results"].as_u64().unwrap_or(5).clamp(1, 10) as usize,
        auto_fetch: input["auto_fetch"].as_bool().unwrap_or(true),
        max_fetches: input["max_fetches"]
            .as_u64()
            .unwrap_or(5)
            .clamp(0, MAX_WEB_RESEARCH_FETCHES as u64) as usize,
        preview_chars: input["preview_chars"]
            .as_u64()
            .unwrap_or(3000)
            .clamp(500, 12_000) as usize,
        seed_urls: collect_string_list(input, "urls").unwrap_or_default(),
    })
}

async fn run_web_research_searches(
    request: &WebResearchBatchInput,
    web_ctx: Option<&WebToolsContext>,
    urls: &mut Vec<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut searches = Vec::new();
    for query in &request.queries {
        ensure_no_secret_literal("web_research_batch", "query", query)?;
        let search_input = serde_json::json!({
            "query": query,
            "max_results": request.results_per_query,
        });
        let result = if let Some(ctx) = web_ctx {
            ctx.search.search(query, request.results_per_query).await
        } else {
            tool_web_search_legacy(&search_input).await
        };
        match result {
            Ok(text) => {
                let result_urls = extract_urls_from_text(&text);
                if request.auto_fetch {
                    urls.extend(result_urls.clone());
                }
                searches.push(serde_json::json!({
                    "query": query,
                    "success": true,
                    "preview": truncate_owned(&text, request.preview_chars),
                    "urls": result_urls,
                }));
            }
            Err(error) => searches.push(serde_json::json!({
                "query": query,
                "success": false,
                "error": error,
            })),
        }
    }
    Ok(searches)
}

fn dedupe_research_urls(urls: Vec<String>, max_fetches: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    urls.into_iter()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .filter(|url| seen.insert(url.clone()))
        .take(max_fetches)
        .collect()
}

async fn run_web_research_fetches(
    urls: Vec<String>,
    web_ctx: Option<&WebToolsContext>,
    preview_chars: usize,
) -> Result<Vec<serde_json::Value>, String> {
    let mut fetched = Vec::new();
    for url in urls {
        ensure_no_secret_literal("web_research_batch", "url", &url)?;
        if let Some(violation) = check_taint_net_fetch(&url) {
            fetched.push(serde_json::json!({
                "url": url,
                "success": false,
                "error": format!("Taint violation: {violation}"),
            }));
            continue;
        }
        let fetch_input = serde_json::json!({ "url": url });
        let result = if let Some(ctx) = web_ctx {
            ctx.fetch
                .fetch_with_options(
                    fetch_input["url"].as_str().unwrap_or_default(),
                    "GET",
                    None,
                    None,
                )
                .await
        } else {
            tool_web_fetch_legacy(&fetch_input).await
        };
        match result {
            Ok(text) => fetched.push(serde_json::json!({
                "url": fetch_input["url"],
                "success": true,
                "chars": text.chars().count(),
                "preview": truncate_owned(&text, preview_chars),
            })),
            Err(error) => fetched.push(serde_json::json!({
                "url": fetch_input["url"],
                "success": false,
                "error": error,
            })),
        }
    }
    Ok(fetched)
}

fn web_research_batch_response(
    request: &WebResearchBatchInput,
    searches: Vec<serde_json::Value>,
    fetched: Vec<serde_json::Value>,
) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "success": true,
        "tool": "web_research_batch",
        "queries": &request.queries,
        "searches": searches,
        "fetched": fetched,
        "note": "Use these compact previews to synthesize. Fetch exact sources individually only when a precise quote/detail is needed.",
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

fn extract_urls_from_text(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';'
            )
        });
        let trimmed = trimmed.trim_end_matches(['.', ':']);
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            urls.push(trimmed.to_string());
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_web_research_batch_trims_limits_and_clamps() {
        let input = json!({
            "queries": [" alpha ", "", "beta", "gamma", "delta", "epsilon", "zeta"],
            "max_results": 99,
            "max_fetches": 99,
            "preview_chars": 10,
            "urls": [" https://example.com/a "],
        });

        let parsed = parse_web_research_batch_input(&input).unwrap();

        assert_eq!(
            parsed.queries,
            vec!["alpha", "beta", "gamma", "delta", "epsilon"]
        );
        assert_eq!(parsed.results_per_query, 10);
        assert_eq!(parsed.max_fetches, MAX_WEB_RESEARCH_FETCHES);
        assert_eq!(parsed.preview_chars, 500);
        assert_eq!(parsed.seed_urls, vec![" https://example.com/a "]);
    }

    #[test]
    fn dedupe_research_urls_trims_preserves_order_and_limit() {
        let urls = dedupe_research_urls(
            vec![
                " https://a.test ".to_string(),
                "".to_string(),
                "https://b.test".to_string(),
                "https://a.test".to_string(),
                "https://c.test".to_string(),
            ],
            2,
        );

        assert_eq!(urls, vec!["https://a.test", "https://b.test"]);
    }

    #[test]
    fn extract_urls_from_text_strips_common_punctuation() {
        let urls = extract_urls_from_text(
            r#"See (https://example.com/a), "http://example.org/b." and <https://c.test/x:>"#,
        );

        assert_eq!(
            urls,
            vec![
                "https://example.com/a",
                "http://example.org/b",
                "https://c.test/x",
            ]
        );
    }
}
