//! Grouped web research with typed discovery and retrieval provenance.

use crate::tools::{
    check_url_content_guard, collect_string_list, ensure_no_secret_literal, truncate_owned,
};
use crate::web_content::wrap_external_content;
use crate::web_evidence::{
    canonical_source_url, source_domain, WebPageEvidence, WebSearchEvidence,
    WebSearchResultEvidence,
};
use crate::web_search::WebToolsContext;
use futures::future::join_all;
use std::collections::{HashMap, HashSet};

const MAX_WEB_RESEARCH_QUERIES: usize = 5;
const MAX_WEB_RESEARCH_FETCHES: usize = 10;
const MAX_WEB_RESEARCH_SOURCES: usize = 60;

pub(crate) async fn tool_web_research_batch(
    input: &serde_json::Value,
    web_ctx: &WebToolsContext,
) -> Result<String, String> {
    let request = parse_web_research_batch_input(input)?;
    let searches = run_web_research_searches(&request, web_ctx).await?;
    let candidates = collect_research_candidates(&request, &searches)?;
    let fetched = run_web_research_fetches(&candidates, web_ctx, request.max_fetches).await;
    web_research_batch_response(&request, &searches, &candidates, &fetched)
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
        .unwrap_or_default();
    let queries: Vec<String> = queries
        .into_iter()
        .map(|query| query.trim().to_string())
        .filter(|query| !query.is_empty())
        .take(MAX_WEB_RESEARCH_QUERIES)
        .collect();
    let seed_urls = collect_string_list(input, "urls")
        .unwrap_or_default()
        .into_iter()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .take(MAX_WEB_RESEARCH_FETCHES)
        .collect::<Vec<_>>();
    if queries.is_empty() && seed_urls.is_empty() {
        return Err("web_research_batch requires at least one non-empty query or URL".to_string());
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
        seed_urls,
    })
}

type SearchAttempt = (String, Result<WebSearchEvidence, String>);

async fn run_web_research_searches(
    request: &WebResearchBatchInput,
    web_ctx: &WebToolsContext,
) -> Result<Vec<SearchAttempt>, String> {
    for query in &request.queries {
        ensure_no_secret_literal("web_research_batch", "query", query)?;
    }

    let searches = request.queries.iter().map(|query| async move {
        (
            query.clone(),
            web_ctx
                .search
                .search_evidence(query, request.results_per_query)
                .await,
        )
    });
    Ok(join_all(searches).await)
}

#[derive(Debug, Clone)]
struct ResearchCandidate {
    source_id: String,
    title: String,
    url: String,
    canonical_url: String,
    snippet: String,
    discovered_by: Vec<String>,
    providers: Vec<String>,
    explicit: bool,
    fetch_requested: bool,
}

fn collect_research_candidates(
    request: &WebResearchBatchInput,
    searches: &[SearchAttempt],
) -> Result<Vec<ResearchCandidate>, String> {
    let mut candidates = Vec::new();
    let mut by_url = HashMap::<String, usize>::new();

    for url in &request.seed_urls {
        ensure_no_secret_literal("web_research_batch", "url", url)?;
        let canonical_url = canonical_source_url(url)
            .ok_or_else(|| format!("Invalid explicit research URL: {url}"))?;
        let source = WebSearchResultEvidence::new("Explicit source", url, "")
            .ok_or_else(|| format!("Invalid explicit research URL: {url}"))?;
        merge_candidate(
            &mut candidates,
            &mut by_url,
            source,
            "explicit",
            "explicit",
            true,
            true,
        );
        debug_assert!(by_url.contains_key(&canonical_url));
    }

    for (query, attempt) in searches {
        let Ok(search) = attempt else {
            continue;
        };
        for result in &search.results {
            merge_candidate(
                &mut candidates,
                &mut by_url,
                result.clone(),
                query,
                &search.provider,
                false,
                request.auto_fetch,
            );
            if candidates.len() >= MAX_WEB_RESEARCH_SOURCES {
                return Ok(candidates);
            }
        }
    }

    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn merge_candidate(
    candidates: &mut Vec<ResearchCandidate>,
    by_url: &mut HashMap<String, usize>,
    source: WebSearchResultEvidence,
    discovered_by: &str,
    provider: &str,
    explicit: bool,
    fetch_requested: bool,
) {
    if let Some(index) = by_url.get(&source.canonical_url).copied() {
        let candidate = &mut candidates[index];
        push_unique(&mut candidate.discovered_by, discovered_by);
        push_unique(&mut candidate.providers, provider);
        candidate.explicit |= explicit;
        candidate.fetch_requested |= fetch_requested;
        if candidate.title.is_empty() || candidate.title == "Explicit source" {
            candidate.title = source.title;
        }
        if candidate.snippet.is_empty() {
            candidate.snippet = source.snippet;
        }
        return;
    }

    let index = candidates.len();
    by_url.insert(source.canonical_url.clone(), index);
    candidates.push(ResearchCandidate {
        source_id: source.source_id,
        title: source.title,
        url: source.url,
        canonical_url: source.canonical_url,
        snippet: source.snippet,
        discovered_by: vec![discovered_by.to_string()],
        providers: vec![provider.to_string()],
        explicit,
        fetch_requested,
    });
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

type FetchAttempt = (String, Result<WebPageEvidence, String>);

async fn run_web_research_fetches(
    candidates: &[ResearchCandidate],
    web_ctx: &WebToolsContext,
    max_fetches: usize,
) -> Vec<FetchAttempt> {
    let selected = candidates
        .iter()
        .filter(|candidate| candidate.fetch_requested)
        .take(max_fetches)
        .map(|candidate| async move {
            let result = if let Some(violation) = check_url_content_guard(&candidate.url) {
                Err(violation)
            } else {
                web_ctx.fetch.fetch_evidence(&candidate.url).await
            };
            (candidate.source_id.clone(), result)
        });
    join_all(selected).await
}

fn web_research_batch_response(
    request: &WebResearchBatchInput,
    searches: &[SearchAttempt],
    candidates: &[ResearchCandidate],
    fetched: &[FetchAttempt],
) -> Result<String, String> {
    let fetches = fetched
        .iter()
        .map(|(source_id, result)| (source_id.as_str(), result))
        .collect::<HashMap<_, _>>();
    let mut citation_ready = 0usize;
    let mut failed_fetches = 0usize;
    let mut domains = HashSet::new();

    let sources = candidates
        .iter()
        .map(|candidate| {
            let citation_label = citation_label(candidate);
            match fetches.get(candidate.source_id.as_str()) {
                Some(Ok(page)) => {
                    let ready = page.citation_ready();
                    if ready {
                        citation_ready += 1;
                        if let Some(domain) = source_domain(&page.final_url) {
                            domains.insert(domain);
                        }
                    } else {
                        failed_fetches += 1;
                    }
                    serde_json::json!({
                        "source_id": candidate.source_id,
                        "title": candidate.title,
                        "url": candidate.canonical_url,
                        "final_url": page.final_url,
                        "discovered_by": candidate.discovered_by,
                        "providers": candidate.providers,
                        "explicit": candidate.explicit,
                        "retrieval_status": if ready { "retrieved" } else { "http_error" },
                        "http_status": page.status,
                        "retrieved_at": page.retrieved_at,
                        "content_sha256": page.content_sha256,
                        "content_chars": page.content_chars,
                        "retained_chars": page.retained_chars,
                        "truncated": page.truncated,
                        "preview": truncate_owned(&page.content, request.preview_chars),
                        "citation_ready": ready,
                        "citation_markdown": if ready {
                            Some(format!("[{citation_label}]({})", candidate.canonical_url))
                        } else {
                            None
                        },
                    })
                }
                Some(Err(error)) => {
                    failed_fetches += 1;
                    serde_json::json!({
                        "source_id": candidate.source_id,
                        "title": candidate.title,
                        "url": candidate.canonical_url,
                        "discovered_by": candidate.discovered_by,
                        "providers": candidate.providers,
                        "explicit": candidate.explicit,
                        "retrieval_status": "unavailable",
                        "error": error,
                        "snippet": truncate_owned(&candidate.snippet, 1000),
                        "citation_ready": false,
                    })
                }
                None => serde_json::json!({
                    "source_id": candidate.source_id,
                    "title": candidate.title,
                    "url": candidate.canonical_url,
                    "discovered_by": candidate.discovered_by,
                    "providers": candidate.providers,
                    "explicit": candidate.explicit,
                    "retrieval_status": "discovery_only",
                    "snippet": truncate_owned(&candidate.snippet, 1000),
                    "citation_ready": false,
                }),
            }
        })
        .collect::<Vec<_>>();

    let search_rows = searches
        .iter()
        .map(|(query, attempt)| match attempt {
            Ok(search) => serde_json::json!({
                "query": query,
                "success": true,
                "provider": search.provider,
                "retrieved_at": search.retrieved_at,
                "source_ids": search.results.iter().map(|source| &source.source_id).collect::<Vec<_>>(),
                "provider_summary": search.provider_summary,
                "provider_summary_status": search.provider_summary.as_ref().map(|_| "provider_generated_discovery_only"),
            }),
            Err(error) => serde_json::json!({
                "query": query,
                "success": false,
                "error": error,
            }),
        })
        .collect::<Vec<_>>();

    let payload = serde_json::to_string_pretty(&serde_json::json!({
        "success": true,
        "tool": "web_research_batch",
        "queries": &request.queries,
        "searches": search_rows,
        "sources": sources,
        "coverage": {
            "sources_discovered": candidates.len(),
            "sources_fetched": fetched.len(),
            "citation_ready": citation_ready,
            "fetch_failed": failed_fetches,
            "independent_domains": domains.len(),
            "ready_for_synthesis": citation_ready > 0,
        },
        "citation_contract": {
            "discovery_snippets_are_citable": false,
            "only_citation_ready_sources_may_be_cited": true,
            "inline_format": "Use the exact citation_markdown value immediately after the supported sentence.",
            "important_claims": "Before delivery, call web_citation_audit with the draft, cited URLs, and exact quotes copied from retrieved previews/pages.",
            "unverified_marker": "Mark a load-bearing claim that could not be sourced as [unverified].",
            "scope": "Successful retrieval proves page access and content identity, not semantic truth of a claim.",
        },
    }))
    .map_err(|error| format!("Serialize web research evidence: {error}"))?;

    Ok(wrap_external_content("web-research-evidence", &payload))
}

fn citation_label(candidate: &ResearchCandidate) -> String {
    let raw = if candidate.title.trim().is_empty() || candidate.title == "Explicit source" {
        source_domain(&candidate.canonical_url).unwrap_or_else(|| candidate.source_id.clone())
    } else {
        candidate.title.clone()
    };
    raw.replace(['[', ']'], "")
        .trim()
        .chars()
        .take(120)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_evidence::WebSearchResultEvidence;
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
        assert_eq!(parsed.seed_urls, vec!["https://example.com/a"]);
    }

    #[test]
    fn parse_web_research_batch_accepts_explicit_urls_without_search() {
        let parsed = parse_web_research_batch_input(&json!({
            "urls": ["https://example.com/source"]
        }))
        .unwrap();

        assert!(parsed.queries.is_empty());
        assert_eq!(parsed.seed_urls, vec!["https://example.com/source"]);
    }

    #[test]
    fn candidates_dedupe_fragments_and_merge_query_provenance() {
        let request = WebResearchBatchInput {
            queries: vec!["alpha".into(), "beta".into()],
            results_per_query: 5,
            auto_fetch: true,
            max_fetches: 5,
            preview_chars: 1000,
            seed_urls: vec!["https://example.com/page#intro".into()],
        };
        let result = WebSearchResultEvidence::new(
            "Example",
            "https://example.com/page",
            "Exact discovery snippet",
        )
        .unwrap();
        let searches = vec![
            (
                "alpha".into(),
                Ok(WebSearchEvidence::new(
                    "alpha",
                    "brave",
                    None,
                    vec![result.clone()],
                )),
            ),
            (
                "beta".into(),
                Ok(WebSearchEvidence::new("beta", "tavily", None, vec![result])),
            ),
        ];

        let candidates = collect_research_candidates(&request, &searches).unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "Example");
        assert_eq!(
            candidates[0].discovered_by,
            vec!["explicit", "alpha", "beta"]
        );
        assert_eq!(candidates[0].providers, vec!["explicit", "brave", "tavily"]);
        assert!(candidates[0].explicit);
        assert!(candidates[0].fetch_requested);
    }

    #[test]
    fn response_marks_only_successful_page_retrieval_as_citation_ready() {
        let request = WebResearchBatchInput {
            queries: vec!["alpha".into()],
            results_per_query: 5,
            auto_fetch: true,
            max_fetches: 5,
            preview_chars: 1000,
            seed_urls: Vec::new(),
        };
        let source =
            WebSearchResultEvidence::new("Verified page", "https://example.com/page", "snippet")
                .unwrap();
        let searches = vec![(
            "alpha".into(),
            Ok(WebSearchEvidence::new("alpha", "brave", None, vec![source])),
        )];
        let candidates = collect_research_candidates(&request, &searches).unwrap();
        let fetched = vec![(
            candidates[0].source_id.clone(),
            Ok(WebPageEvidence {
                requested_url: "https://example.com/page".into(),
                final_url: "https://example.com/page".into(),
                status: 200,
                content_type: "text/plain".into(),
                retrieved_at: "2026-08-08T00:00:00Z".into(),
                content_sha256: "abc".into(),
                content_chars: 23,
                retained_chars: 23,
                truncated: false,
                content: "Verbatim source proof.".into(),
            }),
        )];

        let response =
            web_research_batch_response(&request, &searches, &candidates, &fetched).unwrap();

        assert!(response.contains("\"citation_ready\": true"));
        assert!(response.contains("[Verified page](https://example.com/page)"));
        assert!(response.contains("retrieval proves page access"));
    }
}
