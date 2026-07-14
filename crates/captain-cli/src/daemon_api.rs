use captain_api::server::read_daemon_info;

use crate::{cli_captain_home, ui};

pub(crate) fn find_daemon() -> Option<String> {
    let home_dir = cli_captain_home();

    let mut candidates: Vec<String> = Vec::new();
    if let Some(info) = read_daemon_info(&home_dir) {
        candidates.push(info.listen_addr);
    }
    if let Ok(text) = std::fs::read_to_string(home_dir.join("config.toml")) {
        if let Ok(t) = text.parse::<toml::Value>() {
            if let Some(addr) = t.get("api_listen").and_then(|v| v.as_str()) {
                candidates.push(addr.to_string());
            }
        }
    }
    if std::env::var_os("CAPTAIN_HOME").is_none() {
        candidates.push("127.0.0.1:50051".to_string());
        candidates.push("127.0.0.1:4200".to_string());
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(500))
        .timeout(std::time::Duration::from_secs(2))
        .no_proxy()
        .build()
        .ok()?;
    for raw in candidates {
        let addr = raw.replace("0.0.0.0", "127.0.0.1");
        let url = format!("http://{addr}/api/health");
        if client
            .get(&url)
            .send()
            .is_ok_and(|r| r.status().is_success())
        {
            return Some(format!("http://{addr}"));
        }
    }
    None
}

pub(crate) fn daemon_client() -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .no_proxy();

    let headers = daemon_auth_headers();
    if !headers.is_empty() {
        builder = builder.default_headers(headers);
    }

    builder.build().expect("Failed to build HTTP client")
}

pub(crate) fn daemon_json(
    resp: Result<reqwest::blocking::Response, reqwest::Error>,
) -> serde_json::Value {
    match resp {
        Ok(r) => {
            let status = r.status();
            let body = r.json::<serde_json::Value>().unwrap_or_default();
            if status.is_server_error() {
                ui::error_with_fix(
                    &format!("Daemon returned error ({})", status),
                    "Check daemon logs: ~/.captain/tui.log",
                );
            }
            body
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timed out") || msg.contains("Timeout") {
                ui::error_with_fix(
                    "Request timed out",
                    "The agent may be processing a complex request. Try again, or check `captain status`",
                );
            } else if msg.contains("Connection refused") || msg.contains("connect") {
                ui::error_with_fix(
                    "Cannot connect to daemon",
                    "Is the daemon running? Start it with: captain start",
                );
            } else {
                ui::error_with_fix(
                    &format!("Daemon communication error: {msg}"),
                    "Check `captain status` or restart: captain start",
                );
            }
            std::process::exit(1);
        }
    }
}

pub(crate) fn daemon_auth_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();

    if let Some(token) = read_env_token("CAPTAIN_SESSION_TOKEN").or_else(read_local_session_token) {
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }

    if let Some(key) = read_api_key() {
        if !headers.contains_key(reqwest::header::AUTHORIZATION) {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
                headers.insert(reqwest::header::AUTHORIZATION, val);
            }
        }
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&key) {
            headers.insert("x-api-key", val);
        }
    }

    headers
}

pub(crate) fn require_daemon(command: &str) -> String {
    find_daemon().unwrap_or_else(|| {
        ui::error_with_fix(
            &format!("`captain {command}` requires a running daemon"),
            "Start the daemon: captain start",
        );
        ui::hint("Or try `captain chat` which works without a daemon");
        std::process::exit(1);
    })
}

fn read_env_token(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn read_local_session_token() -> Option<String> {
    let fallback = captain_types::config::AuthConfig::default();
    let api_key = read_api_key().unwrap_or_default();
    let snapshot =
        captain_api::session_auth::load_web_auth_snapshot(&cli_captain_home(), &api_key, &fallback);
    if !snapshot.auth.enabled {
        return None;
    }
    let username = snapshot.auth.username.trim();
    if username.is_empty() || snapshot.auth.password_hash.trim().is_empty() {
        return None;
    }
    Some(captain_api::session_auth::create_session_token(
        username,
        &snapshot.session_secret(),
        snapshot.auth.session_ttl_hours.max(1),
    ))
}

fn read_api_key() -> Option<String> {
    let config_path = cli_captain_home().join("config.toml");
    if let Ok(text) = std::fs::read_to_string(config_path) {
        if let Ok(table) = text.parse::<toml::Value>() {
            if let Some(key) = table.get("api_key").and_then(|v| v.as_str()) {
                let key = key.trim();
                if !key.is_empty() {
                    return Some(key.to_string());
                }
            }
        }
    }

    let home = cli_captain_home();
    let resolver = captain_extensions::credentials::CredentialResolver::new_with_secrets(
        None,
        Some(&home.join("secrets.env")),
        Some(&home.join(".env")),
    );
    for name in ["CAPTAIN_DAEMON_API_KEY", "CAPTAIN_API_KEY"] {
        if let Some(value) = resolver.resolve(name) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}
