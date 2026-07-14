//! FAL queue-backed image generation.

use base64::Engine;
use captain_types::media::{GeneratedImage, ImageGenRequest, ImageGenResult};
use serde_json::Value;

const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const FAL_QUEUE_ORIGIN: &str = "https://queue.fal.run";
pub const DEFAULT_FAL_IMAGE_MODEL: &str = "fal-ai/flux-2/klein/9b";

/// Generate images via FAL's queue API.
///
/// This keeps FAL as the fast product rail while OpenAI Images remains
/// available for users who explicitly select it.
pub async fn generate_fal_image(
    prompt: &str,
    model: Option<&str>,
    aspect_ratio: Option<&str>,
    count: u8,
    quality: Option<&str>,
) -> Result<ImageGenResult, String> {
    validate_fal_prompt(prompt)?;
    let api_key = std::env::var("FAL_KEY")
        .map_err(|_| "FAL_KEY not set. FAL image generation requires a FAL.ai API key.")?;
    let model_id = normalize_fal_image_model(model.unwrap_or(DEFAULT_FAL_IMAGE_MODEL))?;
    let payload = build_fal_image_payload(
        model_id,
        prompt,
        aspect_ratio.unwrap_or("landscape"),
        count.clamp(1, 4),
        quality,
    )?;

    let client = reqwest::Client::new();
    let response = submit_fal_queue_request(&client, &api_key, model_id, payload).await?;
    let images = extract_fal_image_entries(&response)?;
    let mut generated = Vec::with_capacity(images.len());
    for image in images {
        let data_base64 = download_image_as_base64(&client, &image.url).await?;
        generated.push(GeneratedImage {
            data_base64,
            url: Some(image.url),
        });
    }

    if generated.is_empty() {
        return Err("No images returned by FAL".into());
    }

    Ok(ImageGenResult {
        images: generated,
        model: model_id.to_string(),
        revised_prompt: None,
    })
}

fn validate_fal_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() {
        return Err("Image generation prompt cannot be empty".into());
    }
    if prompt.len() > ImageGenRequest::MAX_PROMPT_LEN {
        return Err(format!(
            "Prompt too long: {} chars (max {})",
            prompt.len(),
            ImageGenRequest::MAX_PROMPT_LEN
        ));
    }
    if prompt
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        return Err("Prompt contains invalid control characters".into());
    }
    Ok(())
}

fn normalize_fal_image_model(model: &str) -> Result<&'static str, String> {
    let normalized = model.trim();
    match normalized {
        "" | "auto" | "fal" | "flux-2-klein" | "flux-klein" | "fal-ai/flux-2/klein/9b" => {
            Ok("fal-ai/flux-2/klein/9b")
        }
        "flux-2-pro" | "flux-pro" | "fal-ai/flux-2-pro" => Ok("fal-ai/flux-2-pro"),
        "gpt-image-1.5" | "fal-gpt-image-1.5" | "fal-ai/gpt-image-1.5" => {
            Ok("fal-ai/gpt-image-1.5")
        }
        "nano-banana-pro" | "fal-ai/nano-banana-pro" => Ok("fal-ai/nano-banana-pro"),
        "z-image-turbo" | "fal-ai/z-image/turbo" => Ok("fal-ai/z-image/turbo"),
        "ideogram-v3" | "fal-ai/ideogram/v3" => Ok("fal-ai/ideogram/v3"),
        "recraft-v4-pro" | "fal-ai/recraft/v4/pro/text-to-image" => {
            Ok("fal-ai/recraft/v4/pro/text-to-image")
        }
        "qwen-image" | "fal-ai/qwen-image" => Ok("fal-ai/qwen-image"),
        other => Err(format!(
            "Unknown FAL image model: {other}. Use one of: fal-ai/flux-2/klein/9b, fal-ai/flux-2-pro, fal-ai/gpt-image-1.5, fal-ai/nano-banana-pro, fal-ai/z-image/turbo, fal-ai/ideogram/v3, fal-ai/recraft/v4/pro/text-to-image, fal-ai/qwen-image."
        )),
    }
}

fn build_fal_image_payload(
    model_id: &str,
    prompt: &str,
    aspect_ratio: &str,
    count: u8,
    quality: Option<&str>,
) -> Result<Value, String> {
    let aspect = normalize_fal_aspect_ratio(aspect_ratio);
    let count = count.clamp(1, 4);
    let payload = match model_id {
        "fal-ai/flux-2/klein/9b" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_preset_size(aspect),
            "num_inference_steps": 4,
            "output_format": "png",
            "enable_safety_checker": false,
        }),
        "fal-ai/flux-2-pro" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_preset_size(aspect),
            "num_inference_steps": 50,
            "guidance_scale": 4.5,
            "num_images": count,
            "output_format": "png",
            "enable_safety_checker": false,
            "safety_tolerance": "5",
            "sync_mode": true,
        }),
        "fal-ai/gpt-image-1.5" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_gpt_literal_size(aspect),
            "quality": quality.unwrap_or("medium"),
            "num_images": count,
            "output_format": "png",
        }),
        "fal-ai/nano-banana-pro" => serde_json::json!({
            "prompt": prompt.trim(),
            "aspect_ratio": fal_literal_aspect_ratio(aspect),
            "num_images": count,
            "output_format": "png",
            "safety_tolerance": "5",
            "resolution": "1K",
        }),
        "fal-ai/z-image/turbo" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_preset_size(aspect),
            "num_inference_steps": 8,
            "num_images": count,
            "output_format": "png",
            "enable_safety_checker": false,
            "enable_prompt_expansion": false,
        }),
        "fal-ai/ideogram/v3" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_preset_size(aspect),
            "rendering_speed": "BALANCED",
            "expand_prompt": true,
            "style": "AUTO",
        }),
        "fal-ai/recraft/v4/pro/text-to-image" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_preset_size(aspect),
            "enable_safety_checker": false,
        }),
        "fal-ai/qwen-image" => serde_json::json!({
            "prompt": prompt.trim(),
            "image_size": fal_preset_size(aspect),
            "num_inference_steps": 30,
            "guidance_scale": 2.5,
            "num_images": count,
            "output_format": "png",
            "acceleration": "regular",
        }),
        other => {
            return Err(format!(
                "Unsupported FAL image model after normalization: {other}"
            ))
        }
    };
    Ok(payload)
}

fn normalize_fal_aspect_ratio(aspect_ratio: &str) -> &'static str {
    match aspect_ratio.trim().to_ascii_lowercase().as_str() {
        "square" | "1:1" | "1024x1024" => "square",
        "portrait" | "9:16" | "1024x1536" | "1024x1792" => "portrait",
        _ => "landscape",
    }
}

fn fal_preset_size(aspect: &str) -> &'static str {
    match aspect {
        "square" => "square_hd",
        "portrait" => "portrait_16_9",
        _ => "landscape_16_9",
    }
}

fn fal_gpt_literal_size(aspect: &str) -> &'static str {
    match aspect {
        "square" => "1024x1024",
        "portrait" => "1024x1536",
        _ => "1536x1024",
    }
}

fn fal_literal_aspect_ratio(aspect: &str) -> &'static str {
    match aspect {
        "square" => "1:1",
        "portrait" => "9:16",
        _ => "16:9",
    }
}

async fn submit_fal_queue_request(
    client: &reqwest::Client,
    api_key: &str,
    model_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let submit_url = format!("{FAL_QUEUE_ORIGIN}/{model_id}");
    let submit = client
        .post(&submit_url)
        .header("Authorization", format!("Key {api_key}"))
        .header("Content-Type", "application/json")
        .header("x-idempotency-key", uuid::Uuid::new_v4().to_string())
        .header(
            "X-Fal-Object-Lifecycle-Preference",
            r#"{"expiration_duration_seconds":86400}"#,
        )
        .json(&payload)
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await
        .map_err(|e| format!("FAL image request failed: {e}"))?;
    if !submit.status().is_success() {
        let status = submit.status();
        let body = submit.text().await.unwrap_or_default();
        return Err(format!(
            "FAL image request failed (HTTP {status}): {}",
            crate::str_utils::safe_truncate_str(&body, 500)
        ));
    }
    let submitted: Value = submit
        .json()
        .await
        .map_err(|e| format!("Failed to parse FAL submit response: {e}"))?;
    let status_url = submitted["status_url"]
        .as_str()
        .ok_or("FAL submit response missing status_url")?
        .to_string();
    let response_url = submitted["response_url"].as_str().map(str::to_string);

    for _ in 0..90 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let status_resp = client
            .get(&status_url)
            .header("Authorization", format!("Key {api_key}"))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("FAL status request failed: {e}"))?;
        let status_code = status_resp.status();
        let status_body = status_resp.text().await.unwrap_or_default();
        let status_json: Value = serde_json::from_str(&status_body).map_err(|e| {
            format!(
                "Failed to parse FAL status response (HTTP {status_code}): {e}: {}",
                crate::str_utils::safe_truncate_str(&status_body, 500)
            )
        })?;
        match status_json["status"].as_str().unwrap_or("") {
            "COMPLETED" => {
                let final_url = status_json["response_url"]
                    .as_str()
                    .or(response_url.as_deref())
                    .ok_or("FAL completed status missing response_url")?;
                return fetch_fal_response(client, api_key, final_url).await;
            }
            "IN_QUEUE" | "IN_PROGRESS" => continue,
            other => {
                return Err(format!(
                    "FAL request failed with status '{}': {}",
                    other,
                    crate::str_utils::safe_truncate_str(&status_body, 500)
                ))
            }
        }
    }

    Err("FAL image generation timed out after 180s".into())
}

async fn fetch_fal_response(
    client: &reqwest::Client,
    api_key: &str,
    response_url: &str,
) -> Result<Value, String> {
    let response = client
        .get(response_url)
        .header("Authorization", format!("Key {api_key}"))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("FAL response fetch failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "FAL response fetch failed (HTTP {status}): {}",
            crate::str_utils::safe_truncate_str(&body, 500)
        ));
    }
    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse FAL response: {e}"))
}

struct FalImageEntry {
    url: String,
}

fn extract_fal_image_entries(response: &Value) -> Result<Vec<FalImageEntry>, String> {
    let payload = response.get("response").unwrap_or(response);
    let mut entries = Vec::new();
    if let Some(images) = payload.get("images").and_then(|v| v.as_array()) {
        for image in images {
            if let Some(url) = image.get("url").and_then(|v| v.as_str()) {
                entries.push(FalImageEntry {
                    url: url.to_string(),
                });
            }
        }
    }
    if entries.is_empty() {
        if let Some(url) = payload
            .get("image")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
        {
            entries.push(FalImageEntry {
                url: url.to_string(),
            });
        }
    }
    if entries.is_empty() {
        return Err(format!(
            "FAL response contained no image URLs: {}",
            crate::str_utils::safe_truncate_str(&response.to_string(), 500)
        ));
    }
    Ok(entries)
}

async fn download_image_as_base64(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Generated image download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Generated image download failed (HTTP {})",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Generated image body read failed: {e}"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("Generated image exceeds 10MB limit".into());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[cfg(test)]
#[path = "image_fal_tests.rs"]
mod tests;
