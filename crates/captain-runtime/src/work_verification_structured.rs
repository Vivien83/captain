//! Fail-closed interpretation of structured evidence returned by native tools.
//!
//! A successful tool transport is not necessarily a successful subject: a
//! detached run can still be running, a delegation can be uncertain, and an
//! artifact response must carry its integrity marker. This adapter keeps those
//! protocol details out of the generic ordering policy.

use crate::work_verification::EvidenceStrength;

pub(crate) fn known_evidence_strength(
    tool_name: &str,
    input: &serde_json::Value,
    content: &str,
) -> Option<EvidenceStrength> {
    match tool_name {
        "tool_run_status" => Some(status_inspection_strength(
            content,
            &["completed"],
            &["running"],
        )),
        "tool_run_result" => Some(tool_run_result_strength(content)),
        "agent_job_status" => Some(status_inspection_strength(
            content,
            &["succeeded"],
            &["blocked", "queued", "running", "cancel_requested"],
        )),
        "agent_job_result" => Some(agent_job_result_strength(content)),
        "agent_delegate" if input["wait_for_result"].as_bool().unwrap_or(false) => {
            Some(agent_job_result_strength(content))
        }
        "agent_delegate" => Some(EvidenceStrength::Receipt),
        "artifact_publish" | "artifact_inspect" => Some(artifact_strength(content, true)),
        "artifact_deliver" => Some(artifact_strength(content, false)),
        _ => None,
    }
}

pub(crate) fn output_identity_values(tool_name: &str, content: &str) -> Vec<String> {
    if !matches!(
        tool_name,
        "tool_run_start"
            | "tool_run_status"
            | "tool_run_result"
            | "agent_delegate"
            | "agent_job_status"
            | "agent_job_result"
            | "artifact_publish"
            | "artifact_inspect"
            | "artifact_deliver"
    ) {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let mut identities = Vec::new();
    for pointer in [
        "/run_id",
        "/job_id",
        "/artifact_id",
        "/artifact/artifact_id",
        "/artifact/id",
    ] {
        if let Some(identity) = value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|identity| !identity.is_empty() && identity.len() <= 256)
        {
            identities.push(identity.to_string());
        }
    }
    identities.sort();
    identities.dedup();
    identities.truncate(8);
    identities
}

fn status_inspection_strength(
    content: &str,
    successful_terminal: &[&str],
    active: &[&str],
) -> EvidenceStrength {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return EvidenceStrength::None;
    };
    let Some(status) = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(|status| status.trim().to_ascii_lowercase())
    else {
        return EvidenceStrength::None;
    };
    if successful_terminal.contains(&status.as_str()) || active.contains(&status.as_str()) {
        EvidenceStrength::Inspection
    } else {
        EvidenceStrength::None
    }
}

fn tool_run_result_strength(content: &str) -> EvidenceStrength {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return EvidenceStrength::None;
    };
    match status(&value) {
        Some("running") => EvidenceStrength::Inspection,
        Some("completed")
            if value.get("is_error").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("result").is_some_and(|result| !result.is_null()) =>
        {
            EvidenceStrength::Check
        }
        _ => EvidenceStrength::None,
    }
}

fn agent_job_result_strength(content: &str) -> EvidenceStrength {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return EvidenceStrength::None;
    };
    match status(&value) {
        Some("blocked" | "queued" | "running" | "cancel_requested") => EvidenceStrength::Inspection,
        Some("succeeded")
            if value
                .get("result_available")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && value.get("result").is_some_and(|result| !result.is_null()) =>
        {
            EvidenceStrength::Check
        }
        _ => EvidenceStrength::None,
    }
}

fn status(value: &serde_json::Value) -> Option<&str> {
    value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
}

fn artifact_strength(content: &str, integrity_required: bool) -> EvidenceStrength {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return EvidenceStrength::None;
    };
    if value.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
        return EvidenceStrength::None;
    }
    if integrity_required
        && value.get("integrity").and_then(serde_json::Value::as_str) != Some("sha256_verified")
    {
        return EvidenceStrength::None;
    }
    if integrity_required && !has_verified_artifact_identity(&value) {
        return EvidenceStrength::None;
    }
    if integrity_required {
        EvidenceStrength::Check
    } else {
        EvidenceStrength::Receipt
    }
}

fn has_verified_artifact_identity(value: &serde_json::Value) -> bool {
    let Some(artifact) = value.get("artifact") else {
        return false;
    };
    let id_valid = ["artifact_id", "id"].iter().any(|key| {
        artifact
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| !id.trim().is_empty() && id.len() <= 256)
    });
    let sha_valid = artifact
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    id_valid && sha_valid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strength(tool_name: &str, content: &str) -> Option<EvidenceStrength> {
        known_evidence_strength(tool_name, &serde_json::Value::Null, content)
    }

    #[test]
    fn live_run_requires_a_successful_terminal_state_for_check_evidence() {
        assert_eq!(
            strength("tool_run_status", r#"{"status":"running"}"#),
            Some(EvidenceStrength::Inspection)
        );
        assert_eq!(
            strength(
                "tool_run_result",
                r#"{"status":"completed","is_error":false,"result":"ok"}"#
            ),
            Some(EvidenceStrength::Check)
        );
        assert_eq!(
            strength("tool_run_status", r#"{"status":"completed"}"#),
            Some(EvidenceStrength::Inspection)
        );
        assert_eq!(
            strength("tool_run_status", r#"{"status":"interrupted"}"#),
            Some(EvidenceStrength::None)
        );
    }

    #[test]
    fn delegation_requires_a_successful_terminal_state_for_check_evidence() {
        assert_eq!(
            strength("agent_job_status", r#"{"status":"queued"}"#),
            Some(EvidenceStrength::Inspection)
        );
        assert_eq!(
            strength(
                "agent_job_result",
                r#"{"status":"succeeded","result_available":true,"result":"done"}"#
            ),
            Some(EvidenceStrength::Check)
        );
        assert_eq!(
            strength("agent_job_status", r#"{"status":"succeeded"}"#),
            Some(EvidenceStrength::Inspection)
        );
        assert_eq!(
            strength("agent_job_result", r#"{"status":"uncertain"}"#),
            Some(EvidenceStrength::None)
        );
    }

    #[test]
    fn delegation_wait_mode_distinguishes_launch_from_result() {
        let running = r#"{"job_id":"job-1","status":"running"}"#;
        assert_eq!(
            known_evidence_strength(
                "agent_delegate",
                &serde_json::json!({"wait_for_result": false}),
                running,
            ),
            Some(EvidenceStrength::Receipt)
        );
        assert_eq!(
            known_evidence_strength(
                "agent_delegate",
                &serde_json::json!({"wait_for_result": true}),
                running,
            ),
            Some(EvidenceStrength::Inspection)
        );
    }

    #[test]
    fn artifact_evidence_fails_closed_without_integrity() {
        let verified = serde_json::json!({
            "success": true,
            "integrity": "sha256_verified",
            "artifact": {
                "artifact_id": "artifact-1",
                "sha256": "a".repeat(64),
            }
        })
        .to_string();
        assert_eq!(
            strength("artifact_inspect", &verified),
            Some(EvidenceStrength::Check)
        );
        assert_eq!(
            strength(
                "artifact_inspect",
                r#"{"success":true,"integrity":"sha256_verified"}"#
            ),
            Some(EvidenceStrength::None)
        );
        assert_eq!(
            strength("artifact_publish", r#"{"success":true}"#),
            Some(EvidenceStrength::None)
        );
    }

    #[test]
    fn only_bounded_known_subject_ids_are_extracted() {
        assert_eq!(
            output_identity_values(
                "agent_job_result",
                r#"{"job_id":"job-7","result":"private content"}"#
            ),
            vec!["job-7"]
        );
        assert!(
            output_identity_values("web_fetch", r#"{"run_id":"must-not-become-a-scope"}"#)
                .is_empty()
        );
    }
}
