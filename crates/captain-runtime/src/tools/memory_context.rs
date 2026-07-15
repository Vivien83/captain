//! Compact high-confidence memory context helpers.

use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::tools::{call_mempalace_tool, truncate_owned};
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) const DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY: f64 = 0.75;

pub(crate) fn memory_context_tokens(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "avec", "dans", "pour", "sans", "sur", "des", "les", "une", "the", "and", "for", "with",
        "that", "this", "from", "service",
    ];
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            if memory_context_keep_token(&current, STOPWORDS) {
                tokens.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if !current.is_empty() && memory_context_keep_token(&current, STOPWORDS) {
        tokens.push(current);
    }

    let mut seen = HashSet::new();
    tokens
        .into_iter()
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

fn memory_context_keep_token(token: &str, stopwords: &[&str]) -> bool {
    if stopwords.contains(&token) {
        return false;
    }
    token.chars().count() >= 3
        || (token.chars().count() >= 2 && token.chars().any(|c| c.is_ascii_digit()))
}

fn memory_context_required_terms(term_count: usize) -> usize {
    match term_count {
        0 => 0,
        1 => 1,
        _ => 2,
    }
}

fn memory_context_matched_terms(query_terms: &[String], text: &str) -> usize {
    if query_terms.is_empty() {
        return 0;
    }
    let text_tokens: HashSet<String> = memory_context_tokens(text).into_iter().collect();
    query_terms
        .iter()
        .filter(|term| text_tokens.contains(*term))
        .count()
}

fn memory_context_similarity(value: &serde_json::Value) -> Option<f64> {
    value
        .get("similarity")
        .and_then(|v| v.as_f64())
        .or_else(|| value.get("score").and_then(|v| v.as_f64()))
}

fn memory_context_is_high_confidence(
    matched_terms: usize,
    term_count: usize,
    similarity: Option<f64>,
    min_similarity: f64,
    strict: bool,
) -> bool {
    if !strict {
        return true;
    }
    if let Some(score) = similarity {
        if !score.is_finite() || score < 0.0 {
            return false;
        }
    }
    let required_terms = memory_context_required_terms(term_count);
    if required_terms > 0 && matched_terms >= required_terms {
        return true;
    }
    similarity.is_some_and(|score| (0.0..=1.0).contains(&score) && score >= min_similarity)
}

pub(crate) fn compact_mempalace_search_result(
    query: &str,
    raw: &str,
    max_items: usize,
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> serde_json::Value {
    let query_terms = memory_context_tokens(query);
    let parsed = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => value,
        Err(_) => {
            return compact_raw_mempalace_text(
                query,
                raw,
                &query_terms,
                preview_chars,
                min_similarity,
                strict,
                retractions,
            );
        }
    };

    let candidates = mempalace_candidates(&parsed);
    let (accepted, filtered) = compact_mempalace_candidates(
        &query_terms,
        &candidates,
        preview_chars,
        min_similarity,
        strict,
        retractions,
        max_items,
    );
    mempalace_search_response(
        query,
        accepted,
        filtered,
        candidates.len(),
        query_terms,
        strict,
        min_similarity,
    )
}

fn compact_raw_mempalace_text(
    query: &str,
    raw: &str,
    query_terms: &[String],
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> serde_json::Value {
    let suppressed = crate::memory_retractions::text_matches_any(raw, retractions);
    let matched_terms = memory_context_matched_terms(query_terms, raw);
    let accepted = !suppressed
        && memory_context_is_high_confidence(
            matched_terms,
            query_terms.len(),
            None,
            min_similarity,
            strict,
        );
    let matches = if accepted {
        vec![serde_json::json!({
            "source": "mempalace",
            "preview": truncate_owned(raw, preview_chars),
            "matched_query_terms": matched_terms,
        })]
    } else {
        Vec::new()
    };
    let match_count = matches.len();
    serde_json::json!({
        "success": true,
        "source": "mempalace",
        "query": query,
        "matches": matches,
        "match_count": match_count,
        "filtered": if accepted { 0 } else { 1 },
        "query_terms": query_terms,
        "strict_filter": strict,
        "message": if accepted { "Raw MemPalace text accepted." } else { "No high-confidence MemPalace match after filtering." },
    })
}

fn mempalace_candidates(parsed: &serde_json::Value) -> Vec<serde_json::Value> {
    parsed
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| parsed.as_array().cloned())
        .unwrap_or_default()
}

fn compact_mempalace_candidates(
    query_terms: &[String],
    candidates: &[serde_json::Value],
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
    max_items: usize,
) -> (Vec<serde_json::Value>, usize) {
    let mut accepted = Vec::new();
    let mut filtered = 0usize;
    for candidate in candidates {
        match compact_mempalace_candidate(
            query_terms,
            candidate,
            preview_chars,
            min_similarity,
            strict,
            retractions,
        ) {
            Some(value) => accepted.push(value),
            None => filtered += 1,
        }
    }
    sort_mempalace_matches(&mut accepted);
    accepted.truncate(max_items);
    (accepted, filtered)
}

fn compact_mempalace_candidate(
    query_terms: &[String],
    candidate: &serde_json::Value,
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Option<serde_json::Value> {
    let text = mempalace_candidate_text(candidate);
    if text.trim().is_empty() || crate::memory_retractions::text_matches_any(text, retractions) {
        return None;
    }
    let matched_terms = memory_context_matched_terms(query_terms, text);
    let similarity = memory_context_similarity(candidate);
    if !memory_context_is_high_confidence(
        matched_terms,
        query_terms.len(),
        similarity,
        min_similarity,
        strict,
    ) {
        return None;
    }
    Some(serde_json::json!({
        "source": "mempalace",
        "wing": candidate.get("wing").cloned().unwrap_or(serde_json::Value::Null),
        "room": candidate.get("room").cloned().unwrap_or(serde_json::Value::Null),
        "source_file": candidate.get("source_file").cloned().unwrap_or(serde_json::Value::Null),
        "similarity": similarity,
        "matched_query_terms": matched_terms,
        "preview": truncate_owned(text, preview_chars),
    }))
}

fn mempalace_candidate_text(candidate: &serde_json::Value) -> &str {
    candidate
        .get("text")
        .or_else(|| candidate.get("content"))
        .or_else(|| candidate.get("summary"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

fn sort_mempalace_matches(accepted: &mut [serde_json::Value]) {
    accepted.sort_by(|a, b| {
        let a_terms = a["matched_query_terms"].as_u64().unwrap_or(0);
        let b_terms = b["matched_query_terms"].as_u64().unwrap_or(0);
        let a_sim = a["similarity"].as_f64().unwrap_or(f64::NEG_INFINITY);
        let b_sim = b["similarity"].as_f64().unwrap_or(f64::NEG_INFINITY);
        b_terms.cmp(&a_terms).then_with(|| {
            b_sim
                .partial_cmp(&a_sim)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
}

fn mempalace_search_response(
    query: &str,
    accepted: Vec<serde_json::Value>,
    filtered: usize,
    total_candidates: usize,
    query_terms: Vec<String>,
    strict: bool,
    min_similarity: f64,
) -> serde_json::Value {
    let match_count = accepted.len();
    serde_json::json!({
        "success": true,
        "source": "mempalace",
        "query": query,
        "matches": accepted,
        "match_count": match_count,
        "filtered": filtered,
        "total_candidates": total_candidates,
        "query_terms": query_terms,
        "strict_filter": strict,
        "min_similarity": min_similarity,
        "message": if accepted.is_empty() { "No high-confidence MemPalace match after filtering." } else { "High-confidence MemPalace matches only." },
    })
}

fn compact_graph_memory_result(
    query: &str,
    result: Result<Option<serde_json::Value>, String>,
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> serde_json::Value {
    let query_terms = memory_context_tokens(query);
    let value = match result {
        Ok(Some(value)) => value,
        Ok(None) => {
            return serde_json::json!({
                "success": true,
                "source": "graph",
                "query": query,
                "matches": [],
                "match_count": 0,
                "filtered": 0,
                "message": "No graph memory match.",
            });
        }
        Err(error) => {
            return serde_json::json!({
                "success": false,
                "source": "graph",
                "query": query,
                "error": error,
            });
        }
    };
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    let matched_terms = memory_context_matched_terms(&query_terms, &text);
    let suppressed = crate::memory_retractions::text_matches_any(&text, retractions);
    let accepted = !suppressed
        && memory_context_is_high_confidence(
            matched_terms,
            query_terms.len(),
            None,
            min_similarity,
            strict,
        );
    let matches = if accepted {
        vec![serde_json::json!({
            "source": "graph",
            "matched_query_terms": matched_terms,
            "preview": truncate_owned(&text, preview_chars),
        })]
    } else {
        Vec::new()
    };
    serde_json::json!({
        "success": true,
        "source": "graph",
        "query": query,
        "matches": matches,
        "match_count": matches.len(),
        "filtered": if accepted { 0 } else { 1 },
        "query_terms": query_terms,
        "strict_filter": strict,
        "message": if accepted { "High-confidence graph memory match." } else { "No high-confidence graph memory match after filtering." },
    })
}

fn compact_memory_writes_result(
    query: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
    max_items: usize,
    preview_chars: usize,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Option<serde_json::Value> {
    let conn = kernel.and_then(|kh| kh.memory_writes_conn())?;
    let query_terms = memory_context_tokens(query);
    let scan_limit = (max_items * 12).clamp(max_items, 120);
    let rows = match conn.lock() {
        Ok(guard) => captain_memory::memory_writer::list_recent_active(&guard, scan_limit),
        Err(error) => {
            return Some(serde_json::json!({
                "success": false,
                "source": "memory_writes",
                "query": query,
                "error": format!("memory_writes lock poisoned: {error}"),
            }));
        }
    };
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            return Some(serde_json::json!({
                "success": false,
                "source": "memory_writes",
                "query": query,
                "error": format!("memory_writes recall failed: {error}"),
            }));
        }
    };
    let (accepted, filtered) = compact_memory_write_rows(
        &rows,
        &query_terms,
        max_items,
        preview_chars,
        strict,
        retractions,
    );
    Some(memory_writes_response(
        query,
        accepted,
        filtered,
        rows.len(),
        query_terms,
        strict,
    ))
}

fn compact_memory_write_rows(
    rows: &[captain_memory::memory_writer::MemoryWrite],
    query_terms: &[String],
    max_items: usize,
    preview_chars: usize,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> (Vec<serde_json::Value>, usize) {
    let mut accepted = Vec::new();
    let mut filtered = 0usize;
    for row in rows {
        match compact_memory_write_row(row, query_terms, preview_chars, strict, retractions) {
            Some(value) => accepted.push(value),
            None => filtered += 1,
        }
    }
    if accepted.len() > max_items {
        filtered += accepted.len() - max_items;
        accepted.truncate(max_items);
    }
    (accepted, filtered)
}

fn compact_memory_write_row(
    row: &captain_memory::memory_writer::MemoryWrite,
    query_terms: &[String],
    preview_chars: usize,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Option<serde_json::Value> {
    let fact_text = memory_write_fact_text(row);
    if crate::memory_retractions::text_matches_any(&fact_text, retractions) {
        return None;
    }
    let matched_terms = memory_context_matched_terms(query_terms, &fact_text);
    if !memory_context_is_high_confidence(
        matched_terms,
        query_terms.len(),
        None,
        DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
        strict,
    ) {
        return None;
    }
    Some(serde_json::json!({
        "source": "memory_writes",
        "subject": row.subject,
        "predicate": row.predicate,
        "object": row.object,
        "wing": row.wing,
        "room": row.room,
        "write_source": row.source,
        "sync_status": row.sync_status.as_str(),
        "created_at": row.created_at,
        "matched_query_terms": matched_terms,
        "preview": truncate_owned(&fact_text, preview_chars),
    }))
}

fn memory_write_fact_text(row: &captain_memory::memory_writer::MemoryWrite) -> String {
    format!("{} {} {}", row.subject, row.predicate, row.object)
}

fn memory_writes_response(
    query: &str,
    accepted: Vec<serde_json::Value>,
    filtered: usize,
    total_candidates: usize,
    query_terms: Vec<String>,
    strict: bool,
) -> serde_json::Value {
    let match_count = accepted.len();
    serde_json::json!({
        "success": true,
        "source": "memory_writes",
        "query": query,
        "matches": accepted,
        "match_count": match_count,
        "filtered": filtered,
        "total_candidates": total_candidates,
        "query_terms": query_terms,
        "strict_filter": strict,
        "message": if match_count == 0 { "No high-confidence local memory_writes match after filtering." } else { "High-confidence local memory_writes matches." },
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn compact_memory_context_result(
    query: &str,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    backend: captain_types::config::MemoryBackend,
    max_items: usize,
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
) -> serde_json::Value {
    let retractions = kernel.map(|kh| kh.memory_retractions()).unwrap_or_default();
    if crate::memory_retractions::text_matches_any(query, &retractions) {
        return retracted_memory_context_response(query);
    }

    let mut sources = Vec::new();
    if let Some(source) = compact_memory_writes_result(
        query,
        kernel,
        max_items,
        preview_chars,
        strict,
        &retractions,
    ) {
        sources.push(source);
    }
    if matches!(backend, captain_types::config::MemoryBackend::Mempalace) {
        sources.push(
            compact_mempalace_context_source(
                query,
                mcp_connections,
                max_items,
                preview_chars,
                min_similarity,
                strict,
                &retractions,
            )
            .await,
        );
    }

    if let Some(source) = compact_graph_context_source(
        query,
        kernel,
        preview_chars,
        min_similarity,
        strict,
        &retractions,
    ) {
        sources.push(source);
    }

    memory_context_sources_response(query, sources)
}

fn retracted_memory_context_response(query: &str) -> serde_json::Value {
    serde_json::json!({
        "success": true,
        "query": query,
        "sources": [],
        "match_count": 0,
        "filtered": 0,
        "message": "Query matches an active memory retraction guard; active memory suppressed.",
    })
}

async fn compact_mempalace_context_source(
    query: &str,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    max_items: usize,
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> serde_json::Value {
    let mcp_input = serde_json::json!({
        "query": query,
        "limit": (max_items * 4).clamp(max_items, 20),
    });
    match call_mempalace_tool(
        "mcp_mempalace_mempalace_search",
        &mcp_input,
        mcp_connections,
    )
    .await
    {
        Ok(raw) => compact_mempalace_search_result(
            query,
            &raw,
            max_items,
            preview_chars,
            min_similarity,
            strict,
            retractions,
        ),
        Err(error) => serde_json::json!({
            "success": false,
            "source": "mempalace",
            "query": query,
            "error": error,
        }),
    }
}

fn compact_graph_context_source(
    query: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
    preview_chars: usize,
    min_similarity: f64,
    strict: bool,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Option<serde_json::Value> {
    kernel.map(|kh| {
        compact_graph_memory_result(
            query,
            kh.memory_recall(query).map_err(|e| e.to_string()),
            preview_chars,
            min_similarity,
            strict,
            retractions,
        )
    })
}

fn memory_context_sources_response(
    query: &str,
    sources: Vec<serde_json::Value>,
) -> serde_json::Value {
    let match_count: usize = sources
        .iter()
        .filter_map(|source| source["match_count"].as_u64())
        .map(|n| n as usize)
        .sum();
    let filtered: usize = sources
        .iter()
        .filter_map(|source| source["filtered"].as_u64())
        .map(|n| n as usize)
        .sum();
    let success = sources
        .iter()
        .all(|source| source["success"].as_bool().unwrap_or(true));

    serde_json::json!({
        "success": success,
        "query": query,
        "sources": sources,
        "match_count": match_count,
        "filtered": filtered,
        "message": if match_count == 0 { "No high-confidence memory context. Do not infer facts from filtered candidates." } else { "Use only these high-confidence memory matches." },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mempalace_candidates_reads_results_object_or_array() {
        let object = json!({"results": [{"text": "alpha"}]});
        let array = json!([{"text": "beta"}, {"summary": "gamma"}]);

        assert_eq!(mempalace_candidates(&object).len(), 1);
        assert_eq!(mempalace_candidates(&array).len(), 2);
        assert!(mempalace_candidates(&json!({"items": []})).is_empty());
    }

    #[test]
    fn compact_mempalace_candidates_sorts_terms_before_similarity() {
        let query_terms = memory_context_tokens("alpha beta");
        let candidates = vec![
            json!({"text": "alpha only", "similarity": 0.99}),
            json!({"text": "alpha beta match", "similarity": 0.10}),
        ];

        let (accepted, filtered) = compact_mempalace_candidates(
            &query_terms,
            &candidates,
            500,
            DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
            true,
            &[],
            5,
        );

        assert_eq!(filtered, 0);
        assert_eq!(accepted.len(), 2);
        assert_eq!(accepted[0]["matched_query_terms"], 2);
        assert_eq!(accepted[1]["matched_query_terms"], 1);
    }
}
