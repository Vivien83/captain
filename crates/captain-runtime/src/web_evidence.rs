//! Typed provenance carried between native web discovery, retrieval, and audit.

use crate::web_content::wrap_external_content;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WebSearchResultEvidence {
    pub source_id: String,
    pub title: String,
    pub url: String,
    pub canonical_url: String,
    pub snippet: String,
}

impl WebSearchResultEvidence {
    pub(crate) fn new(title: &str, url: &str, snippet: &str) -> Option<Self> {
        let canonical_url = canonical_source_url(url)?;
        Some(Self {
            source_id: source_id_for_url(&canonical_url),
            title: title.trim().to_string(),
            url: url.trim().to_string(),
            canonical_url,
            snippet: snippet.trim().to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WebSearchEvidence {
    pub query: String,
    pub provider: String,
    pub retrieved_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_summary: Option<String>,
    pub results: Vec<WebSearchResultEvidence>,
}

impl WebSearchEvidence {
    pub(crate) fn new(
        query: &str,
        provider: &str,
        provider_summary: Option<String>,
        results: Vec<WebSearchResultEvidence>,
    ) -> Self {
        Self {
            query: query.to_string(),
            provider: provider.to_string(),
            retrieved_at: chrono::Utc::now().to_rfc3339(),
            provider_summary,
            results,
        }
    }

    pub(crate) fn render_tool_result(&self) -> Result<String, String> {
        let payload = serde_json::to_string_pretty(&serde_json::json!({
            "success": true,
            "tool": "web_search",
            "query": self.query,
            "provider": self.provider,
            "retrieved_at": self.retrieved_at,
            "provider_summary": self.provider_summary,
            "results": self.results.iter().map(|result| serde_json::json!({
                "source_id": result.source_id,
                "title": result.title,
                "url": result.url,
                "canonical_url": result.canonical_url,
                "snippet": result.snippet,
                "retrieval_status": "discovery_only",
                "citation_ready": false,
            })).collect::<Vec<_>>(),
            "next_action": "Search snippets are discovery evidence only. Fetch a source before citing it.",
        }))
        .map_err(|error| format!("Serialize web search evidence: {error}"))?;
        Ok(wrap_external_content("web-search-evidence", &payload))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WebPageEvidence {
    pub requested_url: String,
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub retrieved_at: String,
    pub content_sha256: String,
    pub content_chars: usize,
    pub retained_chars: usize,
    pub truncated: bool,
    pub content: String,
}

impl WebPageEvidence {
    pub(crate) fn citation_ready(&self) -> bool {
        (200..300).contains(&self.status) && !self.content.trim().is_empty()
    }

    pub(crate) fn render_tool_result(&self) -> String {
        format!(
            "HTTP {}\nFinal URL: {}\nRetrieved: {}\nContent SHA-256: {}\n\n{}",
            self.status,
            self.final_url,
            self.retrieved_at,
            self.content_sha256,
            wrap_external_content(&self.requested_url, &self.content)
        )
    }
}

pub(crate) fn canonical_source_url(raw: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    let mut canonical = url.to_string();
    if url.path() != "/" {
        canonical = canonical.trim_end_matches('/').to_string();
    }
    Some(canonical)
}

pub(crate) fn source_id_for_url(url: &str) -> String {
    let canonical = canonical_source_url(url).unwrap_or_else(|| url.trim().to_string());
    let digest = Sha256::digest(canonical.as_bytes());
    format!("SRC-{}", hex::encode(&digest[..5]).to_ascii_uppercase())
}

pub(crate) fn source_domain(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}

pub(crate) fn sha256_text(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls_and_ids_are_stable_across_fragments() {
        let first = canonical_source_url(" HTTPS://Example.COM/path/#section ").unwrap();
        let second = canonical_source_url("https://example.com/path").unwrap();

        assert_eq!(first, "https://example.com/path");
        assert_eq!(first, second);
        assert_eq!(source_id_for_url(&first), source_id_for_url(&second));
    }

    #[test]
    fn page_is_citation_ready_only_after_successful_nonempty_retrieval() {
        let mut page = WebPageEvidence {
            requested_url: "https://example.com".into(),
            final_url: "https://example.com/".into(),
            status: 200,
            content_type: "text/plain".into(),
            retrieved_at: "2026-08-08T00:00:00Z".into(),
            content_sha256: sha256_text("proof"),
            content_chars: 5,
            retained_chars: 5,
            truncated: false,
            content: "proof".into(),
        };
        assert!(page.citation_ready());
        page.status = 404;
        assert!(!page.citation_ready());
        page.status = 200;
        page.content.clear();
        assert!(!page.citation_ready());
    }
}
