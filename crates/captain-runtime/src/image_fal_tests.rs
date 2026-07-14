use super::*;

#[test]
fn test_fal_payload_uses_model_native_size_shape() {
    let payload = build_fal_image_payload(
        "fal-ai/gpt-image-1.5",
        "poster",
        "portrait",
        2,
        Some("medium"),
    )
    .unwrap();
    assert_eq!(payload["image_size"], "1024x1536");
    assert_eq!(payload["num_images"], 2);
    assert_eq!(payload["quality"], "medium");

    let payload =
        build_fal_image_payload("fal-ai/nano-banana-pro", "poster", "landscape", 1, None).unwrap();
    assert_eq!(payload["aspect_ratio"], "16:9");
    assert_eq!(payload["resolution"], "1K");
}

#[test]
fn test_extract_fal_image_entries_supports_queue_response_shape() {
    let response = serde_json::json!({
        "status": "COMPLETED",
        "response": {
            "images": [
                {"url": "https://example.test/one.png", "width": 1024, "height": 1024}
            ]
        }
    });
    let entries = extract_fal_image_entries(&response).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "https://example.test/one.png");
}
