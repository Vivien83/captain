use std::path::{Path, PathBuf};

/// Canonical runtime version. Git release tags use a leading `v`, while the
/// CLI and API expose the semantic version without transport-specific syntax.
pub fn canonical_version(value: &str) -> &str {
    let value = value.trim();
    let without_prefix = value.strip_prefix('v').or_else(|| value.strip_prefix('V'));
    without_prefix
        .filter(|candidate| candidate.starts_with(|ch: char| ch.is_ascii_digit()))
        .unwrap_or(value)
}

fn read_version_file(path: &Path) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let value = canonical_version(&value);
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn resolve_captain_version(
    build_version: Option<&str>,
    exe_dir: Option<&Path>,
    captain_home: Option<&Path>,
    default_home: Option<&Path>,
) -> String {
    if let Some(version) = build_version {
        let version = canonical_version(version);
        if !version.is_empty() {
            return version.to_string();
        }
    }
    if let Some(dir) = exe_dir {
        if let Some(version) = read_version_file(&dir.join("VERSION")) {
            return version;
        }
    }
    if let Some(home) = captain_home {
        if let Some(version) = read_version_file(&home.join("VERSION")) {
            return version;
        }
    }
    if let Some(home) = default_home {
        if let Some(version) = read_version_file(&home.join(".captain").join("VERSION")) {
            return version;
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn captain_version() -> String {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let captain_home = std::env::var_os("CAPTAIN_HOME").map(PathBuf::from);
    let default_home = dirs::home_dir();
    resolve_captain_version(
        option_env!("CAPTAIN_BUILD_VERSION"),
        exe_dir.as_deref(),
        captain_home.as_deref(),
        default_home.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_version_takes_precedence() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("VERSION"), "0.1.0-file\n").unwrap();

        assert_eq!(
            resolve_captain_version(Some(" 0.1.0-build "), Some(tmp.path()), None, None),
            "0.1.0-build"
        );
    }

    #[test]
    fn release_tag_prefix_is_not_exposed_by_the_runtime() {
        assert_eq!(
            canonical_version("v0.1.0-dev.2026-07-12a"),
            "0.1.0-dev.2026-07-12a"
        );
        assert_eq!(canonical_version(" V2.0.0 "), "2.0.0");
        assert_eq!(canonical_version("version-next"), "version-next");
        assert_eq!(
            resolve_captain_version(Some("v0.1.0-build"), None, None, None),
            "0.1.0-build"
        );
    }

    #[test]
    fn exe_version_file_is_first_file_fallback() {
        let exe = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(exe.path().join("VERSION"), "0.1.0-exe\n").unwrap();
        std::fs::write(home.path().join("VERSION"), "0.1.0-home\n").unwrap();

        assert_eq!(
            resolve_captain_version(None, Some(exe.path()), Some(home.path()), None),
            "0.1.0-exe"
        );
    }

    #[test]
    fn captain_home_version_file_is_second_file_fallback() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("VERSION"), "0.1.0-home\n").unwrap();

        assert_eq!(
            resolve_captain_version(None, None, Some(home.path()), None),
            "0.1.0-home"
        );
    }

    #[test]
    fn default_home_version_file_is_last_file_fallback() {
        let default_home = tempfile::tempdir().unwrap();
        let captain_dir = default_home.path().join(".captain");
        std::fs::create_dir(&captain_dir).unwrap();
        std::fs::write(captain_dir.join("VERSION"), "0.1.0-default\n").unwrap();

        assert_eq!(
            resolve_captain_version(None, None, None, Some(default_home.path())),
            "0.1.0-default"
        );
    }

    #[test]
    fn blank_values_fall_back_to_package_version() {
        let exe = tempfile::tempdir().unwrap();
        std::fs::write(exe.path().join("VERSION"), " \n").unwrap();

        assert_eq!(
            resolve_captain_version(Some("  "), Some(exe.path()), None, None),
            env!("CARGO_PKG_VERSION")
        );
    }
}
