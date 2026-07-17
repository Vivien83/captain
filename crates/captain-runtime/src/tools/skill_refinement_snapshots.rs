use std::path::{Path, PathBuf};

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    captain_types::durable_fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            captain_types::durable_fs::atomic_copy(&from, &to)?;
        }
    }
    Ok(())
}

pub(crate) fn create_skill_directory_snapshot(
    skill_path: &Path,
    skill: &str,
    refinement_id: &str,
    reason: &str,
) -> Result<serde_json::Value, String> {
    if !skill_path.exists() {
        return Err("skill directory is unavailable for snapshot".to_string());
    }
    if !skill_path.is_dir() {
        return Err("skill snapshot source is not a directory".to_string());
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let skill_part = safe_snapshot_component(skill);
    let id_part = safe_snapshot_component(&refinement_id.chars().take(8).collect::<String>());
    let reason_part = safe_snapshot_component(reason);
    let snapshot_id = format!("{ts}-{skill_part}-{id_part}-{reason_part}");
    let dest = skill_refinement_snapshot_root().join(&snapshot_id);
    copy_dir_recursive(skill_path, &dest).map_err(|e| format!("snapshot copy failed: {e}"))?;
    Ok(serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "kind": "directory",
        "reason": reason,
        "snapshot_id": snapshot_id,
    }))
}

pub(crate) fn recorded_snapshot_path(snapshot: &serde_json::Value) -> Result<PathBuf, String> {
    if let Some(snapshot_id) = snapshot.get("snapshot_id").and_then(|value| value.as_str()) {
        return snapshot_id_path(snapshot_id);
    }
    if let Some(path) = snapshot
        .get("snapshot_path")
        .and_then(|value| value.as_str())
    {
        return Ok(PathBuf::from(path));
    }
    Err("No usable snapshot recorded for this refinement".to_string())
}

pub(crate) fn trusted_skill_snapshot_path(path: &Path) -> bool {
    let Ok(root) = skill_refinement_snapshot_root().canonicalize() else {
        return false;
    };
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    path.starts_with(root)
}

fn snapshot_id_path(snapshot_id: &str) -> Result<PathBuf, String> {
    let snapshot_id = snapshot_id.trim();
    if snapshot_id.is_empty()
        || !snapshot_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("Snapshot id is invalid".to_string());
    }
    Ok(skill_refinement_snapshot_root().join(snapshot_id))
}

fn captain_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".captain")
}

fn skill_refinement_snapshot_root() -> PathBuf {
    captain_home_dir().join("skill-refinement-snapshots")
}

fn safe_snapshot_component(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    out.truncate(80);
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}
