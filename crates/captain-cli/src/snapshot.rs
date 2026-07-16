use std::path::{Path, PathBuf};

use crate::commands::daemon::cmd_stop_result;
use crate::{
    captain_version, cli_captain_home, copy_dir_recursive, find_daemon, prompt_input,
    restrict_dir_permissions, restrict_file_permissions, ui,
};

fn unix_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn unix_timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn snapshot_dir() -> PathBuf {
    cli_captain_home().join("snapshots")
}

fn snapshot_archive_path(id_or_path: &str) -> PathBuf {
    let path = PathBuf::from(id_or_path);
    if path.exists() {
        return path;
    }
    let name = if id_or_path.ends_with(".tar.gz") {
        id_or_path.to_string()
    } else {
        format!("{id_or_path}.tar.gz")
    };
    snapshot_dir().join(name)
}

fn create_snapshot(reason: Option<&str>) -> Result<PathBuf, String> {
    let captain_dir = cli_captain_home();
    if !captain_dir.exists() {
        return Err(format!(
            "Captain home does not exist: {}",
            captain_dir.display()
        ));
    }
    let snapshots = snapshot_dir();
    std::fs::create_dir_all(&snapshots)
        .map_err(|e| format!("create {}: {e}", snapshots.display()))?;

    let id = format!("snapshot-{}", unix_timestamp_millis());
    let archive = snapshots.join(format!("{id}.tar.gz"));
    let status = std::process::Command::new("tar")
        .args([
            "-czf",
            &archive.display().to_string(),
            "--exclude",
            "./snapshots",
            "-C",
            &captain_dir.display().to_string(),
            ".",
        ])
        .status()
        .map_err(|e| format!("run tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar failed with status {status}"));
    }

    let meta = serde_json::json!({
        "id": id,
        "archive": archive.display().to_string(),
        "captain_home": captain_dir.display().to_string(),
        "created_at_unix": unix_timestamp_secs(),
        "reason": reason.unwrap_or("manual"),
        "version": captain_version(),
    });
    let meta_path = snapshots.join(format!("{id}.json"));
    let _ = std::fs::write(
        &meta_path,
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    );
    Ok(archive)
}

pub(crate) fn cmd_snapshot_create(reason: Option<&str>, json: bool) {
    match create_snapshot(reason) {
        Ok(path) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status": "ok", "archive": path.display().to_string()})
                );
            } else {
                ui::success(&format!("Snapshot created: {}", path.display()));
            }
        }
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({"status": "fail", "error": e}));
            } else {
                ui::error(&format!("Snapshot failed: {e}"));
            }
            std::process::exit(1);
        }
    }
}

fn snapshot_entries() -> Vec<(PathBuf, u64, u64)> {
    let mut out = Vec::new();
    let dir = snapshot_dir();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("gz") {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push((path, modified, meta.len()));
        }
    }
    out.sort_by_key(|o| std::cmp::Reverse(o.1));
    out
}

pub(crate) fn cmd_snapshot_list(json: bool) {
    let entries = snapshot_entries();
    if json {
        let rows: Vec<_> = entries
            .iter()
            .map(|(path, modified, size)| {
                let id = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_suffix(".tar.gz"))
                    .unwrap_or("");
                serde_json::json!({
                    "id": id,
                    "path": path.display().to_string(),
                    "modified_unix": modified,
                    "size_bytes": size,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_default()
        );
        return;
    }
    if entries.is_empty() {
        println!("No snapshots found.");
        return;
    }
    println!("Snapshots:");
    for (path, modified, size) in entries {
        println!(
            "  {}  {} bytes  modified={}",
            path.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
            size,
            modified
        );
    }
}

fn rollback_snapshot_restore(captain_dir: &Path, backup_dir: &Path) {
    let _ = std::fs::remove_dir_all(captain_dir);
    if backup_dir.exists() {
        if let Err(e) = std::fs::rename(backup_dir, captain_dir) {
            ui::error(&format!(
                "Restore rollback failed. Previous state remains at {}: {e}",
                backup_dir.display()
            ));
        } else {
            ui::check_warn("Restore failed; previous Captain home was restored.");
        }
    }
}

fn stop_daemon_or_abort_destructive_operation(operation_label: &str) {
    if find_daemon().is_none() {
        return;
    }

    println!("  Stopping running daemon...");
    if !cmd_stop_result() {
        ui::warn_with_fix(
            &format!("{operation_label} deferred because Captain is still running."),
            "Run `captain status` to inspect active work, then retry after the daemon stops cleanly.",
        );
        std::process::exit(1);
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    if find_daemon().is_some() {
        ui::warn_with_fix(
            &format!("{operation_label} deferred because the daemon still answers health checks."),
            "Run `captain status` before retrying; Captain will not mutate data under a live daemon.",
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_snapshot_restore(id: &str, confirm: bool) {
    let archive = snapshot_archive_path(id);
    if !archive.exists() {
        ui::error(&format!("Snapshot not found: {}", archive.display()));
        std::process::exit(1);
    }
    if !confirm {
        println!("  This will replace {}", cli_captain_home().display());
        println!("  Snapshot: {}", archive.display());
        let answer = prompt_input("  Type 'restore' to confirm: ");
        if answer.trim() != "restore" {
            println!("  Cancelled.");
            return;
        }
    }

    stop_daemon_or_abort_destructive_operation("Snapshot restore");

    let temp_archive =
        std::env::temp_dir().join(format!("captain-restore-{}.tar.gz", unix_timestamp_secs()));
    if let Err(e) = std::fs::copy(&archive, &temp_archive) {
        ui::error(&format!("Failed to stage snapshot: {e}"));
        std::process::exit(1);
    }

    let captain_dir = cli_captain_home();
    let backup_dir =
        captain_dir.with_file_name(format!(".captain.restore-backup-{}", unix_timestamp_secs()));
    if captain_dir.exists() {
        if let Err(e) = std::fs::rename(&captain_dir, &backup_dir) {
            ui::error(&format!("Failed to move current home aside: {e}"));
            std::process::exit(1);
        }
    }
    if let Err(e) = std::fs::create_dir_all(&captain_dir) {
        rollback_snapshot_restore(&captain_dir, &backup_dir);
        ui::error(&format!(
            "Failed to recreate {}: {e}",
            captain_dir.display()
        ));
        std::process::exit(1);
    }
    restrict_dir_permissions(&captain_dir);

    let status = std::process::Command::new("tar")
        .args([
            "-xzf",
            &temp_archive.display().to_string(),
            "-C",
            &captain_dir.display().to_string(),
        ])
        .status();
    match status {
        Ok(s) if s.success() => {
            ui::success(&format!("Restored snapshot into {}", captain_dir.display()));
            if backup_dir.exists() {
                ui::hint(&format!("Previous state kept at {}", backup_dir.display()));
            }
        }
        Ok(s) => {
            rollback_snapshot_restore(&captain_dir, &backup_dir);
            ui::error(&format!("tar restore failed with status {s}"));
            std::process::exit(1);
        }
        Err(e) => {
            rollback_snapshot_restore(&captain_dir, &backup_dir);
            ui::error(&format!("Failed to run tar: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_snapshot_prune(keep: usize, dry_run: bool, confirm: bool) {
    let entries = snapshot_entries();
    let to_delete = entries.into_iter().skip(keep).collect::<Vec<_>>();
    if to_delete.is_empty() {
        println!("No snapshots to prune.");
        return;
    }
    if !dry_run && !confirm {
        println!("  {} snapshot(s) will be deleted.", to_delete.len());
        let answer = prompt_input("  Type 'prune' to confirm: ");
        if answer.trim() != "prune" {
            println!("  Cancelled.");
            return;
        }
    }
    for (path, _, _) in to_delete {
        if dry_run {
            println!("would delete {}", path.display());
        } else {
            let _ = std::fs::remove_file(&path);
            let id = path
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_suffix(".tar.gz"))
                .unwrap_or("");
            let sidecar = path.with_file_name(format!("{id}.json"));
            let _ = std::fs::remove_file(sidecar);
            println!("deleted {}", path.display());
        }
    }
}

fn preserve_reset_artifacts(
    captain_dir: &Path,
    preserve_secrets: bool,
    preserve_snapshots: bool,
) -> Result<Option<PathBuf>, String> {
    if !preserve_secrets && !preserve_snapshots {
        return Ok(None);
    }
    let staging =
        std::env::temp_dir().join(format!("captain-reset-preserve-{}", unix_timestamp_secs()));
    std::fs::create_dir_all(&staging).map_err(|e| format!("create staging: {e}"))?;
    if preserve_secrets {
        for name in ["config.toml", ".env", "secrets.env", "vault.enc"] {
            let src = captain_dir.join(name);
            if src.exists() {
                let _ = std::fs::copy(&src, staging.join(name));
            }
        }
    }
    if preserve_snapshots {
        let src = captain_dir.join("snapshots");
        if src.exists() {
            copy_dir_recursive(&src, &staging.join("snapshots"));
        }
    }
    Ok(Some(staging))
}

fn restore_reset_artifacts(captain_dir: &Path, staging: Option<PathBuf>) {
    let Some(staging) = staging else {
        return;
    };
    let _ = std::fs::create_dir_all(captain_dir);
    for name in ["config.toml", ".env", "secrets.env", "vault.enc"] {
        let src = staging.join(name);
        if src.exists() {
            let dst = captain_dir.join(name);
            let _ = std::fs::copy(&src, &dst);
            restrict_file_permissions(&dst);
        }
    }
    let snapshots = staging.join("snapshots");
    if snapshots.exists() {
        copy_dir_recursive(&snapshots, &captain_dir.join("snapshots"));
    }
    let _ = std::fs::remove_dir_all(staging);
}

pub(crate) fn cmd_reset(
    confirm: bool,
    factory: bool,
    no_snapshot: bool,
    preserve_secrets: bool,
    preserve_snapshots: bool,
) {
    let captain_dir = cli_captain_home();

    if !captain_dir.exists() {
        println!(
            "Nothing to reset — {} does not exist.",
            captain_dir.display()
        );
        return;
    }

    let snapshot_before = factory && !no_snapshot;
    let keep_snapshots = preserve_snapshots || snapshot_before;

    if !confirm {
        if factory {
            println!("  This will factory-reset {}", captain_dir.display());
            println!("  The Captain CLI remains installed.");
        } else {
            println!("  This will delete all data in {}", captain_dir.display());
        }
        println!("  Including: config, database, agent manifests, credentials.");
        if snapshot_before {
            println!("  A recovery snapshot will be created first.");
        }
        println!();
        let answer = prompt_input("  Are you sure? Type 'yes' to confirm: ");
        if answer.trim() != "yes" {
            println!("  Cancelled.");
            return;
        }
    }

    if factory {
        stop_daemon_or_abort_destructive_operation("Factory reset");
    }

    if snapshot_before {
        match create_snapshot(Some("factory-reset")) {
            Ok(path) => ui::success(&format!("Recovery snapshot: {}", path.display())),
            Err(e) => {
                ui::error(&format!("Cannot create recovery snapshot: {e}"));
                std::process::exit(1);
            }
        }
    }

    let staging = preserve_reset_artifacts(&captain_dir, preserve_secrets, keep_snapshots)
        .unwrap_or_else(|e| {
            ui::error(&format!("Failed to preserve reset artifacts: {e}"));
            std::process::exit(1);
        });

    match std::fs::remove_dir_all(&captain_dir) {
        Ok(()) => ui::success(&format!("Removed {}", captain_dir.display())),
        Err(e) => {
            ui::error(&format!("Failed to remove {}: {e}", captain_dir.display()));
            std::process::exit(1);
        }
    }

    if factory {
        if let Err(e) = std::fs::create_dir_all(&captain_dir) {
            ui::error(&format!(
                "Failed to recreate {}: {e}",
                captain_dir.display()
            ));
            std::process::exit(1);
        }
        restrict_dir_permissions(&captain_dir);
        for sub in ["data", "agents"] {
            let _ = std::fs::create_dir_all(captain_dir.join(sub));
        }
        restore_reset_artifacts(&captain_dir, staging);
        ui::success("Factory reset complete. Run `captain setup` to configure a fresh install.");
    }
}
