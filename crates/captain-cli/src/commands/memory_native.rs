use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use captain_runtime::native_mempalace::{
    self, NativeMempalaceMetadata, NativeMempalacePaths, MEMPALACE_LOCK_SHA256, MEMPALACE_VERSION,
    METADATA_SCHEMA_VERSION, PYTHON_VERSION, UV_VERSION,
};
use captain_types::config::{KernelConfig, MemoryBackend};
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::ui;

const DOWNLOAD_LIMIT_BYTES: u64 = 100 * 1024 * 1024;
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const LIVE_PROBE_TIMEOUT: Duration = Duration::from_secs(90);
const MEMPALACE_PYPROJECT: &[u8] =
    include_bytes!("../../../../assets/native/mempalace/pyproject.toml");
const MEMPALACE_UV_LOCK: &[u8] = include_bytes!("../../../../assets/native/mempalace/uv.lock");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UvAsset {
    file_name: &'static str,
    sha256: &'static str,
    archive_kind: ArchiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    TarGz,
    Zip,
}

#[derive(Debug)]
struct CommandResult {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeMempalaceStartOutcome {
    NotRequired,
    Ready,
    Repaired,
    Disabled,
}

struct InstallLock {
    file: File,
}

impl InstallLock {
    fn acquire(runtime_dir: &Path) -> Result<Self, String> {
        create_private_dir(runtime_dir)?;
        let path = runtime_dir.join("install.lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|e| format!("open MemPalace install lock {}: {e}", path.display()))?;
        set_private_file_permissions(&path)?;
        file.try_lock_exclusive().map_err(|e| {
            format!(
                "another MemPalace installation or repair is already running ({}): {e}",
                path.display()
            )
        })?;
        file.set_len(0)
            .map_err(|e| format!("reset MemPalace install lock: {e}"))?;
        writeln!(
            file,
            "pid={} acquired_at={}",
            std::process::id(),
            chrono::Utc::now().to_rfc3339()
        )
        .map_err(|e| format!("write MemPalace install lock: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("sync MemPalace install lock: {e}"))?;
        Ok(Self { file })
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

struct PendingGeneration {
    path: PathBuf,
    committed: bool,
}

impl PendingGeneration {
    fn new(path: PathBuf) -> Result<Self, String> {
        create_private_dir(&path)?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PendingGeneration {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

pub(crate) fn cmd_memory_native_status(json: bool) {
    let status = native_mempalace::status();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).unwrap_or_default()
        );
        return;
    }

    ui::section("Native MemPalace");
    ui::kv("Expected version", status.expected_version);
    ui::kv(
        "Installed version",
        status.installed_version.as_deref().unwrap_or("-"),
    );
    ui::kv(
        "Runtime",
        readiness_label(status.runtime_ready, status.installed_version.is_some()),
    );
    ui::kv(
        "Palace",
        readiness_label(status.data_ready, status.palace_path.is_some()),
    );
    ui::kv(
        "Private permissions",
        if status.permissions_ready {
            "ready"
        } else {
            "degraded"
        },
    );
    ui::kv("Expected platform", &status.expected_platform);
    ui::kv(
        "Installed platform",
        status.installed_platform.as_deref().unwrap_or("-"),
    );
    ui::kv("Status", if status.ready { "ready" } else { "degraded" });
    ui::kv("Runtime directory", &status.runtime_dir);
    ui::kv(
        "Runtime generation",
        status.runtime_generation.as_deref().unwrap_or("-"),
    );
    ui::kv(
        "Runtime generations",
        &format!(
            "{} compatible, {} stale, {} incomplete",
            status.complete_generations, status.stale_generations, status.incomplete_generations
        ),
    );
    ui::kv("Data home", status.data_home.as_deref().unwrap_or("-"));
    ui::kv("Palace path", status.palace_path.as_deref().unwrap_or("-"));
    if status.legacy_data_preserved {
        ui::hint("Existing ~/.mempalace data is preserved and used in place.");
    }
    if !status.ready {
        ui::hint(status.install_hint);
    }
    if status.incomplete_generations > 0 {
        ui::hint("An interrupted runtime generation will be removed by the next repair.");
    }
    if status.stale_generations > 0 {
        ui::hint("A previous complete runtime generation is retained for safe process rollover.");
    }
}

fn readiness_label(ready: bool, present: bool) -> &'static str {
    if ready {
        "ready"
    } else if present {
        "degraded"
    } else {
        "missing"
    }
}

pub(crate) fn cmd_memory_native_doctor(json: bool) {
    let status = native_mempalace::status();
    let live = if status.runtime_ready {
        live_probe().map_err(|e| e.to_string())
    } else {
        Err("managed runtime is not installed".to_string())
    };
    let healthy = status.ready && live.is_ok();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": status,
                "live_probe": match &live {
                    Ok(version) => serde_json::json!({"ok": true, "version": version}),
                    Err(error) => serde_json::json!({"ok": false, "error": error}),
                },
            }))
            .unwrap_or_default()
        );
        if !healthy {
            std::process::exit(1);
        }
        return;
    }

    cmd_memory_native_status(false);
    match live {
        Ok(version) => ui::check_ok(&format!("Live executable: {version}")),
        Err(error) => ui::check_fail(&format!("Live executable: {error}")),
    }
    if !healthy {
        std::process::exit(1);
    }
}

pub(crate) fn cmd_memory_native_install(best_effort: bool, force: bool) {
    ui::section("Native MemPalace Install");
    println!("  Installing MemPalace {MEMPALACE_VERSION} with isolated Python {PYTHON_VERSION}.");

    let result = install_managed_mempalace(force).and_then(|_| ensure_integration_enabled());
    let status = native_mempalace::status();
    if result.is_ok() && status.ready {
        ui::success("Native MemPalace runtime and palace are ready.");
        return;
    }

    let error = result
        .err()
        .unwrap_or_else(|| "post-install readiness check failed".to_string());
    ui::check_warn(&error);
    if !best_effort {
        ui::error_with_fix(
            "Native MemPalace install incomplete",
            "Run `captain memory doctor`, then retry `captain memory install --force`.",
        );
        std::process::exit(1);
    }
}

pub(crate) fn ensure_native_mempalace_for_start() -> Result<NativeMempalaceStartOutcome, String> {
    if !mempalace_install_enabled(std::env::var("CAPTAIN_MEMPALACE_INSTALL").ok().as_deref()) {
        return Ok(NativeMempalaceStartOutcome::Disabled);
    }
    let status = native_mempalace::status();
    let outcome = if status.ready {
        match live_probe() {
            Ok(_) => NativeMempalaceStartOutcome::Ready,
            Err(error) => {
                eprintln!(
                    "Managed MemPalace live probe failed; rebuilding the isolated runtime: {error}"
                );
                install_managed_mempalace(true)?;
                NativeMempalaceStartOutcome::Repaired
            }
        }
    } else {
        install_managed_mempalace(false)?;
        NativeMempalaceStartOutcome::Repaired
    };
    ensure_integration_enabled()?;
    if !native_mempalace::status().ready {
        Err("managed MemPalace remains degraded after automatic repair".to_string())
    } else {
        Ok(outcome)
    }
}

pub(crate) fn ensure_native_mempalace_for_config(
    config: &KernelConfig,
) -> Result<NativeMempalaceStartOutcome, String> {
    std::env::set_var("CAPTAIN_HOME", &config.home_dir);
    if config.memory.backend != MemoryBackend::Mempalace {
        return Ok(NativeMempalaceStartOutcome::NotRequired);
    }
    ensure_native_mempalace_for_start()
}

pub(crate) fn prepare_kernel_config(config_path: Option<&Path>) -> Result<KernelConfig, String> {
    let config = captain_kernel::config::load_config(config_path);
    ensure_native_mempalace_for_config(&config)?;
    Ok(config)
}

fn mempalace_install_enabled(value: Option<&str>) -> bool {
    !matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("0" | "false" | "no" | "n")
    )
}

pub(crate) fn cmd_memory_mcp_serve() {
    let status = native_mempalace::status();
    if !status.ready {
        eprintln!(
            "Captain managed MemPalace is not ready. Run `captain memory install --force` (runtime_ready={}, data_ready={}).",
            status.runtime_ready, status.data_ready
        );
        std::process::exit(78);
    }
    let paths = native_mempalace::default_paths();
    let metadata = match native_mempalace::resolved_metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(78);
        }
    };
    let _ = std::fs::create_dir_all(paths.captain_home.join("logs"));
    let mut command = Command::new(&paths.mcp_binary);
    command
        .arg("--palace")
        .arg(&metadata.palace_path)
        .arg("--transport")
        .arg("stdio");
    configure_mempalace_environment(&mut command, &paths, &metadata, true);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!("Failed to exec managed MemPalace MCP server: {error}");
        std::process::exit(70);
    }
    #[cfg(not(unix))]
    {
        match command.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => {
                eprintln!("Failed to launch managed MemPalace MCP server: {error}");
                std::process::exit(70);
            }
        }
    }
}

fn install_managed_mempalace(force: bool) -> Result<(), String> {
    let base_paths = native_mempalace::default_paths();
    let _install_lock = InstallLock::acquire(&base_paths.runtime_dir)?;
    let mut current = native_mempalace::status();
    if current.metadata_valid && !current.permissions_ready {
        if let Some(metadata) = native_mempalace::load_metadata(&base_paths.metadata_file) {
            harden_existing_install_permissions(&base_paths, &metadata)?;
            current = native_mempalace::status();
        }
    }
    if current.ready && !force {
        match live_probe() {
            Ok(_) => return Ok(()),
            Err(error) => eprintln!(
                "Managed MemPalace files are present but the live probe failed; repairing: {error}"
            ),
        }
    }

    for dir in [
        &base_paths.runtime_dir,
        &base_paths.uv_dir,
        &base_paths.generations_dir,
    ] {
        create_private_dir(dir)?;
    }
    cleanup_incomplete_generations(&base_paths)?;

    if validate_uv_version(&base_paths).is_err() {
        install_uv(&base_paths)?;
    }
    validate_uv_version(&base_paths)?;

    let generation = uuid::Uuid::new_v4().to_string();
    let paths = native_mempalace::paths_for_generation(&base_paths, &generation)?;
    let mut pending_generation = PendingGeneration::new(paths.generation_dir.clone())?;
    for dir in [&paths.project_dir, &paths.python_dir, &paths.cache_dir] {
        create_private_dir(dir)?;
    }
    install_mempalace_tool(&paths)?;

    let prior_metadata = native_mempalace::load_metadata(&base_paths.metadata_file);
    let (data_home, palace_path, legacy_data_preserved) = prior_metadata
        .clone()
        .map(|metadata| {
            (
                metadata.data_home,
                metadata.palace_path,
                metadata.legacy_data_preserved,
            )
        })
        .unwrap_or_else(native_mempalace::choose_data_layout);
    create_private_dir(&data_home.join(".mempalace"))?;

    let metadata = NativeMempalaceMetadata {
        schema_version: METADATA_SCHEMA_VERSION,
        mempalace_version: MEMPALACE_VERSION.to_string(),
        uv_version: UV_VERSION.to_string(),
        python_version: PYTHON_VERSION.to_string(),
        lock_sha256: MEMPALACE_LOCK_SHA256.to_string(),
        runtime_generation: generation,
        platform: native_mempalace::runtime_platform(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        data_home,
        palace_path,
        legacy_data_preserved,
    };

    validate_python_version(&paths, &metadata)?;
    validate_mempalace_version(&paths, &metadata)?;
    initialize_palace_if_needed(&paths, &metadata)?;
    set_private_dir_permissions(&metadata.data_home.join(".mempalace"))?;
    set_private_dir_permissions(&metadata.palace_path)?;
    validate_palace(&paths, &metadata)?;
    write_generation_complete_marker(&paths)?;
    write_metadata_atomic(&base_paths.metadata_file, &metadata)?;

    if !native_mempalace::status().ready {
        restore_metadata(&base_paths.metadata_file, prior_metadata.as_ref())?;
        return Err("managed MemPalace files exist but final status is not ready".to_string());
    }
    pending_generation.commit();
    let previous_generation = prior_metadata
        .as_ref()
        .map(|previous| previous.runtime_generation.as_str());
    prune_obsolete_generations(
        &base_paths,
        &metadata.runtime_generation,
        previous_generation,
    );
    if let Err(error) = std::fs::remove_dir_all(&paths.cache_dir) {
        if paths.cache_dir.exists() {
            eprintln!(
                "Warning: managed MemPalace download cache could not be removed at {}: {error}",
                paths.cache_dir.display()
            );
        }
    }
    Ok(())
}

fn harden_existing_install_permissions(
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
) -> Result<(), String> {
    let mempalace_home = metadata.data_home.join(".mempalace");
    for directory in [
        &paths.runtime_dir,
        &paths.uv_dir,
        &paths.generations_dir,
        &paths.generation_dir,
        &paths.project_dir,
        &paths.tool_dir,
        &paths.python_dir,
        &mempalace_home,
        &metadata.palace_path,
    ] {
        if directory.is_dir() {
            set_private_dir_permissions(directory)?;
        }
    }
    if paths.metadata_file.is_file() {
        set_private_file_permissions(&paths.metadata_file)?;
    }
    Ok(())
}

fn cleanup_incomplete_generations(paths: &NativeMempalacePaths) -> Result<(), String> {
    let entries = std::fs::read_dir(&paths.generations_dir).map_err(|e| {
        format!(
            "inspect MemPalace runtime generations {}: {e}",
            paths.generations_dir.display()
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let managed_generation = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok());
        if !managed_generation
            || !path.is_dir()
            || path == paths.generation_dir
            || path.join("COMPLETE").is_file()
        {
            continue;
        }
        std::fs::remove_dir_all(&path).map_err(|e| {
            format!(
                "remove incomplete MemPalace runtime generation {}: {e}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn prune_obsolete_generations(
    paths: &NativeMempalacePaths,
    active_generation: &str,
    previous_generation: Option<&str>,
) {
    let Ok(entries) = std::fs::read_dir(&paths.generations_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if uuid::Uuid::parse_str(name).is_err()
            || name == active_generation
            || previous_generation == Some(name)
            || !path.join("COMPLETE").is_file()
        {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&path) {
            eprintln!(
                "Warning: obsolete MemPalace runtime generation {} could not be removed: {error}",
                path.display()
            );
        }
    }
}

fn validate_uv_version(paths: &NativeMempalacePaths) -> Result<(), String> {
    if !paths.uv_binary.is_file() {
        return Err(format!(
            "managed uv is missing at {}",
            paths.uv_binary.display()
        ));
    }
    let mut command = Command::new(&paths.uv_binary);
    command.arg("--version");
    let result = run_with_timeout(&mut command, PROBE_TIMEOUT)?;
    if !result.status.success() {
        return Err(command_failure("uv --version", &result));
    }
    let output = format!("{}\n{}", result.stdout, result.stderr);
    let expected = format!("uv {UV_VERSION}");
    if !output.contains(&expected) {
        return Err(format!(
            "managed uv version mismatch: expected {expected}, got {}",
            output.trim()
        ));
    }
    Ok(())
}

fn install_uv(paths: &NativeMempalacePaths) -> Result<(), String> {
    let asset = uv_asset_for(std::env::consts::OS, std::env::consts::ARCH)?;
    let temp = tempfile::tempdir().map_err(|e| format!("create uv temp directory: {e}"))?;
    let archive_path = temp.path().join(asset.file_name);
    let expected_sha = if let Some(local) = std::env::var_os("CAPTAIN_MEMPALACE_UV_ARCHIVE") {
        let local = PathBuf::from(local);
        std::fs::copy(&local, &archive_path)
            .map_err(|e| format!("copy uv archive {}: {e}", local.display()))?;
        std::env::var("CAPTAIN_MEMPALACE_UV_SHA256").map_err(|_| {
            "CAPTAIN_MEMPALACE_UV_SHA256 is required with CAPTAIN_MEMPALACE_UV_ARCHIVE".to_string()
        })?
    } else {
        let url = format!(
            "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{}",
            asset.file_name
        );
        download_file(&url, &archive_path)?;
        asset.sha256.to_string()
    };
    verify_sha256(&archive_path, expected_sha.trim())?;

    let staged_uv = temp
        .path()
        .join(if cfg!(windows) { "uv.exe" } else { "uv" });
    extract_uv(&archive_path, &staged_uv, asset.archive_kind)?;
    set_executable(&staged_uv)?;
    let staged_target = paths.uv_dir.join(format!(".uv-{}", uuid::Uuid::new_v4()));
    std::fs::copy(&staged_uv, &staged_target)
        .map_err(|e| format!("stage managed uv at {}: {e}", staged_target.display()))?;
    set_executable(&staged_target)?;
    File::open(&staged_target)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("sync staged uv {}: {e}", staged_target.display()))?;
    atomic_replace_file(&staged_target, &paths.uv_binary)
}

fn install_mempalace_tool(paths: &NativeMempalacePaths) -> Result<(), String> {
    write_locked_runtime_project(paths)?;
    let mut command = Command::new(&paths.uv_binary);
    command
        .arg("sync")
        .arg("--frozen")
        .arg("--no-dev")
        .arg("--project")
        .arg(&paths.project_dir)
        .arg("--python")
        .arg(PYTHON_VERSION);
    configure_uv_environment(&mut command, paths);
    let result = run_with_timeout(&mut command, INSTALL_TIMEOUT)?;
    require_success("uv sync frozen MemPalace runtime", result)
}

fn write_locked_runtime_project(paths: &NativeMempalacePaths) -> Result<(), String> {
    let actual_lock_hash = format!("{:x}", Sha256::digest(MEMPALACE_UV_LOCK));
    if actual_lock_hash != MEMPALACE_LOCK_SHA256 {
        return Err(format!(
            "embedded MemPalace lock hash mismatch: expected {MEMPALACE_LOCK_SHA256}, got {actual_lock_hash}"
        ));
    }
    create_private_dir(&paths.project_dir)?;
    std::fs::write(
        paths.project_dir.join("pyproject.toml"),
        MEMPALACE_PYPROJECT,
    )
    .map_err(|e| format!("write managed MemPalace pyproject: {e}"))?;
    let pyproject = paths.project_dir.join("pyproject.toml");
    let lock = paths.project_dir.join("uv.lock");
    set_private_file_permissions(&pyproject)?;
    std::fs::write(&lock, MEMPALACE_UV_LOCK)
        .map_err(|e| format!("write managed MemPalace lock: {e}"))?;
    set_private_file_permissions(&lock)
}

fn validate_mempalace_version(
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
) -> Result<(), String> {
    let mut command = Command::new(&paths.mempalace_binary);
    command.arg("--version");
    configure_mempalace_environment(&mut command, paths, metadata, false);
    let result = run_with_timeout(&mut command, PROBE_TIMEOUT)?;
    if !result.status.success() {
        return Err(command_failure("mempalace --version", &result));
    }
    let output = format!("{}\n{}", result.stdout, result.stderr);
    if !output.contains(MEMPALACE_VERSION) {
        return Err(format!(
            "managed MemPalace version mismatch: expected {MEMPALACE_VERSION}, got {}",
            output.trim()
        ));
    }
    Ok(())
}

fn validate_python_version(
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
) -> Result<(), String> {
    let mut command = Command::new(&paths.python_binary);
    command.arg("--version");
    configure_mempalace_environment(&mut command, paths, metadata, false);
    let result = run_with_timeout(&mut command, PROBE_TIMEOUT)?;
    if !result.status.success() {
        return Err(command_failure("managed python --version", &result));
    }
    let output = format!("{}\n{}", result.stdout, result.stderr);
    let expected = format!("Python {PYTHON_VERSION}");
    if !output.contains(&expected) {
        return Err(format!(
            "managed Python version mismatch: expected {expected}, got {}",
            output.trim()
        ));
    }
    Ok(())
}

fn initialize_palace_if_needed(
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
) -> Result<(), String> {
    if native_mempalace::palace_is_initialized(&metadata.palace_path) {
        return Ok(());
    }
    let source_dir = paths
        .captain_home
        .join("data")
        .join("mempalace")
        .join("bootstrap");
    create_private_dir(&source_dir)?;
    let bootstrap_document = source_dir.join("CAPTAIN_MEMORY.md");
    std::fs::write(
        &bootstrap_document,
        "# Captain Memory\n\nManaged local memory runtime readiness document.\n",
    )
    .map_err(|e| format!("write MemPalace bootstrap source: {e}"))?;
    set_private_file_permissions(&bootstrap_document)?;

    let mut command = Command::new(&paths.mempalace_binary);
    command
        .arg("--palace")
        .arg(&metadata.palace_path)
        .arg("init")
        .arg("--backend")
        .arg("chroma")
        .arg("--yes")
        .arg("--auto-mine")
        .arg("--no-llm")
        .arg(&source_dir);
    configure_mempalace_environment(&mut command, paths, metadata, false);
    let result = run_with_timeout(&mut command, INSTALL_TIMEOUT)?;
    require_success("initialize managed MemPalace palace", result)?;
    if !native_mempalace::palace_is_initialized(&metadata.palace_path) {
        return Err(format!(
            "MemPalace init completed without creating palace storage at {}",
            metadata.palace_path.display()
        ));
    }
    Ok(())
}

fn validate_palace(
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
) -> Result<(), String> {
    validate_palace_with_timeouts(paths, metadata, PROBE_TIMEOUT, INSTALL_TIMEOUT)
}

fn validate_palace_with_timeouts(
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
    status_timeout: Duration,
    search_timeout: Duration,
) -> Result<(), String> {
    let mut status = Command::new(&paths.mempalace_binary);
    status
        .arg("--palace")
        .arg(&metadata.palace_path)
        .arg("status");
    configure_mempalace_environment(&mut status, paths, metadata, false);
    require_success(
        "managed MemPalace status probe",
        run_with_timeout(&mut status, status_timeout)?,
    )?;

    let mut search = Command::new(&paths.mempalace_binary);
    search
        .arg("--palace")
        .arg(&metadata.palace_path)
        .arg("search")
        .arg("Captain managed local memory runtime readiness");
    configure_mempalace_environment(&mut search, paths, metadata, false);
    let result = run_with_timeout(&mut search, search_timeout)?;
    require_success("managed MemPalace semantic search probe", result)
}

fn live_probe() -> Result<String, String> {
    let paths = native_mempalace::default_paths();
    let metadata = native_mempalace::resolved_metadata()?;
    let mut command = Command::new(&paths.mempalace_binary);
    command.arg("--version");
    configure_mempalace_environment(&mut command, &paths, &metadata, false);
    let result = run_with_timeout(&mut command, LIVE_PROBE_TIMEOUT)?;
    if !result.status.success() {
        return Err(command_failure("mempalace --version", &result));
    }
    let output = if result.stdout.trim().is_empty() {
        result.stderr.trim()
    } else {
        result.stdout.trim()
    };
    if !output.contains(MEMPALACE_VERSION) {
        return Err(format!(
            "expected MemPalace {MEMPALACE_VERSION}, got {output}"
        ));
    }
    validate_palace_with_timeouts(&paths, &metadata, LIVE_PROBE_TIMEOUT, LIVE_PROBE_TIMEOUT)?;
    Ok(output.to_string())
}

fn ensure_integration_enabled() -> Result<(), String> {
    use captain_extensions::registry::IntegrationRegistry;
    use captain_extensions::InstalledIntegration;
    use std::collections::HashMap;

    let home = native_mempalace::captain_home_dir();
    let mut registry = IntegrationRegistry::new(&home);
    registry.load_bundled();
    registry
        .load_installed()
        .map_err(|e| format!("load integration registry: {e}"))?;
    if registry.is_installed("mempalace") {
        registry
            .set_enabled("mempalace", true)
            .map_err(|e| format!("enable MemPalace integration: {e}"))?;
        return Ok(());
    }
    registry
        .install(InstalledIntegration {
            id: "mempalace".to_string(),
            installed_at: chrono::Utc::now(),
            enabled: true,
            oauth_provider: None,
            config: HashMap::new(),
        })
        .map_err(|e| format!("register native MemPalace integration: {e}"))
}

fn configure_uv_environment(command: &mut Command, paths: &NativeMempalacePaths) {
    command
        .env("UV_PROJECT_ENVIRONMENT", &paths.tool_dir)
        .env("UV_PYTHON_INSTALL_DIR", &paths.python_dir)
        .env("UV_CACHE_DIR", &paths.cache_dir)
        .env("UV_NO_CONFIG", "1")
        .env("UV_MANAGED_PYTHON", "1")
        .env("UV_HTTP_TIMEOUT", "60")
        .env("UV_HTTP_RETRIES", "3");
}

fn configure_mempalace_environment(
    command: &mut Command,
    paths: &NativeMempalacePaths,
    metadata: &NativeMempalaceMetadata,
    isolated: bool,
) {
    let preserved = if isolated {
        [
            "PATH",
            "LANG",
            "LC_ALL",
            "TZ",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "REQUESTS_CA_BUNDLE",
            "CURL_CA_BUNDLE",
            "SystemRoot",
            "TEMP",
            "TMP",
        ]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key, value)))
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if isolated {
        command.env_clear();
        for (key, value) in preserved {
            command.env(key, value);
        }
    }
    configure_uv_environment(command, paths);
    command
        .env("CAPTAIN_HOME", &paths.captain_home)
        .env("HOME", &metadata.data_home)
        .env("USERPROFILE", &metadata.data_home)
        .env("XDG_CACHE_HOME", metadata.data_home.join(".cache"))
        .env(
            "HF_HOME",
            metadata.data_home.join(".cache").join("huggingface"),
        )
        .env("HF_HUB_DOWNLOAD_TIMEOUT", "60")
        .env("HF_HUB_ETAG_TIMEOUT", "30")
        .env("MEMPALACE_PALACE_PATH", &metadata.palace_path)
        .env("MEMPALACE_MCP_IDLE_HOURS", "0")
        .env(
            "MEMPALACE_LOG_FILE",
            paths.captain_home.join("logs").join("mempalace.log"),
        );
}

fn uv_asset_for(os: &str, arch: &str) -> Result<UvAsset, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(UvAsset {
            file_name: "uv-aarch64-apple-darwin.tar.gz",
            sha256: "33540eb7c883ab857eff79bd5ac2aa31fe27b595abecb4a9c003a2c998447232",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("macos", "x86_64") => Ok(UvAsset {
            file_name: "uv-x86_64-apple-darwin.tar.gz",
            sha256: "2ad79983127ffca7d77b77ce6a24278d7e4f7b817a1acf72fea5f8124b4aac5e",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("linux", "aarch64") => Ok(UvAsset {
            file_name: "uv-aarch64-unknown-linux-gnu.tar.gz",
            sha256: "03e9fe0a81b0718d0bc84625de3885df6cc3f89a8b6af6121d6b9f6113fb6533",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("linux", "x86_64") => Ok(UvAsset {
            file_name: "uv-x86_64-unknown-linux-gnu.tar.gz",
            sha256: "e490a6464492183c5d4534a5527fb4440f7f2bb2f228162ad7e4afe076dc0224",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("windows", "x86_64") => Ok(UvAsset {
            file_name: "uv-x86_64-pc-windows-msvc.zip",
            sha256: "0a23463216d09c6a72ff80ef5dc5a795f07dc1575cb84d24596c2f124a441b7b",
            archive_kind: ArchiveKind::Zip,
        }),
        _ => Err(format!(
            "unsupported managed MemPalace platform: {os}/{arch}"
        )),
    }
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(5 * 60))
        .user_agent("captain-managed-mempalace")
        .build()
        .map_err(|e| format!("build MemPalace download client: {e}"))?;
    let mut last_error = String::new();
    for attempt in 1..=3 {
        match client
            .get(url)
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(mut response) => {
                if response
                    .content_length()
                    .is_some_and(|size| size > DOWNLOAD_LIMIT_BYTES)
                {
                    return Err(format!("uv archive exceeds {DOWNLOAD_LIMIT_BYTES} bytes"));
                }
                let mut output = File::create(destination)
                    .map_err(|e| format!("create {}: {e}", destination.display()))?;
                let mut limited = response.by_ref().take(DOWNLOAD_LIMIT_BYTES + 1);
                let copied = std::io::copy(&mut limited, &mut output)
                    .map_err(|e| format!("download {url}: {e}"))?;
                if copied == 0 || copied > DOWNLOAD_LIMIT_BYTES {
                    return Err(format!("invalid uv archive size: {copied} bytes"));
                }
                output
                    .sync_all()
                    .map_err(|e| format!("sync {}: {e}", destination.display()))?;
                return Ok(());
            }
            Err(error) => {
                last_error = error.to_string();
                if attempt < 3 {
                    std::thread::sleep(Duration::from_secs(attempt));
                }
            }
        }
    }
    Err(format!("download failed for {url}: {last_error}"))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "uv archive checksum mismatch: expected {expected}, got {actual}"
        ))
    }
}

fn extract_uv(archive: &Path, output: &Path, kind: ArchiveKind) -> Result<(), String> {
    match kind {
        ArchiveKind::TarGz => extract_uv_tar_gz(archive, output),
        ArchiveKind::Zip => extract_uv_zip(archive, output),
    }
}

fn extract_uv_tar_gz(archive: &Path, output: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("open uv archive: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries().map_err(|e| format!("read uv tar: {e}"))? {
        let mut entry = entry.map_err(|e| format!("read uv tar entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| format!("read uv tar path: {e}"))?;
        if path.file_name().and_then(|name| name.to_str()) != Some("uv") {
            continue;
        }
        let mut target = File::create(output)
            .map_err(|e| format!("create extracted uv {}: {e}", output.display()))?;
        std::io::copy(&mut entry, &mut target).map_err(|e| format!("extract uv binary: {e}"))?;
        target
            .sync_all()
            .map_err(|e| format!("sync extracted uv: {e}"))?;
        return Ok(());
    }
    Err("uv archive did not contain the uv executable".to_string())
}

fn extract_uv_zip(archive: &Path, output: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("open uv zip: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("read uv zip: {e}"))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("read uv zip entry: {e}"))?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if path.file_name().and_then(|name| name.to_str()) != Some("uv.exe") {
            continue;
        }
        let mut target = File::create(output)
            .map_err(|e| format!("create extracted uv {}: {e}", output.display()))?;
        std::io::copy(&mut entry, &mut target)
            .map_err(|e| format!("extract uv executable: {e}"))?;
        target
            .sync_all()
            .map_err(|e| format!("sync extracted uv: {e}"))?;
        return Ok(());
    }
    Err("uv archive did not contain uv.exe".to_string())
}

fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<CommandResult, String> {
    let stdout = tempfile::NamedTempFile::new().map_err(|e| format!("temp stdout: {e}"))?;
    let stderr = tempfile::NamedTempFile::new().map_err(|e| format!("temp stderr: {e}"))?;
    command
        .stdout(Stdio::from(
            stdout.reopen().map_err(|e| format!("reopen stdout: {e}"))?,
        ))
        .stderr(Stdio::from(
            stderr.reopen().map_err(|e| format!("reopen stderr: {e}"))?,
        ));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("launch {}: {e}", command.get_program().to_string_lossy()))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("wait for managed command: {e}"))?
        {
            break status;
        }
        if started.elapsed() >= timeout {
            terminate_managed_process_tree(&mut child);
            return Err(format!(
                "managed command timed out after {} seconds: {}",
                timeout.as_secs(),
                command.get_program().to_string_lossy()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let stdout_text = std::fs::read_to_string(stdout.path()).unwrap_or_default();
    let stderr_text = std::fs::read_to_string(stderr.path()).unwrap_or_default();
    Ok(CommandResult {
        status,
        stdout: stdout_text,
        stderr: stderr_text,
    })
}

fn terminate_managed_process_tree(child: &mut std::process::Child) {
    let pid = child.id();
    let killed = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(captain_runtime::terminate_process_tree(pid, 1_000))
    })
    .join();
    if !matches!(killed, Ok(Ok(_))) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn require_success(context: &str, result: CommandResult) -> Result<(), String> {
    if result.status.success() {
        Ok(())
    } else {
        Err(command_failure(context, &result))
    }
}

fn command_failure(context: &str, result: &CommandResult) -> String {
    let detail = if result.stderr.trim().is_empty() {
        result.stdout.trim()
    } else {
        result.stderr.trim()
    };
    format!(
        "{context} failed with {}: {}",
        result.status,
        cap_error(detail, 2_000)
    )
}

fn cap_error(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push_str("...");
    output
}

fn write_metadata_atomic(path: &Path, metadata: &NativeMempalaceMetadata) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("metadata path has no parent: {}", path.display()))?;
    create_private_dir(parent)?;
    let temp = parent.join(format!(".install-{}.json", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|e| format!("serialize MemPalace metadata: {e}"))?;
    {
        let mut file = create_private_file(&temp)?;
        file.write_all(&bytes)
            .map_err(|e| format!("write MemPalace metadata: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("sync MemPalace metadata: {e}"))?;
    }
    atomic_replace_file(&temp, path)
}

fn restore_metadata(path: &Path, previous: Option<&NativeMempalaceMetadata>) -> Result<(), String> {
    if let Some(previous) = previous {
        return write_metadata_atomic(path, previous);
    }
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| format!("remove failed MemPalace metadata {}: {e}", path.display()))?;
        if let Some(parent) = path.parent() {
            sync_parent_directory(parent)?;
        }
    }
    Ok(())
}

fn write_generation_complete_marker(paths: &NativeMempalacePaths) -> Result<(), String> {
    let marker = paths.generation_dir.join("COMPLETE");
    let mut file = create_private_file(&marker)?;
    writeln!(
        file,
        "mempalace={MEMPALACE_VERSION}\npython={PYTHON_VERSION}\nlock_sha256={MEMPALACE_LOCK_SHA256}"
    )
    .map_err(|e| format!("write MemPalace generation marker: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("sync MemPalace generation marker: {e}"))?;
    sync_parent_directory(&paths.generation_dir)
}

#[cfg(unix)]
fn atomic_replace_file(staged: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(staged, destination).map_err(|e| {
        format!(
            "atomically activate {} as {}: {e}",
            staged.display(),
            destination.display()
        )
    })?;
    if let Some(parent) = destination.parent() {
        sync_parent_directory(parent)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn atomic_replace_file(staged: &Path, destination: &Path) -> Result<(), String> {
    let backup = destination.with_extension(format!("backup-{}", uuid::Uuid::new_v4()));
    let had_destination = destination.exists();
    if had_destination {
        std::fs::rename(destination, &backup).map_err(|e| {
            format!(
                "stage existing file {} for replacement: {e}",
                destination.display()
            )
        })?;
    }
    if let Err(error) = std::fs::rename(staged, destination) {
        if had_destination {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(format!(
            "activate {} as {}: {error}",
            staged.display(),
            destination.display()
        ));
    }
    if had_destination {
        let _ = std::fs::remove_file(backup);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| format!("sync directory {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|e| format!("metadata {}: {e}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|e| format!("chmod {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|e| format!("create private directory {}: {e}", path.display()))?;
    set_private_dir_permissions(path)
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("restrict directory permissions {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("create private file {}: {e}", path.display()))?;
    set_private_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("restrict file permissions {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_assets_are_pinned_for_every_release_platform() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
            ("windows", "x86_64"),
        ] {
            let asset = uv_asset_for(os, arch).unwrap();
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(asset.file_name.starts_with("uv-"));
        }
        assert!(uv_asset_for("windows", "aarch64").is_err());
    }

    #[test]
    fn checksum_verifier_accepts_exact_content_only() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"captain-memory").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"captain-memory"));
        assert!(verify_sha256(temp.path(), &digest).is_ok());
        assert!(verify_sha256(temp.path(), &"0".repeat(64)).is_err());
    }

    #[test]
    fn embedded_mempalace_lock_matches_runtime_pin() {
        assert_eq!(
            format!("{:x}", Sha256::digest(MEMPALACE_UV_LOCK)),
            MEMPALACE_LOCK_SHA256
        );
        let lock = std::str::from_utf8(MEMPALACE_UV_LOCK).unwrap();
        assert!(lock.contains("name = \"mempalace\""));
        assert!(lock.contains("version = \"3.5.0\""));
        assert!(lock.contains("hash = \"sha256:"));
    }

    #[test]
    fn install_lock_rejects_concurrent_repairs_and_recovers_after_drop() {
        let temp = tempfile::tempdir().unwrap();
        let first = InstallLock::acquire(temp.path()).unwrap();
        assert!(InstallLock::acquire(temp.path()).is_err());
        drop(first);
        assert!(InstallLock::acquire(temp.path()).is_ok());
    }

    #[test]
    fn uncommitted_generation_is_removed_but_committed_generation_survives() {
        let temp = tempfile::tempdir().unwrap();
        let failed = temp.path().join("failed");
        {
            let _pending = PendingGeneration::new(failed.clone()).unwrap();
        }
        assert!(!failed.exists());

        let committed = temp.path().join("committed");
        {
            let mut pending = PendingGeneration::new(committed.clone()).unwrap();
            pending.commit();
        }
        assert!(committed.is_dir());
    }

    #[test]
    fn atomic_file_activation_replaces_complete_content() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("active");
        let staged = temp.path().join("staged");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&staged, b"new-complete").unwrap();

        atomic_replace_file(&staged, &destination).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"new-complete");
        assert!(!staged.exists());
    }

    #[test]
    fn automatic_install_requires_an_explicit_opt_out() {
        assert!(mempalace_install_enabled(None));
        assert!(mempalace_install_enabled(Some("1")));
        assert!(mempalace_install_enabled(Some("yes")));
        assert!(!mempalace_install_enabled(Some("0")));
        assert!(!mempalace_install_enabled(Some(" FALSE ")));
        assert!(!mempalace_install_enabled(Some("No")));
    }

    #[test]
    fn readiness_labels_distinguish_missing_from_degraded() {
        assert_eq!(readiness_label(true, true), "ready");
        assert_eq!(readiness_label(false, true), "degraded");
        assert_eq!(readiness_label(false, false), "missing");
    }

    #[test]
    fn generation_pruning_keeps_active_and_rollback_only() {
        let temp = tempfile::tempdir().unwrap();
        let mut paths = native_mempalace::default_paths();
        paths.generations_dir = temp.path().to_path_buf();
        let active = uuid::Uuid::new_v4().to_string();
        let previous = uuid::Uuid::new_v4().to_string();
        let obsolete = uuid::Uuid::new_v4().to_string();
        for generation in [&active, &previous, &obsolete] {
            let directory = temp.path().join(generation);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("COMPLETE"), b"complete").unwrap();
        }
        let unmanaged = temp.path().join("operator-data");
        std::fs::create_dir(&unmanaged).unwrap();
        std::fs::write(unmanaged.join("COMPLETE"), b"do not touch").unwrap();

        prune_obsolete_generations(&paths, &active, Some(&previous));

        assert!(temp.path().join(active).is_dir());
        assert!(temp.path().join(previous).is_dir());
        assert!(!temp.path().join(obsolete).exists());
        assert!(unmanaged.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn managed_memory_paths_are_private_by_default() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("private");
        create_private_dir(&directory).unwrap();
        let file_path = directory.join("private.json");
        drop(create_private_file(&file_path).unwrap());

        let directory_mode = std::fs::metadata(directory).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(file_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn permission_only_repair_reuses_the_existing_generation() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let mut base = native_mempalace::default_paths();
        base.captain_home = temp.path().to_path_buf();
        base.runtime_dir = temp.path().join("native/mempalace");
        base.uv_dir = base.runtime_dir.join("uv");
        base.metadata_file = base.runtime_dir.join("install.json");
        base.generations_dir = base.runtime_dir.join("generations");
        let generation = uuid::Uuid::new_v4().to_string();
        let paths = native_mempalace::paths_for_generation(&base, &generation).unwrap();
        let data_home = temp.path().join("data/mempalace/home");
        let palace_path = data_home.join(".mempalace/palace");
        let metadata = NativeMempalaceMetadata {
            schema_version: METADATA_SCHEMA_VERSION,
            mempalace_version: MEMPALACE_VERSION.into(),
            uv_version: UV_VERSION.into(),
            python_version: PYTHON_VERSION.into(),
            lock_sha256: MEMPALACE_LOCK_SHA256.into(),
            runtime_generation: generation.clone(),
            platform: native_mempalace::runtime_platform(),
            installed_at: "now".into(),
            data_home,
            palace_path,
            legacy_data_preserved: false,
        };
        let mempalace_home = metadata.data_home.join(".mempalace");
        for directory in [
            &paths.runtime_dir,
            &paths.uv_dir,
            &paths.generations_dir,
            &paths.generation_dir,
            &paths.project_dir,
            &paths.tool_dir,
            &paths.python_dir,
            &mempalace_home,
            &metadata.palace_path,
        ] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(&paths.metadata_file, b"metadata").unwrap();
        std::fs::set_permissions(&paths.metadata_file, std::fs::Permissions::from_mode(0o644))
            .unwrap();

        harden_existing_install_permissions(&paths, &metadata).unwrap();

        assert_eq!(
            paths.generation_dir.file_name().unwrap(),
            generation.as_str()
        );
        assert_eq!(
            std::fs::metadata(&paths.runtime_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&metadata.palace_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&paths.metadata_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_command_timeout_terminates_the_process_group() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);

        let error = run_with_timeout(&mut command, Duration::from_millis(50)).unwrap_err();

        assert!(error.contains("timed out"));
    }
}
