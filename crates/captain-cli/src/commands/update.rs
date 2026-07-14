//! `captain update` — self-update the installed binary.
//!
//! Mirrors install.sh's resolution order (CAPTAIN_DIST_BASE_URL mirror
//! first, then GitHub Releases with optional CAPTAIN_GITHUB_TOKEN) and its
//! swap recipe (temp file next to the target, macOS ad-hoc codesign, atomic
//! rename — safe while the daemon runs since the old inode stays mapped
//! until restart). The restart itself reuses `captain service restart`,
//! which already knows launchd/systemd/tmux/background.

use sha2::Digest;
use std::path::{Path, PathBuf};

use super::service_runtime::installed_captain_binary;
use crate::{prompt_input, ui, ServiceManagerArg};

const DEFAULT_GITHUB_REPO: &str = "Vivien83/captain";

pub(crate) fn cmd_update(check: bool, yes: bool, version: Option<String>) {
    ui::section("Captain Update");

    if running_in_container() {
        ui::hint("Captain runs inside a container: update by rebuilding/pulling the image");
        ui::suggest_cmd(
            "From the repo checkout",
            "docker compose -f docker-compose.yml up -d --build",
        );
        return;
    }

    let platform = match detect_platform() {
        Ok(p) => p,
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    };
    let current = captain_types::version::captain_version();
    let remote = match version.map(Ok).unwrap_or_else(resolve_latest_version) {
        Ok(v) => v,
        Err(e) => {
            ui::error_with_fix(
                &format!("Could not determine the latest version: {e}"),
                "Set CAPTAIN_DIST_BASE_URL to a release mirror, or CAPTAIN_GITHUB_TOKEN for the private GitHub repo.",
            );
            std::process::exit(1);
        }
    };

    ui::kv("Installed", &current);
    ui::kv("Available", &remote);
    if versions_match(&remote, &current) {
        ui::success("Captain is already up to date.");
        return;
    }
    if check {
        ui::hint("Run `captain update` to install it.");
        return;
    }

    if !yes {
        let answer = prompt_input(&format!("  Update {current} -> {remote} now? [Y/n] "));
        if !(answer.is_empty() || answer.starts_with(['y', 'Y'])) {
            ui::hint("Update cancelled.");
            return;
        }
    }

    if let Err(e) = perform_update(&remote, &platform) {
        ui::error_with_fix(
            &format!("Update failed: {e}"),
            "The current binary is untouched. Retry, or reinstall with scripts/install.sh.",
        );
        std::process::exit(1);
    }

    if crate::find_daemon().is_some() {
        ui::step("Restarting the daemon on the new binary...");
        super::service::cmd_service_restart(ServiceManagerArg::Auto);
    } else {
        ui::success(&format!("Captain updated to {remote}."));
    }
}

fn perform_update(version: &str, platform: &str) -> Result<(), String> {
    let target = installed_captain_binary();
    let archive_name = format!("captain-{platform}.tar.gz");

    ui::step(&format!("Downloading {archive_name} ({version})..."));
    let archive_bytes = download_release_asset(version, &archive_name)?;
    let checksum_line = download_release_asset(version, &format!("{archive_name}.sha256"))
        .ok()
        .and_then(|b| String::from_utf8(b).ok());

    match checksum_line {
        Some(line) => {
            let expected = line.split_whitespace().next().unwrap_or("").to_lowercase();
            let actual = format!("{:x}", sha2::Sha256::digest(&archive_bytes));
            if expected.is_empty() || expected != actual {
                return Err(format!(
                    "checksum mismatch for {archive_name} (expected {expected}, got {actual})"
                ));
            }
            ui::success("Checksum verified.");
        }
        None => ui::check_warn("No .sha256 published for this bundle — skipping verification."),
    }

    let staging = tempfile::tempdir().map_err(|e| format!("temp dir: {e}"))?;
    let archive_path = staging.path().join(&archive_name);
    std::fs::write(&archive_path, &archive_bytes).map_err(|e| format!("write archive: {e}"))?;
    let tar_ok = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(staging.path())
        .status()
        .map_err(|e| format!("launch tar: {e}"))?
        .success();
    if !tar_ok {
        return Err("archive extraction failed".to_string());
    }
    let new_binary = find_binary(staging.path())
        .ok_or_else(|| "archive did not contain a `captain` binary".to_string())?;

    swap_binary(&new_binary, &target)?;
    write_version_marker(version);
    ui::success(&format!("Installed {} -> {}", version, target.display()));
    Ok(())
}

fn swap_binary(new_binary: &Path, target: &Path) -> Result<(), String> {
    let dir = target
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", target.display()))?;
    let tmp = dir.join(format!(".captain.update.{}", std::process::id()));
    std::fs::copy(new_binary, &tmp).map_err(|e| format!("stage binary: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-cr"])
            .arg(&tmp)
            .status();
        let signed = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&tmp)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !signed {
            ui::check_warn(
                "Ad-hoc codesign failed — on Apple Silicon the new binary may be killed by Gatekeeper.",
            );
        }
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("atomic swap into {}: {e}", target.display())
    })
}

fn find_binary(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join("captain");
    if direct.is_file() {
        return Some(direct);
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_binary(&path) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some("captain") {
            return Some(path);
        }
    }
    None
}

fn write_version_marker(version: &str) {
    let home = crate::cli_captain_home();
    if std::fs::create_dir_all(&home).is_ok() {
        let _ = std::fs::write(home.join("VERSION"), format!("{version}\n"));
    }
}

fn resolve_latest_version() -> Result<String, String> {
    if let Some(base) = dist_base_url() {
        let url = format!("{base}/latest.txt");
        let text = http_client()?
            .get(&url)
            .send()
            .map_err(|e| format!("{url}: {e}"))?
            .error_for_status()
            .map_err(|e| format!("{url}: {e}"))?
            .text()
            .map_err(|e| format!("{url}: {e}"))?;
        let version = text.trim().to_string();
        if version.is_empty() {
            return Err(format!("{url} is empty"));
        }
        return Ok(version);
    }

    let repo = github_repo();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut request = http_client()?.get(&url).header("User-Agent", "captain-cli");
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }
    let body: serde_json::Value = request
        .send()
        .map_err(|e| format!("{url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("{url}: {e} (no release published yet?)"))?
        .json()
        .map_err(|e| format!("{url}: {e}"))?;
    body["tag_name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{url}: response has no tag_name"))
}

fn download_release_asset(version: &str, asset: &str) -> Result<Vec<u8>, String> {
    if let Some(base) = dist_base_url() {
        let url = format!("{base}/{version}/{asset}");
        return fetch_bytes(http_client()?.get(&url), &url);
    }
    // Private repos reject the browser download URL even with a Bearer token
    // (plain 404) — assets must go through the API. Public repos work either
    // way, so token presence decides the path.
    if github_token().is_some() {
        return download_github_asset_via_api(version, asset);
    }
    let url = format!(
        "https://github.com/{}/releases/download/{version}/{asset}",
        github_repo()
    );
    fetch_bytes(http_client()?.get(&url), &url)
}

fn download_github_asset_via_api(version: &str, asset: &str) -> Result<Vec<u8>, String> {
    let repo = github_repo();
    let token = github_token().expect("checked by caller");
    let release_url = format!("https://api.github.com/repos/{repo}/releases/tags/{version}");
    let release: serde_json::Value = http_client()?
        .get(&release_url)
        .header("User-Agent", "captain-cli")
        .bearer_auth(&token)
        .send()
        .map_err(|e| format!("{release_url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("{release_url}: {e}"))?
        .json()
        .map_err(|e| format!("{release_url}: {e}"))?;
    let asset_id = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(asset))
        .and_then(|a| a["id"].as_i64())
        .ok_or_else(|| format!("release {version} has no asset named {asset}"))?;
    let url = format!("https://api.github.com/repos/{repo}/releases/assets/{asset_id}");
    fetch_bytes(
        http_client()?
            .get(&url)
            .header("User-Agent", "captain-cli")
            .header("Accept", "application/octet-stream")
            .bearer_auth(&token),
        &url,
    )
}

fn fetch_bytes(request: reqwest::blocking::RequestBuilder, url: &str) -> Result<Vec<u8>, String> {
    request
        .send()
        .map_err(|e| format!("{url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("{url}: {e}"))?
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("{url}: {e}"))
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))
}

fn dist_base_url() -> Option<String> {
    std::env::var("CAPTAIN_DIST_BASE_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
}

fn github_repo() -> String {
    std::env::var("CAPTAIN_GITHUB_REPO")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string())
}

fn github_token() -> Option<String> {
    std::env::var("CAPTAIN_GITHUB_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn versions_match(left: &str, right: &str) -> bool {
    captain_types::version::canonical_version(left)
        == captain_types::version::canonical_version(right)
}

pub(crate) fn running_in_container() -> bool {
    container_marker(Path::new("/.dockerenv"), Path::new("/proc/1/cgroup"))
}

fn container_marker(dockerenv: &Path, cgroup: &Path) -> bool {
    if dockerenv.exists() {
        return true;
    }
    std::fs::read_to_string(cgroup).is_ok_and(|content| {
        content.contains("docker") || content.contains("containerd") || content.contains("kubepods")
    })
}

fn detect_platform() -> Result<String, String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("unsupported architecture: {other}")),
    };
    match std::env::consts::OS {
        "macos" => Ok(format!("{arch}-apple-darwin")),
        "linux" => Ok(format!("{arch}-unknown-linux-gnu")),
        other => Err(format!(
            "self-update is not supported on {other}; re-run the installer instead"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_platform_matches_install_sh_naming() {
        let platform = detect_platform().unwrap();
        assert!(platform.ends_with("-apple-darwin") || platform.ends_with("-unknown-linux-gnu"));
        assert!(platform.starts_with("x86_64-") || platform.starts_with("aarch64-"));
    }

    #[test]
    fn release_tag_and_runtime_version_compare_equal() {
        assert!(versions_match(
            "v0.1.0-dev.2026-07-12a",
            "0.1.0-dev.2026-07-12a"
        ));
        assert!(!versions_match("v0.1.1", "0.1.0"));
    }

    #[test]
    fn container_marker_detects_dockerenv_and_cgroup() {
        let tmp = tempfile::tempdir().unwrap();
        let dockerenv = tmp.path().join(".dockerenv");
        let cgroup = tmp.path().join("cgroup");

        assert!(!container_marker(&dockerenv, &cgroup));

        std::fs::write(&cgroup, "0::/system.slice/docker-abcdef.scope\n").unwrap();
        assert!(container_marker(&dockerenv, &cgroup));

        std::fs::write(&cgroup, "0::/init.scope\n").unwrap();
        assert!(!container_marker(&dockerenv, &cgroup));

        std::fs::write(&dockerenv, "").unwrap();
        assert!(container_marker(&dockerenv, &cgroup));
    }

    #[test]
    fn find_binary_locates_nested_captain() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("bundle").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("captain"), b"fake").unwrap();
        let found = find_binary(tmp.path()).unwrap();
        assert!(found.ends_with("bundle/bin/captain"));
    }
}
