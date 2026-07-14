//! Vision provider calls for media understanding.

/// Default prompt sent to the vision API when no caller-supplied prompt exists.
/// French - Captain is configured in French by default.
pub(crate) const DEFAULT_VISION_PROMPT: &str = "Décris cette image avec précision : sujets visibles, actions ou mouvements, environnement, texte lisible, ambiance générale. Sois factuel et concis (3-6 phrases max).";

const ANTHROPIC_DEFAULT_BASE: &str = "https://api.anthropic.com";
const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com";
const GEMINI_DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

fn anthropic_base() -> String {
    std::env::var("ANTHROPIC_API_BASE").unwrap_or_else(|_| ANTHROPIC_DEFAULT_BASE.to_string())
}

fn openai_base() -> String {
    std::env::var("OPENAI_API_BASE").unwrap_or_else(|_| OPENAI_DEFAULT_BASE.to_string())
}

fn gemini_base() -> String {
    std::env::var("GEMINI_API_BASE").unwrap_or_else(|_| GEMINI_DEFAULT_BASE.to_string())
}

/// Detect which vision provider is available based on environment variables.
pub(crate) fn detect_vision_provider() -> Option<&'static str> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return Some("anthropic");
    }
    if std::env::var("OPENAI_API_KEY").is_ok() {
        return Some("openai");
    }
    if std::env::var("GEMINI_API_KEY").is_ok() || std::env::var("GOOGLE_API_KEY").is_ok() {
        return Some("gemini");
    }
    None
}

pub(crate) fn vision_prompt(context_hint: Option<&str>) -> String {
    match context_hint.map(str::trim) {
        Some(hint) if !hint.is_empty() => format!("{}\n\n{}", DEFAULT_VISION_PROMPT, hint),
        _ => DEFAULT_VISION_PROMPT.to_string(),
    }
}

pub(crate) async fn describe_with_provider(
    provider: &str,
    image_bytes: &[u8],
    mime: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    match provider {
        "anthropic" => describe_with_anthropic(image_bytes, mime, model, prompt).await,
        "openai" => describe_with_openai(image_bytes, mime, model, prompt).await,
        "gemini" => describe_with_gemini(image_bytes, mime, model, prompt).await,
        other => Err(format!("Unsupported vision provider: {}", other)),
    }
}

/// Get the default vision model for a provider.
#[cfg(test)]
fn default_vision_model(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-sonnet-4-6",
        "openai" => "gpt-4o",
        "gemini" => "gemini-2.5-flash",
        _ => "unknown",
    }
}

/// Pick the optimal vision model based on workload size.
pub(crate) fn pick_vision_model(provider: &str, frames_count: Option<usize>) -> &'static str {
    match (provider, frames_count) {
        ("anthropic", Some(n)) if n <= 5 => "claude-haiku-4-5-20251001",
        ("anthropic", _) => "claude-sonnet-4-6",
        ("openai", _) => "gpt-4o",
        ("gemini", _) => "gemini-2.5-flash",
        _ => "unknown",
    }
}

async fn describe_with_anthropic(
    image_bytes: &[u8],
    mime: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    use base64::Engine;

    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set")?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": mime,
                        "data": b64,
                    }
                },
                {
                    "type": "text",
                    "text": prompt,
                }
            ]
        }]
    });

    let url = format!("{}/v1/messages", anthropic_base());
    let resp = reqwest::Client::new()
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Anthropic vision request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, body));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Anthropic response decode failed: {}", e))?;

    json.get("content")
        .and_then(|c| c.get(0))
        .and_then(|first| first.get("text"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("Anthropic response missing content[0].text: {}", json))
}

async fn describe_with_openai(
    image_bytes: &[u8],
    mime: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    use base64::Engine;

    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set")?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    let data_url = format!("data:{};base64,{}", mime, b64);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        }]
    });

    let url = format!("{}/v1/chat/completions", openai_base());
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("OpenAI vision request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, body));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("OpenAI response decode failed: {}", e))?;

    json.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|first| first.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "OpenAI response missing choices[0].message.content: {}",
                json
            )
        })
}

async fn describe_with_gemini(
    image_bytes: &[u8],
    mime: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    use base64::Engine;

    let api_key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        .map_err(|_| "GEMINI_API_KEY or GOOGLE_API_KEY not set")?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

    let body = serde_json::json!({
        "contents": [{
            "parts": [
                {
                    "inline_data": {
                        "mime_type": mime,
                        "data": b64,
                    }
                },
                {
                    "text": prompt,
                }
            ]
        }]
    });

    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        gemini_base(),
        model,
        api_key
    );
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Gemini vision request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, body));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Gemini response decode failed: {}", e))?;

    json.get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|first| first.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|first| first.get("text"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "Gemini response missing candidates[0].content.parts[0].text: {}",
                json
            )
        })
}

#[cfg(test)]
#[path = "media_vision_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "media_vision_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "media_vision_anthropic_tests.rs"]
mod anthropic_tests;

#[cfg(test)]
#[path = "media_vision_cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
#[path = "media_vision_provider_tests.rs"]
mod provider_tests;

#[cfg(test)]
#[path = "media_vision_prompt_tests.rs"]
mod prompt_tests;
