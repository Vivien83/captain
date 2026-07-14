//! Embedding driver for vector-based semantic memory.
//!
//! Provides an `EmbeddingDriver` trait and an OpenAI-compatible implementation
//! that works with any provider offering a `/v1/embeddings` endpoint (OpenAI,
//! Groq, Together, Fireworks, Ollama, etc.).

use async_trait::async_trait;
use captain_types::model_catalog::{
    FIREWORKS_BASE_URL, GROQ_BASE_URL, LMSTUDIO_BASE_URL, MISTRAL_BASE_URL, OLLAMA_BASE_URL,
    OPENAI_BASE_URL, TOGETHER_BASE_URL, VLLM_BASE_URL,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// Error type for embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Missing API key: {0}")]
    MissingApiKey(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

/// Configuration for creating an embedding driver.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Provider name (openai, groq, together, ollama, etc.).
    pub provider: String,
    /// Model name (e.g., "text-embedding-3-small", "all-MiniLM-L6-v2").
    pub model: String,
    /// API key (resolved from env var).
    pub api_key: String,
    /// Base URL for the API.
    pub base_url: String,
}

/// Trait for computing text embeddings.
#[async_trait]
pub trait EmbeddingDriver: Send + Sync {
    /// Compute embedding vectors for a batch of texts.
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Compute embedding for a single text.
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let results = self.embed(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::Parse("Empty embedding response".to_string()))
    }

    /// Return the dimensionality of embeddings produced by this driver.
    fn dimensions(&self) -> usize;
}

/// OpenAI-compatible embedding driver.
///
/// Works with any provider that implements the `/v1/embeddings` endpoint:
/// OpenAI, Groq, Together, Fireworks, Ollama, vLLM, LM Studio, etc.
pub struct OpenAIEmbeddingDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    model: String,
    client: reqwest::Client,
    dims: usize,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

impl OpenAIEmbeddingDriver {
    /// Create a new OpenAI-compatible embedding driver.
    pub fn new(config: EmbeddingConfig) -> Result<Self, EmbeddingError> {
        // Infer dimensions from model name (common models)
        let dims = infer_dimensions(&config.model);

        Ok(Self {
            api_key: Zeroizing::new(config.api_key),
            base_url: config.base_url,
            model: config.model,
            client: reqwest::Client::new(),
            dims,
        })
    }
}

/// Infer embedding dimensions from model name.
fn infer_dimensions(model: &str) -> usize {
    match model {
        // OpenAI
        "text-embedding-3-small" => 1536,
        "text-embedding-3-large" => 3072,
        "text-embedding-ada-002" => 1536,
        // Sentence Transformers / local models
        "all-MiniLM-L6-v2" => 384,
        "all-MiniLM-L12-v2" => 384,
        "all-mpnet-base-v2" => 768,
        "nomic-embed-text" => 768,
        "mxbai-embed-large" => 1024,
        // Default to 1536 (most common)
        _ => 1536,
    }
}

#[async_trait]
impl EmbeddingDriver for OpenAIEmbeddingDriver {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let url = format!("{}/embeddings", self.base_url);
        let body = EmbedRequest {
            model: &self.model,
            input: texts,
        };

        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.as_str().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key.as_str()));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| EmbeddingError::Http(e.to_string()))?;
        let status = resp.status().as_u16();

        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(EmbeddingError::Api {
                status,
                message: body_text,
            });
        }

        let data: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| EmbeddingError::Parse(e.to_string()))?;

        // Update dimensions from actual response if available
        let embeddings: Vec<Vec<f32>> = data.data.into_iter().map(|d| d.embedding).collect();

        debug!(
            "Embedded {} texts (dims={})",
            embeddings.len(),
            embeddings.first().map(|e| e.len()).unwrap_or(0)
        );

        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

// ---------------------------------------------------------------------------
// Local embedding driver (fastembed — ONNX Runtime, no external dependency)
// ---------------------------------------------------------------------------

#[cfg(feature = "local-embeddings")]
pub struct LocalEmbeddingDriver {
    model: std::sync::Mutex<fastembed::TextEmbedding>,
    dims: usize,
}

#[cfg(feature = "local-embeddings")]
impl LocalEmbeddingDriver {
    /// Create a local embedding driver using fastembed + ONNX Runtime.
    /// Downloads the model from HuggingFace on first run (~90 MB),
    /// then caches it locally.
    pub fn new(model_name: &str) -> Result<Self, EmbeddingError> {
        let fe_model = match model_name {
            "all-MiniLM-L6-v2" => fastembed::EmbeddingModel::AllMiniLML6V2,
            "all-MiniLM-L6-v2-q" => fastembed::EmbeddingModel::AllMiniLML6V2Q,
            "BGESmallENV15" | "bge-small-en-v1.5" => fastembed::EmbeddingModel::BGESmallENV15,
            other => {
                warn!(model = %other, "Unknown local model, falling back to AllMiniLML6V2");
                fastembed::EmbeddingModel::AllMiniLML6V2
            }
        };

        let dims = infer_dimensions(model_name);

        // Pin the cache under CAPTAIN_HOME instead of fastembed's cwd-relative
        // default. The daemon already ends up there (its cwd is CAPTAIN_HOME),
        // but CLI commands and Docker image builds run from arbitrary
        // directories and would otherwise download the ~90 MB model into a
        // cache the daemon never looks at.
        let cache_dir = crate::native_embeddings::captain_home_dir().join(".fastembed_cache");
        let opts = fastembed::InitOptions::new(fe_model)
            .with_cache_dir(cache_dir)
            .with_show_download_progress(true);
        let ort_path =
            crate::native_embeddings::configure_environment().map_err(EmbeddingError::Config)?;

        let model = fastembed::TextEmbedding::try_new(opts)
            .map_err(|e| EmbeddingError::Http(format!("fastembed init failed: {e}")))?;

        debug!(
            model = %model_name,
            dims,
            onnx_runtime = %ort_path.display(),
            "Local embedding driver ready (fastembed/ONNX)"
        );

        Ok(Self {
            model: std::sync::Mutex::new(model),
            dims,
        })
    }
}

#[cfg(feature = "local-embeddings")]
#[async_trait]
impl EmbeddingDriver for LocalEmbeddingDriver {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let docs: Vec<String> = texts.iter().map(|t| t.to_string()).collect();

        // fastembed is synchronous — run on blocking thread to avoid starving tokio
        let mut model_guard = self
            .model
            .lock()
            .map_err(|e| EmbeddingError::Http(format!("Lock poisoned: {e}")))?;

        let embeddings = model_guard
            .embed(docs, None)
            .map_err(|e| EmbeddingError::Http(format!("fastembed embed failed: {e}")))?;

        debug!(
            "Local embed: {} texts, dims={}",
            embeddings.len(),
            self.dims
        );
        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

/// Create a local embedding driver (fastembed/ONNX, no external service).
#[cfg(feature = "local-embeddings")]
pub fn create_local_embedding_driver(
    model: &str,
) -> Result<Box<dyn EmbeddingDriver + Send + Sync>, EmbeddingError> {
    let driver = LocalEmbeddingDriver::new(model)?;
    Ok(Box::new(driver))
}

/// Create an embedding driver from kernel config.
///
/// Special-case: `provider == "local"` short-circuits to the in-process
/// fastembed/ONNX driver — no HTTP, no external service. The `model`
/// argument must be a name recognised by `LocalEmbeddingDriver::new`
/// (e.g. `"all-MiniLM-L6-v2"`, `"bge-small-en-v1.5"`). All other
/// `api_key_env` / `custom_base_url` arguments are ignored in this case.
pub fn create_embedding_driver(
    provider: &str,
    model: &str,
    api_key_env: &str,
    custom_base_url: Option<&str>,
) -> Result<Box<dyn EmbeddingDriver + Send + Sync>, EmbeddingError> {
    if provider == "local" {
        #[cfg(feature = "local-embeddings")]
        {
            return create_local_embedding_driver(model);
        }
        #[cfg(not(feature = "local-embeddings"))]
        {
            return Err(EmbeddingError::Config(
                "provider='local' requested but the 'local-embeddings' feature is disabled at build time".into(),
            ));
        }
    }
    let api_key = if api_key_env.is_empty() {
        String::new()
    } else {
        std::env::var(api_key_env).unwrap_or_default()
    };

    let base_url = custom_base_url
        .filter(|u| !u.is_empty())
        .map(|u| {
            let trimmed = u.trim_end_matches('/');
            // All OpenAI-compatible embedding providers need /v1 in the path.
            // If the user supplied a bare host URL (e.g. "http://192.168.0.1:11434"),
            // append /v1 so the final request hits {base}/v1/embeddings.
            let needs_v1 = matches!(
                provider,
                "openai"
                    | "groq"
                    | "together"
                    | "fireworks"
                    | "mistral"
                    | "ollama"
                    | "vllm"
                    | "lmstudio"
            );
            if needs_v1 && !trimmed.ends_with("/v1") {
                format!("{trimmed}/v1")
            } else {
                trimmed.to_string()
            }
        })
        .unwrap_or_else(|| match provider {
            "openai" => OPENAI_BASE_URL.to_string(),
            "groq" => GROQ_BASE_URL.to_string(),
            "together" => TOGETHER_BASE_URL.to_string(),
            "fireworks" => FIREWORKS_BASE_URL.to_string(),
            "mistral" => MISTRAL_BASE_URL.to_string(),
            "ollama" => OLLAMA_BASE_URL.to_string(),
            "vllm" => VLLM_BASE_URL.to_string(),
            "lmstudio" => LMSTUDIO_BASE_URL.to_string(),
            other => {
                warn!("Unknown embedding provider '{other}', using OpenAI-compatible format");
                format!("https://{other}/v1")
            }
        });

    // SECURITY: Warn when embedding requests will be sent to an external API
    let is_local = base_url.contains("localhost")
        || base_url.contains("127.0.0.1")
        || base_url.contains("[::1]");
    if !is_local {
        warn!(
            provider = %provider,
            base_url = %base_url,
            "Embedding driver configured to send data to external API — text content will leave this machine"
        );
    }

    let config = EmbeddingConfig {
        provider: provider.to_string(),
        model: model.to_string(),
        api_key,
        base_url,
    };

    let driver = OpenAIEmbeddingDriver::new(config)?;
    Ok(Box::new(driver))
}

/// Compute cosine similarity between two vectors.
///
/// Returns a value in [-1.0, 1.0] where 1.0 = identical direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

/// Serialize an embedding vector to bytes (for SQLite BLOB storage).
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize an embedding vector from bytes.
pub fn embedding_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
