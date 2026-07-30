//! Explicit environment inheritance for guarded host subprocesses.
//!
//! Clearing and selectively rebuilding an environment reduces accidental
//! secret inheritance. It does not isolate a process from the host OS.

/// Environment variables considered safe to inherit on all platforms.
pub const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
];

/// Additional environment variables considered safe on Windows.
#[cfg(windows)]
pub const SAFE_ENV_VARS_WINDOWS: &[&str] = &[
    "USERPROFILE",
    "SYSTEMROOT",
    "APPDATA",
    "LOCALAPPDATA",
    "COMSPEC",
    "WINDIR",
    "PATHEXT",
];

pub(crate) fn copy_safe_env(mut set: impl FnMut(&str, String)) {
    for key in SAFE_ENV_VARS {
        if let Ok(value) = std::env::var(key) {
            set(key, value);
        }
    }
    #[cfg(windows)]
    for key in SAFE_ENV_VARS_WINDOWS {
        if let Ok(value) = std::env::var(key) {
            set(key, value);
        }
    }
}

pub(crate) fn copy_allowed_env(allowed_env_vars: &[String], mut set: impl FnMut(&str, String)) {
    for key in allowed_env_vars {
        if let Ok(value) = std::env::var(key) {
            set(key, value);
        }
    }
}
