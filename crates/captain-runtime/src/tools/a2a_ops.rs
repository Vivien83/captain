use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) async fn tool_a2a_discover(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;

    if crate::web_fetch::check_ssrf(url).is_err() {
        return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
    }

    let client = crate::a2a::A2aClient::new();
    let card = client.discover(url).await?;
    serde_json::to_string_pretty(&card).map_err(|e| format!("Serialization error: {e}"))
}

pub(crate) async fn tool_a2a_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    let url = if let Some(url) = input["agent_url"].as_str() {
        if crate::web_fetch::check_ssrf(url).is_err() {
            return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
        }
        url.to_string()
    } else if let Some(name) = input["agent_name"].as_str() {
        kh.get_a2a_agent_url(name)
            .ok_or_else(|| format!("No known A2A agent with name '{name}'. Use a2a_discover first or provide agent_url directly."))?
    } else {
        return Err("Missing 'agent_url' or 'agent_name' parameter".to_string());
    };

    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let task = client.send_task(&url, message, session_id).await?;
    serde_json::to_string_pretty(&task).map_err(|e| format!("Serialization error: {e}"))
}
