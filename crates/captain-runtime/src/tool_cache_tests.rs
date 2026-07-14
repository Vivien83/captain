use super::*;
use crate::cache::{MokaCache, MokaCacheConfig};

fn fresh_cache() -> Arc<MokaCache> {
    Arc::new(MokaCache::new(MokaCacheConfig {
        max_capacity: 64,
        ttl: Duration::from_secs(600),
        tti: Duration::from_secs(600),
    }))
}

#[tokio::test]
async fn readers_default_policies() {
    let trc = ToolResultCache::new(fresh_cache());
    assert!(trc.is_cacheable("file_read"));
    assert!(trc.is_cacheable("document_extract"));
    assert!(trc.is_cacheable("web_search"));
    assert!(trc.is_cacheable("web_fetch"));
    assert!(trc.is_cacheable("knowledge_query"));
    assert!(trc.is_cacheable("memory_recall"));
}

#[tokio::test]
async fn writers_are_not_cacheable() {
    let trc = ToolResultCache::new(fresh_cache());
    assert!(!trc.is_cacheable("file_write"));
    assert!(!trc.is_cacheable("web_download"));
    assert!(!trc.is_cacheable("memory_store"));
    assert!(!trc.is_cacheable("shell_exec"));
    assert!(!trc.is_cacheable("browser_batch"));
    assert!(!trc.is_cacheable("browser_keys"));
    assert!(!trc.is_cacheable("browser_select"));
    assert!(!trc.is_cacheable("browser_hover"));
    assert!(!trc.is_cacheable("browser_select"));
    assert!(!trc.is_cacheable("browser_hover"));
}

#[tokio::test]
async fn unknown_tool_is_not_cacheable() {
    let trc = ToolResultCache::new(fresh_cache());
    assert!(!trc.is_cacheable("totally_made_up"));
}

#[tokio::test]
async fn store_then_get_round_trips() {
    let trc = ToolResultCache::new(fresh_cache());
    let args = serde_json::json!({ "path": "/tmp/foo" });
    let out = CachedToolResult {
        output: "hello".into(),
        is_error: false,
    };
    trc.store("file_read", &args, &out).await.unwrap();

    let got = trc.get("file_read", &args).await.unwrap();
    assert_eq!(got.as_ref(), Some(&out));
}

#[tokio::test]
async fn error_results_are_not_stored() {
    let trc = ToolResultCache::new(fresh_cache());
    let args = serde_json::json!({ "path": "/tmp/foo" });
    let err = CachedToolResult {
        output: "nope".into(),
        is_error: true,
    };
    trc.store("file_read", &args, &err).await.unwrap();
    assert_eq!(trc.get("file_read", &args).await.unwrap(), None);
}

#[tokio::test]
async fn different_args_produce_different_keys() {
    let trc = ToolResultCache::new(fresh_cache());
    let a = serde_json::json!({ "path": "/tmp/a" });
    let b = serde_json::json!({ "path": "/tmp/b" });
    assert_ne!(trc.key_for("file_read", &a), trc.key_for("file_read", &b));
}

#[tokio::test]
async fn file_write_invalidates_file_read_same_path() {
    let trc = ToolResultCache::new(fresh_cache());
    let args = serde_json::json!({ "path": "/tmp/foo" });

    trc.store(
        "file_read",
        &args,
        &CachedToolResult {
            output: "old".into(),
            is_error: false,
        },
    )
    .await
    .unwrap();
    assert!(trc.get("file_read", &args).await.unwrap().is_some());

    trc.invalidate_after_write("file_write", &args)
        .await
        .unwrap();
    assert!(trc.get("file_read", &args).await.unwrap().is_none());
}

#[tokio::test]
async fn file_write_invalidates_file_read_even_when_writer_args_differ() {
    let trc = ToolResultCache::new(fresh_cache());
    let read_args = serde_json::json!({ "path": "/tmp/foo" });
    let write_args = serde_json::json!({ "path": "/tmp/foo", "content": "new" });

    trc.store(
        "file_read",
        &read_args,
        &CachedToolResult {
            output: "old".into(),
            is_error: false,
        },
    )
    .await
    .unwrap();
    assert!(trc.get("file_read", &read_args).await.unwrap().is_some());

    trc.invalidate_after_write("file_write", &write_args)
        .await
        .unwrap();
    assert!(trc.get("file_read", &read_args).await.unwrap().is_none());
}

#[tokio::test]
async fn writer_invalidation_only_bumps_configured_reader_namespaces() {
    let trc = ToolResultCache::new(fresh_cache());
    let file_a = serde_json::json!({ "path": "/tmp/a" });
    let file_b = serde_json::json!({ "path": "/tmp/b" });
    let web = serde_json::json!({ "url": "https://example.com" });
    let result = CachedToolResult {
        output: "cached".into(),
        is_error: false,
    };

    trc.store("file_read", &file_a, &result).await.unwrap();
    trc.store("file_read", &file_b, &result).await.unwrap();
    trc.store("document_extract", &file_a, &result)
        .await
        .unwrap();
    trc.store("web_fetch", &web, &result).await.unwrap();

    trc.invalidate_after_write("file_write", &serde_json::Value::Null)
        .await
        .unwrap();

    assert!(trc.get("file_read", &file_a).await.unwrap().is_none());
    assert!(trc.get("file_read", &file_b).await.unwrap().is_none());
    assert!(trc
        .get("document_extract", &file_a)
        .await
        .unwrap()
        .is_none());
    assert_eq!(trc.get("web_fetch", &web).await.unwrap(), Some(result));
}

#[tokio::test]
async fn memory_store_invalidates_memory_recall_and_knowledge_query() {
    let trc = ToolResultCache::new(fresh_cache());
    let scope = serde_json::Value::Null;

    trc.store(
        "memory_recall",
        &scope,
        &CachedToolResult {
            output: "mem".into(),
            is_error: false,
        },
    )
    .await
    .unwrap();
    trc.store(
        "knowledge_query",
        &scope,
        &CachedToolResult {
            output: "graph".into(),
            is_error: false,
        },
    )
    .await
    .unwrap();
    assert!(trc.get("memory_recall", &scope).await.unwrap().is_some());
    assert!(trc.get("knowledge_query", &scope).await.unwrap().is_some());

    trc.invalidate_after_write("memory_store", &scope)
        .await
        .unwrap();

    assert!(trc.get("memory_recall", &scope).await.unwrap().is_none());
    assert!(trc.get("knowledge_query", &scope).await.unwrap().is_none());
}

#[tokio::test]
async fn memory_forget_invalidates_memory_recall_and_knowledge_query() {
    let trc = ToolResultCache::new(fresh_cache());
    let scope = serde_json::Value::Null;

    trc.store(
        "memory_recall",
        &scope,
        &CachedToolResult {
            output: "stale memory".into(),
            is_error: false,
        },
    )
    .await
    .unwrap();
    trc.store(
        "knowledge_query",
        &scope,
        &CachedToolResult {
            output: "stale graph".into(),
            is_error: false,
        },
    )
    .await
    .unwrap();

    trc.invalidate_after_write("memory_forget", &scope)
        .await
        .unwrap();

    assert!(trc.get("memory_recall", &scope).await.unwrap().is_none());
    assert!(trc.get("knowledge_query", &scope).await.unwrap().is_none());
}
