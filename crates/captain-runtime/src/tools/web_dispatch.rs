//! Web dispatch with existing early-block behavior preserved.

use std::path::Path;
use std::sync::Arc;

use captain_types::tool::ToolResult;

use crate::kernel_handle::KernelHandle;
use crate::web_search::WebToolsContext;

use super::{
    check_url_content_guard, ensure_no_secret_literal, render_error_with_suggestion,
    tool_web_citation_audit, tool_web_download, tool_web_research_batch,
};

const WEB_CONTEXT_UNAVAILABLE: &str =
    "Protected web context unavailable; request refused. Restart Captain and retry.";

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
        "web_citation_audit" => WebDispatchOutcome::Result(match web_ctx {
            Some(ctx) => tool_web_citation_audit(input, ctx).await,
            None => Err(WEB_CONTEXT_UNAVAILABLE.to_string()),
        }),
        "web_research_batch" => WebDispatchOutcome::Result(match web_ctx {
            Some(ctx) => tool_web_research_batch(input, ctx).await,
            None => Err(WEB_CONTEXT_UNAVAILABLE.to_string()),
        }),
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
    if let Some(violation) = check_url_content_guard(url) {
        return WebDispatchOutcome::Blocked(blocked_web_fetch(tool_use_id, &violation));
    }

    let method = input["method"].as_str().unwrap_or("GET");
    let headers = input.get("headers").and_then(|v| v.as_object());
    let body = input["body"].as_str();
    let Some(ctx) = web_ctx else {
        return WebDispatchOutcome::Result(Err(WEB_CONTEXT_UNAVAILABLE.to_string()));
    };
    let result = ctx
        .fetch
        .fetch_with_options(url, method, headers, body)
        .await;
    WebDispatchOutcome::Result(result)
}

async fn dispatch_web_search(
    input: &serde_json::Value,
    web_ctx: Option<&WebToolsContext>,
) -> Result<String, String> {
    let ctx = web_ctx.ok_or_else(|| WEB_CONTEXT_UNAVAILABLE.to_string())?;
    let query = input["query"].as_str().unwrap_or("");
    let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;
    ctx.search.search(query, max_results).await
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
        transient_content: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn protected_web_tools_fail_closed_without_context() {
        let cases = [
            (
                "web_citation_audit",
                serde_json::json!({
                    "draft": "A sourced statement.[Example](https://example.com)",
                    "sources": [{"url": "https://example.com", "quotes": []}]
                }),
            ),
            (
                "web_fetch",
                serde_json::json!({"url": "https://example.com"}),
            ),
            ("web_search", serde_json::json!({"query": "captain agent"})),
            (
                "web_research_batch",
                serde_json::json!({"query": "captain agent"}),
            ),
        ];

        for (tool_name, input) in cases {
            let outcome =
                dispatch_web_tool("tool-use", tool_name, &input, None, None, None, None).await;
            match outcome {
                WebDispatchOutcome::Result(Err(error)) => {
                    assert_eq!(error, WEB_CONTEXT_UNAVAILABLE, "{tool_name}");
                }
                WebDispatchOutcome::Blocked(result) => {
                    panic!("{tool_name} was blocked before its context check: {result:?}");
                }
                WebDispatchOutcome::Result(Ok(output)) => {
                    panic!("{tool_name} unexpectedly ran without a context: {output}");
                }
            }
        }
    }
}
