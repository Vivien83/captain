use super::test_support::{temp_png_attachment, VISION_TEST_LOCK};
use super::DEFAULT_VISION_PROMPT;
use crate::media_understanding::MediaEngine;
use captain_types::media::MediaConfig;

#[tokio::test]
async fn test_describe_image_openai_wiremock_success() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer test-openai-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": "A red circle."}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("OPENAI_API_BASE", server.uri());
        std::env::set_var("OPENAI_API_KEY", "test-openai-key");
    }

    let png_bytes = vec![0xCA, 0xFE, 0xBA, 0xBE];
    let (_k, attachment) = temp_png_attachment(&png_bytes);
    let config = MediaConfig {
        image_provider: Some("openai".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let result = engine.describe_image(&attachment).await;

    let received: Vec<Request> = server.received_requests().await.unwrap();

    unsafe {
        std::env::remove_var("OPENAI_API_BASE");
        std::env::remove_var("OPENAI_API_KEY");
    }

    let mu = result.expect("describe_image must succeed");
    assert_eq!(mu.description, "A red circle.");
    assert_eq!(mu.provider, "openai");
    assert_eq!(mu.model, "gpt-4o");

    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["max_tokens"], 1024);
    let content = &body["messages"][0]["content"];
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], DEFAULT_VISION_PROMPT);
    assert_eq!(content[1]["type"], "image_url");
    let url = content[1]["image_url"]["url"].as_str().unwrap();
    assert!(
        url.starts_with("data:image/png;base64,"),
        "expected data URL, got {url}"
    );
}

#[tokio::test]
async fn test_describe_image_gemini_wiremock_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "A blue triangle."}]
                }
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    unsafe {
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::set_var("GEMINI_API_BASE", server.uri());
        std::env::set_var("GEMINI_API_KEY", "test-gemini-key");
    }

    let png_bytes = vec![0xFE, 0xED, 0xFA, 0xCE];
    let (_k, attachment) = temp_png_attachment(&png_bytes);
    let config = MediaConfig {
        image_provider: Some("gemini".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let result = engine.describe_image(&attachment).await;

    let received: Vec<Request> = server.received_requests().await.unwrap();

    unsafe {
        std::env::remove_var("GEMINI_API_BASE");
        std::env::remove_var("GEMINI_API_KEY");
    }

    let mu = result.expect("describe_image must succeed");
    assert_eq!(mu.description, "A blue triangle.");
    assert_eq!(mu.provider, "gemini");
    assert_eq!(mu.model, "gemini-2.5-flash");

    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    let parts = &body["contents"][0]["parts"];
    assert_eq!(parts[0]["inline_data"]["mime_type"], "image/png");
    assert!(parts[0]["inline_data"]["data"].is_string());
    assert_eq!(parts[1]["text"], DEFAULT_VISION_PROMPT);

    let url = received[0].url.as_str();
    assert!(url.contains("key=test-gemini-key"), "url was: {url}");
}
