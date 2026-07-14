//! Shared output formatting helpers for tool responses.

pub(crate) fn truncate_owned(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        text.to_string()
    } else {
        format!(
            "{}… [truncated {} bytes]",
            captain_types::truncate_str(text, max_bytes),
            text.len().saturating_sub(max_bytes)
        )
    }
}
