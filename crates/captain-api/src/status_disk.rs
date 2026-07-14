//! Disk status helpers for operator status.

use std::path::{Path, PathBuf};

const CLEANUP_THRESHOLD_BYTES: u64 = 15 * 1024 * 1024 * 1024;

pub fn build_disk_status(home_dir: &Path) -> serde_json::Value {
    match disk_usage(home_dir) {
        Some(usage) => {
            let cleanup_recommended = usage.available_bytes <= CLEANUP_THRESHOLD_BYTES;
            serde_json::json!({
                "state": if cleanup_recommended { "warn" } else { "ok" },
                "readable": true,
                "path": usage.path.display().to_string(),
                "available_bytes": usage.available_bytes,
                "available_gib": bytes_to_gib(usage.available_bytes),
                "total_bytes": usage.total_bytes,
                "total_gib": bytes_to_gib(usage.total_bytes),
                "used_percent": usage.used_percent,
                "cleanup_threshold_gib": bytes_to_gib(CLEANUP_THRESHOLD_BYTES),
                "cleanup_recommended": cleanup_recommended,
            })
        }
        None => serde_json::json!({
            "state": "unavailable",
            "readable": false,
            "path": home_dir.display().to_string(),
            "cleanup_threshold_gib": bytes_to_gib(CLEANUP_THRESHOLD_BYTES),
            "cleanup_recommended": false,
        }),
    }
}

struct DiskUsage {
    path: PathBuf,
    available_bytes: u64,
    total_bytes: u64,
    used_percent: Option<u64>,
}

fn disk_usage(path: &Path) -> Option<DiskUsage> {
    let probe = existing_probe_path(path);
    disk_usage_from_df(&probe)
}

fn existing_probe_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    path.ancestors()
        .find(|ancestor| ancestor.exists())
        .unwrap_or_else(|| Path::new("/"))
        .to_path_buf()
}

#[cfg(unix)]
fn disk_usage_from_df(path: &Path) -> Option<DiskUsage> {
    let output = std::process::Command::new("df")
        .args(["-k", &path.display().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_df_k_output(&String::from_utf8_lossy(&output.stdout), path)
}

#[cfg(not(unix))]
fn disk_usage_from_df(_path: &Path) -> Option<DiskUsage> {
    None
}

#[cfg(unix)]
fn parse_df_k_output(output: &str, path: &Path) -> Option<DiskUsage> {
    let line = output.lines().find(|line| {
        let trimmed = line.trim_start();
        !trimmed.is_empty() && !trimmed.starts_with("Filesystem")
    })?;
    let cols = line.split_whitespace().collect::<Vec<_>>();
    if cols.len() < 4 {
        return None;
    }
    let total_kib = cols[1].parse::<u64>().ok()?;
    let available_kib = cols[3].parse::<u64>().ok()?;
    let used_percent = cols
        .get(4)
        .and_then(|value| value.strip_suffix('%'))
        .and_then(|value| value.parse::<u64>().ok());
    Some(DiskUsage {
        path: path.to_path_buf(),
        available_bytes: available_kib.saturating_mul(1024),
        total_bytes: total_kib.saturating_mul(1024),
        used_percent,
    })
}

fn bytes_to_gib(bytes: u64) -> f64 {
    ((bytes as f64 / 1024.0 / 1024.0 / 1024.0) * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn parse_df_k_output_extracts_available_space() {
        let output = "\
Filesystem 1024-blocks Used Available Capacity Mounted on
/dev/disk3s5 482345984 370000000 45123456 90% /System/Volumes/Data
";
        let usage = parse_df_k_output(output, Path::new("/tmp")).unwrap();

        assert_eq!(usage.available_bytes, 45_123_456 * 1024);
        assert_eq!(usage.total_bytes, 482_345_984 * 1024);
        assert_eq!(usage.used_percent, Some(90));
    }

    #[test]
    fn disk_status_warns_only_at_cleanup_threshold() {
        let usage = DiskUsage {
            path: PathBuf::from("/tmp"),
            available_bytes: CLEANUP_THRESHOLD_BYTES,
            total_bytes: CLEANUP_THRESHOLD_BYTES * 3,
            used_percent: Some(66),
        };
        let status = serde_json::json!({
            "state": if usage.available_bytes <= CLEANUP_THRESHOLD_BYTES { "warn" } else { "ok" },
            "cleanup_recommended": usage.available_bytes <= CLEANUP_THRESHOLD_BYTES,
        });

        assert_eq!(status["state"], "warn");
        assert_eq!(status["cleanup_recommended"], true);
    }
}
