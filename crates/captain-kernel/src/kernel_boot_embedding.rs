use super::kernel_model_support::default_embedding_model_for_provider;
use captain_runtime::embedding::{create_embedding_driver, EmbeddingDriver};
use captain_types::config::KernelConfig;
use std::sync::Arc;
use tracing::{info, warn};

pub(super) fn build_boot_embedding_driver(
    config: &KernelConfig,
) -> Option<Arc<dyn EmbeddingDriver + Send + Sync>> {
    let configured_model = &config.memory.embedding_model;
    if let Some(ref provider) = config.memory.embedding_provider {
        // Explicit config takes priority. If the user left the local default,
        // choose a provider-specific default so cloud APIs do not receive a
        // local model name.
        let model = effective_embedding_model(configured_model, provider);
        let api_key_env = config.memory.embedding_api_key_env.as_deref().unwrap_or("");
        let custom_url = config
            .provider_urls
            .get(provider.as_str())
            .map(|s| s.as_str());
        return match create_embedding_driver(provider, model, api_key_env, custom_url) {
            Ok(d) => {
                info!(provider = %provider, model = %model, "Embedding driver configured from memory config");
                Some(Arc::from(d))
            }
            Err(e) => {
                warn!(provider = %provider, error = %e, "Embedding driver init failed — falling back to text search");
                None
            }
        };
    }

    // `is_ok()` alone would treat an empty var as configured — docker compose
    // always exports every listed key (as "" when unset on the host), which
    // made this pick OpenAI with no key and 401 on every embedding call,
    // silently breaking Tool RAG in the default Docker install.
    let openai_key_present = std::env::var("OPENAI_API_KEY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if openai_key_present {
        let model = effective_embedding_model(configured_model, "openai");
        let openai_url = config.provider_urls.get("openai").map(|s| s.as_str());
        return match create_embedding_driver("openai", model, "OPENAI_API_KEY", openai_url) {
            Ok(d) => {
                info!(model = %model, "Embedding driver auto-detected: OpenAI");
                Some(Arc::from(d))
            }
            Err(e) => {
                warn!(error = %e, "OpenAI embedding auto-detect failed");
                None
            }
        };
    }

    let model = effective_embedding_model(configured_model, "ollama");
    let ollama_url = config.provider_urls.get("ollama").map(|s| s.as_str());
    // Auto-detection only: creating the HTTP driver always succeeds, so
    // without a reachability check a machine with no Ollama (a Docker
    // container, a VPS) ends up with a driver whose every call fails and
    // the local ONNX fallback below is never reached. An explicit
    // embedding_provider="ollama" in config skips this probe (handled
    // above) and keeps trusting the user's configuration.
    if !ollama_reachable(ollama_url) {
        return local_embedding_fallback(
            configured_model,
            captain_runtime::embedding::EmbeddingError::Http(
                "Ollama is not reachable for embedding auto-detection".to_string(),
            ),
        );
    }
    match create_embedding_driver("ollama", model, "", ollama_url) {
        Ok(d) => {
            info!(model = %model, "Embedding driver auto-detected: Ollama (local)");
            Some(Arc::from(d))
        }
        Err(_ollama_err) => local_embedding_fallback(configured_model, _ollama_err),
    }
}

fn ollama_reachable(custom_url: Option<&str>) -> bool {
    let url = custom_url.unwrap_or(captain_types::model_catalog::OLLAMA_BASE_URL);
    let host_port = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    let target = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:11434")
    };
    let Ok(mut addrs) = std::net::ToSocketAddrs::to_socket_addrs(target.as_str()) else {
        return false;
    };
    addrs.next().is_some_and(|addr| {
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
    })
}

fn effective_embedding_model<'a>(configured_model: &'a str, provider: &str) -> &'a str {
    if configured_model == "all-MiniLM-L6-v2" {
        default_embedding_model_for_provider(provider)
    } else {
        configured_model
    }
}

#[cfg(feature = "local-embeddings")]
fn local_embedding_fallback(
    configured_model: &str,
    _ollama_err: captain_runtime::embedding::EmbeddingError,
) -> Option<Arc<dyn EmbeddingDriver + Send + Sync>> {
    match captain_runtime::embedding::create_local_embedding_driver(configured_model) {
        Ok(d) => {
            info!(model = %configured_model, "Embedding driver: local fastembed/ONNX");
            Some(Arc::from(d))
        }
        Err(e) => {
            warn!(error = %e, "Local embedding init failed — using text search fallback");
            None
        }
    }
}

#[cfg(not(feature = "local-embeddings"))]
fn local_embedding_fallback(
    _configured_model: &str,
    ollama_err: captain_runtime::embedding::EmbeddingError,
) -> Option<Arc<dyn EmbeddingDriver + Send + Sync>> {
    tracing::debug!("No embedding driver available (Ollama probe failed: {ollama_err}) — using text search fallback");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_embedding_model_replaces_legacy_default_for_provider() {
        assert_eq!(
            effective_embedding_model("all-MiniLM-L6-v2", "openai"),
            "text-embedding-3-small"
        );
        assert_eq!(
            effective_embedding_model("all-MiniLM-L6-v2", "ollama"),
            "nomic-embed-text"
        );
    }

    #[test]
    fn effective_embedding_model_keeps_explicit_model() {
        assert_eq!(
            effective_embedding_model("custom-embedding-model", "openai"),
            "custom-embedding-model"
        );
    }
}
