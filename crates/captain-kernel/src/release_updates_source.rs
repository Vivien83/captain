//! Release source resolution and install-environment detection.

use std::path::Path;
use std::time::Duration;

use captain_types::release_update::{
    current_release_platform, is_prerelease_version, select_newest_compatible_release,
    ReleaseDescriptor, RuntimeUpdateInstallMode,
};

use super::DEFAULT_GITHUB_REPO;

pub(super) async fn fetch_release_candidate(
    current: &str,
    self_update_supported: bool,
) -> Result<Option<ReleaseDescriptor>, String> {
    if let Some(base) = dist_base_url() {
        let url = format!("{base}/latest.txt");
        let payload = release_http_client()?
            .get(&url)
            .send()
            .await
            .map_err(|error| format!("{url}: {error}"))?
            .error_for_status()
            .map_err(|error| format!("{url}: {error}"))?
            .bytes()
            .await
            .map_err(|error| format!("{url}: {error}"))?;
        let version = parse_mirror_version(&payload, &url)?;
        let release = ReleaseDescriptor {
            tag_name: version.clone(),
            html_url: format!("{base}/{version}"),
            draft: false,
            prerelease: is_prerelease_version(&version)?,
            published_at: None,
            assets: Vec::new(),
        };
        return select_newest_compatible_release(current, &[release], &[]);
    }

    let repo = github_repo();
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
    let mut request = release_http_client()?
        .get(&url)
        .header("User-Agent", "captain-runtime-update-monitor");
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }
    let releases = request
        .send()
        .await
        .map_err(|error| format!("{url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{url}: {error}"))?
        .json::<Vec<ReleaseDescriptor>>()
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    let required = if self_update_supported {
        let platform = current_release_platform()?;
        vec![
            format!("captain-{platform}.tar.gz"),
            format!("captain-{platform}.tar.gz.sha256"),
        ]
    } else {
        Vec::new()
    };
    let required_refs = required.iter().map(String::as_str).collect::<Vec<_>>();
    select_newest_compatible_release(current, &releases, &required_refs)
}

pub(super) fn parse_mirror_version(payload: &[u8], source: &str) -> Result<String, String> {
    if payload.len() > 256 {
        return Err(format!("{source} exceeds the 256-byte version contract"));
    }
    let version = std::str::from_utf8(payload)
        .map_err(|error| format!("{source} is not UTF-8: {error}"))?
        .trim();
    if version.is_empty() {
        return Err(format!("{source} is empty"));
    }
    Ok(version.to_string())
}

pub(super) fn runtime_update_install_mode() -> RuntimeUpdateInstallMode {
    classify_install_mode(running_in_container(), current_release_platform().is_ok())
}

pub(super) fn classify_install_mode(
    in_container: bool,
    platform_supported: bool,
) -> RuntimeUpdateInstallMode {
    if in_container {
        RuntimeUpdateInstallMode::Container
    } else if platform_supported {
        RuntimeUpdateInstallMode::SelfUpdate
    } else {
        RuntimeUpdateInstallMode::Manual
    }
}

pub(super) fn running_in_container() -> bool {
    container_marker(Path::new("/.dockerenv"), Path::new("/proc/1/cgroup"))
}

pub(super) fn container_marker(dockerenv: &Path, cgroup: &Path) -> bool {
    dockerenv.exists()
        || std::fs::read_to_string(cgroup).is_ok_and(|content| {
            content.contains("docker")
                || content.contains("containerd")
                || content.contains("kubepods")
        })
}

fn release_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("release HTTP client: {error}"))
}

fn dist_base_url() -> Option<String> {
    std::env::var("CAPTAIN_DIST_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

fn github_repo() -> String {
    std::env::var("CAPTAIN_GITHUB_REPO")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string())
}

fn github_token() -> Option<String> {
    std::env::var("CAPTAIN_GITHUB_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
