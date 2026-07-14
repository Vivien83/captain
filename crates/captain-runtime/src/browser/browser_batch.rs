use super::{BrowserCommand, DEFAULT_OBSERVE_ELEMENTS, MAX_NETWORK_EVENTS, MAX_OBSERVE_ELEMENTS};

#[derive(Debug)]
pub(super) enum BrowserBatchOp {
    Command(BrowserCommand),
    Status,
    NetworkLog { limit: usize, clear: bool },
    Diagnostics { limit: usize, clear: bool },
    Close,
}

pub(super) fn parse_browser_batch_op(
    step: &serde_json::Value,
) -> Result<(String, BrowserBatchOp), String> {
    let action = normalized_batch_action(step)?;
    let op = match action.as_str() {
        "navigate" | "browser_navigate" => parse_navigate_batch_op(step)?,
        "click" | "browser_click" => parse_click_batch_op(step)?,
        "type" | "browser_type" => parse_type_batch_op(step)?,
        "keys" | "press" | "browser_keys" => parse_keys_batch_op(step)?,
        "select" | "browser_select" => parse_select_batch_op(step)?,
        "hover" | "browser_hover" => parse_hover_batch_op(step)?,
        "scroll" | "browser_scroll" => parse_scroll_batch_op(step),
        "wait" | "browser_wait" => parse_wait_batch_op(step)?,
        "run_js" | "js" | "browser_run_js" => parse_run_js_batch_op(step)?,
        "back" | "browser_back" => BrowserBatchOp::Command(BrowserCommand::Back),
        "read_page" | "read" | "browser_read_page" => {
            BrowserBatchOp::Command(BrowserCommand::ReadPage)
        }
        "screenshot" | "browser_screenshot" => BrowserBatchOp::Command(BrowserCommand::Screenshot),
        "observe" | "browser_observe" => parse_observe_batch_op(step),
        "status" | "browser_status" => BrowserBatchOp::Status,
        "network_log" | "browser_network_log" => parse_network_log_batch_op(step),
        "diagnostics" | "browser_diagnostics" => parse_diagnostics_batch_op(step),
        "close" | "browser_close" => BrowserBatchOp::Close,
        other => return Err(unknown_browser_batch_action(other)),
    };
    Ok((action, op))
}

fn normalized_batch_action(step: &serde_json::Value) -> Result<String, String> {
    Ok(step["action"]
        .as_str()
        .ok_or("Each batch step requires an 'action' string")?
        .trim()
        .to_ascii_lowercase())
}

fn required_batch_string(
    step: &serde_json::Value,
    key: &str,
    error: &'static str,
) -> Result<String, String> {
    step[key]
        .as_str()
        .map(str::to_string)
        .ok_or(error.to_string())
}

fn required_batch_string_either(
    step: &serde_json::Value,
    first: &str,
    second: &str,
    error: &'static str,
) -> Result<String, String> {
    step[first]
        .as_str()
        .or_else(|| step[second].as_str())
        .map(str::to_string)
        .ok_or(error.to_string())
}

fn parse_navigate_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    let url = required_batch_string(step, "url", "navigate requires 'url'")?;
    crate::web_fetch::check_ssrf(&url)?;
    Ok(BrowserBatchOp::Command(BrowserCommand::Navigate { url }))
}

fn parse_click_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::Click {
        selector: required_batch_string(step, "selector", "click requires 'selector'")?,
    }))
}

fn parse_type_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::Type {
        selector: required_batch_string(step, "selector", "type requires 'selector'")?,
        text: required_batch_string(step, "text", "type requires 'text'")?,
    }))
}

fn parse_keys_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::Keys {
        keys: required_batch_string_either(step, "keys", "text", "keys requires 'keys'")?,
    }))
}

fn parse_select_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::Select {
        selector: required_batch_string(step, "selector", "select requires 'selector'")?,
        value: required_batch_string_either(step, "value", "option", "select requires 'value'")?,
    }))
}

fn parse_hover_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::Hover {
        selector: required_batch_string(step, "selector", "hover requires 'selector'")?,
    }))
}

fn parse_scroll_batch_op(step: &serde_json::Value) -> BrowserBatchOp {
    BrowserBatchOp::Command(BrowserCommand::Scroll {
        direction: step["direction"].as_str().unwrap_or("down").to_string(),
        amount: step["amount"].as_i64().unwrap_or(600) as i32,
    })
}

fn parse_wait_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::Wait {
        selector: required_batch_string(step, "selector", "wait requires 'selector'")?,
        timeout_ms: step["timeout_ms"].as_u64().unwrap_or(5000),
    }))
}

fn parse_run_js_batch_op(step: &serde_json::Value) -> Result<BrowserBatchOp, String> {
    Ok(BrowserBatchOp::Command(BrowserCommand::RunJs {
        expression: required_batch_string(step, "expression", "run_js requires 'expression'")?,
    }))
}

fn parse_observe_batch_op(step: &serde_json::Value) -> BrowserBatchOp {
    BrowserBatchOp::Command(BrowserCommand::Observe {
        max_elements: step["max_elements"]
            .as_u64()
            .unwrap_or(DEFAULT_OBSERVE_ELEMENTS as u64)
            .clamp(1, MAX_OBSERVE_ELEMENTS as u64) as usize,
    })
}

fn browser_log_limit(step: &serde_json::Value) -> usize {
    step["limit"]
        .as_u64()
        .unwrap_or(50)
        .clamp(1, MAX_NETWORK_EVENTS as u64) as usize
}

fn parse_network_log_batch_op(step: &serde_json::Value) -> BrowserBatchOp {
    BrowserBatchOp::NetworkLog {
        limit: browser_log_limit(step),
        clear: step["clear"].as_bool().unwrap_or(false),
    }
}

fn parse_diagnostics_batch_op(step: &serde_json::Value) -> BrowserBatchOp {
    BrowserBatchOp::Diagnostics {
        limit: browser_log_limit(step),
        clear: step["clear"].as_bool().unwrap_or(false),
    }
}

fn unknown_browser_batch_action(action: &str) -> String {
    format!(
        "Unknown browser_batch action '{action}'. Use navigate, click, type, keys, select, hover, scroll, wait, run_js, read_page, screenshot, observe, status, network_log, diagnostics, back, or close."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_browser_batch_navigate_checks_ssrf_and_action() {
        let (action, op) = parse_browser_batch_op(&serde_json::json!({
            "action": "navigate",
            "url": "https://example.com"
        }))
        .expect("valid navigate step");
        assert_eq!(action, "navigate");
        match op {
            BrowserBatchOp::Command(BrowserCommand::Navigate { url }) => {
                assert_eq!(url, "https://example.com")
            }
            _ => panic!("expected navigate command"),
        }
    }

    #[test]
    fn parse_browser_batch_interaction_primitives() {
        let (_, keys) = parse_browser_batch_op(&serde_json::json!({
            "action": "keys",
            "keys": "Enter"
        }))
        .expect("valid keys step");
        assert!(matches!(
            keys,
            BrowserBatchOp::Command(BrowserCommand::Keys { .. })
        ));

        let (_, select) = parse_browser_batch_op(&serde_json::json!({
            "action": "select",
            "selector": "#country",
            "value": "France"
        }))
        .expect("valid select step");
        assert!(matches!(
            select,
            BrowserBatchOp::Command(BrowserCommand::Select { .. })
        ));

        let (_, hover) = parse_browser_batch_op(&serde_json::json!({
            "action": "hover",
            "selector": "@e3"
        }))
        .expect("valid hover step");
        assert!(matches!(
            hover,
            BrowserBatchOp::Command(BrowserCommand::Hover { .. })
        ));
    }

    #[test]
    fn parse_browser_batch_defaults_and_clamps_observability_ops() {
        let (_, observe) = parse_browser_batch_op(&serde_json::json!({
            "action": "observe",
            "max_elements": 999_999
        }))
        .expect("valid observe step");
        match observe {
            BrowserBatchOp::Command(BrowserCommand::Observe { max_elements }) => {
                assert_eq!(max_elements, MAX_OBSERVE_ELEMENTS)
            }
            _ => panic!("expected observe command"),
        }

        let (_, network) = parse_browser_batch_op(&serde_json::json!({
            "action": "network_log",
            "limit": 0,
            "clear": true
        }))
        .expect("valid network log step");
        match network {
            BrowserBatchOp::NetworkLog { limit, clear } => {
                assert_eq!(limit, 1);
                assert!(clear);
            }
            _ => panic!("expected network log op"),
        }

        let (_, diagnostics) = parse_browser_batch_op(&serde_json::json!({
            "action": "diagnostics"
        }))
        .expect("valid diagnostics step");
        match diagnostics {
            BrowserBatchOp::Diagnostics { limit, clear } => {
                assert_eq!(limit, 50);
                assert!(!clear);
            }
            _ => panic!("expected diagnostics op"),
        }
    }

    #[test]
    fn parse_browser_batch_rejects_unknown_action() {
        let err = parse_browser_batch_op(&serde_json::json!({"action": "teleport"}))
            .expect_err("unknown browser action must fail");
        assert!(err.contains("Unknown browser_batch action"));
    }
}
