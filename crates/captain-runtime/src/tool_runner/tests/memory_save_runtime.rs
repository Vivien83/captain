use super::*;

/// Commit-A — `memory_save` tool is registered with the right schema.
#[test]
fn commit_a_memory_save_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let save = tools
        .iter()
        .find(|t| t.name == "memory_save")
        .expect("memory_save must be registered");

    let required = save.input_schema["required"]
        .as_array()
        .expect("required array");
    let required: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    for f in ["subject", "predicate", "object", "category"] {
        assert!(required.contains(&f), "missing required field '{f}'");
    }
    let cats = save.input_schema["properties"]["category"]["enum"]
        .as_array()
        .expect("category enum");
    let cats: Vec<&str> = cats.iter().filter_map(|v| v.as_str()).collect();
    for c in ["info", "skill", "error_success", "solution", "other"] {
        assert!(cats.contains(&c), "category must include '{c}'");
    }
    assert!(
        save.description.contains("PII") || save.description.contains("credentials"),
        "description should warn about PII"
    );
}

pub(super) struct MemSaveStubKernel;

#[async_trait::async_trait]
impl KernelHandle for MemSaveStubKernel {
    async fn spawn_agent(
        &self,
        _manifest: &str,
        _parent: Option<&str>,
    ) -> Result<(String, String), String> {
        Err("stub".into())
    }

    async fn send_to_agent(&self, _id: &str, _msg: &str) -> Result<String, String> {
        Err("stub".into())
    }

    fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }

    fn kill_agent(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn find_agents(&self, _q: &str) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }

    async fn task_post(
        &self,
        _t: &str,
        _d: &str,
        _a: Option<&str>,
        _c: Option<&str>,
    ) -> Result<String, String> {
        Err("stub".into())
    }

    async fn task_claim(&self, _id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(&self, _id: &str, _r: &str) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn commit_a_memory_save_rejects_unknown_category() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let input = serde_json::json!({
        "subject": "user",
        "predicate": "prefers",
        "object": "vert sapin",
        "category": "FOOBAR",
    });
    let res = tool_memory_save(&input, None, Some(&kh)).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("category"));
}

#[tokio::test]
async fn commit_a_memory_save_rejects_pii_in_object() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let input = serde_json::json!({
        "subject": "user",
        "predicate": "phone",
        "object": "Mon numéro est 06 12 34 56 78",
        "category": "info",
    });
    let res = tool_memory_save(&input, None, Some(&kh)).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.contains("PII") || err.contains("phone_fr"),
        "PII rejection should fire, got: {err}"
    );
}

#[tokio::test]
async fn commit_a_memory_save_rejects_too_long_object() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let huge: String = "a".repeat(1500);
    let input = serde_json::json!({
        "subject": "user",
        "predicate": "ranted",
        "object": huge,
        "category": "info",
    });
    let res = tool_memory_save(&input, None, Some(&kh)).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("too long"));
}

#[tokio::test]
async fn commit_a_memory_save_rejects_empty_subject() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let input = serde_json::json!({
        "subject": "   ",
        "predicate": "x",
        "object": "y",
        "category": "info",
    });
    let res = tool_memory_save(&input, None, Some(&kh)).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn memory_save_execute_tool_keeps_structured_contract() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let input = serde_json::json!({
        "subject": "host:example",
        "predicate": "runs",
        "object": "example service",
        "category": "info",
    });
    let result = execute_tool(
        "test-id",
        "memory_save",
        &input,
        Some(&kh),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert!(result.is_error);
    assert!(
        !result.content.contains("Missing 'key' parameter"),
        "memory_save must not be normalized into memory_store: {}",
        result.content
    );
    assert!(
        result.content.contains("MemPalace") || result.content.contains("memory_writes"),
        "expected memory_save path error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn origin_channel_is_scoped_to_current_tool_dispatch() {
    assert_eq!(current_origin_channel(), None);

    let seen = with_origin_channel(Some("telegram".to_string()), async {
        current_origin_channel()
    })
    .await;

    assert_eq!(seen.as_deref(), Some("telegram"));
    assert_eq!(current_origin_channel(), None);
}

#[tokio::test]
async fn memory_store_write_failure_is_not_masked_as_success() {
    let result = execute_tool(
        "test-id",
        "memory_store",
        &serde_json::json!({"key": "test:key", "value": "value"}),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert!(result.is_error);
    assert!(
        !result.content.contains("Temporarily unavailable"),
        "memory writes must fail visibly, got: {}",
        result.content
    );
}
