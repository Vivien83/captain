use captain_types::config::BrowserConfig;
use std::path::Path;

const CHROMIUM_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const CHROMIUM_ALLOWED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "DISPLAY",
    "WAYLAND_DISPLAY",
];

pub(super) fn chromium_launch_args(
    config: &BrowserConfig,
    user_data_dir: &Path,
    running_as_root: bool,
) -> Vec<String> {
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        format!("--user-data-dir={}", user_data_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-extensions".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-sync".to_string(),
        "--disable-translate".to_string(),
        "--disable-features=TranslateUI".to_string(),
        "--metrics-recording-only".to_string(),
        format!(
            "--window-size={},{}",
            config.viewport_width, config.viewport_height
        ),
        format!("--user-agent={CHROMIUM_USER_AGENT}"),
        "about:blank".to_string(),
    ];
    if config.headless {
        args.insert(0, "--headless=new".to_string());
        args.push("--disable-gpu".to_string());
    }
    if running_as_root {
        args.push("--no-sandbox".to_string());
    }
    args
}

pub(super) fn apply_chromium_env(cmd: &mut tokio::process::Command) {
    cmd.env_clear();
    for key in CHROMIUM_ALLOWED_ENV {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
}

pub(super) fn cdp_list_url(ws_url: &str) -> Result<String, String> {
    let port = ws_url
        .split("://")
        .nth(1)
        .and_then(|s| s.split(':').nth(1))
        .and_then(|s| s.split('/').next())
        .ok_or("Cannot parse port from CDP URL")?;
    Ok(format!("http://127.0.0.1:{port}/json/list"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_launch_args_include_profile_viewport_and_root_sandbox() {
        let config = BrowserConfig {
            headless: true,
            viewport_width: 1440,
            viewport_height: 900,
            ..BrowserConfig::default()
        };
        let args = chromium_launch_args(&config, Path::new("/tmp/captain-profile"), true);

        assert_eq!(args.first().map(String::as_str), Some("--headless=new"));
        assert!(args.contains(&"--disable-gpu".to_string()));
        assert!(args.contains(&"--no-sandbox".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/captain-profile".to_string()));
        assert!(args.contains(&"--window-size=1440,900".to_string()));
        assert!(args.iter().any(|arg| arg.starts_with("--user-agent=")));
    }

    #[test]
    fn cdp_list_url_extracts_devtools_port() {
        let url = cdp_list_url("ws://127.0.0.1:49231/devtools/browser/abc").unwrap();
        assert_eq!(url, "http://127.0.0.1:49231/json/list");
        assert!(cdp_list_url("not-a-websocket-url").is_err());
    }
}
