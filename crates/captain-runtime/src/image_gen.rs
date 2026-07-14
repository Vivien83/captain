//! Image generation through OpenAI Images API plus FAL queue support.

use base64::Engine;
use captain_types::media::{GeneratedImage, ImageGenRequest, ImageGenResult};
use tracing::warn;

pub use crate::image_fal::{generate_fal_image, DEFAULT_FAL_IMAGE_MODEL};

const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// Generate images via OpenAI's image generation API.
///
/// Requires OPENAI_API_KEY to be set.
pub async fn generate_image(request: &ImageGenRequest) -> Result<ImageGenResult, String> {
    request.validate()?;

    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY not set. Image generation requires an OpenAI API key.")?;

    let model_str = request.model.to_string();

    let mut body = serde_json::json!({
        "model": model_str,
        "prompt": request.prompt,
        "n": request.count,
        "size": request.size,
    });

    match request.model {
        captain_types::media::ImageGenModel::DallE3 => {
            body["quality"] = serde_json::json!(request.quality);
            body["response_format"] = serde_json::json!("b64_json");
        }
        captain_types::media::ImageGenModel::DallE2 => {
            body["response_format"] = serde_json::json!("b64_json");
        }
        captain_types::media::ImageGenModel::GptImage2
        | captain_types::media::ImageGenModel::GptImage15
        | captain_types::media::ImageGenModel::GptImage1
        | captain_types::media::ImageGenModel::GptImage1Mini => {
            body["quality"] = serde_json::json!(request.quality);
            body["output_format"] = serde_json::json!("png");
        }
    }

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/images/generations")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("Image generation API request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        let truncated = crate::str_utils::safe_truncate_str(&error_body, 500);
        return Err(format!(
            "Image generation failed (HTTP {}): {}",
            status, truncated
        ));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse image generation response: {e}"))?;

    let mut images = Vec::new();
    let mut revised_prompt = None;

    if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
        for item in data {
            let b64 = item
                .get("b64_json")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if b64.len() > MAX_IMAGE_BYTES {
                warn!("Generated image data exceeds 10MB, skipping");
                continue;
            }

            images.push(GeneratedImage {
                data_base64: b64,
                url,
            });

            if revised_prompt.is_none() {
                revised_prompt = item
                    .get("revised_prompt")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
        }
    }

    if images.is_empty() {
        return Err("No images returned by the API".into());
    }

    Ok(ImageGenResult {
        images,
        model: model_str,
        revised_prompt,
    })
}

/// Save generated images to workspace output directory.
pub fn save_images_to_workspace(
    result: &ImageGenResult,
    workspace: &std::path::Path,
) -> Result<Vec<String>, String> {
    let output_dir = workspace.join("output");
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create output dir: {e}"))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let mut paths = Vec::new();

    for (i, image) in result.images.iter().enumerate() {
        let filename = if result.images.len() == 1 {
            format!("image_{timestamp}.png")
        } else {
            format!("image_{timestamp}_{i}.png")
        };

        let path = output_dir.join(&filename);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&image.data_base64)
            .map_err(|e| format!("Failed to decode base64 image: {e}"))?;

        if decoded.len() > MAX_IMAGE_BYTES {
            return Err("Decoded image exceeds 10MB limit".into());
        }

        std::fs::write(&path, &decoded)
            .map_err(|e| format!("Failed to write image to {}: {e}", path.display()))?;

        paths.push(path.display().to_string());
    }

    Ok(paths)
}

#[cfg(test)]
#[path = "image_gen_tests.rs"]
mod tests;
