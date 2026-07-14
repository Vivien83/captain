//! Native local embedding runtime discovery.
//!
//! FastEmbed uses ONNX Runtime. For release portability we load ONNX Runtime
//! dynamically from Captain's native asset directory instead of linking it at
//! build time.

use serde::Serialize;
use std::path::{Path, PathBuf};

pub const ONNXRUNTIME_VERSION: &str = "1.23.2";
pub const ONNXRUNTIME_ENV: &str = "CAPTAIN_ONNXRUNTIME_LIB";
pub const ORT_DYLIB_ENV: &str = "ORT_DYLIB_PATH";

#[derive(Debug, Clone, Serialize)]
pub struct NativeEmbeddingsStatus {
    pub home_dir: String,
    pub ready: bool,
    pub provider: &'static str,
    pub runtime: &'static str,
    pub version: &'static str,
    pub library: Option<String>,
    pub ort_dylib_path: Option<String>,
    pub install_hint: &'static str,
}

#[derive(Debug, Clone)]
pub struct NativeEmbeddingsPaths {
    pub home: PathBuf,
    pub runtime_dir: PathBuf,
    pub library: PathBuf,
}

pub fn captain_home_dir() -> PathBuf {
    if let Ok(value) = std::env::var("CAPTAIN_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return expand_tilde(trimmed);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".captain")
}

pub fn default_paths() -> NativeEmbeddingsPaths {
    let home = captain_home_dir();
    let runtime_dir = home.join("native").join("onnxruntime");
    let library = runtime_dir.join(primary_library_name());
    NativeEmbeddingsPaths {
        home,
        runtime_dir,
        library,
    }
}

pub fn status() -> NativeEmbeddingsStatus {
    let paths = default_paths();
    let configured = env_path(ORT_DYLIB_ENV);
    let library = configured.clone().or_else(find_library);
    NativeEmbeddingsStatus {
        home_dir: paths.home.display().to_string(),
        ready: library.is_some(),
        provider: "local",
        runtime: "onnxruntime",
        version: ONNXRUNTIME_VERSION,
        library: library.map(display_path),
        ort_dylib_path: configured.map(display_path),
        install_hint: "captain embeddings install",
    }
}

pub fn configure_environment() -> Result<PathBuf, String> {
    if let Some(path) = env_path(ORT_DYLIB_ENV) {
        return Ok(path);
    }

    match find_library() {
        Some(path) => {
            std::env::set_var(ORT_DYLIB_ENV, &path);
            Ok(path)
        }
        None => Err(format!(
            "ONNX Runtime library not found. Run `captain embeddings install` or reinstall Captain to provision {} under {}.",
            primary_library_name(),
            default_paths().runtime_dir.display()
        )),
    }
}

pub fn find_library() -> Option<PathBuf> {
    env_path(ONNXRUNTIME_ENV)
        .or_else(|| {
            default_library_candidates()
                .into_iter()
                .find(|p| is_usable_file(p))
        })
        .or_else(find_on_path)
}

pub fn primary_library_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "libonnxruntime.so"
    }
}

pub fn versioned_library_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libonnxruntime.1.23.2.dylib"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "libonnxruntime.so.1.23.2"
    }
}

fn default_library_candidates() -> Vec<PathBuf> {
    let paths = default_paths();
    let mut candidates = vec![
        paths.library,
        paths.runtime_dir.join(versioned_library_name()),
    ];

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(primary_library_name()));
            candidates.push(
                dir.join("native")
                    .join("onnxruntime")
                    .join(primary_library_name()),
            );
            candidates.push(
                dir.join("native")
                    .join("onnxruntime")
                    .join(versioned_library_name()),
            );
        }
    }

    candidates
}

fn find_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(primary_library_name());
        if is_usable_file(&candidate) {
            return Some(candidate);
        }
        let candidate = dir.join(versioned_library_name());
        if is_usable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = expand_tilde(trimmed);
    if is_usable_file(&path) {
        Some(path)
    } else {
        None
    }
}

fn is_usable_file(path: &Path) -> bool {
    path.is_file()
}

fn display_path(path: PathBuf) -> String {
    path.display().to_string()
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}
