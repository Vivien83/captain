use std::path::{Path, PathBuf};

/// Read an OpenAI API key from the Codex CLI credential file.
///
/// Checks `$CODEX_HOME/auth.json` or `~/.codex/auth.json`.
/// Returns `Some(api_key)` if the file exists and contains a valid, non-expired token.
/// Only checks presence - the actual key value is used transiently, never stored.
pub fn read_codex_credential() -> Option<String> {
    for auth in read_codex_auth_files() {
        let Some(token) = extract_codex_token(&auth.parsed) else {
            continue;
        };
        if !codex_auth_expired(&auth.parsed, token) {
            return Some(token.to_string());
        }
    }
    None
}

/// Phase-k.5: like `read_codex_credential` but proactively refreshes the
/// access_token when the persisted `expires_at` is within a 120 s skew window
/// (or already past). On a successful refresh, the new tokens overwrite the
/// existing auth.json so the next call sees a healthy credential. Falls back
/// to whatever the file contains if no refresh_token is present or the refresh
/// HTTP call fails - never silently returns an expired token without trying.
///
/// Sync wrapper around the async `codex_oauth::refresh_tokens` so the existing
/// sync driver path (`drivers::build_driver`) can use it without rippling
/// async into the LlmDriver constructor surface.
pub fn read_codex_credential_with_refresh() -> Option<String> {
    let files = read_codex_auth_files();
    for auth in &files {
        let Some(token) = extract_codex_token(&auth.parsed) else {
            continue;
        };
        if codex_auth_needs_refresh(&auth.parsed) {
            if let Some(refreshed) = refresh_codex_auth_file_blocking(auth) {
                return Some(refreshed);
            }
        }
        if !codex_auth_expired(&auth.parsed, token) {
            return Some(token.to_string());
        }
    }
    None
}

/// Force-refresh the Codex OAuth access token and persist the new credential.
/// Intended for async request paths after a backend 401/403 response.
pub async fn refresh_codex_credential_now() -> Option<String> {
    refresh_or_rotate_codex_credential("").await
}

/// Refresh the active Codex credential, or rotate to another valid account
/// under `$CODEX_HOME/accounts/*.json` / `$CODEX_HOME/auth-*.json`.
pub async fn refresh_or_rotate_codex_credential(current_access_token: &str) -> Option<String> {
    let files = read_codex_auth_files();

    for auth in files.iter().filter(|auth| {
        extract_codex_token(&auth.parsed)
            .map(|token| token == current_access_token)
            .unwrap_or(false)
    }) {
        if let Some(token) = refresh_codex_auth_file(auth).await {
            if token != current_access_token {
                return Some(token);
            }
        }
    }

    for auth in &files {
        let Some(token) = extract_codex_token(&auth.parsed) else {
            continue;
        };
        if token != current_access_token && !codex_auth_expired(&auth.parsed, token) {
            return Some(token.to_string());
        }
    }

    for auth in &files {
        let Some(token) = extract_codex_token(&auth.parsed) else {
            continue;
        };
        if token != current_access_token {
            if let Some(refreshed) = refresh_codex_auth_file(auth).await {
                return Some(refreshed);
            }
        }
    }

    None
}

#[derive(Debug, Clone)]
struct CodexAuthFile {
    path: PathBuf,
    parsed: serde_json::Value,
}

fn read_codex_auth_files() -> Vec<CodexAuthFile> {
    let Some(home) = codex_home() else {
        return Vec::new();
    };
    codex_auth_paths_for_home(&home)
        .into_iter()
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            let parsed = serde_json::from_str::<serde_json::Value>(&content).ok()?;
            Some(CodexAuthFile { path, parsed })
        })
        .collect()
}

fn codex_auth_paths_for_home(codex_home: &Path) -> Vec<PathBuf> {
    let mut paths = vec![codex_home.join("auth.json")];

    if let Ok(entries) = std::fs::read_dir(codex_home.join("accounts")) {
        let mut extra = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        extra.sort();
        paths.extend(extra);
    }

    if let Ok(entries) = std::fs::read_dir(codex_home) {
        let mut extra = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.extension().and_then(|s| s.to_str()) == Some("json")
                    && path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(|name| name != "auth.json" && name.starts_with("auth-"))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        extra.sort();
        paths.extend(extra);
    }

    paths.sort();
    paths.dedup();
    if let Some(primary_idx) = paths
        .iter()
        .position(|p| p == &codex_home.join("auth.json"))
    {
        let primary = paths.remove(primary_idx);
        paths.insert(0, primary);
    }
    paths
}

fn codex_auth_expired(parsed: &serde_json::Value, token: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    parsed
        .get("expires_at")
        .and_then(|v| v.as_i64())
        .map(|expires_at| now >= expires_at)
        .unwrap_or(false)
        || codex_token_expired(token)
}

fn codex_auth_needs_refresh(parsed: &serde_json::Value) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    parsed
        .get("expires_at")
        .and_then(|v| v.as_i64())
        .map(|expires_at| expires_at <= now + 120)
        .unwrap_or(false)
}

fn codex_refresh_token(parsed: &serde_json::Value) -> Option<String> {
    parsed
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .or_else(|| parsed.get("refresh_token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn refresh_codex_auth_file_blocking(auth: &CodexAuthFile) -> Option<String> {
    let refresh_token = codex_refresh_token(&auth.parsed)?;
    let rt = tokio::runtime::Runtime::new().ok()?;
    let creds = rt
        .block_on(crate::codex_oauth::refresh_tokens(&refresh_token))
        .ok()?;
    persist_codex_credentials(&auth.path, creds)
}

async fn refresh_codex_auth_file(auth: &CodexAuthFile) -> Option<String> {
    let refresh_token = codex_refresh_token(&auth.parsed)?;
    let creds = crate::codex_oauth::refresh_tokens(&refresh_token)
        .await
        .ok()?;
    persist_codex_credentials(&auth.path, creds)
}

fn persist_codex_credentials(
    path: &Path,
    creds: crate::codex_oauth::CodexCredentials,
) -> Option<String> {
    let access_token = creds.access_token.clone();
    let payload = serde_json::json!({
        "tokens": {
            "access_token": creds.access_token,
            "refresh_token": creds.refresh_token,
        },
        "api_key": access_token.clone(),
        "expires_at": creds.expires_at,
        "last_refresh": creds.last_refresh,
        "auth_mode": creds.auth_mode,
        "source": creds.source,
    });
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, serde_json::to_string_pretty(&payload).ok()?).ok()?;
    Some(access_token)
}

pub(crate) fn codex_home() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            #[cfg(target_os = "windows")]
            {
                std::env::var("USERPROFILE")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".codex"))
            }
            #[cfg(not(target_os = "windows"))]
            {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".codex"))
            }
        })
}

fn extract_codex_token(parsed: &serde_json::Value) -> Option<&str> {
    parsed
        .get("api_key")
        .or_else(|| parsed.get("token"))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("access_token")))
        .or_else(|| parsed.get("tokens").and_then(|t| t.get("id_token")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    use base64::Engine;

    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn codex_chatgpt_account_id(token: &str) -> Option<String> {
    decode_jwt_payload(token)?
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(|s| s.as_str())
        .map(str::to_string)
}

pub fn codex_token_scopes(token: &str) -> Vec<String> {
    let Some(payload) = decode_jwt_payload(token) else {
        return Vec::new();
    };
    if let Some(arr) = payload.get("scp").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
    }
    if let Some(arr) = payload.get("scopes").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
    }
    payload
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn codex_token_expired(token: &str) -> bool {
    let Some(payload) = decode_jwt_payload(token) else {
        return false;
    };
    let Some(exp) = payload.get("exp").and_then(|v| v.as_i64()) else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now >= exp
}

/// Public-safe readiness check for the Codex OAuth path.
///
/// Returns `None` when no OAuth file exists so callers can report a normal
/// missing-login error. Returns `Some(...)` only when an OAuth file exists but
/// is structurally unusable.
pub fn codex_oauth_readiness_error() -> Option<String> {
    let auth_path = codex_home()?.join("auth.json");
    let content = std::fs::read_to_string(&auth_path).ok()?;
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(parsed) => parsed,
        Err(e) => return Some(format!("Codex auth.json is malformed: {e}")),
    };
    let Some(token) = extract_codex_token(&parsed) else {
        return Some(
            "Codex auth.json exists but contains no access token. Run `captain login codex`."
                .to_string(),
        );
    };
    if codex_token_expired(token) {
        return Some("Codex OAuth token is expired. Run `captain login codex`.".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_jwt(scopes: &[&str]) -> String {
        use base64::Engine;

        let payload = serde_json::json!({
            "scp": scopes,
            "exp": 4_102_444_800i64
        });
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        format!("header.{encoded}.signature")
    }

    #[test]
    fn codex_token_scopes_reads_connector_scopes_without_blocking() {
        let token = fake_jwt(&["openid", "api.connectors.invoke"]);
        assert_eq!(
            codex_token_scopes(&token),
            vec!["openid".to_string(), "api.connectors.invoke".to_string()]
        );
    }

    #[test]
    fn codex_auth_paths_include_primary_pool_and_root_alternates() {
        let root = std::env::temp_dir().join(format!(
            "captain-codex-auth-paths-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let accounts = root.join("accounts");
        std::fs::create_dir_all(&accounts).unwrap();
        std::fs::write(root.join("auth.json"), "{}").unwrap();
        std::fs::write(root.join("auth-alt.json"), "{}").unwrap();
        std::fs::write(accounts.join("pro.json"), "{}").unwrap();

        let paths = codex_auth_paths_for_home(&root);
        assert_eq!(paths.first(), Some(&root.join("auth.json")));
        assert!(paths.contains(&root.join("auth-alt.json")));
        assert!(paths.contains(&accounts.join("pro.json")));

        let _ = std::fs::remove_dir_all(root);
    }
}
