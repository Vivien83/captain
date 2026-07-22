//! Bounded validation shared by workflow-learning SQLite stores.

use crate::workflow_learning_types::WorkflowLearningControlError;

pub(crate) fn validate_token(
    label: &str,
    value: &str,
    max_len: usize,
) -> Result<(), WorkflowLearningControlError> {
    let valid = !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "{label} is not a bounded safe token"
        )))
    }
}

pub(crate) fn validate_hash(label: &str, value: &str) -> Result<(), WorkflowLearningControlError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "{label} must be a 64-character hex digest"
        )))
    }
}

pub(crate) fn validate_json(
    label: &str,
    value: &str,
    max_len: usize,
) -> Result<(), WorkflowLearningControlError> {
    validate_text(label, value, 2, max_len)?;
    serde_json::from_str::<serde_json::Value>(value).map_err(|error| {
        WorkflowLearningControlError::InvalidInput(format!("{label} is invalid JSON: {error}"))
    })?;
    Ok(())
}

pub(crate) fn validate_text(
    label: &str,
    value: &str,
    min_len: usize,
    max_len: usize,
) -> Result<(), WorkflowLearningControlError> {
    if value.len() >= min_len && value.len() <= max_len && !value.contains('\0') {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "{label} must contain {min_len}..={max_len} safe bytes"
        )))
    }
}
