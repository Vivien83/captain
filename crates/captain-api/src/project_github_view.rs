use serde_json::{json, Value};

const ID_LIMIT: usize = 120;

pub(crate) fn github_repositories_view(raw: &Value) -> Vec<Value> {
    raw.as_array()
        .map(|repos| repos.iter().map(github_repository_view).collect())
        .unwrap_or_default()
}

pub(crate) fn github_user_view(raw: &Value) -> Value {
    json!({
        "login": raw.get("login").and_then(Value::as_str).unwrap_or(""),
        "id": github_id_value(raw.get("id")),
    })
}

pub(crate) fn github_repositories_error_view(status: &str) -> Value {
    json!({
        "error": format!("GitHub returned {status}"),
        "repositories": [],
        "authenticated": true,
    })
}

pub(crate) fn github_repositories_transport_error_view(error: &str) -> Value {
    json!({
        "error": safe_github_repositories_error(error),
        "repositories": [],
        "authenticated": true,
    })
}

pub(crate) fn github_status_error_view(error: &str) -> Value {
    json!({
        "configured": true,
        "authenticated": false,
        "token_env": "GITHUB_TOKEN",
        "error": safe_github_status_error(error),
    })
}

pub(crate) fn github_token_validation_error_view(error: &str) -> Value {
    let safe = safe_github_status_error(error);
    let message = if safe.starts_with("GitHub returned ") {
        format!("GitHub token validation failed: {safe}")
    } else {
        "GitHub token validation failed; verify network access and token configuration".to_string()
    };
    json!({ "error": message })
}

pub(crate) fn github_token_persist_error_view(_error: &str) -> Value {
    json!({
        "error": "GitHub token could not be saved; verify Captain secrets storage permissions"
    })
}

pub(crate) fn github_token_remove_error_view(_error: &str) -> Value {
    json!({
        "error": "GitHub token could not be removed; verify Captain secrets storage permissions"
    })
}

pub(crate) fn normalize_github_full_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let mut parts = trimmed.split('/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    if parts.next().is_some() || !valid_github_name_part(owner) || !valid_github_name_part(repo) {
        return Err("github_full_name must look like owner/repo".to_string());
    }
    Ok(format!("{owner}/{repo}"))
}

pub(crate) fn github_clone_url_for_full_name(full_name: &str) -> String {
    format!("https://github.com/{full_name}.git")
}

pub(crate) fn github_source_metadata(
    full_name: &str,
    repo_id: Option<&Value>,
    branch: Option<&str>,
    local_path: &str,
) -> Value {
    json!({
        "type": "github",
        "full_name": full_name,
        "repo_id": github_id_value(repo_id),
        "branch": branch,
        "local_path": local_path,
    })
}

pub(crate) fn github_repository_view(repo: &Value) -> Value {
    json!({
        "id": github_id_value(repo.get("id")),
        "name": repo.get("name").and_then(Value::as_str).unwrap_or(""),
        "full_name": repo.get("full_name").and_then(Value::as_str).unwrap_or(""),
        "private": repo.get("private").and_then(Value::as_bool).unwrap_or(false),
        "default_branch": repo
            .get("default_branch")
            .and_then(Value::as_str)
            .unwrap_or("main"),
        "updated_at": repo.get("updated_at").and_then(Value::as_str).unwrap_or(""),
    })
}

fn github_id_value(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(id)) => Value::Number(id.clone()),
        Some(Value::String(id)) => {
            let id = bounded_text(id, ID_LIMIT);
            if id.is_empty() {
                Value::Null
            } else {
                json!(id)
            }
        }
        _ => Value::Null,
    }
}

fn valid_github_name_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains("..")
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn bounded_text(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn safe_github_status_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    let trimmed = error.trim();
    if let Some(status) = trimmed.strip_prefix("GitHub returned ") {
        let status = status
            .split([':', ';', '\n', '\r'])
            .next()
            .unwrap_or(status)
            .trim();
        return format!("GitHub returned {}", bounded_text(status, 80));
    }
    if lower.contains("response parse failed") {
        return "GitHub response could not be parsed".to_string();
    }
    if lower.contains("response failed") {
        return "GitHub response could not be read".to_string();
    }
    "GitHub authentication check failed; verify network access and token configuration".to_string()
}

fn safe_github_repositories_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("response parse failed") {
        return "GitHub repositories response could not be parsed".to_string();
    }
    if lower.contains("response failed") {
        return "GitHub repositories response could not be read".to_string();
    }
    "GitHub repositories request failed; verify network access and token configuration".to_string()
}

#[cfg(test)]
#[path = "project_github_view_tests.rs"]
mod project_github_view_tests;
