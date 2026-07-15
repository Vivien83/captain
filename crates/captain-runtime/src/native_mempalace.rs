//! Managed MemPalace runtime discovery.
//!
//! Captain installs the MemPalace executable, its Python runtime, and uv in
//! `CAPTAIN_HOME/native/mempalace`. Memory data remains separate so runtime
//! repairs never replace the user's palace.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MEMPALACE_VERSION: &str = "3.5.0";
pub const UV_VERSION: &str = "0.11.28";
pub const PYTHON_VERSION: &str = "3.13.14";
pub const MEMPALACE_LOCK_SHA256: &str =
    "cc4a5669229761dbf65d9668659a3792edb8e86af060aa6bd6fe6dca6ae24e39";
pub const METADATA_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeMempalaceMetadata {
    pub schema_version: u32,
    pub mempalace_version: String,
    pub uv_version: String,
    pub python_version: String,
    pub lock_sha256: String,
    pub runtime_generation: String,
    pub platform: String,
    pub installed_at: String,
    pub data_home: PathBuf,
    pub palace_path: PathBuf,
    pub legacy_data_preserved: bool,
}

#[derive(Debug, Clone)]
pub struct NativeMempalacePaths {
    pub captain_home: PathBuf,
    pub runtime_dir: PathBuf,
    pub generations_dir: PathBuf,
    pub generation_dir: PathBuf,
    pub uv_dir: PathBuf,
    pub uv_binary: PathBuf,
    pub project_dir: PathBuf,
    pub tool_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub python_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub metadata_file: PathBuf,
    pub python_binary: PathBuf,
    pub mempalace_binary: PathBuf,
    pub mcp_binary: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeMempalaceStatus {
    pub ready: bool,
    pub runtime_ready: bool,
    pub data_ready: bool,
    pub permissions_ready: bool,
    pub expected_version: &'static str,
    pub installed_version: Option<String>,
    pub uv_version: &'static str,
    pub python_version: &'static str,
    pub expected_platform: String,
    pub installed_platform: Option<String>,
    pub captain_home: String,
    pub runtime_dir: String,
    pub runtime_generation: Option<String>,
    pub generation_dir: Option<String>,
    pub complete_generations: usize,
    pub stale_generations: usize,
    pub incomplete_generations: usize,
    pub mempalace_binary: Option<String>,
    pub mcp_binary: Option<String>,
    pub python_binary: Option<String>,
    pub data_home: Option<String>,
    pub palace_path: Option<String>,
    pub legacy_data_preserved: bool,
    pub metadata_valid: bool,
    pub install_hint: &'static str,
}

pub fn default_paths() -> NativeMempalacePaths {
    let captain_home = captain_home_dir();
    let runtime_dir = captain_home.join("native").join("mempalace");
    let uv_dir = runtime_dir.join("uv");
    let metadata_file = runtime_dir.join("install.json");
    let generations_dir = runtime_dir.join("generations");
    let generation = load_metadata(&metadata_file)
        .map(|metadata| metadata.runtime_generation)
        .filter(|value| valid_generation(value))
        .unwrap_or_else(|| "uninstalled".to_string());
    build_paths(
        captain_home,
        runtime_dir,
        uv_dir,
        metadata_file,
        generations_dir,
        generation,
    )
}

pub fn paths_for_generation(
    base: &NativeMempalacePaths,
    generation: &str,
) -> Result<NativeMempalacePaths, String> {
    if !valid_generation(generation) {
        return Err(format!(
            "invalid MemPalace runtime generation: {generation}"
        ));
    }
    Ok(build_paths(
        base.captain_home.clone(),
        base.runtime_dir.clone(),
        base.uv_dir.clone(),
        base.metadata_file.clone(),
        base.generations_dir.clone(),
        generation.to_string(),
    ))
}

fn build_paths(
    captain_home: PathBuf,
    runtime_dir: PathBuf,
    uv_dir: PathBuf,
    metadata_file: PathBuf,
    generations_dir: PathBuf,
    generation: String,
) -> NativeMempalacePaths {
    let generation_dir = generations_dir.join(generation);
    let project_dir = generation_dir.join("project");
    let tool_dir = generation_dir.join("venv");
    let bin_dir = if cfg!(windows) {
        tool_dir.join("Scripts")
    } else {
        tool_dir.join("bin")
    };
    NativeMempalacePaths {
        captain_home,
        uv_binary: uv_dir.join(executable_name("uv")),
        project_dir,
        tool_dir,
        python_dir: generation_dir.join("python"),
        cache_dir: generation_dir.join("cache"),
        metadata_file,
        python_binary: bin_dir.join(executable_name("python")),
        mempalace_binary: bin_dir.join(executable_name("mempalace")),
        mcp_binary: bin_dir.join(executable_name("mempalace-mcp")),
        runtime_dir,
        generations_dir,
        generation_dir,
        uv_dir,
        bin_dir,
    }
}

pub fn status() -> NativeMempalaceStatus {
    let paths = default_paths();
    let metadata = load_metadata(&paths.metadata_file);
    let (complete_generations, stale_generations, incomplete_generations) =
        generation_inventory(&paths.generations_dir);
    let metadata_valid = metadata.as_ref().is_some_and(metadata_matches_runtime);
    let permissions_ready = metadata.as_ref().is_some_and(|m| {
        private_dir_permissions(&paths.runtime_dir)
            && private_file_permissions(&paths.metadata_file)
            && private_dir_permissions(&m.data_home.join(".mempalace"))
            && private_dir_permissions(&m.palace_path)
    });
    let runtime_ready = metadata_valid
        && permissions_ready
        && is_usable_file(&paths.uv_binary)
        && is_usable_file(&paths.python_binary)
        && is_usable_file(&paths.mempalace_binary)
        && is_usable_file(&paths.mcp_binary)
        && generation_is_complete(&paths.generation_dir);
    let data_ready = metadata
        .as_ref()
        .is_some_and(|m| m.data_home.is_dir() && palace_is_initialized(&m.palace_path));

    NativeMempalaceStatus {
        ready: runtime_ready && data_ready,
        runtime_ready,
        data_ready,
        permissions_ready,
        expected_version: MEMPALACE_VERSION,
        installed_version: metadata.as_ref().map(|m| m.mempalace_version.clone()),
        uv_version: UV_VERSION,
        python_version: PYTHON_VERSION,
        expected_platform: runtime_platform(),
        installed_platform: metadata.as_ref().map(|m| m.platform.clone()),
        captain_home: paths.captain_home.display().to_string(),
        runtime_dir: paths.runtime_dir.display().to_string(),
        runtime_generation: metadata.as_ref().map(|m| m.runtime_generation.clone()),
        generation_dir: metadata_valid.then(|| paths.generation_dir.display().to_string()),
        complete_generations,
        stale_generations,
        incomplete_generations,
        mempalace_binary: is_usable_file(&paths.mempalace_binary)
            .then(|| paths.mempalace_binary.display().to_string()),
        mcp_binary: is_usable_file(&paths.mcp_binary)
            .then(|| paths.mcp_binary.display().to_string()),
        python_binary: is_usable_file(&paths.python_binary)
            .then(|| paths.python_binary.display().to_string()),
        data_home: metadata.as_ref().map(|m| m.data_home.display().to_string()),
        palace_path: metadata
            .as_ref()
            .map(|m| m.palace_path.display().to_string()),
        legacy_data_preserved: metadata.as_ref().is_some_and(|m| m.legacy_data_preserved),
        metadata_valid,
        install_hint: "captain memory install",
    }
}

pub fn load_metadata(path: &Path) -> Option<NativeMempalaceMetadata> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn resolved_metadata() -> Result<NativeMempalaceMetadata, String> {
    let paths = default_paths();
    let metadata = load_metadata(&paths.metadata_file).ok_or_else(|| {
        format!(
            "managed MemPalace metadata is missing at {}; run `captain memory install`",
            paths.metadata_file.display()
        )
    })?;
    if !metadata_matches_runtime(&metadata) {
        return Err(format!(
            "managed MemPalace metadata is stale; expected MemPalace {MEMPALACE_VERSION}, uv {UV_VERSION}, Python {PYTHON_VERSION}. Run `captain memory install --force`"
        ));
    }
    Ok(metadata)
}

pub fn choose_data_layout() -> (PathBuf, PathBuf, bool) {
    let captain_home = captain_home_dir();
    let managed_home = captain_home.join("data").join("mempalace").join("home");
    let managed_palace = managed_home.join(".mempalace").join("palace");
    let Some(user_home) = dirs::home_dir() else {
        return (managed_home, managed_palace, false);
    };
    let default_captain_home = user_home.join(".captain");
    let legacy_root = user_home.join(".mempalace");
    let preserve_legacy = same_path(&captain_home, &default_captain_home)
        && (legacy_root.join("palace").exists()
            || legacy_root.join("knowledge_graph.sqlite3").exists()
            || legacy_root.join("config.json").exists());
    if preserve_legacy {
        (user_home, legacy_root.join("palace"), true)
    } else {
        (managed_home, managed_palace, false)
    }
}

pub fn palace_is_initialized(path: &Path) -> bool {
    path.is_dir()
        && (path.join("chroma.sqlite3").is_file() || path.join("mempalace_embedder.json").is_file())
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

pub fn runtime_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn metadata_matches_runtime(metadata: &NativeMempalaceMetadata) -> bool {
    metadata.schema_version == METADATA_SCHEMA_VERSION
        && metadata.mempalace_version == MEMPALACE_VERSION
        && metadata.uv_version == UV_VERSION
        && metadata.python_version == PYTHON_VERSION
        && metadata.lock_sha256 == MEMPALACE_LOCK_SHA256
        && metadata.platform == runtime_platform()
        && valid_generation(&metadata.runtime_generation)
}

fn valid_generation(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

fn generation_is_complete(path: &Path) -> bool {
    let Ok(marker) = std::fs::read_to_string(path.join("COMPLETE")) else {
        return false;
    };
    marker
        .lines()
        .any(|line| line == format!("mempalace={MEMPALACE_VERSION}"))
        && marker
            .lines()
            .any(|line| line == format!("python={PYTHON_VERSION}"))
        && marker
            .lines()
            .any(|line| line == format!("lock_sha256={MEMPALACE_LOCK_SHA256}"))
}

fn generation_inventory(path: &Path) -> (usize, usize, usize) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return (0, 0, 0);
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .fold((0, 0, 0), |(complete, stale, incomplete), entry| {
            if generation_is_complete(&entry.path()) {
                (complete + 1, stale, incomplete)
            } else if entry.path().join("COMPLETE").is_file() {
                (complete, stale + 1, incomplete)
            } else {
                (complete, stale, incomplete + 1)
            }
        })
}

fn executable_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn is_usable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    true
}

#[cfg(unix)]
fn private_dir_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_dir()
        && std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn private_dir_permissions(path: &Path) -> bool {
    path.is_dir()
}

#[cfg(unix)]
fn private_file_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn private_file_permissions(path: &Path) -> bool {
    path.is_file()
}

fn same_path(left: &Path, right: &Path) -> bool {
    std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf())
        == std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_requires_every_pinned_component() {
        let metadata = NativeMempalaceMetadata {
            schema_version: METADATA_SCHEMA_VERSION,
            mempalace_version: MEMPALACE_VERSION.into(),
            uv_version: UV_VERSION.into(),
            python_version: PYTHON_VERSION.into(),
            lock_sha256: MEMPALACE_LOCK_SHA256.into(),
            runtime_generation: uuid::Uuid::nil().to_string(),
            platform: runtime_platform(),
            installed_at: "now".into(),
            data_home: PathBuf::from("/tmp/home"),
            palace_path: PathBuf::from("/tmp/palace"),
            legacy_data_preserved: false,
        };
        assert!(metadata_matches_runtime(&metadata));

        let mut stale = metadata;
        stale.mempalace_version = "0.0.0".into();
        assert!(!metadata_matches_runtime(&stale));
    }

    #[test]
    fn metadata_from_another_platform_is_never_activated() {
        let metadata = NativeMempalaceMetadata {
            schema_version: METADATA_SCHEMA_VERSION,
            mempalace_version: MEMPALACE_VERSION.into(),
            uv_version: UV_VERSION.into(),
            python_version: PYTHON_VERSION.into(),
            lock_sha256: MEMPALACE_LOCK_SHA256.into(),
            runtime_generation: uuid::Uuid::nil().to_string(),
            platform: "different-platform".into(),
            installed_at: "now".into(),
            data_home: PathBuf::from("/tmp/home"),
            palace_path: PathBuf::from("/tmp/palace"),
            legacy_data_preserved: false,
        };

        assert!(!metadata_matches_runtime(&metadata));
    }

    #[cfg(unix)]
    #[test]
    fn private_permission_checks_reject_group_or_world_access() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("memory");
        let file = temp.path().join("install.json");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(&file, b"{}").unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(private_dir_permissions(&directory));
        assert!(private_file_permissions(&file));

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!private_dir_permissions(&directory));
        assert!(!private_file_permissions(&file));
    }

    #[test]
    fn initialized_palace_requires_real_storage_marker() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!palace_is_initialized(temp.path()));
        std::fs::write(temp.path().join("chroma.sqlite3"), b"sqlite").unwrap();
        assert!(palace_is_initialized(temp.path()));
    }

    #[test]
    fn generation_ids_cannot_escape_the_managed_runtime() {
        assert!(valid_generation(&uuid::Uuid::new_v4().to_string()));
        assert!(!valid_generation("../../outside"));
        assert!(!valid_generation("uninstalled"));
    }

    #[test]
    fn generation_marker_binds_every_runtime_pin() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!generation_is_complete(temp.path()));
        std::fs::write(
            temp.path().join("COMPLETE"),
            format!(
                "mempalace={MEMPALACE_VERSION}\npython={PYTHON_VERSION}\nlock_sha256={MEMPALACE_LOCK_SHA256}\n"
            ),
        )
        .unwrap();
        assert!(generation_is_complete(temp.path()));
    }

    #[test]
    fn generation_inventory_distinguishes_stale_from_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let active = temp.path().join("active");
        let stale = temp.path().join("stale");
        let interrupted = temp.path().join("interrupted");
        for path in [&active, &stale, &interrupted] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(
            active.join("COMPLETE"),
            format!(
                "mempalace={MEMPALACE_VERSION}\npython={PYTHON_VERSION}\nlock_sha256={MEMPALACE_LOCK_SHA256}\n"
            ),
        )
        .unwrap();
        std::fs::write(stale.join("COMPLETE"), "mempalace=older\n").unwrap();

        assert_eq!(generation_inventory(temp.path()), (1, 1, 1));
    }
}
