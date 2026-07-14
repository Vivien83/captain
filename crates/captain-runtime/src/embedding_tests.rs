use super::*;

#[test]
fn test_cosine_similarity_identical() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    let sim = cosine_similarity(&a, &b);
    assert!((sim - 1.0).abs() < 1e-6);
}

#[test]
fn test_cosine_similarity_orthogonal() {
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    let sim = cosine_similarity(&a, &b);
    assert!(sim.abs() < 1e-6);
}

#[test]
fn test_cosine_similarity_opposite() {
    let a = vec![1.0, 0.0];
    let b = vec![-1.0, 0.0];
    let sim = cosine_similarity(&a, &b);
    assert!((sim + 1.0).abs() < 1e-6);
}

#[test]
fn test_cosine_similarity_real_vectors() {
    let a = vec![0.1, 0.2, 0.3, 0.4];
    let b = vec![0.1, 0.2, 0.3, 0.4];
    let sim = cosine_similarity(&a, &b);
    assert!((sim - 1.0).abs() < 1e-5);

    let c = vec![0.4, 0.3, 0.2, 0.1];
    let sim2 = cosine_similarity(&a, &c);
    assert!(sim2 > 0.0 && sim2 < 1.0);
}

#[test]
fn test_cosine_similarity_empty() {
    let sim = cosine_similarity(&[], &[]);
    assert_eq!(sim, 0.0);
}

#[test]
fn test_cosine_similarity_length_mismatch() {
    let a = vec![1.0, 2.0];
    let b = vec![1.0, 2.0, 3.0];
    let sim = cosine_similarity(&a, &b);
    assert_eq!(sim, 0.0);
}

#[test]
fn test_embedding_roundtrip() {
    let embedding = vec![0.1, -0.5, 1.23456, 0.0, -1e10, 1e10];
    let bytes = embedding_to_bytes(&embedding);
    let recovered = embedding_from_bytes(&bytes);
    assert_eq!(embedding.len(), recovered.len());
    for (a, b) in embedding.iter().zip(recovered.iter()) {
        assert!((a - b).abs() < f32::EPSILON);
    }
}

#[test]
fn test_embedding_bytes_empty() {
    let bytes = embedding_to_bytes(&[]);
    assert!(bytes.is_empty());
    let recovered = embedding_from_bytes(&bytes);
    assert!(recovered.is_empty());
}

#[test]
fn test_infer_dimensions() {
    assert_eq!(infer_dimensions("text-embedding-3-small"), 1536);
    assert_eq!(infer_dimensions("all-MiniLM-L6-v2"), 384);
    assert_eq!(infer_dimensions("nomic-embed-text"), 768);
    assert_eq!(infer_dimensions("unknown-model"), 1536);
}

#[test]
fn test_create_embedding_driver_ollama() {
    let driver = create_embedding_driver("ollama", "all-MiniLM-L6-v2", "", None);
    assert!(driver.is_ok());
    assert_eq!(driver.unwrap().dimensions(), 384);
}

#[test]
fn test_create_embedding_driver_custom_url_with_v1() {
    let driver = create_embedding_driver(
        "ollama",
        "nomic-embed-text",
        "",
        Some("http://192.168.0.1:11434/v1"),
    );
    assert!(driver.is_ok());
}

#[test]
fn test_create_embedding_driver_custom_url_without_v1() {
    let driver = create_embedding_driver(
        "ollama",
        "nomic-embed-text",
        "",
        Some("http://192.168.0.1:11434"),
    );
    assert!(driver.is_ok());
}

#[test]
fn test_create_embedding_driver_custom_url_trailing_slash() {
    let driver = create_embedding_driver(
        "ollama",
        "nomic-embed-text",
        "",
        Some("http://192.168.0.1:11434/"),
    );
    assert!(driver.is_ok());
}

/// R.A.1 - provider="local" must short-circuit to the in-process
/// fastembed driver (no HTTP). Test runs only when the local-embeddings
/// feature is enabled (the default).
#[cfg(feature = "local-embeddings")]
#[test]
fn ra1_provider_local_routes_to_fastembed_no_http() {
    let driver = create_embedding_driver(
        "local",
        "all-MiniLM-L6-v2",
        "",
        // Custom URL must be ignored when provider=local; the HTTP path must
        // not be taken even if a URL is supplied.
        Some("https://should-be-ignored.example/v1"),
    );
    if crate::native_embeddings::find_library().is_none() {
        assert!(
            matches!(driver, Err(EmbeddingError::Config(_))),
            "local embeddings without ONNX Runtime should fail as a local config issue"
        );
        return;
    }
    assert!(
        driver.is_ok(),
        "local fastembed driver creation failed: {:?}",
        driver.err()
    );
    // If creation went through HTTP, this would fail later on first embed()
    // instead of returning a local driver here.
}
