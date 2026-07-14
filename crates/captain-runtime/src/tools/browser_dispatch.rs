//! Browser tool dispatch helpers.

use crate::browser::BrowserManager;

use super::{check_taint_browser_batch, check_taint_net_fetch};

const BROWSER_UNAVAILABLE: &str =
    "Browser tools not available. Ensure Chrome/Chromium is installed.";

pub(crate) async fn dispatch_browser_tool(
    tool_name: &str,
    input: &serde_json::Value,
    browser_ctx: Option<&BrowserManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    if tool_name == "browser_batch" {
        if let Some(violation) = check_taint_browser_batch(input) {
            return Err(format!("Taint violation: {violation}"));
        }
    }
    if tool_name == "browser_navigate" {
        let url = input["url"].as_str().unwrap_or("");
        if let Some(violation) = check_taint_net_fetch(url) {
            return Err(format!("Taint violation: {violation}"));
        }
    }

    let Some(mgr) = browser_ctx else {
        return Err(BROWSER_UNAVAILABLE.to_string());
    };
    let aid = caller_agent_id.unwrap_or("default");
    match tool_name {
        "browser_batch" => crate::browser::tool_browser_batch(input, mgr, aid).await,
        "browser_navigate" => crate::browser::tool_browser_navigate(input, mgr, aid).await,
        "browser_click" => crate::browser::tool_browser_click(input, mgr, aid).await,
        "browser_type" => crate::browser::tool_browser_type(input, mgr, aid).await,
        "browser_keys" => crate::browser::tool_browser_keys(input, mgr, aid).await,
        "browser_select" => crate::browser::tool_browser_select(input, mgr, aid).await,
        "browser_hover" => crate::browser::tool_browser_hover(input, mgr, aid).await,
        "browser_screenshot" => crate::browser::tool_browser_screenshot(input, mgr, aid).await,
        "browser_read_page" => crate::browser::tool_browser_read_page(input, mgr, aid).await,
        "browser_close" => crate::browser::tool_browser_close(input, mgr, aid).await,
        "browser_scroll" => crate::browser::tool_browser_scroll(input, mgr, aid).await,
        "browser_wait" => crate::browser::tool_browser_wait(input, mgr, aid).await,
        "browser_run_js" => crate::browser::tool_browser_run_js(input, mgr, aid).await,
        "browser_back" => crate::browser::tool_browser_back(input, mgr, aid).await,
        "browser_status" => crate::browser::tool_browser_status(input, mgr, aid).await,
        "browser_network_log" => crate::browser::tool_browser_network_log(input, mgr, aid).await,
        "browser_observe" => crate::browser::tool_browser_observe(input, mgr, aid).await,
        "browser_diagnostics" => crate::browser::tool_browser_diagnostics(input, mgr, aid).await,
        other => Err(format!("Unknown browser tool: {other}")),
    }
}
