use std::path::{Path, PathBuf};

use crate::ui;

pub(crate) fn cmd_embeddings_status(json: bool) {
    let status = captain_runtime::native_embeddings::status();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).unwrap_or_default()
        );
        return;
    }

    ui::section("Native Embeddings");
    ui::kv("Home", &status.home_dir);
    ui::kv_ok("Runtime", status.runtime);
    ui::kv("Version", status.version);
    ui::kv("Status", if status.ready { "ready" } else { "pending" });
    ui::kv("Library", status.library.as_deref().unwrap_or("-"));
    ui::kv(
        "ORT_DYLIB_PATH",
        status.ort_dylib_path.as_deref().unwrap_or("-"),
    );
    if !status.ready {
        ui::hint(status.install_hint);
    }
}

pub(crate) fn cmd_embeddings_doctor(json: bool) {
    cmd_embeddings_status(json);
}

pub(crate) fn cmd_embeddings_install(best_effort: bool, force: bool) {
    ui::section("Native Embeddings Install");
    println!("  Installing local embeddings runtime: ONNX Runtime CPU.");

    let result = install_native_embeddings_runtime(force);
    let status = captain_runtime::native_embeddings::status();
    if status.ready {
        ui::success("Native embeddings runtime ready.");
        warmup_local_embedding_model(best_effort);
        return;
    }

    if let Err(e) = result {
        ui::check_warn(&e);
    }
    if !best_effort {
        ui::error_with_fix(
            "Native embeddings install incomplete",
            "Run `captain embeddings doctor` then retry `captain embeddings install`.",
        );
        std::process::exit(1);
    }
}

/// Downloads the embedding model into CAPTAIN_HOME/.fastembed_cache right at
/// install time, so the daemon's first Tool RAG / memory search doesn't stall
/// on a ~90 MB HuggingFace download (or fail outright on an offline host).
#[cfg(feature = "local-embeddings")]
fn warmup_local_embedding_model(best_effort: bool) {
    println!("  Preloading local embedding model (first time downloads ~90 MB)...");
    match captain_runtime::embedding::create_local_embedding_driver("all-MiniLM-L6-v2") {
        Ok(_) => ui::success("Local embedding model cached."),
        Err(e) => {
            ui::check_warn(&format!("Embedding model preload failed: {e}"));
            if !best_effort {
                ui::error_with_fix(
                    "Native embeddings install incomplete",
                    "Retry `captain embeddings install` with network access.",
                );
                std::process::exit(1);
            }
        }
    }
}

#[cfg(not(feature = "local-embeddings"))]
fn warmup_local_embedding_model(_best_effort: bool) {}

fn install_native_embeddings_runtime(force: bool) -> Result<(), String> {
    let status = captain_runtime::native_embeddings::status();
    if status.ready && !force {
        return Ok(());
    }

    let paths = captain_runtime::native_embeddings::default_paths();
    std::fs::create_dir_all(&paths.runtime_dir)
        .map_err(|e| format!("create {}: {e}", paths.runtime_dir.display()))?;

    let url = onnxruntime_archive_url()?;
    let curl = find_command_path("curl").ok_or("curl not found; cannot download ONNX Runtime")?;
    let tar = find_command_path("tar").ok_or("tar not found; cannot extract ONNX Runtime")?;
    let temp_dir =
        std::env::temp_dir().join(format!("captain-onnxruntime-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("create temp dir: {e}"))?;
    let archive = temp_dir.join("onnxruntime.tgz");

    let download = std::process::Command::new(&curl)
        .args(["-L", "--fail", "--retry", "2", "-o"])
        .arg(&archive)
        .arg(&url)
        .output()
        .map_err(|e| format!("failed to launch curl: {e}"))?;
    if !download.status.success() {
        let stderr = String::from_utf8_lossy(&download.stderr);
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(format!("download failed for {url}: {}", stderr.trim()));
    }

    let extract = std::process::Command::new(&tar)
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&temp_dir)
        .output()
        .map_err(|e| format!("failed to launch tar: {e}"))?;
    if !extract.status.success() {
        let stderr = String::from_utf8_lossy(&extract.stderr);
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(format!("extract ONNX Runtime failed: {}", stderr.trim()));
    }

    let source = find_file_recursive(
        &temp_dir,
        captain_runtime::native_embeddings::versioned_library_name(),
    )
    .or_else(|| {
        find_file_recursive(
            &temp_dir,
            captain_runtime::native_embeddings::primary_library_name(),
        )
    })
    .ok_or_else(|| {
        format!(
            "ONNX Runtime archive did not contain {}",
            captain_runtime::native_embeddings::primary_library_name()
        )
    })?;

    std::fs::copy(&source, &paths.library)
        .map_err(|e| format!("install {}: {e}", paths.library.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&paths.library)
            .map_err(|e| format!("metadata {}: {e}", paths.library.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&paths.library, perms)
            .map_err(|e| format!("chmod {}: {e}", paths.library.display()))?;
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

fn onnxruntime_archive_url() -> Result<String, String> {
    let version = std::env::var("CAPTAIN_ONNXRUNTIME_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| captain_runtime::native_embeddings::ONNXRUNTIME_VERSION.to_string());
    let package = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "aarch64") => "osx-arm64",
        ("macos", "x86_64") => "osx-x86_64",
        (os, arch) => {
            return Err(format!(
                "unsupported native embeddings platform: {os}/{arch}"
            ));
        }
    };
    Ok(format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{version}/onnxruntime-{package}-{version}.tgz"
    ))
}

fn find_file_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(file_name) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, file_name) {
                return Some(found);
            }
        }
    }
    None
}

fn find_command_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
