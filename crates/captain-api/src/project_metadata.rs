use crate::project_lifecycle::lifecycle_json;

pub(crate) fn project_source_from_metadata(metadata: &serde_json::Value) -> serde_json::Value {
    metadata
        .pointer("/launch/source")
        .cloned()
        .or_else(|| metadata.get("source").cloned())
        .unwrap_or_else(|| serde_json::json!({ "type": "legacy" }))
}

pub(crate) fn project_workspace_from_metadata(metadata: &serde_json::Value) -> serde_json::Value {
    metadata
        .pointer("/launch/workspace")
        .cloned()
        .or_else(|| metadata.get("workspace").cloned())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub(crate) fn project_metadata(
    launch: Option<serde_json::Value>,
    phase: &str,
) -> serde_json::Value {
    let lifecycle = lifecycle_json(phase);
    match launch {
        Some(mut launch) => {
            if let Some(obj) = launch.as_object_mut() {
                obj.insert("lifecycle".to_string(), lifecycle.clone());
            }
            serde_json::json!({
                "launch": launch,
                "lifecycle": lifecycle,
                "product_target": "autonomous_development_project",
            })
        }
        None => serde_json::json!({
            "lifecycle": lifecycle,
            "product_target": "autonomous_development_project",
        }),
    }
}

pub(crate) fn metadata_set_runtime(metadata: &mut serde_json::Value, runtime: serde_json::Value) {
    if !metadata.is_object() {
        *metadata = serde_json::json!({});
    }
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("runtime".to_string(), runtime);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_and_workspace_prefer_launch_metadata_then_legacy() {
        let metadata = json!({
            "source": {"type": "legacy-source"},
            "workspace": {"mode": "legacy-workspace"},
            "launch": {
                "source": {"type": "github"},
                "workspace": {"mode": "prepared"}
            }
        });
        let legacy = json!({
            "source": {"type": "local"},
            "workspace": {"mode": "manual"}
        });

        assert_eq!(
            project_source_from_metadata(&metadata)["type"],
            json!("github")
        );
        assert_eq!(
            project_workspace_from_metadata(&metadata)["mode"],
            json!("prepared")
        );
        assert_eq!(
            project_source_from_metadata(&legacy)["type"],
            json!("local")
        );
        assert_eq!(
            project_workspace_from_metadata(&legacy)["mode"],
            json!("manual")
        );
        assert_eq!(
            project_source_from_metadata(&json!({}))["type"],
            json!("legacy")
        );
        assert!(project_workspace_from_metadata(&json!({})).is_object());
    }

    #[test]
    fn project_metadata_injects_lifecycle_into_launch_and_top_level() {
        let metadata = project_metadata(Some(json!({"source": {"type": "local"}})), "build");

        assert_eq!(metadata["product_target"], "autonomous_development_project");
        assert_eq!(metadata["lifecycle"]["current_phase"], json!("build"));
        assert_eq!(
            metadata["launch"]["lifecycle"]["current_phase"],
            json!("build")
        );
        assert_eq!(metadata["launch"]["source"]["type"], json!("local"));
    }

    #[test]
    fn project_metadata_without_launch_still_has_lifecycle() {
        let metadata = project_metadata(None, "observe");

        assert_eq!(metadata["product_target"], "autonomous_development_project");
        assert_eq!(metadata["lifecycle"]["current_phase"], json!("observe"));
        assert!(metadata.get("launch").is_none());
    }

    #[test]
    fn metadata_set_runtime_preserves_object_fields() {
        let mut metadata = json!({
            "lifecycle": {"current_phase": "build"},
            "product_target": "autonomous_development_project"
        });

        metadata_set_runtime(&mut metadata, json!({"status": "running"}));

        assert_eq!(metadata["lifecycle"]["current_phase"], json!("build"));
        assert_eq!(metadata["runtime"]["status"], json!("running"));
    }

    #[test]
    fn metadata_set_runtime_replaces_non_object_metadata() {
        let mut metadata = json!("legacy-corrupt-metadata");

        metadata_set_runtime(&mut metadata, json!({"status": "ready"}));

        assert_eq!(metadata, json!({"runtime": {"status": "ready"}}));
    }
}
