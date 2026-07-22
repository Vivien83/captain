//! Shared release-catalog and detached-update contracts.

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::version::canonical_version;

pub const RUNTIME_UPDATE_RESULT_SCHEMA_VERSION: u16 = 1;
pub const RUNTIME_UPDATE_RESULT_FILENAME: &str = "runtime-update-result.json";
pub const RUNTIME_UPDATE_CALLBACK_TOKEN_LEN: usize = 20;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReleaseDescriptor {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

impl ReleaseDescriptor {
    pub fn canonical_tag(&self) -> &str {
        canonical_version(&self.tag_name)
    }

    pub fn has_asset(&self, name: &str) -> bool {
        self.assets.iter().any(|asset| asset.name == name)
    }
}

pub fn release_lookup_token(version: &str) -> String {
    let digest = Sha256::digest(canonical_version(version).as_bytes());
    hex::encode(digest)[..RUNTIME_UPDATE_CALLBACK_TOKEN_LEN].to_string()
}

pub fn is_prerelease_version(version: &str) -> Result<bool, String> {
    Ok(!parse_version(version)?.pre.is_empty())
}

pub fn release_platform(os: &str, arch: &str) -> Result<String, String> {
    let arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("unsupported architecture: {other}")),
    };
    match os {
        "macos" => Ok(format!("{arch}-apple-darwin")),
        "linux" => Ok(format!("{arch}-unknown-linux-gnu")),
        other => Err(format!(
            "self-update is not supported on {other}; use the platform installer instead"
        )),
    }
}

pub fn current_release_platform() -> Result<String, String> {
    release_platform(std::env::consts::OS, std::env::consts::ARCH)
}

/// Pick the newest release that belongs to the installed version's channel.
///
/// Stable installations never opt into prereleases implicitly. An installation
/// already on a prerelease can advance through later prereleases and eventually
/// to the corresponding stable release. Required assets make an incomplete
/// publication invisible until it is actually installable.
pub fn select_newest_compatible_release(
    current: &str,
    releases: &[ReleaseDescriptor],
    required_assets: &[&str],
) -> Result<Option<ReleaseDescriptor>, String> {
    let current = parse_version(current)?;
    let current_is_prerelease = !current.pre.is_empty();
    let mut candidates = releases
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = parse_version(release.canonical_tag()).ok()?;
            let release_is_prerelease = release.prerelease || !version.pre.is_empty();
            if (!current_is_prerelease && release_is_prerelease)
                || version <= current
                || required_assets
                    .iter()
                    .any(|asset| !release.has_asset(asset))
            {
                return None;
            }
            Some((version, release))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_version, left), (right_version, right)| {
        left_version.cmp(right_version).then_with(|| {
            left.published_at
                .as_deref()
                .unwrap_or_default()
                .cmp(right.published_at.as_deref().unwrap_or_default())
        })
    });
    Ok(candidates.last().map(|(_, release)| (*release).clone()))
}

fn parse_version(value: &str) -> Result<Version, String> {
    Version::parse(canonical_version(value)).map_err(|error| {
        format!(
            "release version '{}' is not valid semantic versioning: {error}",
            value.trim()
        )
    })
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUpdateAttemptStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeUpdateAttemptResult {
    pub schema_version: u16,
    pub attempt_id: String,
    pub requested_version: String,
    pub status: RuntimeUpdateAttemptStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    pub message: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUpdateTelegramAction {
    Install,
    Defer,
    Refuse,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUpdateNoticeKind {
    Available,
    Reminder,
    InstallFailed,
    Installed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUpdateInstallMode {
    SelfUpdate,
    Container,
    Manual,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeUpdateCard {
    pub notice: RuntimeUpdateNoticeKind,
    pub token: String,
    pub decision_version: u64,
    pub current_version: String,
    pub available_version: String,
    pub release_url: String,
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub install_mode: RuntimeUpdateInstallMode,
    pub checked_at: String,
    pub next_check_at: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUpdateResolutionStatus {
    InstallStarted,
    Deferred,
    Refused,
    ContainerManual,
    PlatformManual,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeUpdateOperatorResolution {
    pub status: RuntimeUpdateResolutionStatus,
    pub current_version: String,
    pub available_version: String,
    pub retire_keyboard: bool,
    pub next_prompt_at: Option<String>,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeUpdateOperatorContext {
    pub chat_id: String,
    pub source_message_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, assets: &[&str]) -> ReleaseDescriptor {
        ReleaseDescriptor {
            tag_name: tag.to_string(),
            html_url: format!("https://example.test/{tag}"),
            draft: false,
            prerelease,
            published_at: Some("2026-07-19T12:00:00Z".to_string()),
            assets: assets
                .iter()
                .map(|name| ReleaseAsset {
                    name: (*name).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn prerelease_installations_follow_newer_prereleases() {
        let releases = vec![
            release(
                "v0.1.0-alpha.8",
                true,
                &["captain.tar.gz", "captain.tar.gz.sha256"],
            ),
            release(
                "v0.1.0-alpha.9",
                true,
                &["captain.tar.gz", "captain.tar.gz.sha256"],
            ),
        ];

        let selected = select_newest_compatible_release(
            "0.1.0-alpha.8",
            &releases,
            &["captain.tar.gz", "captain.tar.gz.sha256"],
        )
        .unwrap()
        .unwrap();

        assert_eq!(selected.tag_name, "v0.1.0-alpha.9");
    }

    #[test]
    fn stable_installations_do_not_opt_into_prereleases() {
        let releases = vec![release("v0.2.0-alpha.1", true, &[])];
        assert!(select_newest_compatible_release("0.1.0", &releases, &[])
            .unwrap()
            .is_none());
    }

    #[test]
    fn prerelease_installations_can_advance_to_stable() {
        let releases = vec![release("v0.1.0", false, &[])];
        assert_eq!(
            select_newest_compatible_release("0.1.0-alpha.9", &releases, &[])
                .unwrap()
                .unwrap()
                .canonical_tag(),
            "0.1.0"
        );
    }

    #[test]
    fn incomplete_and_draft_releases_are_ignored() {
        let mut draft = release("v0.1.0-alpha.10", true, &["captain.tar.gz"]);
        draft.draft = true;
        let releases = vec![release("v0.1.0-alpha.9", true, &["captain.tar.gz"]), draft];

        assert!(select_newest_compatible_release(
            "0.1.0-alpha.8",
            &releases,
            &["captain.tar.gz", "captain.tar.gz.sha256"],
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn malformed_installed_version_fails_closed() {
        let error = select_newest_compatible_release(
            "not-a-version",
            &[release("v1.0.0", false, &[])],
            &[],
        )
        .unwrap_err();
        assert!(error.contains("not valid semantic versioning"));
    }

    #[test]
    fn lookup_tokens_are_stable_compact_and_version_specific() {
        let first = release_lookup_token("v0.1.0-alpha.8");
        assert_eq!(first, release_lookup_token("0.1.0-alpha.8"));
        assert_eq!(first.len(), RUNTIME_UPDATE_CALLBACK_TOKEN_LEN);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, release_lookup_token("0.1.0-alpha.9"));
    }

    #[test]
    fn release_platform_matches_public_bundle_names() {
        assert_eq!(
            release_platform("macos", "aarch64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            release_platform("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
        assert!(release_platform("windows", "x86_64").is_err());
    }
}
