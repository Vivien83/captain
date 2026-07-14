use super::test_support::{temp_png_attachment, temp_png_attachment_full, VISION_TEST_LOCK};
use super::DEFAULT_VISION_PROMPT;
use crate::media_understanding::MediaEngine;
use captain_types::media::MediaConfig;

#[tokio::test]
async fn test_describe_image_anthropic_wiremock_success() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": [{"type": "text", "text": "A black square on white."}],
            "stop_reason": "end_turn"
        })))
        .expect(1)
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("ANTHROPIC_API_BASE", server.uri());
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    }

    let png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let (_keepalive, attachment) = temp_png_attachment(&png_bytes);
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let result = engine.describe_image(&attachment).await;

    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    let mu = result.expect("describe_image should succeed");
    assert_eq!(mu.description, "A black square on white.");
    assert_eq!(mu.provider, "anthropic");
    assert_eq!(mu.model, "claude-sonnet-4-6");
}

#[tokio::test]
async fn test_describe_image_anthropic_propagates_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"unauthorized"}"#))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("ANTHROPIC_API_BASE", server.uri());
        std::env::set_var("ANTHROPIC_API_KEY", "bad-key");
    }

    let (_k, attachment) = temp_png_attachment(&[0x89, 0x50, 0x4E, 0x47]);
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let result = engine.describe_image(&attachment).await;

    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    let err = result.expect_err("must return Err on 401");
    assert!(
        err.contains("Vision API error") && err.contains("401"),
        "expected 'Vision API error (401 ...)', got: {err}"
    );
}

#[tokio::test]
async fn test_describe_image_anthropic_request_shape() {
    use base64::Engine;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": [{"type": "text", "text": "ok"}]
        })))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("ANTHROPIC_API_BASE", server.uri());
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    }

    let png_bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let (_k, attachment) = temp_png_attachment(&png_bytes);
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    engine
        .describe_image(&attachment)
        .await
        .expect("must succeed");

    let received: Vec<Request> = server.received_requests().await.unwrap();

    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    assert_eq!(received.len(), 1, "exactly one POST expected");
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 1024);
    let content = &body["messages"][0]["content"];
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["type"], "base64");
    assert_eq!(content[0]["source"]["media_type"], "image/png");
    let expected_b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    assert_eq!(content[0]["source"]["data"], expected_b64);
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], DEFAULT_VISION_PROMPT);
}

#[tokio::test]
async fn test_describe_image_anthropic_picks_haiku_on_small_batch() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": [{"type": "text", "text": "ok"}]
        })))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("ANTHROPIC_API_BASE", server.uri());
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    }

    let (_k, attachment) = temp_png_attachment_full(&[0xDE, 0xAD], None, Some(1));
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let mu = engine
        .describe_image(&attachment)
        .await
        .expect("must succeed");

    let received: Vec<Request> = server.received_requests().await.unwrap();
    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    assert_eq!(mu.model, "claude-haiku-4-5-20251001");
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["model"], "claude-haiku-4-5-20251001");
}

#[tokio::test]
async fn test_describe_image_anthropic_keeps_sonnet_on_large_batch() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": [{"type": "text", "text": "ok"}]
        })))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("ANTHROPIC_API_BASE", server.uri());
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    }

    let (_k, attachment) = temp_png_attachment_full(&[0xDE, 0xAD], None, Some(30));
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let mu = engine
        .describe_image(&attachment)
        .await
        .expect("must succeed");

    let received: Vec<Request> = server.received_requests().await.unwrap();
    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    assert_eq!(mu.model, "claude-sonnet-4-6");
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["model"], "claude-sonnet-4-6");
}
