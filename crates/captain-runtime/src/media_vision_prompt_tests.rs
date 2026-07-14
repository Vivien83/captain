use super::test_support::{temp_png_attachment_with_hint, VISION_TEST_LOCK};
use super::DEFAULT_VISION_PROMPT;
use crate::media_understanding::MediaEngine;
use captain_types::media::{MediaAttachment, MediaConfig, MediaSource, MediaType};

#[tokio::test]
async fn test_describe_image_url_source_rejected() {
    let _g = VISION_TEST_LOCK.lock().await;
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    }

    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);
    let attachment = MediaAttachment {
        media_type: MediaType::Image,
        mime_type: "image/png".into(),
        source: MediaSource::Url {
            url: "https://example.com/test.png".into(),
        },
        size_bytes: 1024,
        context_hint: None,
        batch_size_hint: None,
    };
    let result = engine.describe_image(&attachment).await;

    unsafe {
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    let err = result.expect_err("URL source must be rejected");
    assert!(
        err.contains("URL-based image source not supported"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_describe_image_uses_context_hint_in_prompt() {
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

    let hint = "Décris l'action principale\nFrame 3/10 d'une vidéo, t = 2.0s.";
    let (_k, attachment) = temp_png_attachment_with_hint(&[0xDE, 0xAD], Some(hint.to_string()));
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

    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    let text = body["messages"][0]["content"][1]["text"]
        .as_str()
        .expect("text part missing");

    assert!(text.contains(DEFAULT_VISION_PROMPT));
    assert!(text.contains("Décris l'action principale"));
    assert!(text.contains("Frame 3/10 d'une vidéo, t = 2.0s."));
}

#[tokio::test]
async fn test_describe_image_default_prompt_when_no_hint() {
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

    let (_k, attachment) = temp_png_attachment_with_hint(&[0xCA, 0xFE], None);
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

    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    let text = body["messages"][0]["content"][1]["text"]
        .as_str()
        .expect("text part missing");
    assert_eq!(text, DEFAULT_VISION_PROMPT);
}

#[tokio::test]
async fn test_describe_image_blank_hint_falls_back_to_default() {
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

    let (_k, attachment) =
        temp_png_attachment_with_hint(&[0xFE, 0xED], Some("   \n\t  ".to_string()));
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

    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    let text = body["messages"][0]["content"][1]["text"]
        .as_str()
        .expect("text part missing");
    assert_eq!(text, DEFAULT_VISION_PROMPT);
}
