use super::*;

#[test]
fn test_sanitize_canvas_basic_html() {
    let html = "<h1>Hello World</h1><p>This is a test.</p>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), html);
}

#[test]
fn test_sanitize_canvas_rejects_script() {
    let html = "<div><script>alert('xss')</script></div>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("script"));
}

#[test]
fn test_sanitize_canvas_rejects_iframe() {
    let html = "<iframe src='https://evil.com'></iframe>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("iframe"));
}

#[test]
fn test_sanitize_canvas_rejects_event_handler() {
    let html = "<div onclick=\"alert('xss')\">click me</div>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("event handler"));
}

#[test]
fn test_sanitize_canvas_rejects_onload() {
    let html = "<img src='x' onerror = \"alert(1)\">";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
}

#[test]
fn test_sanitize_canvas_rejects_javascript_url() {
    let html = "<a href=\"javascript:alert('xss')\">click</a>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("javascript:"));
}

#[test]
fn test_sanitize_canvas_rejects_data_html() {
    let html = "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>";
    let result = sanitize_canvas_html(html, 512 * 1024);
    assert!(result.is_err());
}

#[test]
fn test_sanitize_canvas_rejects_empty() {
    let result = sanitize_canvas_html("", 512 * 1024);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Empty"));
}

#[test]
fn test_sanitize_canvas_size_limit() {
    let html = "x".repeat(1024);
    let result = sanitize_canvas_html(&html, 100);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too large"));
}

#[tokio::test]
async fn test_canvas_present_tool() {
    let input = serde_json::json!({
        "html": "<h1>Test Canvas</h1><p>Hello world</p>",
        "title": "Test"
    });
    let tmp = std::env::temp_dir().join("captain_canvas_test");
    let _ = std::fs::create_dir_all(&tmp);
    let result = tool_canvas_present(&input, Some(tmp.as_path())).await;
    assert!(result.is_ok());
    let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(output["canvas_id"].is_string());
    assert_eq!(output["title"], "Test");
    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}
