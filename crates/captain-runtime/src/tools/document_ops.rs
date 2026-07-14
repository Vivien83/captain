use crate::kernel_handle::KernelHandle;
use crate::tools::tool_channel_send;
use std::path::Path;
use std::sync::Arc;

pub(crate) async fn tool_document_pipeline(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let document_input = input.get("document").unwrap_or(input).clone();
    let created = crate::document_tools::create_document(&document_input, workspace_root).await?;
    let created_json: serde_json::Value =
        serde_json::from_str(&created).unwrap_or_else(|_| serde_json::json!({ "raw": created }));
    let mut out = serde_json::json!({
        "success": true,
        "tool": "document_pipeline",
        "document": created_json,
    });

    if let Some(send) = input.get("send").filter(|v| v.is_object()) {
        let path = out["document"]["path"]
            .as_str()
            .ok_or("document_pipeline could not determine created document path")?;
        let mut send_input = send.clone();
        if let Some(obj) = send_input.as_object_mut() {
            obj.insert(
                "file_path".to_string(),
                serde_json::Value::String(path.to_string()),
            );
            if !obj.contains_key("message") {
                let title = document_input["title"]
                    .as_str()
                    .unwrap_or("Document Captain");
                obj.insert(
                    "message".to_string(),
                    serde_json::Value::String(format!("Document généré : {title}")),
                );
            }
        }
        out["delivery"] =
            match tool_channel_send(&send_input, kernel, workspace_root, caller_agent_id).await {
                Ok(delivery) => serde_json::json!({ "success": true, "result": delivery }),
                Err(error) => serde_json::json!({ "success": false, "error": error }),
            };
    }

    serde_json::to_string_pretty(&out).map_err(|e| format!("Serialize error: {e}"))
}
