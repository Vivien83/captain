use crate::project_github_view as github_view;
use crate::project_workspace::github_token;
use crate::routes::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::path::Path as FsPath;
use std::sync::Arc;

const GITHUB_TOKEN_ENV: &str = "GITHUB_TOKEN";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const GITHUB_REPOS_URL: &str = "https://api.github.com/user/repos?per_page=100&sort=updated&affiliation=owner,collaborator,organization_member";

pub async fn github_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let Some(token) = github_token(&state) else {
        return github_status_unconfigured_response();
    };

    match github_user(&token).await {
        Ok(user) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "configured": true,
                "authenticated": true,
                "token_env": GITHUB_TOKEN_ENV,
                "user": user,
            })),
        ),
        Err(error) => (
            StatusCode::OK,
            Json(github_view::github_status_error_view(&error)),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct GithubTokenReq {
    pub token: String,
    #[serde(default = "default_true")]
    pub validate: bool,
}

pub async fn configure_github_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GithubTokenReq>,
) -> impl IntoResponse {
    let token = match normalize_github_token(&req.token) {
        Ok(token) => token,
        Err(error) => return bad_request(error),
    };
    let user = if req.validate {
        match github_user(&token).await {
            Ok(user) => Some(user),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(github_view::github_token_validation_error_view(&error)),
                )
            }
        }
    } else {
        None
    };

    state.kernel.store_credential(GITHUB_TOKEN_ENV, &token);
    if let Err(error) = write_secret_env_safe(
        &state.kernel.config.home_dir.join("secrets.env"),
        GITHUB_TOKEN_ENV,
        &token,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(github_view::github_token_persist_error_view(&error)),
        );
    }
    std::env::set_var(GITHUB_TOKEN_ENV, &token);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "saved",
            "configured": true,
            "authenticated": req.validate,
            "token_env": GITHUB_TOKEN_ENV,
            "user": user,
        })),
    )
}

pub async fn delete_github_token(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.kernel.remove_credential(GITHUB_TOKEN_ENV);
    if let Err(error) = remove_secret_env_safe(
        &state.kernel.config.home_dir.join("secrets.env"),
        GITHUB_TOKEN_ENV,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(github_view::github_token_remove_error_view(&error)),
        );
    }
    std::env::remove_var(GITHUB_TOKEN_ENV);
    github_status_unconfigured_response()
}

pub async fn github_repositories(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let Some(token) = github_token(&state) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "GITHUB_TOKEN is not configured",
                "repositories": [],
                "authenticated": false,
            })),
        );
    };

    let (status, body) = match github_get_text(&token, GITHUB_REPOS_URL).await {
        Ok(response) => response,
        Err(error) => return github_repositories_transport_error_response(&error),
    };
    if !status.is_success() {
        return github_repositories_status_error_response(status);
    }

    let raw = serde_json::from_str(&body).unwrap_or_default();
    let repositories = github_view::github_repositories_view(&raw);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "repositories": repositories,
            "authenticated": true,
        })),
    )
}

fn github_status_unconfigured_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "configured": false,
            "authenticated": false,
            "token_env": GITHUB_TOKEN_ENV,
        })),
    )
}

fn github_repositories_status_error_response(
    status: StatusCode,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(github_view::github_repositories_error_view(
            &status.to_string(),
        )),
    )
}

fn github_repositories_transport_error_response(
    error: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(github_view::github_repositories_transport_error_view(error)),
    )
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

fn default_true() -> bool {
    true
}

fn normalize_github_token(token: &str) -> Result<String, &'static str> {
    let token = token.trim();
    if token.is_empty() {
        return Err("token is required");
    }
    if token.contains('\n') || token.contains('\r') {
        return Err("token must be single-line");
    }
    Ok(token.to_string())
}

async fn github_user(token: &str) -> Result<serde_json::Value, String> {
    let (status, body) = github_get_text(token, GITHUB_USER_URL).await?;
    if !status.is_success() {
        return Err(format!("GitHub returned {status}"));
    }
    let raw = serde_json::from_str(&body)
        .map_err(|error| format!("GitHub response parse failed: {error}"))?;
    Ok(github_view::github_user_view(&raw))
}

async fn github_get_text(token: &str, url: &str) -> Result<(StatusCode, String), String> {
    let resp = reqwest::Client::new()
        .get(url)
        .header(USER_AGENT, "Captain")
        .header(ACCEPT, "application/vnd.github+json")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("GitHub request failed: {error}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|error| format!("GitHub response failed: {error}"))?;
    Ok((status, body))
}

fn write_secret_env_safe(path: &FsPath, key: &str, value: &str) -> Result<(), String> {
    if key.trim().is_empty()
        || key.contains('=')
        || key.contains('\n')
        || key.contains('\r')
        || value.contains('\n')
        || value.contains('\r')
    {
        return Err("invalid secret assignment".to_string());
    }

    let mut lines = if path.exists() {
        std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read secrets.env: {error}"))?
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    lines.retain(|line| !line.starts_with(&format!("{key}=")));
    lines.push(format!("{key}={value}"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create secrets directory: {error}"))?;
    }
    let tmp_path = path.with_extension("env.tmp");
    std::fs::write(&tmp_path, lines.join("\n") + "\n")
        .map_err(|error| format!("failed to write temp secrets.env: {error}"))?;
    set_secret_file_permissions(&tmp_path)?;
    std::fs::rename(&tmp_path, path)
        .map_err(|error| format!("failed to commit secrets.env: {error}"))?;
    set_secret_file_permissions(path)?;
    Ok(())
}

fn remove_secret_env_safe(path: &FsPath, key: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let lines = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read secrets.env: {error}"))?
        .lines()
        .filter(|line| !line.starts_with(&format!("{key}=")))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let tmp_path = path.with_extension("env.tmp");
    std::fs::write(&tmp_path, lines.join("\n") + "\n")
        .map_err(|error| format!("failed to write temp secrets.env: {error}"))?;
    set_secret_file_permissions(&tmp_path)?;
    std::fs::rename(&tmp_path, path)
        .map_err(|error| format!("failed to commit secrets.env: {error}"))?;
    set_secret_file_permissions(path)?;
    Ok(())
}

fn set_secret_file_permissions(path: &FsPath) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to set secrets.env permissions: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_github_token_rejects_blank_and_multiline() {
        assert_eq!(normalize_github_token("  ghp_demo  ").unwrap(), "ghp_demo");
        assert_eq!(
            normalize_github_token(" ").unwrap_err(),
            "token is required"
        );
        assert_eq!(
            normalize_github_token("ghp_demo\nOTHER=value").unwrap_err(),
            "token must be single-line"
        );
    }

    #[test]
    fn write_secret_env_safe_replaces_existing_token() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        std::fs::write(&path, "OTHER=1\nGITHUB_TOKEN=old\n").unwrap();

        write_secret_env_safe(&path, GITHUB_TOKEN_ENV, "new").unwrap();

        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "OTHER=1\nGITHUB_TOKEN=new\n"
        );
    }

    #[test]
    fn write_secret_env_safe_rejects_injection() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");

        let error = write_secret_env_safe(&path, GITHUB_TOKEN_ENV, "secret\nOTHER=1").unwrap_err();

        assert_eq!(error, "invalid secret assignment");
        assert!(!path.exists());
    }

    #[test]
    fn remove_secret_env_safe_removes_token_only() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        std::fs::write(&path, "OTHER=1\nGITHUB_TOKEN=old\n").unwrap();

        remove_secret_env_safe(&path, GITHUB_TOKEN_ENV).unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "OTHER=1\n");
    }

    #[test]
    fn github_repositories_status_error_omits_response_body() {
        let (_, Json(view)) = github_repositories_status_error_response(StatusCode::UNAUTHORIZED);

        assert_eq!(view["error"], "GitHub returned 401 Unauthorized");
        assert_eq!(view["repositories"], serde_json::json!([]));
        assert_eq!(view["authenticated"], true);
        assert!(!serde_json::to_string(&view)
            .unwrap()
            .contains("Bad credentials"));
    }
}
