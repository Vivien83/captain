//! Image analysis and generation runtime handlers.

use std::path::Path;

use captain_types::media::{ImageGenModel, ImageGenRequest, ImageGenResult};
use tracing::warn;

pub(crate) async fn tool_image_analyze(input: &serde_json::Value) -> Result<String, String> {
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let prompt = input["prompt"].as_str().unwrap_or("");
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image '{path}': {e}"))?;
    let file_size = data.len();
    let format = detect_image_format(&data);
    let dimensions = extract_image_dimensions(&data, &format);

    let base64_preview = if file_size <= 512 * 1024 {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&data)
    } else {
        use base64::Engine;
        let preview_bytes = &data[..64 * 1024];
        format!(
            "{}... [truncated, {} total bytes]",
            base64::engine::general_purpose::STANDARD.encode(preview_bytes),
            file_size
        )
    };

    let mut result = serde_json::json!({
        "path": path,
        "format": format,
        "file_size_bytes": file_size,
        "file_size_human": format_file_size(file_size),
    });
    if let Some((w, h)) = dimensions {
        result["width"] = serde_json::json!(w);
        result["height"] = serde_json::json!(h);
    }
    if !prompt.is_empty() {
        result["prompt"] = serde_json::json!(prompt);
        result["note"] = serde_json::json!(
            "Vision analysis requires a vision-capable LLM. The base64 data is included for downstream processing."
        );
    }
    result["base64_preview"] = serde_json::json!(base64_preview);

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) async fn tool_image_generate(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let request = parse_image_generate_input(input)?;
    let plan = image_generation_plan(&request)?;
    let (provider_used, result) = generate_planned_image(&request, plan).await?;
    let saved_paths = save_generated_images_to_workspace(&result, workspace_root);
    let image_urls = save_generated_images_to_uploads(&result);
    image_generate_response(provider_used, &result, saved_paths, image_urls)
}

struct ImageGenerateInput<'a> {
    prompt: &'a str,
    provider: &'a str,
    explicit_model: Option<&'a str>,
    model_str: &'a str,
    aspect_ratio: &'a str,
    quality: Option<&'a str>,
    count: u8,
    size: Option<&'a str>,
}

struct ImageGenerationPlan<'a> {
    provider_used: &'static str,
    backend: ImageGenerationBackend<'a>,
}

enum ImageGenerationBackend<'a> {
    Fal { model: Option<&'a str> },
    OpenAi { request: ImageGenRequest },
}

fn parse_image_generate_input(input: &serde_json::Value) -> Result<ImageGenerateInput<'_>, String> {
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;
    let explicit_model = input["model"].as_str();
    Ok(ImageGenerateInput {
        prompt,
        provider: input["provider"].as_str().unwrap_or("auto"),
        explicit_model,
        model_str: explicit_model.unwrap_or("auto"),
        aspect_ratio: input["aspect_ratio"].as_str().unwrap_or("landscape"),
        quality: input["quality"].as_str(),
        count: input["count"].as_u64().unwrap_or(1).min(4) as u8,
        size: input["size"].as_str(),
    })
}

fn image_generation_plan<'a>(
    request: &ImageGenerateInput<'a>,
) -> Result<ImageGenerationPlan<'a>, String> {
    if should_use_fal_provider(request.provider, request.explicit_model)? {
        return Ok(ImageGenerationPlan {
            provider_used: "fal",
            backend: ImageGenerationBackend::Fal {
                model: request.explicit_model.map(|_| request.model_str),
            },
        });
    }
    Ok(ImageGenerationPlan {
        provider_used: "openai",
        backend: ImageGenerationBackend::OpenAi {
            request: openai_image_request(request)?,
        },
    })
}

fn should_use_fal_provider(provider: &str, explicit_model: Option<&str>) -> Result<bool, String> {
    match provider {
        "fal" => Ok(true),
        "openai" => Ok(false),
        "auto" => Ok(explicit_model.is_some_and(is_fal_image_model_name)
            || (explicit_model.is_none() && std::env::var("FAL_KEY").is_ok())),
        other => Err(format!(
            "Unknown image provider: {other}. Use 'auto', 'fal', or 'openai'."
        )),
    }
}

fn openai_image_request(request: &ImageGenerateInput<'_>) -> Result<ImageGenRequest, String> {
    let openai_model_str = if request.model_str == "auto" {
        "gpt-image-1"
    } else {
        request.model_str
    };
    let model = resolve_openai_image_model(openai_model_str)?;
    Ok(ImageGenRequest {
        prompt: request.prompt.to_string(),
        model,
        size: request
            .size
            .map(str::to_string)
            .unwrap_or_else(|| default_openai_image_size(model, request.aspect_ratio).to_string()),
        quality: request
            .quality
            .unwrap_or_else(|| default_openai_image_quality(model))
            .to_string(),
        count: request.count,
    })
}

fn resolve_openai_image_model(model: &str) -> Result<ImageGenModel, String> {
    match model {
        "dall-e-3" | "dalle3" | "dalle-3" => Ok(ImageGenModel::DallE3),
        "dall-e-2" | "dalle2" | "dalle-2" => Ok(ImageGenModel::DallE2),
        "gpt-image-2" | "gpt_image_2" => Ok(ImageGenModel::GptImage2),
        "gpt-image-1.5" | "gpt_image_1_5" | "gpt-image-15" => Ok(ImageGenModel::GptImage15),
        "gpt-image-1" | "gpt_image_1" => Ok(ImageGenModel::GptImage1),
        "gpt-image-1-mini" | "gpt_image_1_mini" => Ok(ImageGenModel::GptImage1Mini),
        _ => Err(format!(
            "Unknown OpenAI image model: {model}. Use 'gpt-image-2', 'gpt-image-1.5', 'gpt-image-1', 'gpt-image-1-mini', 'dall-e-3', or 'dall-e-2'. For FAL models set provider='fal'."
        )),
    }
}

async fn generate_planned_image<'a>(
    request: &ImageGenerateInput<'a>,
    plan: ImageGenerationPlan<'a>,
) -> Result<(&'static str, ImageGenResult), String> {
    match plan.backend {
        ImageGenerationBackend::Fal { model } => Ok((
            plan.provider_used,
            crate::image_gen::generate_fal_image(
                request.prompt,
                model,
                Some(request.aspect_ratio),
                request.count,
                request.quality,
            )
            .await?,
        )),
        ImageGenerationBackend::OpenAi { request } => Ok((
            plan.provider_used,
            crate::image_gen::generate_image(&request).await?,
        )),
    }
}

fn save_generated_images_to_workspace(
    result: &ImageGenResult,
    workspace_root: Option<&Path>,
) -> Vec<String> {
    let Some(workspace) = workspace_root else {
        return Vec::new();
    };
    match crate::image_gen::save_images_to_workspace(result, workspace) {
        Ok(paths) => paths,
        Err(e) => {
            warn!("Failed to save images to workspace: {e}");
            Vec::new()
        }
    }
}

fn save_generated_images_to_uploads(result: &ImageGenResult) -> Vec<String> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("captain_uploads");
    let _ = std::fs::create_dir_all(&upload_dir);
    let mut image_urls = Vec::new();
    for img in &result.images {
        let file_id = uuid::Uuid::new_v4().to_string();
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64) {
            let path = upload_dir.join(&file_id);
            if std::fs::write(&path, &decoded).is_ok() {
                image_urls.push(format!("/api/uploads/{file_id}"));
            }
        }
    }
    image_urls
}

fn image_generate_response(
    provider_used: &str,
    result: &ImageGenResult,
    saved_paths: Vec<String>,
    image_urls: Vec<String>,
) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "provider": provider_used,
        "model": result.model,
        "images_generated": result.images.len(),
        "saved_to": saved_paths,
        "revised_prompt": result.revised_prompt,
        "source_urls": result.images.iter().filter_map(|img| img.url.clone()).collect::<Vec<_>>(),
        "image_urls": image_urls,
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) fn detect_image_format(data: &[u8]) -> String {
    if data.len() < 4 {
        return "unknown".to_string();
    }
    if data.starts_with(b"\x89PNG") {
        "png".to_string()
    } else if data.starts_with(b"\xFF\xD8\xFF") {
        "jpeg".to_string()
    } else if data.starts_with(b"GIF8") {
        "gif".to_string()
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp".to_string()
    } else if data.starts_with(b"BM") {
        "bmp".to_string()
    } else if data.starts_with(b"\x00\x00\x01\x00") {
        "ico".to_string()
    } else {
        "unknown".to_string()
    }
}

pub(crate) fn extract_image_dimensions(data: &[u8], format: &str) -> Option<(u32, u32)> {
    match format {
        "png" if data.len() >= 24 => Some((
            u32::from_be_bytes([data[16], data[17], data[18], data[19]]),
            u32::from_be_bytes([data[20], data[21], data[22], data[23]]),
        )),
        "gif" if data.len() >= 10 => Some((
            u16::from_le_bytes([data[6], data[7]]) as u32,
            u16::from_le_bytes([data[8], data[9]]) as u32,
        )),
        "bmp" if data.len() >= 26 => Some((
            u32::from_le_bytes([data[18], data[19], data[20], data[21]]),
            u32::from_le_bytes([data[22], data[23], data[24], data[25]]),
        )),
        "jpeg" => extract_jpeg_dimensions(data),
        _ => None,
    }
}

pub(crate) fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn extract_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if (0xC0..=0xC3).contains(&marker) && i + 9 < data.len() {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if i + 3 < data.len() {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + seg_len;
        } else {
            break;
        }
    }
    None
}

fn is_fal_image_model_name(model: &str) -> bool {
    let model = model.trim();
    model.starts_with("fal-ai/")
        || matches!(
            model,
            "fal"
                | "flux-2-klein"
                | "flux-klein"
                | "flux-2-pro"
                | "flux-pro"
                | "fal-gpt-image-1.5"
                | "nano-banana-pro"
                | "z-image-turbo"
                | "ideogram-v3"
                | "recraft-v4-pro"
                | "qwen-image"
        )
}

fn default_openai_image_size(
    model: captain_types::media::ImageGenModel,
    aspect_ratio: &str,
) -> &'static str {
    let aspect = match aspect_ratio.trim().to_ascii_lowercase().as_str() {
        "square" | "1:1" => "square",
        "portrait" | "9:16" => "portrait",
        _ => "landscape",
    };
    match model {
        captain_types::media::ImageGenModel::DallE2 => "1024x1024",
        captain_types::media::ImageGenModel::DallE3 => match aspect {
            "portrait" => "1024x1792",
            "landscape" => "1792x1024",
            _ => "1024x1024",
        },
        captain_types::media::ImageGenModel::GptImage2
        | captain_types::media::ImageGenModel::GptImage15
        | captain_types::media::ImageGenModel::GptImage1
        | captain_types::media::ImageGenModel::GptImage1Mini => match aspect {
            "portrait" => "1024x1536",
            "landscape" => "1536x1024",
            _ => "1024x1024",
        },
    }
}

fn default_openai_image_quality(model: captain_types::media::ImageGenModel) -> &'static str {
    match model {
        captain_types::media::ImageGenModel::DallE3 => "hd",
        captain_types::media::ImageGenModel::DallE2 => "standard",
        captain_types::media::ImageGenModel::GptImage2
        | captain_types::media::ImageGenModel::GptImage15
        | captain_types::media::ImageGenModel::GptImage1
        | captain_types::media::ImageGenModel::GptImage1Mini => "auto",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn image_plan_auto_routes_explicit_fal_model_to_fal() {
        let input = json!({
            "prompt": "a clean interface",
            "model": "fal-ai/flux-pro",
        });
        let request = parse_image_generate_input(&input).unwrap();
        let plan = image_generation_plan(&request).unwrap();

        assert_eq!(plan.provider_used, "fal");
        match plan.backend {
            ImageGenerationBackend::Fal { model } => assert_eq!(model, Some("fal-ai/flux-pro")),
            ImageGenerationBackend::OpenAi { .. } => panic!("expected FAL backend"),
        }
    }

    #[test]
    fn image_plan_openai_defaults_size_quality_and_clamps_count() {
        let input = json!({
            "prompt": "a useful status panel",
            "provider": "openai",
            "model": "gpt_image_1_5",
            "aspect_ratio": "portrait",
            "count": 9,
        });
        let request = parse_image_generate_input(&input).unwrap();
        let plan = image_generation_plan(&request).unwrap();

        assert_eq!(plan.provider_used, "openai");
        match plan.backend {
            ImageGenerationBackend::OpenAi { request } => {
                assert!(matches!(request.model, ImageGenModel::GptImage15));
                assert_eq!(request.size, "1024x1536");
                assert_eq!(request.quality, "auto");
                assert_eq!(request.count, 4);
            }
            ImageGenerationBackend::Fal { .. } => panic!("expected OpenAI backend"),
        }
    }

    #[test]
    fn image_plan_rejects_unknown_openai_model() {
        let input = json!({
            "prompt": "test",
            "provider": "openai",
            "model": "fal-ai/flux-pro",
        });
        let request = parse_image_generate_input(&input).unwrap();
        let err = match image_generation_plan(&request) {
            Ok(_) => panic!("expected unknown OpenAI model error"),
            Err(err) => err,
        };

        assert!(err.contains("Unknown OpenAI image model"));
        assert!(err.contains("provider='fal'"));
    }
}
