use super::improvement_common::public_safe_json_value;
use serde_json::{Map, Value};

pub(crate) fn refinement_for_output(item: &Value) -> Value {
    let mut output = item.clone();
    let Some(object) = output.as_object_mut() else {
        return public_safe_json_value(output, "skill_refinement_output");
    };

    sanitize_snapshot(object, "snapshot");
    sanitize_snapshot(object, "restore_backup");
    if let Some(error) = object.remove("snapshot_error") {
        let error = error.as_str().unwrap_or_default();
        object.insert(
            "snapshot".to_string(),
            snapshot_unavailable(public_snapshot_error(error)),
        );
    }
    public_safe_json_value(output, "skill_refinement_output")
}

pub(crate) fn refinements_for_output(items: Vec<Value>) -> Vec<Value> {
    items.iter().map(refinement_for_output).collect::<Vec<_>>()
}

fn sanitize_snapshot(object: &mut Map<String, Value>, key: &str) {
    let Some(snapshot) = object.get(key).and_then(snapshot_summary) else {
        return;
    };
    object.insert(key.to_string(), snapshot);
}

fn snapshot_summary(snapshot: &Value) -> Option<Value> {
    let object = snapshot.as_object()?;
    let mut output = Map::new();
    output.insert("available".to_string(), Value::Bool(true));
    for key in ["created_at", "kind", "reason"] {
        if let Some(value) = object.get(key).filter(|value| value.is_string()) {
            output.insert(key.to_string(), value.clone());
        }
    }
    Some(Value::Object(output))
}

fn snapshot_unavailable(error: &'static str) -> Value {
    let mut output = Map::new();
    output.insert("available".to_string(), Value::Bool(false));
    output.insert("error".to_string(), Value::String(error.to_string()));
    Value::Object(output)
}

fn public_snapshot_error(error: &str) -> &'static str {
    if error.contains("registry unavailable") {
        "skill registry unavailable"
    } else if error.contains("not found in registry") {
        "skill not found in registry"
    } else if error.contains("not file-backed") || error.contains("Bundled skills") {
        "skill is not file-backed"
    } else {
        "skill snapshot unavailable"
    }
}
