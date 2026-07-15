use base64::Engine;
use captain_types::message::{validate_image, ContentBlock};

const MAX_VISUAL_PROMPT_CHARS: usize = 2_000;
const PNG_MIME_TYPE: &str = "image/png";

#[derive(Debug)]
pub(super) struct ScreenshotPayload {
    pub(super) metadata: serde_json::Value,
    pub(super) transient_content: Vec<ContentBlock>,
}

pub(super) fn optional_visual_prompt(input: &serde_json::Value) -> Result<Option<String>, String> {
    let Some(prompt) = input.get("prompt") else {
        return Ok(None);
    };
    if prompt.is_null() {
        return Ok(None);
    }
    let prompt = prompt
        .as_str()
        .ok_or("'prompt' must be a string when provided")?
        .trim();
    if prompt.is_empty() {
        return Err("'prompt' must be a non-empty string when provided".to_string());
    }
    if prompt.chars().count() > MAX_VISUAL_PROMPT_CHARS {
        return Err(format!(
            "'prompt' accepts at most {MAX_VISUAL_PROMPT_CHARS} characters"
        ));
    }
    Ok(Some(prompt.to_string()))
}

pub(super) fn screenshot_payload(
    data: &serde_json::Value,
    prompt: Option<&str>,
) -> Result<ScreenshotPayload, String> {
    let image_base64 = data["image_base64"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("Screenshot capture returned no PNG data")?;
    validate_image(PNG_MIME_TYPE, image_base64)
        .map_err(|error| format!("Screenshot cannot be sent to the active model: {error}"))?;
    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(image_base64)
        .map_err(|error| format!("Screenshot PNG data is invalid: {error}"))?;
    if !image_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Screenshot capture did not return a valid PNG image".to_string());
    }

    let image_urls = save_screenshot_upload(&image_bytes)?;
    let (visual_analysis, transient_content) = match prompt {
        Some(prompt) => (
            serde_json::json!({
                "status": "attached_to_active_model",
                "verified": false,
                "analysis_pending": true,
                "transport": "native_multimodal",
                "prompt": prompt,
                "message": "The screenshot pixels are attached to this tool result for the current conversation model. Analyze the image directly and answer the prompt; do not infer visual facts from DOM or metadata alone.",
            }),
            vec![ContentBlock::Image {
                media_type: PNG_MIME_TYPE.to_string(),
                data: image_base64.to_string(),
            }],
        ),
        None => (
            serde_json::json!({
                "status": "not_requested",
                "verified": false,
                "analysis_pending": false,
                "message": "Screenshot captured for sharing only. Its pixels were not sent to the conversation model. Do not make visual, layout, polish, or absence claims from capture metadata alone; capture again with a non-empty prompt when visual analysis is required.",
            }),
            Vec::new(),
        ),
    };

    Ok(ScreenshotPayload {
        metadata: serde_json::json!({
            "screenshot": true,
            "url": data["url"].as_str().unwrap_or(""),
            "image_urls": image_urls,
            "visual_analysis": visual_analysis,
        }),
        transient_content,
    })
}

fn save_screenshot_upload(image_bytes: &[u8]) -> Result<Vec<String>, String> {
    let upload_dir = std::env::temp_dir().join("captain_uploads");
    std::fs::create_dir_all(&upload_dir)
        .map_err(|error| format!("Screenshot upload directory could not be created: {error}"))?;
    let file_id = uuid::Uuid::new_v4().to_string();
    std::fs::write(upload_dir.join(&file_id), image_bytes)
        .map_err(|error| format!("Screenshot upload could not be saved: {error}"))?;
    Ok(vec![format!("/api/uploads/{file_id}")])
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_PNG: &[u8] = b"\x89PNG\r\n\x1a\nminimal-test-payload";

    fn screenshot_data() -> serde_json::Value {
        serde_json::json!({
            "url": "https://example.com",
            "image_base64": base64::engine::general_purpose::STANDARD.encode(MINIMAL_PNG),
        })
    }

    fn remove_upload(payload: &ScreenshotPayload) {
        let upload_url = payload.metadata["image_urls"][0].as_str().unwrap();
        let file_id = upload_url.strip_prefix("/api/uploads/").unwrap();
        let _ = std::fs::remove_file(std::env::temp_dir().join("captain_uploads").join(file_id));
    }

    #[test]
    fn visual_prompt_is_optional_but_never_blank_or_unbounded() {
        assert_eq!(
            optional_visual_prompt(&serde_json::json!({})).unwrap(),
            None
        );
        assert_eq!(
            optional_visual_prompt(&serde_json::json!({"prompt": "  Check layout  "}))
                .unwrap()
                .as_deref(),
            Some("Check layout")
        );
        assert!(optional_visual_prompt(&serde_json::json!({"prompt": "  "})).is_err());
        assert!(optional_visual_prompt(
            &serde_json::json!({"prompt": "x".repeat(MAX_VISUAL_PROMPT_CHARS + 1)})
        )
        .is_err());
    }

    #[test]
    fn screenshot_without_prompt_is_capture_only() {
        let payload = screenshot_payload(&screenshot_data(), None).unwrap();

        assert_eq!(
            payload.metadata["visual_analysis"]["status"],
            "not_requested"
        );
        assert_eq!(payload.metadata["visual_analysis"]["verified"], false);
        assert!(payload.transient_content.is_empty());
        remove_upload(&payload);
    }

    #[test]
    fn requested_analysis_attaches_pixels_to_the_active_model() {
        let payload =
            screenshot_payload(&screenshot_data(), Some("Check visible overlap")).unwrap();

        assert_eq!(
            payload.metadata["visual_analysis"]["status"],
            "attached_to_active_model"
        );
        assert_eq!(payload.metadata["visual_analysis"]["verified"], false);
        assert_eq!(
            payload.metadata["visual_analysis"]["analysis_pending"],
            true
        );
        assert_eq!(payload.transient_content.len(), 1);
        assert!(matches!(
            &payload.transient_content[0],
            ContentBlock::Image { media_type, data }
                if media_type == PNG_MIME_TYPE && !data.is_empty()
        ));
        remove_upload(&payload);
    }

    #[test]
    fn malformed_screenshot_is_rejected_before_model_injection() {
        let invalid = serde_json::json!({
            "image_base64": base64::engine::general_purpose::STANDARD.encode(b"not-a-png")
        });

        let error = screenshot_payload(&invalid, Some("Inspect")).unwrap_err();

        assert!(error.contains("valid PNG"));
    }
}
