//! Deterministic citation and verbatim-evidence audit for research drafts.

use crate::tools::{check_url_content_guard, ensure_no_secret_literal};
use crate::web_evidence::{
    canonical_source_url, sha256_text, source_domain, source_id_for_url, WebPageEvidence,
};
use crate::web_search::WebToolsContext;
use futures::future::join_all;
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

const MAX_AUDIT_DRAFT_BYTES: usize = 50_000;
const MAX_AUDIT_SOURCES: usize = 12;
const MAX_QUOTES_PER_SOURCE: usize = 3;
const MAX_QUOTE_CHARS: usize = 1_000;
const MIN_QUOTE_CHARS: usize = 16;
const MIN_QUOTE_WORDS: usize = 4;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CitationAuditInput {
    draft: String,
    sources: Vec<CitationAuditSource>,
    #[serde(default = "default_min_coverage")]
    min_coverage: f64,
    #[serde(default = "default_true")]
    require_evidence: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CitationAuditSource {
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    quotes: Vec<String>,
}

fn default_min_coverage() -> f64 {
    0.5
}

fn default_true() -> bool {
    true
}

pub(crate) async fn tool_web_citation_audit(
    input: &serde_json::Value,
    web_ctx: &WebToolsContext,
) -> Result<String, String> {
    let request = parse_audit_input(input)?;
    let pages = fetch_audit_sources(&request, web_ctx).await;
    let report = audit_citations(&request, &pages)?;
    serde_json::to_string_pretty(&report)
        .map_err(|error| format!("Serialize citation audit: {error}"))
}

fn parse_audit_input(input: &serde_json::Value) -> Result<CitationAuditInput, String> {
    let mut request: CitationAuditInput = serde_json::from_value(input.clone())
        .map_err(|error| format!("Invalid web_citation_audit input: {error}"))?;
    if request.draft.trim().is_empty() {
        return Err("web_citation_audit requires a non-empty draft".to_string());
    }
    if request.draft.len() > MAX_AUDIT_DRAFT_BYTES {
        return Err(format!(
            "web_citation_audit draft exceeds {MAX_AUDIT_DRAFT_BYTES} bytes"
        ));
    }
    if request.sources.is_empty() || request.sources.len() > MAX_AUDIT_SOURCES {
        return Err(format!(
            "web_citation_audit requires 1..={MAX_AUDIT_SOURCES} sources"
        ));
    }
    if !request.min_coverage.is_finite() || !(0.0..=1.0).contains(&request.min_coverage) {
        return Err("min_coverage must be between 0 and 1".to_string());
    }

    let mut seen = HashSet::new();
    for source in &mut request.sources {
        ensure_no_secret_literal("web_citation_audit", "url", &source.url)?;
        if let Some(violation) = check_url_content_guard(&source.url) {
            return Err(violation);
        }
        source.url = canonical_source_url(&source.url)
            .ok_or_else(|| format!("Invalid citation URL: {}", source.url))?;
        if !seen.insert(source.url.clone()) {
            return Err(format!("Duplicate citation source URL: {}", source.url));
        }
        if source.quotes.len() > MAX_QUOTES_PER_SOURCE {
            return Err(format!(
                "Source {} exceeds {MAX_QUOTES_PER_SOURCE} evidence quotes",
                source.url
            ));
        }
        for quote in &source.quotes {
            let normalized = normalize_visible_text(quote);
            let words = normalized.split_whitespace().count();
            if normalized.chars().count() < MIN_QUOTE_CHARS || words < MIN_QUOTE_WORDS {
                return Err(format!(
                    "Evidence quote for {} is too short; use at least {MIN_QUOTE_WORDS} words and {MIN_QUOTE_CHARS} characters",
                    source.url
                ));
            }
            if quote.chars().count() > MAX_QUOTE_CHARS {
                return Err(format!(
                    "Evidence quote for {} exceeds {MAX_QUOTE_CHARS} characters",
                    source.url
                ));
            }
        }
    }
    Ok(request)
}

async fn fetch_audit_sources(
    request: &CitationAuditInput,
    web_ctx: &WebToolsContext,
) -> HashMap<String, Result<WebPageEvidence, String>> {
    let fetches = request.sources.iter().map(|source| async move {
        (
            source.url.clone(),
            web_ctx.fetch.fetch_evidence(&source.url).await,
        )
    });
    join_all(fetches).await.into_iter().collect()
}

fn audit_citations(
    request: &CitationAuditInput,
    pages: &HashMap<String, Result<WebPageEvidence, String>>,
) -> Result<serde_json::Value, String> {
    let draft_body = draft_before_sources(&request.draft);
    let citation_urls = extract_markdown_citation_urls(draft_body)?;
    let supplied = request
        .sources
        .iter()
        .map(|source| source.url.as_str())
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for url in &citation_urls {
        if !supplied.contains(url.as_str()) {
            errors.push(format!(
                "Draft cites URL not present in the audited source set: {url}"
            ));
        }
    }
    if citation_urls.is_empty() {
        errors.push("Draft has no inline Markdown source citation".to_string());
    }

    let sentence_stats = provenance_sentence_stats(draft_body)?;
    let coverage = if sentence_stats.total == 0 {
        0.0
    } else {
        sentence_stats.with_provenance as f64 / sentence_stats.total as f64
    };
    if sentence_stats.total == 0 {
        errors.push("Draft has no auditable prose sentence".to_string());
    } else if coverage + f64::EPSILON < request.min_coverage {
        errors.push(format!(
            "Declared provenance coverage is {:.0}% but the required minimum is {:.0}%",
            coverage * 100.0,
            request.min_coverage * 100.0
        ));
    }

    let mut source_checks = Vec::new();
    let mut cited_domains = HashSet::new();
    for source in &request.sources {
        let cited = citation_urls.contains(&source.url);
        if !cited {
            warnings.push(format!(
                "Audited source is not cited inline: {}",
                source.url
            ));
        }

        let mut check = serde_json::json!({
            "source_id": source_id_for_url(&source.url),
            "url": source.url,
            "title": source.title,
            "cited_inline": cited,
            "quotes_submitted": source.quotes.len(),
            "quotes_found": 0,
            "retrieval_status": "unavailable",
        });
        match pages.get(&source.url) {
            Some(Ok(page)) if page.citation_ready() => {
                check["retrieval_status"] = serde_json::json!("retrieved");
                check["http_status"] = serde_json::json!(page.status);
                check["final_url"] = serde_json::json!(page.final_url);
                check["retrieved_at"] = serde_json::json!(page.retrieved_at);
                check["content_sha256"] = serde_json::json!(page.content_sha256);
                let normalized_page = normalize_visible_text(&page.content);
                let mut found = 0usize;
                for (quote_index, quote) in source.quotes.iter().enumerate() {
                    let normalized_quote = normalize_visible_text(quote);
                    if normalized_page.contains(&normalized_quote) {
                        found += 1;
                    } else if cited {
                        errors.push(format!(
                            "Evidence quote {} for {} was not found verbatim in the retrieved page",
                            quote_index + 1,
                            source.url
                        ));
                    }
                }
                check["quotes_found"] = serde_json::json!(found);
                check["quote_sha256"] = serde_json::json!(source
                    .quotes
                    .iter()
                    .map(|quote| sha256_text(&normalize_visible_text(quote)))
                    .collect::<Vec<_>>());
                if cited {
                    if let Some(domain) = source_domain(&page.final_url) {
                        cited_domains.insert(domain);
                    }
                    if request.require_evidence && source.quotes.is_empty() {
                        errors.push(format!(
                            "Cited source {} has no verbatim evidence quote",
                            source.url
                        ));
                    }
                }
            }
            Some(Ok(page)) => {
                check["retrieval_status"] = serde_json::json!("http_error");
                check["http_status"] = serde_json::json!(page.status);
                if cited {
                    errors.push(format!(
                        "Cited source {} returned HTTP {} and is not citation-ready",
                        source.url, page.status
                    ));
                }
            }
            Some(Err(error)) => {
                check["error"] = serde_json::json!(error);
                if cited {
                    errors.push(format!(
                        "Cited source {} could not be retrieved",
                        source.url
                    ));
                }
            }
            None => {
                check["error"] = serde_json::json!("missing retrieval result");
                if cited {
                    errors.push(format!("Cited source {} was not checked", source.url));
                }
            }
        }
        source_checks.push(check);
    }

    if citation_urls.len() > 1 && cited_domains.len() < 2 {
        warnings.push(
            "Multiple citations resolve to only one independent domain; this is not corroboration"
                .to_string(),
        );
    }
    if sentence_stats.unverified > 0 {
        warnings.push(format!(
            "{} sentence(s) explicitly remain [unverified]",
            sentence_stats.unverified
        ));
    }

    let sources_markdown = render_sources_markdown(&request.sources, &citation_urls);
    let valid = errors.is_empty();
    Ok(serde_json::json!({
        "success": true,
        "tool": "web_citation_audit",
        "valid": valid,
        "scope": "Checks URL identity, successful retrieval, inline provenance coverage, and verbatim evidence presence. It does not prove semantic entailment or that a claim is true.",
        "coverage": {
            "prose_sentences": sentence_stats.total,
            "with_declared_provenance": sentence_stats.with_provenance,
            "unverified_sentences": sentence_stats.unverified,
            "ratio": coverage,
            "required": request.min_coverage,
            "independent_domains": cited_domains.len(),
        },
        "source_checks": source_checks,
        "errors": errors,
        "warnings": warnings,
        "sources_markdown": sources_markdown,
        "next_action": if valid {
            "Deliver the audited draft with sources_markdown. Preserve [unverified] markers and avoid claiming semantic verification beyond this audit scope."
        } else {
            "Repair every audit error, fetch stronger or independent evidence when needed, then rerun web_citation_audit before delivery."
        },
    }))
}

fn draft_before_sources(draft: &str) -> &str {
    let mut offset = 0usize;
    for line in draft.split_inclusive('\n') {
        let normalized = line
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_matches('*')
            .trim_end_matches(':')
            .trim();
        if normalized.eq_ignore_ascii_case("sources") {
            return &draft[..offset];
        }
        offset += line.len();
    }
    draft
}

fn extract_markdown_citation_urls(text: &str) -> Result<HashSet<String>, String> {
    let pattern = Regex::new(r#"\[[^\]\n]{1,200}\]\((https?://[^)\s]+)\)"#)
        .map_err(|error| format!("Compile citation matcher: {error}"))?;
    Ok(pattern
        .captures_iter(text)
        .filter_map(|capture| capture.get(1))
        .filter_map(|url| canonical_source_url(url.as_str()))
        .collect())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SentenceStats {
    total: usize,
    with_provenance: usize,
    unverified: usize,
}

fn provenance_sentence_stats(text: &str) -> Result<SentenceStats, String> {
    let citation_pattern = Regex::new(r#"\[[^\]\n]{1,200}\]\(https?://[^)\s]+\)"#)
        .map_err(|error| format!("Compile provenance matcher: {error}"))?;
    let trailing_provenance = Regex::new(r#"([.!?])((?:\s*\[(?:citation|unverified)\])+)(?:\s*)"#)
        .map_err(|error| format!("Compile trailing provenance matcher: {error}"))?;
    let mut stats = SentenceStats::default();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence
            || trimmed.is_empty()
            || trimmed.starts_with('#')
            || (trimmed.starts_with('|') && trimmed.ends_with('|'))
        {
            continue;
        }
        let prose = trimmed
            .trim_start_matches(|character: char| {
                character == '-' || character == '*' || character.is_ascii_digit()
            })
            .trim_start_matches(['.', ')'])
            .trim();
        let prose = citation_pattern.replace_all(prose, "[citation]");
        let prose = trailing_provenance.replace_all(&prose, "$2$1 ");
        for sentence in prose.split_inclusive(['.', '!', '?']) {
            if sentence.split_whitespace().count() < 4 {
                continue;
            }
            stats.total += 1;
            let unverified = sentence.to_ascii_lowercase().contains("[unverified]");
            if unverified {
                stats.unverified += 1;
            }
            if unverified || sentence.contains("[citation]") {
                stats.with_provenance += 1;
            }
        }
    }
    Ok(stats)
}

fn normalize_visible_text(text: &str) -> String {
    let without_links = Regex::new(r#"\[([^\]]+)\]\([^)]+\)"#)
        .ok()
        .map(|pattern| pattern.replace_all(text, "$1").into_owned())
        .unwrap_or_else(|| text.to_string());
    without_links
        .replace('\\', "")
        .replace(['*', '_', '`', '~'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn render_sources_markdown(
    sources: &[CitationAuditSource],
    citation_urls: &HashSet<String>,
) -> String {
    let mut output = String::from("## Sources\n");
    for source in sources
        .iter()
        .filter(|source| citation_urls.contains(&source.url))
    {
        let label = if source.title.trim().is_empty() {
            source_domain(&source.url).unwrap_or_else(|| source_id_for_url(&source.url))
        } else {
            source.title.replace(['[', ']'], "").trim().to_string()
        };
        output.push_str(&format!("\n- [{label}]({})", source.url));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, content: &str) -> WebPageEvidence {
        WebPageEvidence {
            requested_url: url.into(),
            final_url: url.into(),
            status: 200,
            content_type: "text/markdown".into(),
            retrieved_at: "2026-08-08T00:00:00Z".into(),
            content_sha256: sha256_text(content),
            content_chars: content.chars().count(),
            retained_chars: content.chars().count(),
            truncated: false,
            content: content.into(),
        }
    }

    fn request(draft: &str, quote: &str) -> CitationAuditInput {
        CitationAuditInput {
            draft: draft.into(),
            sources: vec![CitationAuditSource {
                url: "https://example.com/proof".into(),
                title: "Primary proof".into(),
                quotes: vec![quote.into()],
            }],
            min_coverage: 0.5,
            require_evidence: true,
        }
    }

    #[test]
    fn audit_accepts_retrieved_verbatim_evidence_through_markdown_markup() {
        let draft = "Water expands when it freezes.[Primary proof](https://example.com/proof)";
        let request = request(draft, "Water expands when it freezes.");
        let pages = HashMap::from([(
            "https://example.com/proof".into(),
            Ok(page(
                "https://example.com/proof",
                "[Water](https://example.com/water) **expands** when it freezes.",
            )),
        )]);

        let report = audit_citations(&request, &pages).unwrap();

        assert_eq!(report["valid"], true);
        assert_eq!(report["source_checks"][0]["quotes_found"], 1);
        assert!(report["sources_markdown"]
            .as_str()
            .unwrap()
            .contains("[Primary proof](https://example.com/proof)"));
    }

    #[test]
    fn audit_rejects_invented_url_and_paraphrased_evidence() {
        let draft = "Water expands when frozen.[Unknown](https://attacker.example/fake)";
        let request = request(draft, "Frozen water grows substantially in size.");
        let pages = HashMap::from([(
            "https://example.com/proof".into(),
            Ok(page(
                "https://example.com/proof",
                "Water expands when it freezes.",
            )),
        )]);

        let report = audit_citations(&request, &pages).unwrap();
        let errors = report["errors"].as_array().unwrap();

        assert_eq!(report["valid"], false);
        assert!(errors.iter().any(|error| error
            .as_str()
            .unwrap()
            .contains("not present in the audited source set")));
    }

    #[test]
    fn unverified_marker_declares_provenance_without_hiding_other_gaps() {
        let stats = provenance_sentence_stats(
            "A sourced claim has enough words.[Proof](https://example.com/proof)\n\
             A genuinely unavailable claim is marked here.[unverified]\n\
             A third external claim has no provenance at all.",
        )
        .unwrap();

        assert_eq!(stats.total, 3);
        assert_eq!(stats.with_provenance, 2);
        assert_eq!(stats.unverified, 1);
    }

    #[test]
    fn parser_rejects_duplicate_sources_after_url_normalization() {
        let input = serde_json::json!({
            "draft": "A sufficiently long claim with provenance.[Proof](https://example.com/proof)",
            "sources": [
                {"url": "https://example.com/proof#one", "quotes": []},
                {"url": "https://example.com/proof", "quotes": []}
            ]
        });

        let error = parse_audit_input(&input).unwrap_err();
        assert!(error.contains("Duplicate citation source URL"));
    }
}
