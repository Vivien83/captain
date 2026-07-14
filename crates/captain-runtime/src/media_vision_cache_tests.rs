use super::test_support::{temp_png_attachment, temp_png_attachment_with_hint, VISION_TEST_LOCK};
use crate::media_understanding::MediaEngine;
use captain_types::media::MediaConfig;

#[tokio::test]
async fn test_describe_image_cache_hits_skip_provider() {
    use std::time::Instant;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let _g = VISION_TEST_LOCK.lock().await;
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": [{"type": "text", "text": "Une scène urbaine au crépuscule."}]
        })))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("ANTHROPIC_API_BASE", server.uri());
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    }

    let png_bytes = vec![0xCA, 0xFE, 0xBA, 0xBE];
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);

    let (_k1, att1) = temp_png_attachment(&png_bytes);
    let mu1 = engine.describe_image(&att1).await.expect("call 1 ok");
    let count_after_1 = server.received_requests().await.unwrap().len();
    assert_eq!(count_after_1, 1, "first call must hit the provider");

    let (_k2, att2) = temp_png_attachment(&png_bytes);
    let t0 = Instant::now();
    let mu2 = engine.describe_image(&att2).await.expect("call 2 ok");
    let elapsed = t0.elapsed();
    let count_after_2 = server.received_requests().await.unwrap().len();

    assert_eq!(
        count_after_2, 1,
        "second identical call must be served from cache"
    );
    assert_eq!(mu2.description, mu1.description);
    assert_eq!(mu2.model, mu1.model);
    assert_eq!(mu2.provider, mu1.provider);
    assert!(
        elapsed.as_millis() < 50,
        "cache hit latency {}ms is suspiciously high",
        elapsed.as_millis()
    );

    let (_k3, att3) = temp_png_attachment_with_hint(&png_bytes, Some("Différente question".into()));
    engine.describe_image(&att3).await.expect("call 3 ok");
    let count_after_3 = server.received_requests().await.unwrap().len();
    assert_eq!(
        count_after_3, 2,
        "different hint must miss the cache and hit the provider"
    );

    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }
}

#[tokio::test]
async fn test_describe_image_cache_invalidates_on_mime_change() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    let bytes = vec![0xAA, 0xBB];
    let config = MediaConfig {
        image_provider: Some("anthropic".into()),
        ..Default::default()
    };
    let engine = MediaEngine::new(config);

    let (_k1, mut att) = temp_png_attachment(&bytes);
    engine.describe_image(&att).await.expect("png ok");

    att.mime_type = "image/jpeg".into();
    engine.describe_image(&att).await.expect("jpeg ok");

    let count = server.received_requests().await.unwrap().len();
    unsafe {
        std::env::remove_var("ANTHROPIC_API_BASE");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }
    assert_eq!(count, 2, "mime change must miss the cache");
}
