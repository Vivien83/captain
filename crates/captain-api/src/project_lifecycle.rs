use crate::project_update_input::PROJECT_LIFECYCLE_PHASES;

pub(crate) fn runtime_progress_for_phase(phase: &str, status: &str) -> u64 {
    if status == "done" {
        return 100;
    }
    if status == "paused" {
        return match phase {
            "observe" => 8,
            "think" => 18,
            "plan" => 32,
            "build" => 52,
            "execute" => 70,
            "verify" => 86,
            "learn" => 96,
            _ => 0,
        };
    }
    match phase {
        "observe" => 10,
        "think" => 22,
        "plan" => 36,
        "build" => 56,
        "execute" => 74,
        "verify" => 88,
        "learn" => 98,
        _ => 0,
    }
}

pub(crate) fn lifecycle_json(phase: &str) -> serde_json::Value {
    serde_json::json!({
        "protocol": "captain.project_lifecycle.v1",
        "required": true,
        "current_phase": phase,
        "phases": PROJECT_LIFECYCLE_PHASES,
    })
}

pub(crate) fn lifecycle_from_metadata(metadata: &serde_json::Value) -> serde_json::Value {
    metadata
        .get("lifecycle")
        .cloned()
        .or_else(|| metadata.pointer("/launch/lifecycle").cloned())
        .unwrap_or_else(|| lifecycle_json("observe"))
}

pub(crate) fn set_lifecycle_phase(
    mut metadata: serde_json::Value,
    phase: &str,
) -> serde_json::Value {
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    let lifecycle = lifecycle_json(phase);
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("lifecycle".to_string(), lifecycle.clone());
        if let Some(launch) = obj.get_mut("launch").and_then(|v| v.as_object_mut()) {
            launch.insert("lifecycle".to_string(), lifecycle);
        }
    }
    metadata
}

pub(crate) fn is_valid_lifecycle_phase(phase: &str) -> bool {
    PROJECT_LIFECYCLE_PHASES.contains(&phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_progress_preserves_running_paused_and_done_values() {
        assert_eq!(runtime_progress_for_phase("observe", "running"), 10);
        assert_eq!(runtime_progress_for_phase("build", "running"), 56);
        assert_eq!(runtime_progress_for_phase("build", "paused"), 52);
        assert_eq!(runtime_progress_for_phase("learn", "done"), 100);
        assert_eq!(runtime_progress_for_phase("unknown", "running"), 0);
    }

    #[test]
    fn lifecycle_from_metadata_prefers_direct_then_launch_then_default() {
        let direct = json!({
            "lifecycle": {"current_phase": "verify"},
            "launch": {"lifecycle": {"current_phase": "build"}}
        });
        let launch_only = json!({
            "launch": {"lifecycle": {"current_phase": "plan"}}
        });

        assert_eq!(
            lifecycle_from_metadata(&direct)["current_phase"],
            json!("verify")
        );
        assert_eq!(
            lifecycle_from_metadata(&launch_only)["current_phase"],
            json!("plan")
        );
        assert_eq!(
            lifecycle_from_metadata(&json!({}))["current_phase"],
            json!("observe")
        );
    }

    #[test]
    fn set_lifecycle_phase_updates_top_level_and_launch_copy() {
        let metadata = set_lifecycle_phase(json!({"launch": {}}), "execute");

        assert_eq!(metadata["lifecycle"]["current_phase"], json!("execute"));
        assert_eq!(
            metadata["launch"]["lifecycle"]["current_phase"],
            json!("execute")
        );
        assert!(is_valid_lifecycle_phase("verify"));
        assert!(!is_valid_lifecycle_phase("invalid"));
    }
}
