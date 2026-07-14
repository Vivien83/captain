//! Web dispatch with existing early-block behavior preserved.

use std::path::Path;
use std::sync::Arc;

use captain_types::tool::ToolResult;

use crate::kernel_handle::KernelHandle;
use crate::web_search::WebToolsContext;

use super::{
    check_taint_net_fetch, ensure_no_secret_literal, render_error_with_suggestion,
    tool_web_download, tool_web_fetch_legacy, tool_web_research_batch, tool_web_search_legacy,
};

pub(crate) enum WebDispatchOutcome {
    Blocked(ToolResult),
    Result(Result<String, String>),
}

pub(crate) async fn dispatch_web_tool(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    web_ctx: Option<&WebToolsContext>,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> WebDispatchOutcome {
    match tool_name {
        "web_research_batch" => {
            WebDispatchOutcome::Result(tool_web_research_batch(input, web_ctx).await)
        }
        "web_download" => WebDispatchOutcome::Result(
            tool_web_download(input, workspace_root, kernel, caller_agent_id).await,
        ),
        "web_fetch" => dispatch_web_fetch(tool_use_id, input, web_ctx).await,
        "web_search" => WebDispatchOutcome::Result(dispatch_web_search(input, web_ctx).await),
        other => WebDispatchOutcome::Result(Err(format!("Unknown web tool: {other}"))),
    }
}

async fn dispatch_web_fetch(
    tool_use_id: &str,
    input: &serde_json::Value,
    web_ctx: Option<&WebToolsContext>,
) -> WebDispatchOutcome {
    let url = input["url"].as_str().unwrap_or("");
    for (field, text) in web_fetch_secret_fields(input, url) {
        if let Err(reason) = ensure_no_secret_literal("web_fetch", field, &text) {
            return WebDispatchOutcome::Blocked(blocked_web_fetch(tool_use_id, &reason));
        }
    }
    if let Some(violation) = check_taint_net_fetch(url) {
        return WebDispatchOutcome::Blocked(blocked_web_fetch(
            tool_use_id,
            &format!("Taint violation: {violation}"),
        ));
    }

    let method = input["method"].as_str().unwrap_or("GET");
    let headers = input.get("headers").and_then(|v| v.as_object());
    let body = input["body"].as_str();
    let result = if let Some(ctx) = web_ctx {
        ctx.fetch
            .fetch_with_options(url, method, headers, body)
            .await
    } else {
        tool_web_fetch_legacy(input).await
    };
    WebDispatchOutcome::Result(result)
}

async fn dispatch_web_search(
    input: &serde_json::Value,
    web_ctx: Option<&WebToolsContext>,
) -> Result<String, String> {
    if let Some(ctx) = web_ctx {
        let query = input["query"].as_str().unwrap_or("");
        let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;
        ctx.search.search(query, max_results).await
    } else {
        tool_web_search_legacy(input).await
    }
}

fn web_fetch_secret_fields(input: &serde_json::Value, url: &str) -> Vec<(&'static str, String)> {
    let mut fields = vec![("url", url.to_string())];
    if let Some(body) = input["body"].as_str() {
        fields.push(("body", body.to_string()));
    }
    if let Some(headers) = input.get("headers") {
        fields.push(("headers", headers.to_string()));
    }
    fields
}

fn blocked_web_fetch(tool_use_id: &str, reason: &str) -> ToolResult {
    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content: render_error_with_suggestion(
            "web_fetch",
            reason,
            &crate::retry_transformer::RetryTransform::None,
        ),
        is_error: true,
    }
}
