//! Web download handler and filename helpers.

use crate::kernel_handle::KernelHandle;
use crate::tools::{
    check_taint_net_fetch, ensure_no_secret_literal, hex_nibble, resolve_file_path_for_caller,
};
use futures::StreamExt;
use sha2::Digest;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_WEB_DOWNLOAD_MAX_BYTES: u64 = 25 * 1024 * 1024;
const HARD_WEB_DOWNLOAD_MAX_BYTES: u64 = 100 * 1024 * 1024;

struct WebDownloadRequest {
    url: String,
    max_bytes: u64,
    overwrite: bool,
    output_path: Option<String>,
}

struct WebDownloadResponse {
    final_url: String,
    response: reqwest::Response,
    redirects: Vec<serde_json::Value>,
}

pub(crate) async fn tool_web_download(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let request = parse_web_download_request(input)?;
    let client = build_web_download_client()?;
    let WebDownloadResponse {
        final_url,
        response,
        redirects,
    } = follow_web_download_redirects(&client, &request.url).await?;

    let status = response.status();
    validate_web_download_response(&response, &final_url, request.max_bytes)?;
    let headers = response.headers().clone();
    let mime_type = response_mime_type(&headers);
    let bytes = read_web_download_body(response, request.max_bytes).await?;
    let output_path =
        output_path_for_download(&request, &final_url, &headers, mime_type.as_deref());
    write_web_download_file(
        &output_path,
        &bytes,
        workspace_root,
        kernel,
        caller_agent_id,
        request.overwrite,
    )
    .await?;

    render_web_download_result(
        &request.url,
        &final_url,
        status.as_u16(),
        &output_path,
        mime_type,
        &bytes,
        redirects,
    )
}

fn parse_web_download_request(input: &serde_json::Value) -> Result<WebDownloadRequest, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    ensure_no_secret_literal("web_download", "url", url)?;
    if let Some(violation) = check_taint_net_fetch(url) {
        return Err(format!("Taint violation: {violation}"));
    }

    let max_bytes = input["max_bytes"]
        .as_u64()
        .unwrap_or(DEFAULT_WEB_DOWNLOAD_MAX_BYTES)
        .clamp(1, HARD_WEB_DOWNLOAD_MAX_BYTES);
    let overwrite = input["overwrite"].as_bool().unwrap_or(false);
    let output_path = input["path"]
        .as_str()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());

    Ok(WebDownloadRequest {
        url: url.to_string(),
        max_bytes,
        overwrite,
        output_path,
    })
}

fn build_web_download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("CaptainAgent/0.1 (+https://github.com/Vivien83/captain)")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

async fn follow_web_download_redirects(
    client: &reqwest::Client,
    url: &str,
) -> Result<WebDownloadResponse, String> {
    let mut current_url = url.to_string();
    let mut redirects = Vec::new();
    let response = loop {
        crate::web_fetch::check_ssrf(&current_url)?;
        let resp = client
            .get(&current_url)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;
        let status = resp.status();
        if status.is_redirection() {
            if redirects.len() >= 10 {
                return Err("Too many redirects while downloading URL".to_string());
            }
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("Redirect without Location header")?;
            let next = resolve_redirect_url(&current_url, location)?;
            crate::web_fetch::check_ssrf(&next)?;
            redirects.push(serde_json::json!({
                "from": current_url,
                "to": next,
                "status": status.as_u16(),
            }));
            current_url = next;
            continue;
        }
        break resp;
    };

    Ok(WebDownloadResponse {
        final_url: current_url,
        response,
        redirects,
    })
}

fn validate_web_download_response(
    response: &reqwest::Response,
    final_url: &str,
    max_bytes: u64,
) -> Result<(), String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Download failed with HTTP {status} for {final_url}"
        ));
    }
    if let Some(len) = response.content_length() {
        if len > max_bytes {
            return Err(format!(
                "Response too large: {len} bytes (max {max_bytes} bytes)"
            ));
        }
    }
    Ok(())
}

fn response_mime_type(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn read_web_download_body(
    response: reqwest::Response,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Failed to read response body: {e}"))?;
        if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
            return Err(format!(
                "Response too large: exceeds {max_bytes} bytes while streaming"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn output_path_for_download(
    request: &WebDownloadRequest,
    final_url: &str,
    headers: &reqwest::header::HeaderMap,
    mime_type: Option<&str>,
) -> String {
    request.output_path.clone().unwrap_or_else(|| {
        format!(
            "downloads/{}",
            download_filename_from_response(final_url, headers, mime_type)
        )
    })
}

async fn write_web_download_file(
    output_path: &str,
    bytes: &[u8],
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    overwrite: bool,
) -> Result<(), String> {
    let resolved =
        resolve_file_path_for_caller(output_path, workspace_root, kernel, caller_agent_id)?;
    if resolved.exists() && !overwrite {
        return Err(format!(
            "Refusing to overwrite existing file: {} (pass overwrite=true)",
            resolved.display()
        ));
    }
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create parent directory: {e}"))?;
    }
    tokio::fs::write(&resolved, bytes)
        .await
        .map_err(|e| format!("Failed to write downloaded file: {e}"))
}

fn render_web_download_result(
    url: &str,
    final_url: &str,
    status: u16,
    output_path: &str,
    mime_type: Option<String>,
    bytes: &[u8],
    redirects: Vec<serde_json::Value>,
) -> Result<String, String> {
    let sha256 = hex::encode(sha2::Sha256::digest(bytes));
    let next_action = next_action_for_download(output_path, mime_type.as_deref());

    serde_json::to_string_pretty(&serde_json::json!({
        "success": true,
        "tool": "web_download",
        "url": url,
        "final_url": final_url,
        "status": status,
        "path": output_path,
        "mime_type": mime_type,
        "size_bytes": bytes.len(),
        "sha256": sha256,
        "redirects": redirects,
        "next_action": next_action,
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String, String> {
    let base = reqwest::Url::parse(current_url)
        .map_err(|e| format!("Invalid current URL during redirect: {e}"))?;
    let next = base
        .join(location)
        .map_err(|e| format!("Invalid redirect Location header: {e}"))?;
    if !matches!(next.scheme(), "http" | "https") {
        return Err(format!("Redirect scheme refused: {}", next.scheme()));
    }
    Ok(next.to_string())
}

fn download_filename_from_response(
    url: &str,
    headers: &reqwest::header::HeaderMap,
    mime_type: Option<&str>,
) -> String {
    if let Some(name) = headers
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .and_then(content_disposition_filename)
    {
        return ensure_extension_for_mime(&sanitize_download_filename(&name), mime_type);
    }

    let from_url = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_string))
        })
        .filter(|segment| !segment.trim().is_empty())
        .unwrap_or_else(|| "download".to_string());
    ensure_extension_for_mime(&sanitize_download_filename(&from_url), mime_type)
}

fn content_disposition_filename(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename*=") {
            let decoded = rest.trim_matches('"');
            if let Some((_, encoded)) = decoded.rsplit_once("''") {
                return Some(percent_decode_minimal(encoded));
            }
            return Some(percent_decode_minimal(decoded));
        }
        if let Some(rest) = part.strip_prefix("filename=") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

fn percent_decode_minimal(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' && idx + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(bytes[idx + 1]), hex_nibble(bytes[idx + 2])) {
                out.push((hi << 4) | lo);
                idx += 3;
                continue;
            }
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn sanitize_download_filename(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 120 {
            break;
        }
    }
    let out = out.trim_matches('.').trim_matches('_').to_string();
    if out.is_empty() {
        "download".to_string()
    } else {
        out
    }
}

pub(crate) fn ensure_extension_for_mime(filename: &str, mime_type: Option<&str>) -> String {
    if Path::new(filename).extension().is_some() {
        return filename.to_string();
    }
    let ext = match mime_type.unwrap_or_default() {
        "application/pdf" => Some("pdf"),
        "text/html" | "application/xhtml+xml" => Some("html"),
        "application/json" => Some("json"),
        "text/csv" => Some("csv"),
        "text/markdown" => Some("md"),
        mime if mime.starts_with("text/") => Some("txt"),
        _ => None,
    };
    match ext {
        Some(ext) => format!("{filename}.{ext}"),
        None => filename.to_string(),
    }
}

fn next_action_for_download(path: &str, mime_type: Option<&str>) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "pdf" | "txt" | "md" | "markdown" | "html" | "htm" | "csv" | "json" | "xml"
    ) || matches!(
        mime_type.unwrap_or_default(),
        "application/pdf" | "application/json" | "text/html" | "text/csv" | "text/markdown"
    ) || mime_type.unwrap_or_default().starts_with("text/")
    {
        "Call document_extract with this path before summarizing or citing the source."
    } else {
        "Use the media/file-specific tool for this binary type; do not infer its contents without reading it."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, CONTENT_DISPOSITION, CONTENT_TYPE};

    #[test]
    fn parse_web_download_request_clamps_and_trims_path() {
        let input = serde_json::json!({
            "url": "https://example.com/report.pdf",
            "max_bytes": HARD_WEB_DOWNLOAD_MAX_BYTES + 1,
            "overwrite": true,
            "path": " reports/report.pdf "
        });

        let request = parse_web_download_request(&input).unwrap();

        assert_eq!(request.url, "https://example.com/report.pdf");
        assert_eq!(request.max_bytes, HARD_WEB_DOWNLOAD_MAX_BYTES);
        assert!(request.overwrite);
        assert_eq!(request.output_path.as_deref(), Some("reports/report.pdf"));
    }

    #[test]
    fn response_mime_type_strips_parameters_and_blanks() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );

        assert_eq!(response_mime_type(&headers).as_deref(), Some("text/html"));

        headers.insert(CONTENT_TYPE, HeaderValue::from_static(" ; charset=utf-8"));
        assert_eq!(response_mime_type(&headers), None);
    }

    #[test]
    fn output_path_for_download_uses_response_filename_and_mime_extension() {
        let request = WebDownloadRequest {
            url: "https://example.com/download".to_string(),
            max_bytes: DEFAULT_WEB_DOWNLOAD_MAX_BYTES,
            overwrite: false,
            output_path: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename*=UTF-8''report%202026"),
        );

        assert_eq!(
            output_path_for_download(
                &request,
                "https://example.com/download",
                &headers,
                Some("application/pdf")
            ),
            "downloads/report_2026.pdf"
        );
    }
}
